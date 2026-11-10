//! Remote access on the settings page: the engine's side of its gateway, which paired devices connect to and which
//! runs inside the machine's engine while remote access is on (host.rs; docs/dev/host-architecture.md, Remote access
//! gateway). The switch is a login service (service.rs: a LaunchAgent, a systemd user unit or a logon task) that starts
//! that engine at login; this side installs it, writes `gateway.json`, and relays what the gateway's control channel
//! reports to the pages that asked. Every engine sharing the data root sees the same gateway, whichever process runs it.

mod autostart;
pub mod host;
pub mod launch_agent;
#[cfg_attr(not(windows), allow(dead_code, reason = "registered on Windows; its XML is tested everywhere"))]
mod schtask;
pub mod service;
pub mod stage;
pub mod tunnel;
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code, reason = "a Linux unit; its text is tested on every Unix"))]
mod systemd;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use acpira_shared::devices::{GatewayRequest, GatewayStatus, PushNote, RemoteAccessOp, RemoteAccessView};
use acpira_shared::local_ipc;
use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use self::service::{Program, Service};

use crate::store::transcript_store::LogFn;

pub type Listener = Arc<dyn Fn(&RemoteAccessView) + Send + Sync>;

/// How often a page's subscription looks for a gateway that is not answering yet
const RETRY: Duration = Duration::from_secs(2);
/// The retry while remote access is off: every page subscribes, and a gateway started by hand still shows up; an op
/// (turning it on) cuts the wait short
const IDLE_RETRY: Duration = Duration::from_secs(30);
/// How long a control request may take (pairing answers at once; nothing waits on the network)
const REQUEST_WAIT: Duration = Duration::from_secs(5);
/// How long a (re)started gateway may take to answer: an engine starting (the login shell's environment, a predecessor
/// letting go of its endpoint), then the gateway's endpoint
const START_WAIT: Duration = Duration::from_secs(30);
/// How long a stopped gateway may take to let go of its control channel (its devices are told it is going first)
const STOP_WAIT: Duration = Duration::from_secs(10);
const VERSION: &str = acpira_shared::version::VERSION;

pub struct RemoteAccess {
  root: PathBuf,
  /// The engine binary this engine runs from: what the login service's copy is made from
  exe: Option<PathBuf>,
  service: Service,
  /// Why no login service can run the engine here, once asked
  no_service: std::sync::OnceLock<Option<String>>,
  log: LogFn,
  gateway: parking_lot::Mutex<Option<GatewayStatus>>,
  busy: std::sync::atomic::AtomicBool,
  error: parking_lot::Mutex<Option<String>>,
  listeners: parking_lot::Mutex<Vec<(u64, Listener)>>,
  seq: AtomicU64,
  /// The control-socket watch, running while a page is subscribed
  watch: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
  /// Wakes the watch from its retry wait after an op
  kick: tokio::sync::Notify,
  me: Weak<RemoteAccess>,
}

impl RemoteAccess {
  pub fn new(root: &Path, user_home: &Path, exe: Option<PathBuf>, log: LogFn) -> Arc<RemoteAccess> {
    Arc::new_cyclic(|me| RemoteAccess {
      root: root.to_owned(),
      exe,
      service: Service::new(root, user_home),
      no_service: Default::default(),
      log,
      gateway: Default::default(),
      busy: Default::default(),
      error: Default::default(),
      listeners: Default::default(),
      seq: AtomicU64::new(1),
      watch: Default::default(),
      kick: tokio::sync::Notify::new(),
      me: me.clone(),
    })
  }

  /// The gateway's control channel
  fn socket(&self) -> PathBuf {
    local_ipc::gateway_endpoint(&self.root)
  }

  fn no_service(&self) -> Option<String> {
    self.no_service.get_or_init(|| self.service.unavailable()).clone()
  }

  fn config(&self) -> Value {
    read_config(&self.root)
  }

