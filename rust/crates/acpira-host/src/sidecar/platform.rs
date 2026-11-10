//! The host platform over the envelope protocol: every IDE action becomes
//! a platformRequest to the shell, facts come from hello and later platformEvents. Capabilities the shell did not declare
//! fall back host-side where that makes sense (file search walks the workspace) and are otherwise logged

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Map, Value};
use tokio::sync::oneshot;

use acpira_shared::protocol::FileHit;
use acpira_shared::settings::is_shared_setting;
use acpira_shared::sidecar::{
  BrowserOpened, Hello, PLATFORM_RPC_METHODS, PlanTarget, PlatformEvent, PlatformRequest, ShellEnv, ShellEnvPatch, SidecarMsg, ToastOpen,
};

use crate::node_files::NodeFiles;
use crate::store::data_dir::home_dir;
use crate::store::shared_settings::SharedSettings;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Whether a settings change touched acpira.<section> (None: anything)
pub type Affects = Arc<dyn Fn(Option<&str>) -> bool + Send + Sync>;
pub type Send_ = Arc<dyn Fn(SidecarMsg) + Send + Sync>;
/// Sends to the shell currently serving IDE actions; the wire it went out on, None when no shell is connected
pub type Route = Arc<dyn Fn(SidecarMsg) -> Option<u64> + Send + Sync>;
/// Hears which background window's own settings changed (`on_wire_settings_changed`)
pub type WireListener = Arc<dyn Fn(u64) + Send + Sync>;
/// Hears which settings a change affects (`on_settings_changed`)
pub type SettingsListener = Arc<dyn Fn(Affects) + Send + Sync>;

tokio::task_local! {
  /// The wire whose view message is being handled (`server.rs`): the platform requests made meanwhile go back to it,
  /// under its own capabilities, and a window's own facts answer the reads made meanwhile
  pub static ORIGIN: u64;
}

/// The wire behind the current task, when there is one
pub fn origin() -> Option<u64> {
  ORIGIN.try_with(|o| *o).ok()
}

struct Pending {
  method: &'static str,
  /// The wire the request went out on: its answers can only come back there, so its closing fails the request
  wire: u64,
  tx: oneshot::Sender<Result<Value>>,
}

/// One shell's facts: what its hello said, as its later platform events changed them
#[derive(Clone, Default)]
struct ShellFacts {
  caps: HashSet<String>,
  env: ShellEnv,
  /// Its own keys (`SHELL_SETTING_KEYS`), the legacy agent list, and until the shared file exists every other key too
  settings: Map<String, Value>,
}

impl ShellFacts {
  fn of(hello: &Hello) -> ShellFacts {
    ShellFacts {
      caps: hello.client.capabilities.iter().cloned().collect(),
      env: hello.env.clone(),
      settings: hello.settings.clone(),
    }
  }
}

struct WireFacts {
  /// A peer's capabilities serve the requests its views cause; its environment and settings are its own machine's and
  /// count for nothing here
  peer: bool,
  facts: ShellFacts,
}

/// Every connected shell's facts, so windows on different workspaces sharing one engine each keep their own
#[derive(Default)]
struct Shells {
  wires: HashMap<u64, WireFacts>,
  /// The window the server marks active (`set_active`): engine-global reads answer from it
  active: Option<u64>,
  /// What engine-global reads answer while no window is active: the hello the platform was built from, then the last
  /// active window's facts (an engine with no window connected behaves as before it left)
  last: ShellFacts,
}

impl Shells {
  fn window(&self, wire: u64) -> Option<&ShellFacts> {
    self.wires.get(&wire).filter(|w| !w.peer).map(|w| &w.facts)
  }

  /// Engine-global facts: the active window's, else the last one's
  fn global(&self) -> &ShellFacts {
    self.active.and_then(|a| self.window(a)).unwrap_or(&self.last)
  }

  /// The facts a read with no viewer means: the window whose view message is being handled, else the global ones
  fn current(&self) -> &ShellFacts {
    origin().and_then(|o| self.window(o)).unwrap_or_else(|| self.global())
  }

  /// A window's facts; a peer (or none) reads the current ones
  fn of(&self, wire: Option<u64>) -> &ShellFacts {
    wire.and_then(|w| self.window(w)).unwrap_or_else(|| self.current())
  }

  /// What a viewer on `wire` reads its folder and list scope from: its own wire's facts, a peer's included (a peer's
  /// hello describes the views it shows, e.g. a project shell with the host's home), else the current ones
  fn viewer(&self, wire: Option<u64>) -> &ShellFacts {
    wire.and_then(|w| self.wires.get(&w)).map(|w| &w.facts).unwrap_or_else(|| self.current())
  }

