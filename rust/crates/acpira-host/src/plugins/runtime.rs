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
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Connection, Inbound, RpcError};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::plugins::host::Active;
use crate::plugins::manifest::{CommandDef, RuntimeDef};
use crate::store::transcript_store::LogFn;

/// The protocol a process and the engine speak; `initialize` refuses any other
pub const PROTOCOL_VERSION: u64 = 1;
/// Longest a process may take to answer `initialize`
pub const INIT_TIMEOUT: Duration = Duration::from_secs(10);
/// After `shutdown`, how long a process has to exit before it is killed
const STOP_GRACE: Duration = Duration::from_secs(2);
/// Failed calls in a row that open the circuit, and how long it stays open
const TRIP_AFTER: u32 = 3;
const TRIP_FOR: Duration = Duration::from_secs(60);
/// Events waiting for one process
const EVENT_QUEUE: usize = 1024;
/// The most a plugin may keep in its key-value store
const KV_MAX_BYTES: usize = 1024 * 1024;
/// The pause before a process that exited is started again: doubling from the first, at most the last
const RESTART_FIRST: Duration = Duration::from_secs(1);
const RESTART_MAX: Duration = Duration::from_secs(60);
/// Stderr lines of one process kept in the engine log per minute
const STDERR_LINES_PER_MIN: u32 = 60;

/// The engine's side of a call a process makes beyond the store (`session/*`, `ui/*`, …): plugin id, method, params
/// A page of this engine taking a message from a plugin's process
pub type PageFn = Arc<dyn Fn(acpira_shared::protocol::HostMsg) + Send + Sync>;

pub type HostCall = Arc<dyn Fn(String, String, Value) -> BoxFuture<Result<Value, RpcError>> + Send + Sync>;

/// One plugin to keep running: its frozen copy and what it declared
#[derive(Clone)]
struct Want {
  id: String,
  version: String,
  hash: String,
  dir: PathBuf,
  def: RuntimeDef,
}

struct Proc {
  child: parking_lot::Mutex<Option<tokio::process::Child>>,
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
}

impl Worker {
  fn answers(&self, hook: &str) -> bool {
    self.hooks.iter().any(|h| h == hook)
  }

