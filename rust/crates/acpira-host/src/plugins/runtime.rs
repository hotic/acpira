//! Resident plugin processes (`contributes.runtime`, docs/dev/plugins.md, Runtime): one process per plugin in effect,
//! spoken to over ndjson JSON-RPC on its stdio (the `acpira-rpc` peer ACP runs on). The engine calls only the hooks the
//! manifest declared and sends only the events it subscribed to, so a plugin costs nothing where it did not ask in.
//!
//! - A new version is started and initialized before the old process is stopped, so a hook call never finds no one.
//! - Each call has a time limit; repeated failures pause the plugin's calls for a while (the circuit opens), and the
//!   page shows why.
//! - Events go through a bounded queue per plugin; a full queue drops and counts, the engine never waits on a plugin.
//! - A process that exits is started again after a growing pause, while the plugin stays in effect.
//! - Each plugin has a small key-value store of its own beside its data folder.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use acpira_rpc::rpc::{BoxFuture, Connection, RpcError};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::plugins::host::Active;
use crate::plugins::manifest::RuntimeDef;
use crate::store::transcript_store::LogFn;

mod inbound;
mod kv;
mod process;

use self::kv::Kv;

/// The protocol a process and the engine speak; `initialize` refuses any other
pub const PROTOCOL_VERSION: u64 = 1;
/// Longest a process may take to answer `initialize`
pub const INIT_TIMEOUT: Duration = Duration::from_secs(10);
/// After `shutdown`, how long a process has to exit before it is killed
const STOP_GRACE: Duration = Duration::from_secs(2);
/// A call the user started with a click and waits on its button for (a draft command, a page's method call): it may
/// take a model call's time (`model/generate` gives up after 120 s), not a hook's few seconds on the prompt's path
pub const CLICK_TIMEOUT: Duration = Duration::from_secs(150);
/// Failed calls in a row that open the circuit, and how long it stays open
const TRIP_AFTER: u32 = 3;
const TRIP_FOR: Duration = Duration::from_secs(60);

/// The engine's side of a call a process makes beyond the store (`session/*`, `ui/*`, …): plugin id, method, params
/// A page of this engine taking a message from a plugin's process
pub type PageFn = Arc<dyn Fn(acpira_shared::protocol::HostMsg) + Send + Sync>;

pub type HostCall = Arc<dyn Fn(String, String, Value) -> BoxFuture<Result<Value, RpcError>> + Send + Sync>;

/// One plugin to keep running: its frozen copy and what it declared
#[derive(Clone)]
struct Want {
  id: String,
  /// The name the user knows it by (`Manifest::display_name`)
  name: String,
  version: String,
  hash: String,
  dir: PathBuf,
  def: RuntimeDef,
  /// Tools of the manifest without a `command`: the process answers them (`tool/<name>`)
  tools: Vec<crate::plugins::manifest::ToolDef>,
  schedules: Vec<crate::plugins::manifest::ScheduleDef>,
  /// `dependencies` then `optionalDependencies`: the only plugins it may call
  dependencies: BTreeMap<String, String>,
  optional: BTreeMap<String, String>,
  /// A `runtime.main` plugin whose commands have no prompt answers the `command` hook without listing it
  commands_hooked: bool,
  menus_hooked: bool,
}

impl Want {
  fn of(id: &str, hash: &str, dir: &Path, m: &crate::plugins::manifest::Manifest) -> Option<Want> {
    let def = m.contributes.runtime.clone()?;
    let commands_hooked = def.main.is_some() && m.contributes.commands.iter().any(|c| c.prompt.as_deref().is_none_or(|p| p.trim().is_empty()));
    // Likewise a command button's menu, answered by `menus` of the definition
    let menus_hooked = def.main.is_some() && m.contributes.commands.iter().any(|c| !c.menu.is_empty());
    Some(Want {
      id: id.to_owned(),
      name: m.display_name().to_owned(),
      version: m.version.clone(),
      hash: hash.to_owned(),
      dir: dir.to_path_buf(),
      def,
      tools: m.contributes.tools.iter().filter(|t| t.command.is_empty()).cloned().collect(),
      schedules: m.contributes.schedules.clone(),
      dependencies: m.dependencies.clone(),
      optional: m.optional_dependencies.clone(),
      commands_hooked,
      menus_hooked,
    })
  }