  /// The facts an event or a write changes, and whether they are the global ones (listeners hear only those)
  fn target_mut(&mut self, wire: Option<u64>) -> (&mut ShellFacts, bool) {
    let wire = wire.or_else(origin).filter(|w| self.window(*w).is_some()).or(self.active.filter(|a| self.window(*a).is_some()));
    let active = self.active;
    match wire.and_then(|w| self.wires.get_mut(&w).map(|f| (w, f))) {
      Some((w, f)) => (&mut f.facts, active == Some(w)),
      None => (&mut self.last, true),
    }
  }
}

/// What engine-global reads saw change when the active window did (`set_active`); `announce` tells the listeners
#[must_use]
#[derive(Default)]
pub struct Moved {
  keys: Vec<String>,
  language: bool,
}

pub struct SidecarPlatform {
  route: Route,
  stderr: Arc<dyn Fn(&str) + Send + Sync>,
  ignore_agents: bool,
  /// Each connected wire's facts (environment, capabilities, settings snapshot) and which window is active
  shells: parking_lot::RwLock<Shells>,
  /// `<ACPIRA_HOME>/settings.json`, where the shared keys live once the runtime has opened it (`use_shared`)
  shared: parking_lot::RwLock<Option<Arc<SharedSettings>>>,
  /// Whether a window's snapshot is known: a platform a peer's hello built (an SSH host's client, a paired device's
  /// gateway) has only the peer's empty one, so it must not create the shared file from it; the first window that joins
  /// does (`add_window`)
  window_seen: std::sync::atomic::AtomicBool,
  pending: parking_lot::Mutex<HashMap<String, Pending>>,
  settings_listeners: parking_lot::Mutex<Vec<SettingsListener>>,
  /// Hear that a background window's own settings or language changed (the global listeners do not)
  wire_settings_listeners: parking_lot::Mutex<Vec<WireListener>>,
  focus_listeners: parking_lot::Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
  files: NodeFiles,
  seq: std::sync::atomic::AtomicU64,
  closed: parking_lot::Mutex<Option<String>>,
}

/// The first engine on the machine creates the shared file from a window's snapshot (whatever it had until now). A
/// failed seed leaves the shell's snapshot in charge
async fn seed_shared(store: &SharedSettings, snapshot: &Map<String, Value>, log: &Arc<dyn Fn(&str) + Send + Sync>) {
  match store.seed(snapshot).await {
    Ok(true) => log(&format!("shared settings created from the shell's: {}", store.path().display())),
    Ok(false) => {}
    Err(e) => log(&format!("shared settings could not be created at {}: {e}", store.path().display())),
  }
}

/// Applies a partial env; whether the host language changed
fn apply_env(cur: &mut ShellEnv, env: ShellEnvPatch) -> bool {
  let changed = env.host_language.as_ref().is_some_and(|l| *l != cur.host_language);
  if let Some(c) = env.cwd {
    cur.cwd = Some(c);
  }
  if let Some(l) = env.host_language {
    cur.host_language = l;
  }
  if let Some(b) = env.blob_base {
    cur.blob_base = Some(b);
  }
  if let Some(p) = env.projects {
    cur.projects = p.then_some(true);
  }
  changed
}

impl SidecarPlatform {
  /// One shell on one wire (stdio sidecar, harness connection, tests)
  pub fn new(send: Send_, hello: &Hello, stderr: Arc<dyn Fn(&str) + Send + Sync>, ignore_agents: bool) -> Arc<Self> {
    Self::routed(
      Arc::new(move |m| {
        send(m);
        Some(0)
      }),
      hello,
      stderr,
      ignore_agents,
    )
  }

  /// Several shells over time (the persistent engine): `route` picks the one serving IDE actions right now. The hello's
  /// facts answer until a window is active
  pub fn routed(route: Route, hello: &Hello, stderr: Arc<dyn Fn(&str) + Send + Sync>, ignore_agents: bool) -> Arc<Self> {
    let mut last = ShellFacts::of(hello);
    if hello.client.is_peer() {
      // A peer's folder and project mode describe its own views, not this machine's shell: until a window joins,
      // viewer-less reads see no workspace and no project shell
      last.env.cwd = None;
      last.env.projects = None;
    }
    Arc::new(SidecarPlatform {
      route,
      stderr,
      ignore_agents,
      shells: parking_lot::RwLock::new(Shells { last, ..Default::default() }),
      shared: Default::default(),
      window_seen: std::sync::atomic::AtomicBool::new(!hello.client.is_peer()),
      pending: Default::default(),
      settings_listeners: Default::default(),
      wire_settings_listeners: Default::default(),
      focus_listeners: Default::default(),
      files: NodeFiles::new(),
      seq: Default::default(),
      closed: Default::default(),
    })
  }