  pub fn view(&self) -> RemoteAccessView {
    let unavailable = if !cfg!(any(target_os = "macos", target_os = "linux", windows)) {
      Some("platform")
    } else if self.exe.is_none() {
      Some("component")
    } else if self.no_service().is_some() {
      Some("service")
    } else {
      None
    };
    RemoteAccessView {
      available: unavailable.is_none(),
      unavailable: unavailable.map(str::to_owned),
      enabled: self.service.installed(),
      busy: self.busy.load(Ordering::Acquire),
      gateway: self.gateway.lock().clone(),
      keep_awake: Some(self.config().get("keepAwake").and_then(Value::as_bool).unwrap_or(true)),
      error: self.error.lock().clone(),
    }
  }

  pub fn subscribe(&self, f: Listener) -> u64 {
    let id = self.seq.fetch_add(1, Ordering::Relaxed);
    self.listeners.lock().push((id, f));
    self.ensure_watch();
    id
  }

  pub fn unsubscribe(&self, id: u64) {
    self.listeners.lock().retain(|(i, _)| *i != id);
  }

  fn emit(&self) {
    let view = self.view();
    let listeners: Vec<Listener> = self.listeners.lock().iter().map(|(_, f)| f.clone()).collect();
    for f in listeners {
      f(&view);
    }
  }

  fn set_gateway(&self, g: Option<GatewayStatus>) {
    let mut cur = self.gateway.lock();
    if *cur != g {
      *cur = g;
      drop(cur);
      self.emit();
    }
  }

  fn ensure_watch(&self) {
    let mut w = self.watch.lock();
    if w.is_none() {
      *w = Some(tokio::spawn(watch_loop(self.me.clone())));
    }
  }

  /// Whether the watch should end: no page is subscribed. Decided under the watch lock, so a page subscribing at the
  /// same moment starts a new one
  fn watch_idle(&self) -> bool {
    let mut w = self.watch.lock();
    if self.listeners.lock().is_empty() {
      *w = None;
      return true;
    }
    false
  }

  pub fn dispose(&self) {
    if let Some(h) = self.watch.lock().take() {
      h.abort();
    }
    self.listeners.lock().clear();
  }

  /// Every op ends with a push of the view; a failure also lands in its `error` until the next op
  pub async fn op(self: &Arc<Self>, op: RemoteAccessOp) -> Result<()> {
    *self.error.lock() = None;
    let r = self.run_op(op).await;
    if let Err(e) = &r {
      *self.error.lock() = Some(format!("{e:#}"));
    }
    self.emit();
    // The gateway may be up now (or gone): look again at once
    self.kick.notify_one();
    r
  }

  async fn run_op(self: &Arc<Self>, op: RemoteAccessOp) -> Result<()> {
    use RemoteAccessOp as O;
    match op {
      O::Enable => self.busy_while(self.enable()).await,
      O::Disable => self.busy_while(self.disable()).await,
      O::Pair => self.request(&GatewayRequest::Pair).await.map(drop),
      O::CancelPair => self.request(&GatewayRequest::CancelPair).await.map(drop),
      O::Revoke { id } => self.request(&GatewayRequest::Revoke { id }).await.map(drop),
      O::Rename { id, name } => self.request(&GatewayRequest::Rename { id, name }).await.map(drop),
      O::AllowLan => self.request(&GatewayRequest::AllowLan).await.map(drop),
      O::SetRelays { relays } => {
        self.edit_config(|c| c["relays"] = serde_json::to_value(&relays).unwrap_or_default()).await?;
        // The endpoint is bound with its relays: a running gateway starts over in its engine (its devices reconnect
        // on their own)
        if self.service.installed() && self.request(&GatewayRequest::Restart).await.is_ok() {
          self
            .busy_while(async {
              self.wait_gone(STOP_WAIT).await;
              self.wait_up(START_WAIT).await
            })
            .await?;
        }
        Ok(())
      }
      O::SetKeepAwake { on } => {
        self.edit_config(|c| c["keepAwake"] = Value::Bool(on)).await?;
        // Not running: it reads the file when it starts
        let _ = self.request(&GatewayRequest::Reload).await;
        Ok(())
      }
      O::SetName { name } => {
        let name = name.trim().to_owned();
        self
          .edit_config(|c| {
            if name.is_empty() {
              c.as_object_mut().map(|o| o.remove("name"));
            } else {
              c["name"] = Value::String(name);
            }
          })
          .await?;
        let _ = self.request(&GatewayRequest::Reload).await;
        Ok(())
      }
    }
  }