  /// The hooks the user confirmed: the manifest's list, plus `command` when its commands are answered in code
  fn hooks(&self) -> Vec<String> {
    let mut hooks = self.def.hooks.clone();
    if self.commands_hooked && !hooks.iter().any(|h| h == "command") {
      hooks.push("command".into());
    }
    if self.menus_hooked && !hooks.iter().any(|h| h == "menu") {
      hooks.push("menu".into());
    }
    hooks
  }

  fn may_call(&self, plugin: &str) -> bool {
    plugin == self.id || self.dependencies.contains_key(plugin) || self.optional.contains_key(plugin)
  }
}

struct Proc {
  child: parking_lot::Mutex<Option<tokio::process::Child>>,
  #[cfg_attr(windows, expect(dead_code, reason = "kills the process group on Unix; the job object ends the tree on Windows"))]
  pid: Option<u32>,
  #[cfg(windows)]
  job: Arc<crate::platform::windows_process::Job>,
}

impl Proc {
  /// `shutdown` was sent: wait out the grace period, then kill what is left (the whole group on Unix)
  async fn stop(&self) {
    let child = self.child.lock().take();
    let Some(mut child) = child else { return };
    if tokio::time::timeout(STOP_GRACE, child.wait()).await.is_ok() {
      return;
    }
    #[cfg(unix)]
    if let Some(pid) = self.pid {
      // SAFETY: kill(2) on the process group our own unreaped child leads (spawned with process_group(0))
      unsafe {
        libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
      }
    }
    #[cfg(windows)]
    self.job.terminate();
    let _ = child.start_kill();
    let _ = child.wait().await;
  }
}

/// A running process
struct Worker {
  want: Want,
  conn: Connection,
  proc: Proc,
  /// The hooks it answers: what the manifest declared, narrowed by its `initialize` answer
  hooks: Vec<String>,
  failures: AtomicU32,
  tripped: parking_lot::Mutex<Option<Instant>>,
  events: mpsc::Sender<Value>,
  dropped: Arc<AtomicU64>,
  /// Set when the engine stops it on purpose, so its exit is not taken for a crash
  stopping: AtomicBool,
  /// What `initialize` told it of its dependencies: a change restarts it, so `ctx.plugins` stays true
  deps: Value,
}

impl Worker {
  fn answers(&self, hook: &str) -> bool {
    self.hooks.iter().any(|h| h == hook)
  }

  /// Asked about this payload: a plugin scoped to its own agent only in that agent's sessions
  fn serves(&self, payload: &Value) -> bool {
    self.want.def.scope.as_deref() != Some(crate::plugins::manifest::SCOPE_AGENT) || payload.get("agent").and_then(Value::as_str) == Some(self.want.id.as_str())
  }

  async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
    self.call_for(method, params, self.want.def.timeout()).await
  }

  /// A call with its own time limit (a tool's, a schedule's)
  async fn call_for(&self, method: &str, params: Value, limit: Duration) -> Result<Value, String> {
    if self.tripped.lock().is_some_and(|at| at.elapsed() < TRIP_FOR) {
      return Err(format!("plugin {} is paused after {TRIP_AFTER} failed calls in a row", self.want.id));
    }
    let r = match tokio::time::timeout(limit, self.conn.request_cancellable(method, params)).await {
      Ok(Ok(v)) => Ok(v),
      Ok(Err(e)) => Err(e.message),
      Err(_) => Err(format!("timed out after {} s", limit.as_secs_f64())),
    };
    match &r {
      Ok(_) => {
        self.failures.store(0, Ordering::Relaxed);
        *self.tripped.lock() = None;
      }
      Err(_) => {
        if self.failures.fetch_add(1, Ordering::Relaxed) + 1 >= TRIP_AFTER {
          self.failures.store(0, Ordering::Relaxed);
          *self.tripped.lock() = Some(Instant::now());
        }
      }
    }
    r
  }

  async fn stop(&self) {
    self.stopping.store(true, Ordering::Relaxed);
    self.conn.notify("shutdown", json!({}));
    self.proc.stop().await;
    self.conn.close();
  }
}

/// What the plugins in effect offer every session besides their processes: slash commands and skill folders. Kept on
/// `Runtimes`, which sessions already hold, and replaced on every load (`PluginHost::reload`)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Contributed {
  /// In plugin id order; a name taken by an earlier plugin is skipped
  pub commands: Vec<PluginCommand>,
  /// The plugins' `skills` folders, in plugin id order
  pub skills: Vec<PathBuf>,
  /// `contributes.providers`, in plugin id order; an id taken by an earlier plugin is skipped
  pub providers: Vec<acpira_shared::providers::Provider>,
}

pub use crate::acp::session::ext::PluginCommand;