  /// The origin wire's capabilities (a peer's too), else the current window's
  fn has_cap(&self, method: &str) -> bool {
    let shells = self.shells.read();
    if let Some(w) = origin().and_then(|o| shells.wires.get(&o)) {
      return w.facts.caps.contains(method);
    }
    shells.current().caps.contains(method)
  }

  /// A peer wire joined: its capabilities serve the requests its views cause, and nothing else changes
  pub fn add_peer(&self, wire: u64, hello: &Hello) {
    self.shells.write().wires.insert(wire, WireFacts { peer: true, facts: ShellFacts::of(hello) });
  }

  /// A window wire joined: its facts are kept apart from every other window's, and serve engine-global reads once the
  /// server makes it active (`set_active`)
  pub fn add_window(&self, wire: u64, hello: &Hello) {
    self.shells.write().wires.insert(wire, WireFacts { peer: false, facts: ShellFacts::of(hello) });
    // The engine started for a peer: the first window's snapshot is the one to create the shared file from
    if !self.window_seen.swap(true, std::sync::atomic::Ordering::AcqRel)
      && let Some(store) = self.shared.read().clone()
    {
      let snapshot = hello.settings.clone();
      let log = self.stderr.clone();
      tokio::spawn(async move { seed_shared(&store, &snapshot, &log).await });
    }
  }

  /// A wire left; an active window's facts stay behind for engine-global reads until another window is active
  pub fn drop_wire(&self, wire: u64) {
    let mut shells = self.shells.write();
    if let Some(w) = shells.wires.remove(&wire)
      && shells.active == Some(wire)
    {
      shells.last = w.facts;
      shells.active = None;
    }
  }

  /// The server's active window changed (None: no window left). Only takes the platform's own lock, so the server calls
  /// it under its wire table and `announce`s the result after letting go: listeners call back through `route`
  pub fn set_active(&self, wire: Option<u64>) -> Moved {
    let (from, to) = {
      let mut shells = self.shells.write();
      let wire = wire.filter(|w| shells.window(*w).is_some());
      if shells.active == wire {
        return Moved::default();
      }
      let from = shells.global().clone();
      shells.last = from.clone();
      shells.active = wire;
      (from, shells.global().clone())
    };
    let mut keys: Vec<String> = to.settings.iter().filter(|(k, v)| from.settings.get(*k) != Some(*v)).map(|(k, _)| k.clone()).collect();
    keys.extend(from.settings.keys().filter(|k| !to.settings.contains_key(*k)).cloned());
    keys.retain(|k| !self.shared_key(k));
    Moved { keys, language: from.env.host_language != to.env.host_language }
  }

  /// Listeners hear about every key whose value differs between the old active window and the new one
  pub fn announce(&self, moved: Moved) {
    if !moved.keys.is_empty() {
      self.settings_moved(moved.keys);
    }
    if moved.language {
      self.language_moved();
    }
  }

  /// The wire went away: requests waiting on its answers fail now instead of at the timeout
  pub fn fail_wire(&self, wire: u64, reason: &str) {
    let failed: Vec<Pending> = {
      let mut pending = self.pending.lock();
      let ids: Vec<String> = pending.iter().filter(|(_, p)| p.wire == wire).map(|(id, _)| id.clone()).collect();
      ids.into_iter().filter_map(|id| pending.remove(&id)).collect()
    };
    for p in failed {
      let _ = p.tx.send(Err(anyhow!("{}: {reason}", p.method)));
    }
  }

  pub fn blob_base(&self) -> Option<String> {
    self.blob_base_of(None)
  }

  /// The blob base of a window's view (None: the current window's)
  pub fn blob_base_of(&self, wire: Option<u64>) -> Option<String> {
    self.shells.read().of(wire).env.blob_base.clone()
  }

  pub fn log(&self, line: &str) {
    (self.stderr)(line);
  }

  pub fn host_language(&self) -> String {
    self.host_language_of(None)
  }