  async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
    if self.tripped.lock().is_some_and(|at| at.elapsed() < TRIP_FOR) {
      return Err(format!("plugin {} is paused after {TRIP_AFTER} failed calls in a row", self.want.id));
    }
    let r = match tokio::time::timeout(self.want.def.timeout(), self.conn.request_cancellable(method, params)).await {
      Ok(Ok(v)) => Ok(v),
      Ok(Err(e)) => Err(e.message),
      Err(_) => Err(format!("timed out after {} s", self.want.def.timeout().as_secs_f64())),
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

#[derive(Debug, Clone, PartialEq)]
pub struct PluginCommand {
  pub plugin: String,
  pub def: CommandDef,
}

impl Contributed {
  pub fn of(active: &[Arc<Active>]) -> Contributed {
    let mut out = Contributed::default();
    for a in active {
      for c in &a.manifest.contributes.commands {
        if !out.commands.iter().any(|x| x.def.name == c.name) {
          out.commands.push(PluginCommand { plugin: a.id.clone(), def: c.clone() });
        }
      }
      if let Some(dir) = a.manifest.contributes.skills.as_deref().and_then(|p| crate::plugins::manifest::inside(&a.dir, p)).filter(|d| d.is_dir()) {
        out.skills.push(dir);
      }
      for p in a.manifest.contributes.providers.iter().filter_map(|raw| crate::plugins::manifest::provider_of(raw, &a.id).ok()) {
        if !out.providers.iter().any(|x| x.id == p.id) {
          out.providers.push(p);
        }
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

pub struct Runtimes {
  /// By id: calls go out in id order
  workers: parking_lot::RwLock<BTreeMap<String, Arc<Worker>>>,
  /// What should be running, kept for restarts
  wanted: parking_lot::Mutex<Vec<Want>>,
  /// Why a plugin's process is not running or paused, for the plugin page
  errors: parking_lot::RwLock<BTreeMap<String, String>>,
  restarts: parking_lot::Mutex<HashMap<String, (u32, Instant)>>,
  host_call: parking_lot::RwLock<Option<HostCall>>,
  contributed: parking_lot::RwLock<Arc<Contributed>>,
  /// Model sources a process set with `providers/set`, by plugin; dropped when the plugin leaves
  live_providers: parking_lot::RwLock<BTreeMap<String, Vec<acpira_shared::providers::Provider>>>,
  /// The pages that hear what a process sends them (`ui/post`, `ui/composer`): every bridge of this engine
  pages: parking_lot::RwLock<Vec<(u64, PageFn)>>,
  next_page: std::sync::atomic::AtomicU64,
  /// One writer of `plugins/providers.json` at a time: a load and a process's `providers/set` may race
  providers_write: parking_lot::Mutex<()>,
  /// Told when `contributed` changed (the session manager republishes the open sessions, whose menus list the commands)
  on_contributed: parking_lot::RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
  /// Where a `prompt` hook's redirect starts its session (the session manager registers itself)
  redirect: parking_lot::RwLock<Option<crate::acp::session::plugin_hooks::RedirectFn>>,
  /// Off until the composition root turns it on: `acpira plugin list` and the tests of other parts start no process
  enabled: AtomicBool,
  sync: tokio::sync::Mutex<()>,
  data_root: PathBuf,
  log: LogFn,
}

impl Runtimes {
  /// `data_root` is the plugins' data folder (`~/.acpira/plugins/.data`)
  pub fn new(data_root: PathBuf, log: LogFn) -> Arc<Runtimes> {
    Arc::new(Runtimes {
      workers: Default::default(),
      wanted: Default::default(),
      errors: Default::default(),
      restarts: Default::default(),
      host_call: Default::default(),
      redirect: Default::default(),
      contributed: Default::default(),
      live_providers: Default::default(),
      providers_write: Default::default(),
      pages: Default::default(),
      next_page: Default::default(),
      on_contributed: Default::default(),
      enabled: AtomicBool::new(false),
      sync: Default::default(),
      data_root,
      log,
    })
  }

  /// Let processes run; `host_call` serves what a process asks of the engine beyond its store
  pub fn enable(&self, host_call: Option<HostCall>) {
    *self.host_call.write() = host_call;
    self.enabled.store(true, Ordering::Relaxed);
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
    self.live_providers.write().insert(plugin.to_owned(), list);
    self.write_providers();
    Ok(n)
  }

  /// `plugins/providers.json` beside the plugin folders: the manifests' sources, then the processes', first id wins.
  /// Only an engine writes it, and only when its content changes, so the agent's stamp check stays quiet
  fn write_providers(&self) {
    if !self.enabled() {
      return;
    }
    let Some(dir) = self.data_root.parent() else { return };
    let _one = self.providers_write.lock();
    let mut providers = self.contributed.read().providers.clone();
    for p in self.live_providers.read().values().flatten() {
      if !providers.iter().any(|x| x.id == p.id) {
        providers.push(p.clone());
      }
    }
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
    let w = self.workers.read().get(plugin).filter(|w| w.answers(hook)).cloned()?;
    let r = w.call(&format!("hook/{hook}"), payload).await;
    if let Err(e) = &r {
      (self.log)(&format!("plugin {plugin}: hook {hook}: {e}"));
    }
    Some(r)
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

  /// Call a method `plugin`'s process offers (`runtime.methods`): `rpc/<method>` with `{ params, from }`, `from` being
  /// the calling plugin's id or `page`. Under the process's own time limit and failure count
  pub async fn rpc(&self, plugin: &str, method: &str, params: Value, from: &str) -> Result<Value, String> {
    let w = self.workers.read().get(plugin).cloned().ok_or_else(|| format!("plugin {plugin} has no running process"))?;
    if !w.want.def.methods.iter().any(|m| m == method) {
      return Err(format!("plugin {plugin} offers no method `{method}`"));
    }
    w.call(&format!("rpc/{method}"), json!({ "params": params, "from": from })).await
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

  /// The ids with a running process, in id order
  pub fn running(&self) -> Vec<String> {
    self.workers.read().keys().cloned().collect()
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

  /// Ask every process that answers `hook`, in id order, one after another; each answer as it came
  pub async fn hook(&self, hook: &str, payload: &Value) -> Vec<HookReply> {
    let workers: Vec<Arc<Worker>> = self.workers.read().values().filter(|w| w.answers(hook)).cloned().collect();
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

  /// Ask every process that answers `hook` in id order, each seeing the payload as the ones before left it: `step` gets
  /// the plugin and its answer (failed calls and null answers are skipped), folds it into the payload and returns false
  /// to stop the chain. The payload as the chain ended
  pub async fn chain(&self, hook: &str, mut payload: Value, mut step: impl FnMut(&str, Value, &mut Value) -> bool) -> Value {
    let workers: Vec<Arc<Worker>> = self.workers.read().values().filter(|w| w.answers(hook)).cloned().collect();
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
    let workers: Vec<Arc<Worker>> = self.workers.read().values().filter(|w| w.want.def.hears(event)).cloned().collect();
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

  /// Bring the processes in line with the plugins in effect: start what is new or changed (the old one is stopped
  /// once the new one answered `initialize`), stop what is gone. Safe mode passes nothing, so everything stops
  pub async fn sync(self: &Arc<Self>, active: &[Arc<Active>]) {
    let wanted: Vec<Want> = active
      .iter()
      .filter_map(|a| {
        a.manifest.contributes.runtime.clone().map(|def| Want { id: a.id.clone(), version: a.manifest.version.clone(), hash: a.hash.clone(), dir: a.dir.clone(), def })
      })
      .collect();
    *self.wanted.lock() = wanted.clone();
    if !self.enabled() {
      return;
    }
    self.apply(wanted).await;
  }

  async fn apply(self: &Arc<Self>, wanted: Vec<Want>) {
    let _one = self.sync.lock().await;
    let mut stopped = vec![];
    for want in &wanted {
      let current = self.workers.read().get(&want.id).cloned();
      if current.as_ref().is_some_and(|w| w.want.hash == want.hash && !w.conn.is_closed()) {
        continue;
      }
      match self.start(want).await {
        Ok(worker) => {
          self.errors.write().remove(&want.id);
          if let Some(old) = self.workers.write().insert(want.id.clone(), worker) {
            stopped.push(old);
          }
        }
        Err(e) => {
          (self.log)(&format!("plugin {}: the runtime did not start: {e}", want.id));
          self.errors.write().insert(want.id.clone(), format!("the runtime did not start: {e}"));
          if let Some(old) = self.workers.write().remove(&want.id) {
            stopped.push(old);
          }
          self.schedule_restart(&want.id);
        }
      }
    }
    let gone: Vec<String> = self.workers.read().keys().filter(|id| !wanted.iter().any(|w| &w.id == *id)).cloned().collect();
    let dropped = {
      let mut live = self.live_providers.write();
      let before = live.len();
      live.retain(|id, _| wanted.iter().any(|w| &w.id == id));
      live.len() != before
    };
    if dropped {
      self.write_providers();
    }
    for id in gone {
      self.errors.write().remove(&id);
      self.restarts.lock().remove(&id);
      if let Some(old) = self.workers.write().remove(&id) {
        stopped.push(old);
      }
    }
    for w in stopped {
      w.stop().await;
    }
  }

  /// Stop every process (the engine is shutting down)
  pub async fn stop_all(&self) {
    self.enabled.store(false, Ordering::Relaxed);
    let workers: Vec<Arc<Worker>> = std::mem::take(&mut *self.workers.write()).into_values().collect();
    for w in workers {
      w.stop().await;
    }
  }

  async fn start(self: &Arc<Self>, want: &Want) -> Result<Arc<Worker>, String> {
    let data = self.data_dir(&want.id);
    let mut cmd = crate::plugins::run::command(&want.def.command, &want.dir, &data, "").await?;
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    let (mut child, job) = crate::platform::windows_process::spawn(&mut cmd).await.map_err(|e| format!("{e}"))?;
    #[cfg(not(windows))]
    let mut child = cmd.spawn().map_err(|e| format!("{e}"))?;
    let pid = child.id();
    let stdin = child.stdin.take().ok_or("stdin is not piped")?;
    let stdout = child.stdout.take().ok_or("stdout is not piped")?;
    let stderr = child.stderr.take().ok_or("stderr is not piped")?;
    self.pipe_stderr(&want.id, stderr);
    let inbound: Arc<dyn Inbound> = Arc::new(PluginInbound { id: want.id.clone(), kv: Kv::new(self.data_root.join(format!("{}.kv.json", want.id))), runtimes: Arc::downgrade(self) });
    let log = self.log.clone();
    let id = want.id.clone();
    let conn = Connection::start(stdout, stdin, Arc::new(move || inbound.clone()), Arc::new(move |l: &str| log(&format!("plugin {id}: {l}"))));
    let proc = Proc {
      child: parking_lot::Mutex::new(Some(child)),
      pid,
      #[cfg(windows)]
      job,
    };
    let init = json!({
      "protocolVersion": PROTOCOL_VERSION,
      "plugin": { "id": want.id, "version": want.version, "snapshot": want.hash, "dir": want.dir, "data": data },
      "engine": { "name": "acpira", "version": env!("CARGO_PKG_VERSION") },
      "hooks": want.def.hooks,
      "events": want.def.events,
      "methods": want.def.methods,
    });
    let answer = match tokio::time::timeout(INIT_TIMEOUT, conn.request("initialize", init)).await {
      Ok(Ok(v)) => v,
      Ok(Err(e)) => {
        conn.close();
        proc.stop().await;
        return Err(format!("initialize failed: {}", e.message));
      }
      Err(_) => {
        conn.close();
        proc.stop().await;
        return Err(format!("no answer to initialize within {} s", INIT_TIMEOUT.as_secs()));
      }
    };
    if answer.get("protocolVersion").and_then(Value::as_u64) != Some(PROTOCOL_VERSION) {
      conn.close();
      proc.stop().await;
      return Err(format!("it speaks protocol {}, this Acpira speaks {PROTOCOL_VERSION}", answer.get("protocolVersion").map(Value::to_string).unwrap_or_else(|| "none".into())));
    }
    // The answer may narrow what it handles, never widen what the user confirmed
    let hooks = match answer.get("hooks").and_then(Value::as_array) {
      Some(list) => want.def.hooks.iter().filter(|h| list.iter().any(|v| v.as_str() == Some(h.as_str()))).cloned().collect(),
      None => want.def.hooks.clone(),
    };
    let (tx, rx) = mpsc::channel(EVENT_QUEUE);
    let worker = Arc::new(Worker {
      want: want.clone(),
      conn,
      proc,
      hooks,
      failures: AtomicU32::new(0),
      tripped: Default::default(),
      events: tx,
      dropped: Arc::new(AtomicU64::new(0)),
      stopping: AtomicBool::new(false),
    });
    self.pump_events(&worker, rx);
    self.watch_exit(&worker);
    Ok(worker)
  }

  /// Deliver queued events in order; the first one after a drop says how many were lost
  fn pump_events(&self, w: &Arc<Worker>, mut rx: mpsc::Receiver<Value>) {
    let conn = w.conn.clone();
    let dropped = w.dropped.clone();
    tokio::spawn(async move {
      while let Some(mut ev) = rx.recv().await {
        if conn.is_closed() {
          break;
        }
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0
          && let Some(o) = ev.as_object_mut()
        {
          o.insert("dropped".into(), json!(lost));
        }
        conn.notify("event", ev);
      }
    });
  }

  /// A process that exits while its plugin is in effect is started again after a growing pause
  fn watch_exit(self: &Arc<Self>, w: &Arc<Worker>) {
    let me = Arc::downgrade(self);
    let weak = Arc::downgrade(w);
    let conn = w.conn.clone();
    tokio::spawn(async move {
      conn.closed().await;
      let (Some(me), Some(w)) = (me.upgrade(), weak.upgrade()) else { return };
      if w.stopping.load(Ordering::Relaxed) {
        return;
      }
      let id = w.want.id.clone();
      let still = me.workers.read().get(&id).is_some_and(|cur| Arc::ptr_eq(cur, &w));
      if !still {
        return;
      }
      me.workers.write().remove(&id);
      w.proc.stop().await;
      (me.log)(&format!("plugin {id}: the runtime exited"));
      me.errors.write().insert(id.clone(), "the runtime exited; starting it again".into());
      me.schedule_restart(&id);
    });
  }

  fn schedule_restart(self: &Arc<Self>, id: &str) {
    let pause = {
      let mut r = self.restarts.lock();
      let e = r.entry(id.to_owned()).or_insert((0, Instant::now()));
      // A process that ran ten minutes without trouble starts over from the first pause
      if e.1.elapsed() > Duration::from_secs(600) {
        e.0 = 0;
      }
      e.1 = Instant::now();
      let pause = RESTART_FIRST.saturating_mul(1 << e.0.min(6)).min(RESTART_MAX);
      e.0 += 1;
      pause
    };
    let me = Arc::downgrade(self);
    let id = id.to_owned();
    tokio::spawn(async move {
      tokio::time::sleep(pause).await;
      let Some(me) = me.upgrade() else { return };
      if !me.enabled() || me.workers.read().contains_key(&id) {
        return;
      }
      let wanted = me.wanted.lock().clone();
      if wanted.iter().any(|w| w.id == id) {
        me.apply(wanted).await;
      }
    });
  }

  /// A process's stderr goes to the engine log, a bounded number of lines a minute
  fn pipe_stderr(&self, id: &str, stderr: tokio::process::ChildStderr) {
    let log = self.log.clone();
    let id = id.to_owned();
    tokio::spawn(async move {
      let mut lines = BufReader::new(stderr).lines();
      let mut window = (Instant::now(), 0u32);
      while let Ok(Some(line)) = lines.next_line().await {
        if window.0.elapsed() > Duration::from_secs(60) {
          window = (Instant::now(), 0);
        }
        window.1 += 1;
        if window.1 <= STDERR_LINES_PER_MIN {
          log(&format!("plugin {id}: {line}"));
        } else if window.1 == STDERR_LINES_PER_MIN + 1 {
          log(&format!("plugin {id}: more stderr this minute left out"));
        }
      }
    });
  }
}

/// Start the process of a frozen plugin, answer `initialize`, stop it: the dynamic check before a plugin with a
/// runtime is enabled
pub async fn probe(id: &str, version: &str, hash: &str, dir: &Path, def: &RuntimeDef, data_root: &Path, log: LogFn) -> Result<(), String> {
  let rt = Runtimes::new(data_root.to_path_buf(), log);
  let want = Want { id: id.to_owned(), version: version.to_owned(), hash: hash.to_owned(), dir: dir.to_path_buf(), def: def.clone() };
  let w = rt.start(&want).await?;
  w.stop().await;
  Ok(())
}

/// Longest text a process may put into the composer
const COMPOSER_MAX_CHARS: usize = 100_000;

/// What a process may ask of the engine: its store, a log line, and whatever `HostCall` serves
struct PluginInbound {
  id: String,
  kv: Kv,
  runtimes: std::sync::Weak<Runtimes>,
}

impl Inbound for PluginInbound {
  fn notification(&self, method: &str, params: Value) {
    if method == "log"
      && let Some(rt) = self.runtimes.upgrade()
    {
      let text = params.get("message").and_then(Value::as_str).unwrap_or_default();
      (rt.log)(&format!("plugin {}: {}", self.id, text.chars().take(2000).collect::<String>()));
    }
  }

  fn request(&self, method: String, params: Value, _cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let key = || params.get("key").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| RpcError::new(-32602, "`key` is needed"));
    let r = match method.as_str() {
      "storage/get" => key().map(|k| json!({ "value": self.kv.get(&k) })),
      "storage/set" => key().and_then(|k| self.kv.set(&k, params.get("value").cloned().unwrap_or(Value::Null)).map(|()| json!({})).map_err(|e| RpcError::new(-32000, e))),
      "storage/delete" => key().and_then(|k| self.kv.delete(&k).map(|()| json!({})).map_err(|e| RpcError::new(-32000, e))),
      "storage/list" => Ok(json!({ "keys": self.kv.keys(params.get("prefix").and_then(Value::as_str).unwrap_or("")) })),
      // To the plugin's own frames on every page of this engine, and text for the composer on screen
      "ui/post" => match (params.get("message"), self.runtimes.upgrade()) {
        (None, _) => Err(RpcError::new(-32602, "`message` is needed")),
        (_, None) => Err(RpcError::new(-32000, "the engine is closing")),
        (Some(message), Some(rt)) => Ok(json!({ "pages": rt.to_pages(acpira_shared::protocol::HostMsg::PluginPost { plugin: self.id.clone(), message: message.clone() }) })),
      },
      "ui/composer" => match (params.get("text").and_then(Value::as_str), self.runtimes.upgrade()) {
        (None, _) => Err(RpcError::new(-32602, "`text` is needed")),
        (_, None) => Err(RpcError::new(-32000, "the engine is closing")),
        (Some(text), Some(rt)) => {
          let replace = params.get("replace").and_then(Value::as_bool).unwrap_or(false);
          let msg = acpira_shared::protocol::HostMsg::PluginComposer { plugin: self.id.clone(), text: text.chars().take(COMPOSER_MAX_CHARS).collect(), replace };
          Ok(json!({ "pages": rt.to_pages(msg) }))
        }
      },
      // Another plugin's method: answered on its own task, since the callee may take its full time limit
      "plugin/call" => {
        let (Some(rt), Some(plugin), Some(name)) = (self.runtimes.upgrade(), params.get("plugin").and_then(Value::as_str), params.get("method").and_then(Value::as_str)) else {
          return Box::pin(async { Err(RpcError::new(-32602, "`plugin` and `method` are needed")) });
        };
        let (plugin, name, args, from) = (plugin.to_owned(), name.to_owned(), params.get("params").cloned().unwrap_or(Value::Null), self.id.clone());
        return Box::pin(async move { rt.rpc(&plugin, &name, args, &from).await.map_err(|e| RpcError::new(-32000, e)) });
      }
      "providers/set" => match (params.get("providers").and_then(Value::as_array), self.runtimes.upgrade()) {
        (None, _) => Err(RpcError::new(-32602, "`providers` must be an array")),
        (_, None) => Err(RpcError::new(-32000, "the engine is closing")),
        (Some(list), Some(rt)) => rt.set_live_providers(&self.id, list).map(|n| json!({ "providers": n })).map_err(|e| RpcError::new(-32602, e)),
      },
      _ => {
        let call = self.runtimes.upgrade().and_then(|rt| rt.host_call.read().clone());
        let id = self.id.clone();
        return match call {
          Some(f) => f(id, method, params),
          None => Box::pin(async move { Err(RpcError::method_not_found(&method)) }),
        };
      }
    };
    Box::pin(async move { r })
  }
}

/// A plugin's key-value store: one JSON object written tmp + rename on every change, at most `KV_MAX_BYTES`
struct Kv {
  path: PathBuf,
  map: parking_lot::Mutex<Option<Map<String, Value>>>,
}

impl Kv {
  fn new(path: PathBuf) -> Kv {
    Kv { path, map: Default::default() }
  }

  fn with<T>(&self, f: impl FnOnce(&mut Map<String, Value>) -> T) -> T {
    let mut guard = self.map.lock();
    let map = guard.get_or_insert_with(|| std::fs::read(&self.path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
    f(map)
  }

  fn get(&self, key: &str) -> Value {
    self.with(|m| m.get(key).cloned().unwrap_or(Value::Null))
  }

  fn keys(&self, prefix: &str) -> Vec<String> {
    self.with(|m| m.keys().filter(|k| k.starts_with(prefix)).cloned().collect())
  }

  fn set(&self, key: &str, value: Value) -> Result<(), String> {
    self.change(|m| {
      m.insert(key.to_owned(), value);
    })
  }

  fn delete(&self, key: &str) -> Result<(), String> {
    self.change(|m| {
      m.remove(key);
    })
  }

  fn change(&self, f: impl FnOnce(&mut Map<String, Value>)) -> Result<(), String> {
    self.with(|m| {
      let mut next = m.clone();
      f(&mut next);
      let bytes = serde_json::to_vec(&next).map_err(|e| e.to_string())?;
      if bytes.len() > KV_MAX_BYTES {
        return Err(format!("the store would hold {} KB, more than the {} KB a plugin may keep", bytes.len() / 1024, KV_MAX_BYTES / 1024));
      }
      if let Some(parent) = self.path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
      }
      let tmp = self.path.with_extension(format!("tmp{}", std::process::id()));
      std::fs::write(&tmp, &bytes).and_then(|()| std::fs::rename(&tmp, &self.path)).map_err(|e| e.to_string())?;
      *m = next;
      Ok(())
    })
  }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
  use super::*;
  use crate::plugins::manifest;

  // A resident plugin in node: answers initialize (narrowing its hooks to those it handles), echoes `hook/prompt` with
  // its version and a stored counter, sleeps through `hook/permission`, exits on `hook/tool.before`, writes every event
  // it hears to events.ndjson in its data folder, and exits when stdin closes
  const PLUGIN: &str = r#"
import { createInterface } from 'node:readline';
import { appendFileSync } from 'node:fs';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let next = 1000, data = '';
const waits = new Map();
const ask = (method, params) => new Promise(r => { const id = next++; waits.set(id, r); send({ jsonrpc: '2.0', id, method, params }); });
const rl = createInterface({ input: process.stdin });
rl.on('close', () => process.exit(0));
rl.on('line', async line => {
  const m = JSON.parse(line);
  if (m.id !== undefined && !m.method) { waits.get(m.id)?.(m.result); return; }
  if (m.method === 'event') { appendFileSync(data + '/events.ndjson', JSON.stringify(m.params) + '\n'); return; }
  if (m.method === 'shutdown') return;
  if (m.method === 'initialize') {
    data = m.params.plugin.data;
    const { mkdirSync } = await import('node:fs'); mkdirSync(data, { recursive: true });
    return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: Number(process.env.PROTO ?? 1), hooks: ['prompt', 'permission', 'tool.before'] } });
  }
  if (m.method === 'hook/prompt') {
    const n = ((await ask('storage/get', { key: 'n' })).value ?? 0) + 1;
    await ask('storage/set', { key: 'n', value: n });
    return send({ jsonrpc: '2.0', id: m.id, result: { ...m.params, version: process.env.VERSION ?? 'v1', n, pid: process.pid } });
  }
  if (m.method === 'hook/permission') return setTimeout(() => send({ jsonrpc: '2.0', id: m.id, result: {} }), 5000);
  if (m.method === 'hook/tool.before') process.exit(3);
  send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no ' + m.method } });
});
"#;

  fn active(dir: &Path, hash: &str, extra: &str) -> Arc<Active> {
    let json = format!(
      r#"{{ "schema": 1, "id": "rt", "version": "1.0.0", "description": "d", "contributes": {{ "runtime": {{ "command": ["node", "plugin.mjs"], "hooks": ["prompt", "permission", "tool.before", "title"], "events": ["session.*"], "timeout": 0.5 {extra} }} }} }}"#
    );
    let (m, _) = manifest::parse(json.as_bytes()).unwrap();
    Arc::new(Active { id: "rt".into(), hash: hash.into(), dir: dir.to_path_buf(), manifest: m })
  }

  fn alive(pid: u64) -> bool {
    // SAFETY: signal 0 only checks that the pid exists
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
  }

  async fn until(f: impl Fn() -> bool, ms: u64) {
    let t = Instant::now();
    while !f() {
      assert!(t.elapsed() < Duration::from_millis(ms), "timed out");
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
  }

  fn setup() -> Option<(tempfile::TempDir, Arc<Runtimes>)> {
    if !crate::plugins::host::on_path("node") {
      return None;
    }
    let root = tempfile::tempdir().unwrap();
    for v in ["a", "b"] {
      let d = root.path().join(v);
      std::fs::create_dir_all(&d).unwrap();
      std::fs::write(d.join("plugin.mjs"), PLUGIN).unwrap();
    }
    let rt = Runtimes::new(root.path().join(".data"), Arc::new(|_: &str| {}));
    rt.enable(None);
    Some((root, rt))
  }

  // Offers `rates.get`; on `rpc/relay` it calls another plugin's method through the engine and returns the answer
  const RPC: &str = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
let next = 1000;
const waits = new Map();
const ask = (method, params) => new Promise(r => { const id = next++; waits.set(id, r); send({ jsonrpc: '2.0', id, method, params }); });
createInterface({ input: process.stdin }).on('close', () => process.exit(0)).on('line', async line => {
  const m = JSON.parse(line);
  if (m.id !== undefined && !m.method) { waits.get(m.id)?.(m.error ? { error: m.error.message } : m.result); return; }
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'rpc/rates.get') return send({ jsonrpc: '2.0', id: m.id, result: { eur: 0.92, from: m.params.from, asked: m.params.params } });
  if (m.method === 'rpc/relay') return send({ jsonrpc: '2.0', id: m.id, result: await ask('plugin/call', m.params.params) });
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no ' + m.method } });
});
"#;

  #[tokio::test]
  async fn a_runtime_offers_its_declared_methods_to_pages_and_other_plugins() {
    let Some((root, rt)) = setup() else { return };
    let plugin = |id: &str, methods: &str| {
      let dir = root.path().join(id);
      std::fs::create_dir_all(&dir).unwrap();
      std::fs::write(dir.join("rpc.mjs"), RPC).unwrap();
      let json = format!(r#"{{ "schema": 1, "id": "{id}", "version": "1.0.0", "description": "d", "contributes": {{ "runtime": {{ "command": ["node", "rpc.mjs"], "methods": {methods} }} }} }}"#);
      let (m, mut f) = manifest::parse(json.as_bytes()).unwrap();
      manifest::check(&m, id, &dir, &|p| p == "node", &mut f);
      assert!(f.ok(), "{f:?}");
      Arc::new(Active { id: id.into(), hash: "h".into(), dir, manifest: m })
    };
    rt.sync(&[plugin("rates", r#"["rates.get"]"#), plugin("caller", r#"["relay"]"#)]).await;
    let page = rt.rpc("rates", "rates.get", json!({ "to": "EUR" }), "page").await.unwrap();
    assert_eq!(page, json!({ "eur": 0.92, "from": "page", "asked": { "to": "EUR" } }));
    // Through the engine from another plugin, which is named as the caller
    let relayed = rt.rpc("caller", "relay", json!({ "plugin": "rates", "method": "rates.get", "params": 1 }), "page").await.unwrap();
    assert_eq!(relayed, json!({ "eur": 0.92, "from": "caller", "asked": 1 }));
    // Only declared methods, only running plugins
    assert!(rt.rpc("rates", "relay", json!({}), "page").await.unwrap_err().contains("no method `relay`"));
    let refused = rt.rpc("caller", "relay", json!({ "plugin": "rates", "method": "secret" }), "page").await.unwrap();
    assert!(refused["error"].as_str().is_some_and(|e| e.contains("no method `secret`")), "{refused}");
    assert!(rt.rpc("gone", "x", json!({}), "page").await.unwrap_err().contains("no running process"));
    rt.stop_all().await;
  }

  #[tokio::test]
  async fn a_scaffolded_runtime_answers_hooks_events_and_methods_through_the_sdk() {
    let Some((root, rt)) = setup() else { return };
    let dir = crate::plugins::sdk::scaffold(root.path(), "kit", &["runtime"]).unwrap();
    let (m, _) = manifest::parse(&std::fs::read(dir.join(manifest::FILE)).unwrap()).unwrap();
    rt.sync(&[Arc::new(Active { id: "kit".into(), hash: "h".into(), dir, manifest: m })]).await;
    assert_eq!(rt.running(), ["kit"]);
    let ctx = |text: &str| json!({ "session": "s", "agent": "a", "cwd": "/", "text": text });
    let replies = rt.hook("prompt", &ctx("fix the TODO")).await;
    assert_eq!(replies[0].result.as_ref().unwrap(), &json!({ "context": "List the TODOs you leave behind." }));
    assert_eq!(rt.hook("prompt", &ctx("hello")).await[0].result.as_ref().unwrap(), &Value::Null);
    rt.emit("turn.ended", || json!({ "session": "s" }));
    rt.emit("turn.started", || json!({ "session": "s" }));
    let ping = || rt.rpc("kit", "ping", json!({}), "page");
    let t = Instant::now();
    while ping().await.unwrap()["turns"] != 1 {
      assert!(t.elapsed() < Duration::from_secs(5), "the event never counted");
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(ping().await.unwrap(), json!({ "pong": true, "turns": 1 }));
    rt.stop_all().await;
  }

  #[tokio::test]
  async fn a_runtime_answers_only_what_it_declared_and_keeps_its_store() {
    let Some((root, rt)) = setup() else { return };
    rt.sync(&[active(&root.path().join("a"), "h1", "")]).await;
    assert_eq!(rt.running(), ["rt"]);
    // Declared and handled; declared but narrowed away by initialize; never declared
    assert!(rt.has_hook("prompt") && !rt.has_hook("title") && !rt.has_hook("context"));
    let r = rt.hook("prompt", &json!({ "text": "hi" })).await;
    let v = r[0].result.as_ref().unwrap();
    assert_eq!((v["text"].as_str(), v["n"].as_u64()), (Some("hi"), Some(1)));
    assert_eq!(rt.hook("prompt", &json!({})).await[0].result.as_ref().unwrap()["n"], 2);
    let kv: Value = serde_json::from_slice(&std::fs::read(root.path().join(".data/rt.kv.json")).unwrap()).unwrap();
    assert_eq!(kv, json!({ "n": 2 }));
    // Events: only the families it subscribed to, built only when someone hears them
    assert!(rt.hears("session.created") && !rt.hears("turn.ended") && !rt.hears("text.delta"));
    rt.emit("turn.ended", || panic!("built for no one"));
    rt.emit("session.created", || json!({ "id": "s1" }));
    let events = root.path().join(".data/rt/events.ndjson");
    until(|| events.exists(), 3000).await;
    let ev: Value = serde_json::from_str(std::fs::read_to_string(&events).unwrap().lines().next().unwrap()).unwrap();
    assert_eq!((ev["type"].as_str(), ev["data"]["id"].as_str()), (Some("session.created"), Some("s1")));
    rt.stop_all().await;
  }

  #[tokio::test]
  async fn a_new_version_takes_over_before_the_old_one_stops_and_nothing_runs_once_it_is_gone() {
    let Some((root, rt)) = setup() else { return };
    rt.sync(&[active(&root.path().join("a"), "h1", "")]).await;
    let old = rt.hook("prompt", &json!({})).await[0].result.as_ref().unwrap()["pid"].as_u64().unwrap();
    std::fs::write(root.path().join("b/plugin.mjs"), PLUGIN.replace("process.env.VERSION ?? 'v1'", "'v2'")).unwrap();
    rt.sync(&[active(&root.path().join("b"), "h2", "")]).await;
    let v = rt.hook("prompt", &json!({})).await[0].result.clone().unwrap();
    assert_eq!(v["version"], "v2");
    let new = v["pid"].as_u64().unwrap();
    assert_ne!(old, new);
    until(|| !alive(old), 3000).await;
    // The store outlives the version
    assert_eq!(v["n"], 2);
    rt.sync(&[]).await;
    assert!(rt.running().is_empty());
    until(|| !alive(new), 3000).await;
  }

  #[tokio::test]
  async fn slow_calls_pause_the_plugin_and_a_crash_starts_it_again() {
    let Some((root, rt)) = setup() else { return };
    rt.sync(&[active(&root.path().join("a"), "h1", "")]).await;
    for _ in 0..TRIP_AFTER {
      let e = rt.hook("permission", &json!({})).await[0].result.clone().unwrap_err();
      assert!(e.contains("timed out"), "{e}");
    }
    let e = rt.hook("prompt", &json!({})).await[0].result.clone().unwrap_err();
    assert!(e.contains("paused"), "{e}");
    assert!(rt.error("rt").unwrap().contains("paused"));
    // A crash: gone at once, back after the first pause
    rt.workers.read().get("rt").unwrap().tripped.lock().take();
    assert!(rt.hook("tool.before", &json!({})).await[0].result.is_err());
    until(|| rt.running().is_empty(), 3000).await;
    assert!(rt.error("rt").unwrap().contains("exited"));
    until(|| !rt.running().is_empty(), 5000).await;
    assert!(rt.hook("prompt", &json!({})).await[0].result.is_ok());
    assert!(rt.error("rt").is_none());
    rt.stop_all().await;
  }

  #[tokio::test]
  async fn the_check_refuses_a_runtime_that_speaks_another_protocol() {
    let Some((root, _rt)) = setup() else { return };
    let a = active(&root.path().join("a"), "h1", "");
    let def = a.manifest.contributes.runtime.clone().unwrap();
    probe("rt", "1.0.0", "h1", &a.dir, &def, &root.path().join(".data"), Arc::new(|_: &str| {})).await.unwrap();
    let mut bad = def.clone();
    bad.command = vec!["sh".into(), "-c".into(), "PROTO=2 exec node plugin.mjs".into()];
    let e = probe("rt", "1.0.0", "h1", &a.dir, &bad, &root.path().join(".data"), Arc::new(|_: &str| {})).await.unwrap_err();
    assert!(e.contains("protocol 2"), "{e}");
    let mut silent = def;
    silent.command = vec!["sleep".into(), "30".into()];
    let t = Instant::now();
    let e = probe("rt", "1.0.0", "h1", &a.dir, &silent, &root.path().join(".data"), Arc::new(|_: &str| {})).await.unwrap_err();
    assert!(e.contains("no answer") && t.elapsed() < INIT_TIMEOUT + Duration::from_secs(4), "{e}");
  }
}