impl Contributed {
  pub fn of(active: &[Arc<Active>]) -> Contributed {
    let mut out = Contributed::default();
    for a in active {
      for c in &a.manifest.contributes.commands {
        if !out.commands.iter().any(|x| x.def.name == c.name) {
          out.commands.push(PluginCommand { plugin: a.id.clone(), def: c.clone() });
        }
      }
      for p in a.manifest.contributes.providers.iter().filter_map(|raw| crate::plugins::manifest::provider_of(raw, &a.id).ok()) {
        if !out.providers.iter().any(|x| x.id == p.id) {
          out.providers.push(p);
        }
      }
    }
    // `Places::skill_roots` lets later layers override earlier ones, so feed skills in reverse load order so earlier plugins win
    for a in active.iter().rev() {
      if let Some(dir) = a.manifest.contributes.skills.as_deref().and_then(|p| crate::plugins::manifest::inside(&a.dir, p)).filter(|d| d.is_dir()) {
        out.skills.push(dir);
      }
    }
    out
  }
}

/// One answer of a hook call
#[derive(Debug, Clone)]
pub struct HookReply {
  pub plugin: String,
  pub result: Result<Value, String>,
}

/// The runtime method a plugin answers to let go of an account it lists
pub const ACCOUNT_REMOVE: &str = "account.remove";

pub struct Runtimes {
  /// By id; dispatch follows `order` (then id order for any id not listed)
  workers: parking_lot::RwLock<BTreeMap<String, Arc<Worker>>>,
  /// Active plugin load order; updated on reload without restarting running processes
  order: parking_lot::RwLock<Vec<String>>,
  /// By plugin: the version whose `accounts/set` list is in effect
  account_lists: parking_lot::Mutex<HashMap<String, String>>,
  /// What should be running, kept for restarts
  wanted: parking_lot::Mutex<Vec<Want>>,
  /// Why a plugin's process is not running or paused, for the plugin page
  errors: parking_lot::RwLock<BTreeMap<String, String>>,
  restarts: parking_lot::Mutex<HashMap<String, (u32, Instant)>>,
  host_call: parking_lot::RwLock<Option<HostCall>>,
  contributed: parking_lot::RwLock<Arc<Contributed>>,
  /// Model sources a process set with `providers/set`, by plugin; dropped when the plugin leaves
  live_providers: parking_lot::RwLock<BTreeMap<String, Vec<acpira_shared::providers::Provider>>>,
  /// Where `accounts/set` lands: the engine's account layer, once the composition root hands it over
  accounts: parking_lot::RwLock<Option<Arc<crate::accounts::account_manager::AccountManager>>>,
  /// The pages that hear what a process sends them (`ui/post`, `ui/composer`): every bridge of this engine
  pages: parking_lot::RwLock<Vec<(u64, PageFn)>>,
  kvs: parking_lot::Mutex<HashMap<String, Arc<Kv>>>,
  next_page: std::sync::atomic::AtomicU64,
  /// One writer of `plugins/providers.json` at a time: a load and a process's `providers/set` may race
  providers_write: parking_lot::Mutex<()>,
  /// Told when `contributed` changed (the session manager republishes the open sessions, whose menus list the commands)
  on_contributed: parking_lot::RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
  /// Where a `prompt` hook's redirect starts its session (the session manager registers itself)
  redirect: parking_lot::RwLock<Option<crate::acp::session::plugin_hooks::RedirectFn>>,
  /// Off until the composition root turns it on: `acpira plugin list` and the tests of other parts start no process
  enabled: AtomicBool,
  /// The composition root promised `host_call` (`enable_before_host`): early calls wait for it
  host_coming: AtomicBool,
  sync: tokio::sync::Mutex<()>,
  /// Wakes callers waiting for an enabled plugin's initial start to finish (`wait_plugin_settled`)
  plugin_settled: tokio::sync::Notify,
  data_root: PathBuf,
  /// The plugins in effect with their versions, whatever they contribute: what a dependency's `present` says
  present: parking_lot::RwLock<BTreeMap<String, String>>,
  log: LogFn,
}