  /// One window's IDE language (None: the current window's)
  pub fn host_language_of(&self, wire: Option<u64>) -> String {
    self.shells.read().of(wire).env.host_language.clone()
  }

  pub fn home(&self) -> String {
    home_dir().to_string_lossy().into_owned()
  }

  pub fn cwd(&self) -> String {
    self.cwd_of(None)
  }

  /// The workspace a viewer on `wire` means (None: the current window's), else the home directory
  pub fn cwd_of(&self, wire: Option<u64>) -> String {
    let cwd = self.shells.read().viewer(wire).env.cwd.clone();
    cwd.unwrap_or_else(|| self.home())
  }

  /// A shell without a single workspace that lists sessions by project folder (`ShellEnv.projects`)
  pub fn projects(&self) -> bool {
    self.projects_of(None)
  }

  /// Whether a viewer on `wire` (None: the current window) belongs to a project shell
  pub fn projects_of(&self, wire: Option<u64>) -> bool {
    self.shells.read().viewer(wire).env.projects == Some(true)
  }

  /// Shared keys come from the shared file and stop following the shell's snapshot (a value Settings Sync brings into
  /// VS Code's settings.json, or an older engine's write there, changes nothing); the rest from the shell
  pub fn read_setting(&self, key: &str) -> Option<Value> {
    self.read_setting_of(None, key)
  }

  /// `read_setting` as a viewer on `wire` (None: the current window) sees it: its own wire's snapshot for the keys the
  /// shared file does not hold
  pub fn read_setting_of(&self, wire: Option<u64>, key: &str) -> Option<Value> {
    if self.ignore_agents && key == "agents" {
      return None;
    }
    if let Some(store) = self.shared_store(key) {
      return store.get(key);
    }
    self.shells.read().viewer(wire).settings.get(key).cloned()
  }

  /// The shared file once it exists, for a key it holds
  fn shared_store(&self, key: &str) -> Option<Arc<SharedSettings>> {
    if !is_shared_setting(key) {
      return None;
    }
    self.shared.read().clone().filter(|s| s.exists())
  }

  fn shared_key(&self, key: &str) -> bool {
    self.shared_store(key).is_some()
  }

  /// Opens the shared settings: the first engine on the machine creates the file from this shell's snapshot, unless a
  /// peer started it (`window_seen`)
  pub async fn use_shared(&self, store: Arc<SharedSettings>) {
    if self.window_seen.load(std::sync::atomic::Ordering::Acquire) {
      let snapshot = self.shells.read().current().settings.clone();
      seed_shared(&store, &snapshot, &self.stderr).await;
    }
    *self.shared.write() = Some(store);
  }

  /// Another engine (or a hand edit) changed the shared file: listeners hear about the keys whose value moved
  pub fn reload_shared(&self) {
    let Some(store) = self.shared.read().clone() else { return };
    let keys = store.reload();
    if !keys.is_empty() {
      self.settings_moved(keys);
    }
  }

  /// A shared key goes to the shared file and the engine's listeners hear it at once (no shell echoes it); the rest: the
  /// snapshot (the window whose view asked, else the active one) changes at once so the push right after the write
  /// already shows it, then that shell persists and echoes
  pub async fn write_setting(&self, key: &str, value: Value) -> Result<()> {
    if let Some(store) = self.shared_store(key) {
      store.write(key, value).await?;
      self.settings_moved(vec![key.to_owned()]);
      return Ok(());
    }
    self.shells.write().target_mut(None).0.settings.insert(key.to_owned(), value.clone());
    if self.has_cap("writeSetting") {
      self.rpc(PlatformRequest::WriteSetting { key: key.to_owned(), value }).await?;
    }
    Ok(())
  }

  pub fn on_settings_changed(&self, f: SettingsListener) {
    self.settings_listeners.lock().push(f);
  }

  /// A window that is not the active one changed its own settings or language: only its own pages need a new view
  pub fn on_wire_settings_changed(&self, f: WireListener) {
    self.wire_settings_listeners.lock().push(f);
  }

  fn wire_settings_moved(&self, wire: Option<u64>) {
    let Some(wire) = wire else { return };
    let ls: Vec<_> = self.wire_settings_listeners.lock().clone();
    for f in ls {
      f(wire);
    }
  }

  pub fn on_window_focus(&self, f: Arc<dyn Fn() + Send + Sync>) {
    self.focus_listeners.lock().push(f);
  }

  pub fn toast(&self, level: &str, text: &str) {
    self.toast_with(level, text, None);
  }