  async fn busy_while(&self, f: impl std::future::Future<Output = Result<()>>) -> Result<()> {
    self.busy.store(true, Ordering::Release);
    self.emit();
    let r = f.await;
    self.busy.store(false, Ordering::Release);
    r
  }

  async fn edit_config(&self, f: impl FnOnce(&mut Value)) -> Result<()> {
    edit_config(&self.root, f).await
  }

  async fn enable(&self) -> Result<()> {
    let Some(exe) = self.exe.clone() else {
      bail!("this engine does not know its own binary");
    };
    if let Some(why) = self.no_service() {
      bail!("remote access needs {why}");
    }
    install(&self.root, &self.service, &exe, &*self.log).await?;
    self.wait_up(START_WAIT).await
  }

  /// Ask the running gateway to stop and wait until its control channel is gone; nothing running is fine
  async fn stop_gateway(&self) {
    if self.request(&GatewayRequest::Stop).await.is_ok() {
      self.wait_gone(STOP_WAIT).await;
    }
  }

  async fn wait_gone(&self, within: Duration) {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline && self.request(&GatewayRequest::Status).await.is_ok() {
      tokio::time::sleep(Duration::from_millis(100)).await;
    }
  }

  /// Wait for the gateway to answer, so a start ends on a running gateway instead of a view that reads as stopped
  /// until the watch reconnects. Its answer is the view's gateway until the watch's next report
  async fn wait_up(&self, within: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + within;
    loop {
      let left = deadline.saturating_duration_since(tokio::time::Instant::now());
      if let Ok(Ok(v)) = tokio::time::timeout(left, self.request(&GatewayRequest::Status)).await
        && let Ok(st) = serde_json::from_value::<GatewayStatus>(v)
      {
        *self.gateway.lock() = Some(st);
        return Ok(());
      }
      if tokio::time::Instant::now() >= deadline {
        let log = service::log_path(&self.root);
        let last = std::fs::read_to_string(&log).ok().and_then(|s| s.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_owned));
        return Err(match last {
          Some(l) => anyhow!("remote access did not start; {} says: {l}", log.display()),
          None => anyhow!("remote access did not start"),
        });
      }
      tokio::time::sleep(Duration::from_millis(250).min(left)).await;
    }
  }

  async fn disable(&self) -> Result<()> {
    let label = self.service.label();
    // The definition first, so no engine takes the gateway up again; then the gateway stops, its engine running on
    self.service.uninstall().await?;
    self.stop_gateway().await;
    stage::remove(&self.root).await;
    self.set_gateway(None);
    (self.log)(&format!("remote access: {label} removed"));
    Ok(())
  }

  /// At engine start: a service installed by an older version, by an earlier build of this version (a local package,
  /// an extension and the desktop app updated apart) or by a build that ran a standalone gateway is reinstalled from
  /// this one, so devices never keep talking to an old engine. An older engine still running beside a newer one leaves
  /// it alone, and so does an earlier build: two engines of one version never take turns reinstalling it. Switched on
  /// but not running (a crash, a load that failed, a stop by hand): started again, as the switch says
  pub async fn refresh_installed(&self) {
    if !self.service.installed() {
      return;
    }
    if let Some(exe) = &self.exe {
      let r = self.busy_while(refresh(&self.root, self.service.user_home(), exe, &self.log)).await;
      if let Err(e) = r {
        (self.log)(&format!("remote access: reinstalling for {VERSION} failed: {e:#}"));
      }
    }
    if self.wait_up(REQUEST_WAIT).await.is_err() {
      // This engine runs the gateway itself once it looks again; another one is started by the service
      if !host::kick()
        && let Err(e) = self.service.start().await
      {
        (self.log)(&format!("remote access: starting {} failed: {e:#}", self.service.label()));
      }
    }
    self.emit();
  }

  /// A session wants the user: the gateway pushes it to the paired phones that asked for notifications. Nothing to
  /// do while no gateway runs here; never waits on it
  pub fn notify(&self, note: PushNote) {
    if !local_ipc::may_serve(&self.socket()) {
      return;
    }
    let Some(me) = self.me.upgrade() else { return };
    tokio::spawn(async move {
      if let Err(e) = me.request(&GatewayRequest::Notify(note)).await {
        (me.log)(&format!("[remote] notify: {e:#}"));
      }
    });
  }

  async fn request(&self, req: &GatewayRequest) -> Result<Value> {
    request(&self.socket(), req).await
  }
}