impl Runtimes {
  /// `data_root` is the plugins' data folder (`~/.acpira/plugins/.data`)
  pub fn new(data_root: PathBuf, log: LogFn) -> Arc<Runtimes> {
    Arc::new(Runtimes {
      workers: Default::default(),
      order: Default::default(),
      account_lists: Default::default(),
      wanted: Default::default(),
      errors: Default::default(),
      restarts: Default::default(),
      host_call: Default::default(),
      redirect: Default::default(),
      contributed: Default::default(),
      live_providers: Default::default(),
      accounts: Default::default(),
      providers_write: Default::default(),
      pages: Default::default(),
      kvs: Default::default(),
      next_page: Default::default(),
      on_contributed: Default::default(),
      enabled: AtomicBool::new(false),
      host_coming: AtomicBool::new(false),
      sync: Default::default(),
      plugin_settled: tokio::sync::Notify::new(),
      data_root,
      present: Default::default(),
      log,
    })
  }

  /// Let processes run; `host_call` serves what a process asks of the engine beyond its store
  pub fn enable(&self, host_call: Option<HostCall>) {
    *self.host_call.write() = host_call;
    self.host_coming.store(false, Ordering::Relaxed);
    self.enabled.store(true, Ordering::Relaxed);
  }

  /// Let processes run before the session API exists; `enable` with it follows, and calls made meanwhile wait for it
  pub fn enable_before_host(&self) {
    self.host_coming.store(true, Ordering::Relaxed);
    self.enabled.store(true, Ordering::Relaxed);
  }

  /// The folder of the skills the engine ships (`sdk::CREATOR_SKILL`), for the built-in agent in safe mode too. Other
  /// agents bring their own, so they are not sent it
  pub fn shipped_skills(&self) -> Option<PathBuf> {
    let dir = self.data_root.parent()?.join(crate::plugins::sdk::SDK_DIR).join(crate::plugins::sdk::SKILLS_DIR);
    dir.is_dir().then_some(dir)
  }

  pub fn contributed(&self) -> Arc<Contributed> {
    self.contributed.read().clone()
  }

  pub fn set_contributed(&self, next: Contributed) {
    // Written even when nothing changed: the file may still hold what an earlier run left
    let same = **self.contributed.read() == next;
    *self.contributed.write() = Arc::new(next);
    self.write_providers();
    if same {
      return;
    }
    if let Some(f) = self.on_contributed.read().clone() {
      f();
    }
  }

  /// Replace what `plugin`'s process contributes as model sources (`providers/set`); every source is checked the way
  /// the manifest's are
  pub fn set_live_providers(&self, plugin: &str, raw: &[Value]) -> Result<usize, String> {
    let list = raw.iter().map(|r| crate::plugins::manifest::provider_of(r, plugin)).collect::<Result<Vec<_>, _>>()?;
    let n = list.len();
    // A process of a plugin that is no longer (or not yet) in effect, its old version or a check's, publishes nothing
    let wanted = self.wanted.lock();
    if !wanted.iter().any(|w| w.id == plugin) {
      return Err(format!("plugin {plugin} is not in effect"));
    }
    self.live_providers.write().insert(plugin.to_owned(), list);
    drop(wanted);
    self.write_providers();
    Ok(n)
  }

  /// The account layer `accounts/set` reaches. A plugin account the user removes goes back to its plugin's
  /// `account.remove` (`{ agent, id }`, the plugin's own id), which lets go of it
  pub fn set_accounts(self: &Arc<Self>, accounts: Arc<crate::accounts::account_manager::AccountManager>) {
    let me = Arc::downgrade(self);
    accounts.set_plugin_remover(Arc::new(move |plugin: String, id: String, agent: String| {
      let me = me.clone();
      Box::pin(async move {
        let rt = me.upgrade().ok_or_else(|| anyhow::anyhow!("the engine is closing"))?;
        rt.rpc(&plugin, ACCOUNT_REMOVE, json!({ "agent": agent, "id": id }), "acpira").await.map(|_| ()).map_err(anyhow::Error::msg)
      })
    }));
    let wait_me = Arc::downgrade(self);
    accounts.set_plugin_waiter(Arc::new(move |plugin: String| {
      let me = wait_me.clone();
      Box::pin(async move {
        if let Some(rt) = me.upgrade() {
          rt.wait_plugin_settled(&plugin).await;
        }
      })
    }));
    *self.accounts.write() = Some(accounts);
  }

  pub fn has_accounts(&self) -> bool {
    self.accounts.read().is_some()
  }