  /// A toast with an "Open" button for `open.path`; the file opens only when the button is clicked
  pub fn toast_with(&self, level: &str, text: &str, open: Option<ToastOpen>) {
    if !self.notify(PlatformRequest::Toast { level: level.to_owned(), text: text.to_owned(), open }) {
      (self.stderr)(&format!("[toast:{level}] {text}"));
    }
  }

  /// A session wants the user (`SessionManager::watch_attention`); dropped when the shell cannot notify
  pub fn attention(&self, session_id: &str, title: &str, body: &str, sound: bool, system: bool) {
    self.notify(PlatformRequest::Notify {
      session_id: session_id.to_owned(),
      title: title.to_owned(),
      body: body.to_owned(),
      open: crate::i18n::t("common.open"),
      sound,
      system,
    });
  }

  /// Without a terminal on the shell side the command is at least shown, so a login / install can be run by hand
  pub fn run_in_terminal(&self, title: String, command: String, args: Vec<String>, env: Option<BTreeMap<String, Option<String>>>) {
    // Both IDEs receive the same ASCII-only Windows invocation, regardless of their configured terminal dialect.
    // Environment edits live inside the encoded child shell so null removals and quoted paths behave identically.
    let (command, args, env) = if cfg!(windows) {
      let mut env = env.unwrap_or_default();
      if !env.keys().any(|key| key.eq_ignore_ascii_case("PATH"))
        && let Some(path) = crate::acp::agents::login_path::merged()
      {
        env.insert("PATH".into(), Some(path));
      }
      let (command, args) = crate::platform::terminal::windows_launch(&command, &args, Some(&env));
      (command, args, None)
    } else {
      (command, args, env)
    };
    let wire_env = env
      .as_ref()
      .map(|e| e.iter().map(|(k, v)| (k.clone(), v.clone().map(Value::from).unwrap_or(Value::Null))).collect::<Map<String, Value>>());
    if self.notify(PlatformRequest::RunInTerminal { title: title.clone(), command: command.clone(), args: args.clone(), env: wire_env }) {
      return;
    }
    let prefix: Vec<String> = env
      .unwrap_or_default()
      .into_iter()
      .map(|(k, v)| match v {
        None => format!("unset {k};"),
        Some(v) => format!("{k}={v}"),
      })
      .collect();
    let mut parts = vec![prefix.join(" "), command];
    parts.extend(args);
    let line: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    self.toast("error", &format!("{title}: {}", line.join(" ")));
  }

  pub async fn open_resolved_file(&self, path: &str, line: Option<i64>) -> Result<()> {
    if self.has_cap("openResolvedFile") {
      self.rpc(PlatformRequest::OpenResolvedFile { path: path.to_owned(), line }).await?;
    } else {
      (self.stderr)(&format!("openResolvedFile unsupported by the shell: {path}{}", line.map(|l| format!(":{l}")).unwrap_or_default()));
    }
    Ok(())
  }

  pub async fn open_plan_document(&self, target: PlanTarget) -> Result<()> {
    if self.has_cap("openPlanDocument") {
      self.rpc(PlatformRequest::OpenPlanDocument { target }).await?;
    } else {
      (self.stderr)("openPlanDocument unsupported by the shell");
    }
    Ok(())
  }

  pub fn open_external(&self, url: &str) {
    if !self.notify(PlatformRequest::OpenExternal { url: url.to_owned() }) {
      (self.stderr)(&format!("openExternal unsupported by the shell: {url}"));
    }
  }

  /// Whether the serving shell can show a page inside the editor (`open_browser` is offered only then)
  pub fn can_open_browser(&self) -> bool {
    self.has_cap("openBrowser")
  }

  /// Show an http(s) page in the editor; where the user's side shows it, or the shell's reason it did not open
  pub async fn open_browser(&self, url: &str, folder: Option<&str>, session: Option<(&str, &str)>) -> Result<BrowserOpened> {
    if !self.has_cap("openBrowser") {
      return Err(anyhow!("this editor cannot show web pages inside it"));
    }
    let request = PlatformRequest::OpenBrowser {
      url: url.to_owned(),
      folder: folder.map(str::to_owned),
      session_id: session.map(|(id, _)| id.to_owned()),
      cwd: session.map(|(_, cwd)| cwd.to_owned()),
    };
    let r = self.rpc(request).await?;
    let mut opened: BrowserOpened = serde_json::from_value(r).unwrap_or_default();
    if opened.url.is_empty() {
      opened.url = url.to_owned();
    }
    Ok(opened)
  }