fn read_config(root: &Path) -> Value {
  std::fs::read(root.join("devices").join("gateway.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| serde_json::json!({}))
}

async fn edit_config(root: &Path, f: impl FnOnce(&mut Value)) -> Result<()> {
  let path = root.join("devices").join("gateway.json");
  crate::store::file_lock::with_file_lock(&path, || async {
    if let Some(dir) = path.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    let mut c = read_config(root);
    if !c.is_object() {
      c = serde_json::json!({});
    }
    f(&mut c);
    crate::store::file_lock::write_atomic(&path, serde_json::to_string_pretty(&c)?.as_bytes(), Some(0o600)).await
  })
  .await
}

/// The service's copy and definition from `exe` unless what is installed is newer (`stage::keeps_installed`) and
/// current. The engine that runs the gateway calls this once it serves, every engine at its start
pub async fn refresh(root: &Path, user_home: &Path, exe: &Path, log: &LogFn) -> Result<()> {
  let service = Service::new(root, user_home);
  let config = read_config(root);
  let installed = config.get("engine").and_then(Value::as_str).and_then(|e| stage::installed_version(Path::new(e)));
  let mine = Some(exe).filter(|exe| !stage::is_copy(root, exe)).and_then(stage::built);
  let theirs = config.get("engineBuilt").and_then(Value::as_u64);
  let program = Program { exe: stage::exe(root) };
  if stage::keeps_installed(VERSION, mine, installed.as_deref(), theirs) {
    // The copy stays; only a definition that went missing or stale (unloaded by hand) is written again
    if program.exe.is_file() && !service.current(&program).await {
      service.install(&program, &**log).await?;
    }
    return Ok(());
  }
  // The engine started from the copy cannot tell its build: whoever installed it keeps it current
  if stage::is_copy(root, exe) {
    return Ok(());
  }
  install(root, &service, exe, &**log).await
}

/// Install the login service from `exe` and start it, as one transaction across the engines sharing the data root
/// (`devices/install.lock`): a standalone gateway of an earlier build stopped, the copy made, the definition written
/// and started (an engine it runs already is left running), the copies of earlier builds removed
async fn install(root: &Path, service: &Service, exe: &Path, log: &(dyn Fn(&str) + Send + Sync)) -> Result<()> {
  let lock = root.join("devices").join("install");
  // The lock file lives in `devices/`, which a data root that never had remote access on does not have yet
  tokio::fs::create_dir_all(root.join("devices")).await.context("creating the devices directory")?;
  let deadline = tokio::time::Instant::now() + START_WAIT + STOP_WAIT;
  loop {
    let r = crate::store::file_lock::with_file_lock(&lock, || install_locked(root, service, exe, log)).await;
    // Another engine is installing: once it is done, this one most likely finds the service current
    match r {
      Err(e) if e.to_string().contains("is locked by another host") && tokio::time::Instant::now() < deadline => continue,
      r => return r,
    }
  }
}

async fn install_locked(root: &Path, service: &Service, exe: &Path, log: &(dyn Fn(&str) + Send + Sync)) -> Result<()> {
  let socket = local_ipc::gateway_endpoint(root);
  let program = Program { exe: stage::exe(root) };
  let built = stage::built(exe).filter(|_| !stage::is_copy(root, exe));
  // A gateway that answers without an engine of its own is a standalone one from before the gateway moved into the
  // engine; its login service is replaced below. On Windows the engine running the gateway runs from the copy: it lets
  // the next one take over first
  let running = request(&socket, &GatewayRequest::Status).await.ok().and_then(|v| serde_json::from_value::<GatewayStatus>(v).ok());
  let copy_changes = !stage::is_copy(root, exe) && !same_build(root, built);
  if running.as_ref().is_some_and(|s| s.engine_pid.is_none() || (cfg!(windows) && copy_changes)) {
    stop(&socket).await;
  }
  stage::stage(root, exe).await?;
  let record = stage::record(root, VERSION).to_string_lossy().into_owned();
  edit_config(root, move |c| {
    c["engine"] = Value::String(record);
    // What `refresh` compares a later build of the same version with
    match built {
      Some(b) => c["engineBuilt"] = Value::from(b),
      None => drop(c.as_object_mut().map(|o| o.remove("engineBuilt"))),
    }
  })
  .await?;
  service.install(&program, log).await?;
  stage::remove_aside(root).await;
  log(&format!("remote access: {} installed ({})", service.label(), program.exe.display()));
  // This process may be the one to run it
  let _ = host::kick();
  Ok(())
}

/// `gateway.json` names this version and build already
fn same_build(root: &Path, built: Option<u64>) -> bool {
  let config = read_config(root);
  let installed = config.get("engine").and_then(Value::as_str).and_then(|e| stage::installed_version(Path::new(e)));
  installed.as_deref() == Some(VERSION) && config.get("engineBuilt").and_then(Value::as_u64) == built
}

/// Ask the gateway on `socket` to stop and wait until its control channel is gone
async fn stop(socket: &Path) {
  if request(socket, &GatewayRequest::Stop).await.is_err() {
    return;
  }
  let deadline = tokio::time::Instant::now() + STOP_WAIT;
  while tokio::time::Instant::now() < deadline && request(socket, &GatewayRequest::Status).await.is_ok() {
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
}

/// One request on the control channel and its answer
async fn request(socket: &Path, req: &GatewayRequest) -> Result<Value> {
  let exchange = async {
    let answer = local_ipc::exchange(socket, &serde_json::to_string(req)?)
      .await
      .map_err(|_| anyhow!("remote access is not running"))?
      .ok_or_else(|| anyhow!("the gateway hung up"))?;
    let v: Value = serde_json::from_str(&answer)?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
      bail!("{e}");
    }
    Ok(v)
  };
  tokio::time::timeout(REQUEST_WAIT, exchange).await.map_err(|_| anyhow!("the gateway did not answer"))?
}

/// Follows the gateway's `watch` stream while a page is subscribed; a gateway that is not running (or restarting) is
/// looked for again every `RETRY`
async fn watch_loop(me: Weak<RemoteAccess>) {
  loop {
    let Some(ra) = me.upgrade() else { return };
    if ra.watch_idle() {
      return;
    }
    if let Ok(s) = local_ipc::connect(&ra.socket()).await {
      use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
      let (r, mut w) = tokio::io::split(s);
      let mut line = serde_json::to_string(&GatewayRequest::Watch).unwrap_or_default();
      line.push('\n');
      if w.write_all(line.as_bytes()).await.is_ok() {
        let mut lines = BufReader::new(r).lines();
        loop {
          match tokio::time::timeout(RETRY, lines.next_line()).await {
            Ok(Ok(Some(l))) => match serde_json::from_str::<GatewayStatus>(&l) {
              Ok(st) => ra.set_gateway(Some(st)),
              Err(e) => (ra.log)(&format!("remote access: unreadable gateway status: {e}")),
            },
            // Quiet: still subscribed?
            Err(_) if !ra.watch_idle() => {}
            Err(_) => {
              // Not pushed: nobody is listening, and the next subscriber must not start from a stale report
              *ra.gateway.lock() = None;
              return;
            }
            Ok(_) => break,
          }
        }
      }
    }
    ra.set_gateway(None);
    let wait = if ra.view().enabled { RETRY } else { IDLE_RETRY };
    // `dispose` aborts this task, so holding `ra` across the wait keeps nothing alive past it
    let _ = tokio::time::timeout(wait, ra.kick.notified()).await;
    drop(ra);
  }
}