  /// Replace the accounts `plugin`'s process of version `hash` lists (`accounts/set`). Only the version in effect
  /// lists: a process being replaced cannot overwrite what its successor said
  pub fn set_live_accounts(&self, plugin: &str, hash: &str, raw: &[Value], removable: bool) -> Result<usize, String> {
    let accounts = self.accounts.read().clone().ok_or("this engine keeps no accounts")?;
    // Held through the change, as for model sources: a plugin leaving meanwhile cannot have its accounts land after
    // its departure dropped them
    let wanted = self.wanted.lock();
    let Some(want) = wanted.iter().find(|w| w.id == plugin) else {
      return Err(format!("plugin {plugin} is not in effect"));
    };
    if want.hash != hash {
      return Err(format!("plugin {plugin}: this version is being replaced"));
    }
    let n = accounts.set_plugin_accounts(plugin, &want.name, raw, removable)?;
    self.account_lists.lock().insert(plugin.to_owned(), hash.to_owned());
    Ok(n)
  }

  /// Drop what `plugin` listed: its process exited, or a new version runs that has not listed its own yet. Nothing a
  /// plugin lists outlives the process that vouches for it
  fn drop_live_accounts(&self, plugin: &str, unless_from: Option<&str>) {
    let mut lists = self.account_lists.lock();
    if lists.get(plugin).is_none_or(|h| unless_from.is_some_and(|keep| h == keep)) {
      return;
    }
    lists.remove(plugin);
    drop(lists);
    if let Some(accounts) = self.accounts.read().clone() {
      let _ = accounts.set_plugin_accounts(plugin, "", &[], false);
    }
  }

  /// `plugins/providers.json` beside the plugin folders: the manifests' sources, then the processes', first id wins.
  /// Only an engine writes it, and only when its content changes, so the agent's stamp check stays quiet
  /// Every model source the plugins in effect contribute: the manifests' first, then what their processes published
  /// (`providers/set`), an id taken earlier skipped. What `write_providers` hands the built-in agent
  pub fn sources(&self) -> Vec<acpira_shared::providers::Provider> {
    let mut providers = self.contributed.read().providers.clone();
    let wanted: Vec<String> = self.wanted.lock().iter().map(|w| w.id.clone()).collect();
    for p in self.live_providers.read().iter().filter(|(id, _)| wanted.contains(id)).flat_map(|(_, list)| list) {
      if !providers.iter().any(|x| x.id == p.id) {
        providers.push(p.clone());
      }
    }
    providers
  }

  fn write_providers(&self) {
    if !self.enabled() {
      return;
    }
    let Some(dir) = self.data_root.parent() else { return };
    let _one = self.providers_write.lock();
    let providers = self.sources();
    let path = dir.join(acpira_shared::providers::PROVIDERS_FILE);
    let file = acpira_shared::providers::ProvidersFile { version: acpira_shared::providers::PROVIDERS_VERSION, providers, ..Default::default() };
    let Ok(text) = serde_json::to_string_pretty(&file) else { return };
    let current = std::fs::read_to_string(&path).ok();
    if current.as_deref() == Some(text.as_str()) || (current.is_none() && file.providers.is_empty()) {
      return;
    }
    // tmp + rename, so an agent reading at the same moment sees the old file or the new one
    let tmp = dir.join(format!(".{}.{}.tmp", acpira_shared::providers::PROVIDERS_FILE, std::process::id()));
    if let Err(e) = std::fs::create_dir_all(dir).and_then(|()| std::fs::write(&tmp, &text)).and_then(|()| std::fs::rename(&tmp, &path)) {
      let _ = std::fs::remove_file(&tmp);
      (self.log)(&format!("plugins: {}: {e}", path.display()));
    }
  }

  pub fn on_contributed(&self, f: Arc<dyn Fn() + Send + Sync>) {
    *self.on_contributed.write() = Some(f);
  }

  /// Ask `plugin`'s process alone (a command it contributed); None when it does not answer `hook`
  pub async fn hook_of(&self, plugin: &str, hook: &str, payload: Value) -> Option<Result<Value, String>> {
    self.hook_within(plugin, hook, payload, None).await
  }

  /// `hook_of` for a hook a click started (a draft command's button), given `CLICK_TIMEOUT`
  pub async fn hook_clicked(&self, plugin: &str, hook: &str, payload: Value) -> Option<Result<Value, String>> {
    self.hook_within(plugin, hook, payload, Some(CLICK_TIMEOUT)).await
  }

  async fn hook_within(&self, plugin: &str, hook: &str, payload: Value, limit: Option<Duration>) -> Option<Result<Value, String>> {
    let w = self.worker_of(plugin).await.filter(|w| w.answers(hook))?;
    let method = format!("hook/{hook}");
    let r = match limit {
      Some(limit) => w.call_for(&method, payload, limit).await,
      None => w.call(&method, payload).await,
    };
    if let Err(e) = &r {
      (self.log)(&format!("plugin {plugin}: hook {hook}: {e}"));
    }
    Some(r)
  }