  pub async fn reveal_in_os(&self, path: &str) -> Result<()> {
    if self.has_cap("revealInOS") {
      self.rpc(PlatformRequest::RevealInOs { path: path.to_owned() }).await?;
    } else {
      (self.stderr)(&format!("revealInOS unsupported by the shell: {path}"));
    }
    Ok(())
  }

  pub fn open_in_editor(&self, session_id: Option<String>) {
    if !self.notify(PlatformRequest::OpenInEditor { session_id }) {
      (self.stderr)("openInEditor unsupported by the shell");
    }
  }

  /// `root` is the folder the view means (`SessionManager::folder_for`); a shell with its own search covers its workspace
  pub async fn search_files(&self, root: Option<&str>, query: &str) -> Result<Vec<FileHit>> {
    if !self.has_cap("searchFiles") {
      let Some(root) = root else { return Ok(vec![]) };
      return Ok(self.files.search(root, query).await);
    }
    let hits = self.rpc(PlatformRequest::SearchFiles { query: query.to_owned() }).await?;
    Ok(hits.as_array().map(|a| a.iter().filter_map(|h| serde_json::from_value::<FileHit>(h.clone()).ok()).collect()).unwrap_or_default())
  }

  /// Fire-and-forget IDE actions; false when the shell did not declare the method
  fn notify(&self, request: PlatformRequest) -> bool {
    if !self.has_cap(request.method()) || self.closed.lock().is_some() {
      return false;
    }
    (self.route)(SidecarMsg::PlatformRequest { request_id: None, request }).is_some()
  }

  async fn rpc(&self, request: PlatformRequest) -> Result<Value> {
    debug_assert!(PLATFORM_RPC_METHODS.contains(&request.method()));
    let method = request.method();
    let id = format!("p{}", self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1);
    let (tx, rx) = oneshot::channel();
    {
      // Checked under the pending lock: close() records the reason before it drains, so a request is either refused here or failed there.
      // Routed under it too, so the wire is recorded before its closing can look for the request
      let mut pending = self.pending.lock();
      if let Some(reason) = self.closed.lock().clone() {
        return Err(anyhow!("sidecar closed ({reason})"));
      }
      let Some(wire) = (self.route)(SidecarMsg::PlatformRequest { request_id: Some(id.clone()), request }) else {
        return Err(anyhow!("{method}: no window is connected"));
      };
      pending.insert(id.clone(), Pending { method, wire, tx });
    }
    match tokio::time::timeout(RPC_TIMEOUT, rx).await {
      Ok(Ok(r)) => r,
      Ok(Err(_)) => Err(anyhow!("{method}: sidecar closed")),
      Err(_) => {
        self.pending.lock().remove(&id);
        Err(anyhow!("{method}: no response from the shell within {}s", RPC_TIMEOUT.as_secs()))
      }
    }
  }

  pub fn on_response(&self, request_id: &str, result: Option<Value>, error: Option<String>) {
    let Some(p) = self.pending.lock().remove(request_id) else {
      (self.stderr)(&format!("platformResponse for unknown request {request_id}"));
      return;
    };
    let _ = p.tx.send(match error {
      Some(e) => Err(anyhow!(e)),
      None => Ok(result.unwrap_or(Value::Null)),
    });
  }

  /// An event from the current shell (a one-wire platform's, tests)
  pub fn on_event(&self, ev: PlatformEvent) {
    self.on_wire_event(None, ev);
  }

  /// An event from a window wire: settings and environment change only that window's facts, and listeners hear about
  /// them only when it is the active one
  pub fn on_wire_event(&self, wire: Option<u64>, ev: PlatformEvent) {
    match ev {
      PlatformEvent::WindowFocus => {
        let ls: Vec<_> = self.focus_listeners.lock().clone();
        for f in ls {
          f();
        }
      }
      PlatformEvent::SettingsChanged { mut keys, settings } => {
        let global = {
          let mut shells = self.shells.write();
          let (facts, global) = shells.target_mut(wire);
          facts.settings = settings;
          global
        };
        // The shell's copies of shared keys no longer count
        keys.retain(|k| !self.shared_key(k));
        if global && !keys.is_empty() {
          self.settings_moved(keys);
        } else if !keys.is_empty() {
          self.wire_settings_moved(wire);
        }
      }
      PlatformEvent::EnvChanged { env } => {
        let (language_changed, global) = {
          let mut shells = self.shells.write();
          let (facts, global) = shells.target_mut(wire);
          (apply_env(&mut facts.env, env), global)
        };
        if language_changed && global {
          self.language_moved();
        } else if language_changed {
          self.wire_settings_moved(wire);
        }
      }
    }
  }