  /// The plugin's store, one per plugin in this engine: an old and a new version running side by side share it
  fn kv(&self, id: &str) -> Arc<Kv> {
    self.kvs.lock().entry(id.to_owned()).or_insert_with(|| Arc::new(Kv::new(self.data_root.join(format!("{id}.kv.json"))))).clone()
  }

  /// Hear what the processes send the pages; the id unsubscribes
  pub fn subscribe_pages(&self, f: PageFn) -> u64 {
    let id = self.next_page.fetch_add(1, Ordering::Relaxed);
    self.pages.write().push((id, f));
    id
  }

  pub fn unsubscribe_pages(&self, id: u64) {
    self.pages.write().retain(|(i, _)| *i != id);
  }

  /// Hand a message to every page; how many took it
  fn to_pages(&self, msg: acpira_shared::protocol::HostMsg) -> usize {
    let pages: Vec<PageFn> = self.pages.read().iter().map(|(_, f)| f.clone()).collect();
    for f in &pages {
      f(msg.clone());
    }
    pages.len()
  }

  /// `plugin`'s process, waited for while it is wanted but not up yet (starting, or restarting after an exit): a piece
  /// or a command button can be used the moment its plugin is enabled, before the process answered `initialize`
  async fn worker_of(&self, plugin: &str) -> Option<Arc<Worker>> {
    let t = std::time::Instant::now();
    loop {
      if let Some(w) = self.workers.read().get(plugin).cloned() {
        return Some(w);
      }
      let wanted = self.wanted.lock().iter().any(|w| w.id == plugin);
      if !wanted || t.elapsed() >= START_WAIT {
        return None;
      }
      tokio::time::sleep(Duration::from_millis(50)).await;
    }
  }

  /// Call a method `plugin`'s process offers (`runtime.methods`): `rpc/<method>` with `{ params, from }`, `from` being
  /// the calling plugin's id or `page`. Under the process's own time limit and failure count
  pub async fn rpc(&self, plugin: &str, method: &str, params: Value, from: &str) -> Result<Value, String> {
    self.rpc_at(plugin, None, method, params, from).await
  }

  /// A settings frame talks to the runtime of the snapshot that supplied its document.
  pub(crate) async fn rpc_at(
    &self,
    plugin: &str,
    snapshot: Option<&str>,
    method: &str,
    params: Value,
    from: &str,
  ) -> Result<Value, String> {
    if let Some(hash) = snapshot {
      self.wait_for_snapshot(plugin, hash).await?;
    }
    let worker = self.worker_of(plugin).await;
    let Some(w) = worker.filter(|w| snapshot.is_none_or(|hash| w.want.hash == hash)) else {
      let why = self.errors.read().get(plugin).map(|e| format!(": {e}")).unwrap_or_default();
      // Callers and tests match "no running process"; only a snapshot-bound call says the process is the wrong one
      let matching = if snapshot.is_some() { "matching " } else { "" };
      return Err(format!("plugin {plugin} has no {matching}running process{why}"));
    };
    if !w.want.def.methods.iter().any(|m| m == method) {
      return Err(format!("plugin {plugin} offers no method `{method}`"));
    }
    // A removal in the account menu is the user's click too: the plugin may let go of the account over the network
    let limit = if from == "page" || method == ACCOUNT_REMOVE { CLICK_TIMEOUT } else { w.want.def.timeout() };
    w.call_for(&format!("rpc/{method}"), json!({ "params": params, "from": from }), limit).await
  }

  /// What `initialize` tells a process of the plugins it depends on: the range it asked for and whether one is in effect
  fn dependencies_of(&self, want: &Want) -> Value {
    let present = self.present.read();
    let mut out = serde_json::Map::new();
    for (id, range) in want.dependencies.iter().chain(want.optional.iter()) {
      let version = present.get(id);
      let fits = version.is_some_and(|v| crate::plugins::manifest::version_meets(v, range));
      out.insert(id.clone(), json!({ "range": range, "present": fits, "version": version }));
    }
    Value::Object(out)
  }

  /// A tool of `plugin` its process answers (a manifest tool without a `command`): the text for the model
  pub async fn tool(&self, plugin: &str, name: &str, arguments: &Value, ctx: &crate::plugins::run::CallContext) -> Result<String, String> {
    let Some(w) = self.worker_of(plugin).await else {
      let why = self.errors.read().get(plugin).map(|e| format!(": {e}")).unwrap_or_default();
      return Err(format!("plugin {plugin} has no running process{why}"));
    };
    let Some(def) = w.want.tools.iter().find(|t| t.name == name) else {
      return Err(format!("plugin {plugin} answers no tool `{name}`"));
    };
    let params = json!({ "arguments": arguments, "workspace": ctx.workspace, "session": ctx.session });
    let answer = w.call_for(&format!("tool/{name}"), params, def.timeout()).await?;
    Ok(answer.get("text").and_then(Value::as_str).map(str::to_owned).unwrap_or_else(|| answer.to_string()))
  }

  /// Start a run of `plugin`'s schedule `id` for the occurrence `due` (milliseconds); waits for it to end
  pub async fn run_schedule(&self, plugin: &str, id: &str, due: i64) -> Result<Value, String> {
    let Some(w) = self.worker_of(plugin).await else {
      let why = self.errors.read().get(plugin).map(|e| format!(": {e}")).unwrap_or_default();
      return Err(format!("plugin {plugin} has no running process{why}"));
    };
    let Some(def) = w.want.schedules.iter().find(|j| j.id == id) else {
      return Err(format!("plugin {plugin} has no schedule `{id}`"));
    };
    w.call_for(&format!("schedule/{id}"), json!({ "due": due }), def.timeout()).await
  }

  /// The schedules of the processes wanted here, with the snapshot each belongs to
  pub fn schedules(&self) -> Vec<(String, String, crate::plugins::manifest::ScheduleDef)> {
    self.wanted.lock().iter().flat_map(|w| w.schedules.iter().map(move |j| (w.id.clone(), w.hash.clone(), j.clone()))).collect()
  }

  pub fn set_redirect(&self, f: crate::acp::session::plugin_hooks::RedirectFn) {
    *self.redirect.write() = Some(f);
  }

  pub fn redirect(&self) -> Option<crate::acp::session::plugin_hooks::RedirectFn> {
    self.redirect.read().clone()
  }

  pub fn enabled(&self) -> bool {
    self.enabled.load(Ordering::Relaxed)
  }

  fn data_dir(&self, id: &str) -> PathBuf {
    self.data_root.join(id)
  }

  /// Why `id`'s process is not running or paused
  pub fn error(&self, id: &str) -> Option<String> {
    let workers = self.workers.read();
    if let Some(w) = workers.get(id)
      && w.tripped.lock().is_some_and(|at| at.elapsed() < TRIP_FOR)
    {
      return Some(format!("paused after {TRIP_AFTER} failed calls in a row; calls resume within a minute"));
    }
    drop(workers);
    self.errors.read().get(id).cloned()
  }

  /// Update the dispatch order for hooks and events without restarting plugin processes
  pub fn set_order(&self, ids: Vec<String>) {
    *self.order.write() = ids;
  }

  /// Running workers in saved load order, with any unlisted id following in id order
  fn ordered_workers(&self) -> Vec<Arc<Worker>> {
    let workers = self.workers.read();
    let order = self.order.read();
    let mut out = Vec::with_capacity(workers.len());
    let mut seen = std::collections::HashSet::with_capacity(workers.len());
    for id in order.iter() {
      if let Some(w) = workers.get(id)
        && seen.insert(id.as_str())
      {
        out.push(w.clone());
      }
    }
    for (id, w) in workers.iter() {
      if seen.insert(id.as_str()) {
        out.push(w.clone());
      }
    }
    out
  }

  /// The ids with a running process, in load order
  pub fn running(&self) -> Vec<String> {
    self.ordered_workers().into_iter().map(|w| w.want.id.clone()).collect()
  }

  /// Every hook some running process answers
  pub fn declared(&self) -> Vec<String> {
    let mut out: Vec<String> = self.workers.read().values().flat_map(|w| w.hooks.iter().cloned()).collect();
    out.sort();
    out.dedup();
    out
  }

  /// Whether any process answers `hook`: the one check a call site makes when no plugin asked in
  pub fn has_hook(&self, hook: &str) -> bool {
    self.workers.read().values().any(|w| w.answers(hook))
  }

  /// Ask every process that answers `hook`, in load order, one after another; each answer as it came
  pub async fn hook(&self, hook: &str, payload: &Value) -> Vec<HookReply> {
    let workers: Vec<Arc<Worker>> = self.ordered_workers().into_iter().filter(|w| w.answers(hook) && w.serves(payload)).collect();
    let mut out = Vec::with_capacity(workers.len());
    for w in workers {
      let result = w.call(&format!("hook/{hook}"), payload.clone()).await;
      if let Err(e) = &result {
        (self.log)(&format!("plugin {}: hook {hook}: {e}", w.want.id));
      }
      out.push(HookReply { plugin: w.want.id.clone(), result });
    }
    out
  }