  fn language_moved(&self) {
    let affects: Affects = Arc::new(|section: Option<&str>| section.is_none() || section == Some("language"));
    let ls: Vec<_> = self.settings_listeners.lock().clone();
    for f in ls {
      f(affects.clone());
    }
  }

  fn settings_moved(&self, keys: Vec<String>) {
    let affects: Affects = Arc::new(move |section: Option<&str>| match section {
      None => !keys.is_empty(),
      Some(s) => keys.iter().any(|k| k == s || k.starts_with(&format!("{s}."))),
    });
    let ls: Vec<_> = self.settings_listeners.lock().clone();
    for f in ls {
      f(affects.clone());
    }
  }

  /// Every request still waiting fails now
  pub fn dispose(&self, reason: &str) {
    *self.closed.lock() = Some(reason.to_owned());
    let pending: Vec<Pending> = self.pending.lock().drain().map(|(_, p)| p).collect();
    for p in pending {
      let _ = p.tx.send(Err(anyhow!("{}: {reason}", p.method)));
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn hello(role: Option<&str>, settings: Value) -> Hello {
    serde_json::from_value(json!({
      "protocolVersion": 1, "requestId": "r",
      "client": { "name": "t", "role": role },
      "settings": settings,
    }))
    .unwrap()
  }

  // An engine a peer started (a paired device's gateway, an SSH host's client) has only the peer's empty snapshot: the
  // shared file waits for the first window, which creates it from its own settings
  #[tokio::test]
  async fn a_peer_first_engine_leaves_the_shared_file_to_the_first_window() {
    let dir = tempfile::tempdir().unwrap();
    let p = SidecarPlatform::routed(Arc::new(|_| None), &hello(Some("peer"), json!({})), Arc::new(|_| {}), false);
    let store = Arc::new(SharedSettings::open(dir.path()));
    p.use_shared(store.clone()).await;
    assert!(!store.exists());
    p.add_window(2, &hello(None, json!({ "disabledAgents": ["kimi"] })));
    p.announce(p.set_active(Some(2)));
    for _ in 0..100 {
      if store.exists() {
        break;
      }
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(store.get("disabledAgents"), Some(json!(["kimi"])));
    assert_eq!(p.read_setting("disabledAgents"), Some(json!(["kimi"])));

    // A window first seeds at once, as before
    let dir = tempfile::tempdir().unwrap();
    let w = SidecarPlatform::routed(Arc::new(|_| None), &hello(None, json!({ "disabledAgents": ["grok"] })), Arc::new(|_| {}), false);
    let store = Arc::new(SharedSettings::open(dir.path()));
    w.use_shared(store.clone()).await;
    assert_eq!(store.get("disabledAgents"), Some(json!(["grok"])));
  }

  fn window(cwd: &str, language: &str, projects: bool, caps: &[&str], settings: Value) -> Hello {
    serde_json::from_value(json!({
      "protocolVersion": 1, "requestId": "r",
      "client": { "name": "t", "capabilities": caps },
      "env": { "cwd": cwd, "hostLanguage": language, "projects": projects },
      "settings": settings,
    }))
    .unwrap()
  }

  fn listen(p: &SidecarPlatform) -> Arc<parking_lot::Mutex<Vec<(bool, bool)>>> {
    let heard = Arc::new(parking_lot::Mutex::new(vec![]));
    let h = heard.clone();
    p.on_settings_changed(Arc::new(move |affects| h.lock().push((affects(Some("sessionScope")), affects(Some("language"))))));
    heard
  }

  // Two windows on one engine: each keeps its own workspace and project mode, engine-wide reads follow the active one,
  // and a background window's changes stay its own
  #[test]
  fn windows_keep_their_own_facts_and_global_reads_follow_the_active_one() {
    let a = window("/a", "en", false, &[], json!({ "sessionScope": "workspace" }));
    let b = window("/b", "zh-CN", true, &[], json!({ "sessionScope": "all" }));
    let p = SidecarPlatform::routed(Arc::new(|_| None), &a, Arc::new(|_| {}), false);
    p.add_window(1, &a);
    p.announce(p.set_active(Some(1)));
    let heard = listen(&p);
    p.add_window(2, &b);
    assert_eq!((p.cwd(), p.cwd_of(Some(1)), p.cwd_of(Some(2))), ("/a".into(), "/a".into(), "/b".into()));
    assert!(!p.projects() && p.projects_of(Some(2)));
    assert!(heard.lock().is_empty(), "joining is not activating");

    p.announce(p.set_active(Some(2)));
    assert_eq!((p.cwd(), p.host_language()), ("/b".into(), "zh-CN".into()));
    assert_eq!(p.read_setting("sessionScope"), Some(json!("all")));
    assert_eq!(*heard.lock(), [(true, false), (false, true)]);
    assert_eq!(p.read_setting_of(Some(1), "sessionScope"), Some(json!("workspace")), "window A's snapshot is untouched");

    // The background window moves to another folder: only its own facts change, and nobody is told
    heard.lock().clear();
    p.on_wire_event(Some(1), serde_json::from_value(json!({ "type": "envChanged", "env": { "cwd": "/a2", "hostLanguage": "fr" } })).unwrap());
    p.on_wire_event(Some(1), serde_json::from_value(json!({ "type": "settingsChanged", "keys": ["sessionScope"], "settings": { "sessionScope": "all" } })).unwrap());
    assert_eq!((p.cwd(), p.cwd_of(Some(1))), ("/b".into(), "/a2".into()));
    assert!(heard.lock().is_empty());
    // ...except the background window's own pages, which hear their wire
    let woken = Arc::new(parking_lot::Mutex::new(vec![]));
    let w = woken.clone();
    p.on_wire_settings_changed(Arc::new(move |wire| w.lock().push(wire)));
    p.on_wire_event(Some(1), serde_json::from_value(json!({ "type": "settingsChanged", "keys": ["sessionScope"], "settings": {} })).unwrap());
    assert_eq!((woken.lock().clone(), heard.lock().is_empty()), (vec![1], true));
    // The active one's change is heard
    p.on_wire_event(Some(2), serde_json::from_value(json!({ "type": "settingsChanged", "keys": ["sessionScope"], "settings": {} })).unwrap());
    assert_eq!(*heard.lock(), [(true, false)]);

    // The active window leaves with none left: its facts stay the engine's until another window is active
    heard.lock().clear();
    p.announce(p.set_active(None));
    p.drop_wire(2);
    p.drop_wire(1);
    assert_eq!((p.cwd(), p.projects()), ("/b".into(), true));
    assert!(heard.lock().is_empty());
  }

  // A peer's hello describes the views it shows (a project shell with the host's home): its own views read that, but
  // viewer-less reads of an engine only peers use see no workspace and no project shell
  #[test]
  fn a_peer_s_project_mode_is_its_views_and_not_the_engine_s() {
    let peer: Hello = serde_json::from_value(json!({
      "protocolVersion": 1, "requestId": "r",
      "client": { "name": "acpira-peer", "role": "peer", "capabilities": [] },
      "env": { "cwd": "/host/home", "hostLanguage": "en", "projects": true },
      "settings": {},
    }))
    .unwrap();
    let p = SidecarPlatform::routed(Arc::new(|_| None), &peer, Arc::new(|_| {}), false);
    p.add_peer(1, &peer);
    assert!(!p.projects());
    assert_eq!(p.cwd(), p.home());
    assert!(p.projects_of(Some(1)));
    assert_eq!(p.cwd_of(Some(1)), "/host/home");
  }

  // A window's view message makes its platform requests under that window's own capabilities and facts, whichever window
  // is active
  #[tokio::test]
  async fn a_window_s_view_message_uses_its_own_capabilities_and_facts() {
    let a = window("/a", "en", false, &["openExternal"], json!({ "sessionScope": "workspace" }));
    let b = window("/b", "en", false, &[], json!({ "sessionScope": "all" }));
    let sent = Arc::new(parking_lot::Mutex::new(0));
    let s = sent.clone();
    let p = SidecarPlatform::routed(
      Arc::new(move |_| {
        *s.lock() += 1;
        Some(1)
      }),
      &a,
      Arc::new(|_| {}),
      false,
    );
    p.add_window(1, &a);
    p.add_window(2, &b);
    p.announce(p.set_active(Some(1)));
    ORIGIN
      .scope(2, async {
        p.open_external("https://example.com");
        assert_eq!(p.cwd(), "/b");
        assert_eq!(p.read_setting("sessionScope"), Some(json!("all")));
      })
      .await;
    assert_eq!(*sent.lock(), 0, "window B cannot open a link");
    p.open_external("https://example.com");
    assert_eq!(*sent.lock(), 1);
  }
}