  /// Ask every process that answers `hook` in load order, each seeing the payload as the ones before left it: `step` gets
  /// the plugin and its answer (failed calls and null answers are skipped), folds it into the payload and returns false
  /// to stop the chain. The payload as the chain ended
  pub async fn chain(&self, hook: &str, mut payload: Value, mut step: impl FnMut(&str, Value, &mut Value) -> bool) -> Value {
    let workers: Vec<Arc<Worker>> = self.ordered_workers().into_iter().filter(|w| w.answers(hook) && w.serves(&payload)).collect();
    for w in workers {
      match w.call(&format!("hook/{hook}"), payload.clone()).await {
        Ok(Value::Null) => {}
        Ok(answer) => {
          if !step(&w.want.id, answer, &mut payload) {
            break;
          }
        }
        Err(e) => (self.log)(&format!("plugin {}: hook {hook}: {e}", w.want.id)),
      }
    }
    payload
  }

  /// Whether any process subscribed to `event`
  pub fn hears(&self, event: &str) -> bool {
    self.workers.read().values().any(|w| w.want.def.hears(event))
  }

  /// Send `event` to every process that subscribed to it; `data` is built only when one did. Never waits: a full
  /// queue drops the event and the next one delivered says how many were dropped
  pub fn emit(&self, event: &str, data: impl FnOnce() -> Value) {
    let workers: Vec<Arc<Worker>> = self.ordered_workers().into_iter().filter(|w| w.want.def.hears(event)).collect();
    if workers.is_empty() {
      return;
    }
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    let msg = json!({ "type": event, "at": at, "data": data() });
    for w in workers {
      if w.events.try_send(msg.clone()).is_err() {
        w.dropped.fetch_add(1, Ordering::Relaxed);
      }
    }
  }
}

impl crate::acp::session::ext::PluginRuntimePort for Runtimes {
  fn has_hook(&self, hook: &str) -> bool {
    Runtimes::has_hook(self, hook)
  }

  fn declared(&self) -> Vec<String> {
    Runtimes::declared(self)
  }

  fn hears(&self, event: &str) -> bool {
    Runtimes::hears(self, event)
  }

  fn emit_value(&self, event: &str, data: Value) {
    Runtimes::emit(self, event, || data);
  }

  fn contributed_commands(&self) -> Vec<PluginCommand> {
    self.contributed().commands.clone()
  }

  fn contributed_skills(&self) -> Vec<PathBuf> {
    self.contributed().skills.clone()
  }

  fn shipped_skills(&self) -> Option<PathBuf> {
    Runtimes::shipped_skills(self)
  }

  fn hook_of<'a>(&'a self, plugin: &'a str, hook: &'a str, payload: Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Result<Value, String>>> + Send + 'a>> {
    Box::pin(Runtimes::hook_of(self, plugin, hook, payload))
  }

  fn hook_clicked<'a>(&'a self, plugin: &'a str, hook: &'a str, payload: Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<Result<Value, String>>> + Send + 'a>> {
    Box::pin(Runtimes::hook_clicked(self, plugin, hook, payload))
  }

  fn chain<'a>(
    &'a self,
    hook: &'a str,
    payload: Value,
    step: crate::acp::session::ext::ChainStep<'a>,
  ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Value> + Send + 'a>> {
    Box::pin(Runtimes::chain(self, hook, payload, step))
  }

  fn redirect(&self) -> Option<crate::acp::session::plugin_hooks::RedirectFn> {
    Runtimes::redirect(self)
  }
}

/// Start the process of a frozen plugin, answer `initialize`, stop it: the dynamic check before a plugin with a
/// runtime is enabled
pub async fn probe(id: &str, hash: &str, dir: &Path, manifest: &crate::plugins::manifest::Manifest, data_root: &Path, log: LogFn) -> Result<(), String> {
  let rt = Runtimes::new(data_root.to_path_buf(), log);
  let want = Want::of(id, hash, dir, manifest).ok_or("no runtime")?;
  let w = rt.start_as(&want, true).await?;
  w.stop().await;
  Ok(())
}

/// How long a call to a plugin's method waits for its wanted process to start
const START_WAIT: Duration = Duration::from_secs(10);

#[cfg(test)]
#[cfg(unix)]
mod tests;
