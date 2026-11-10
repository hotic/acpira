//! The gateway inside the engine (docs/dev/host-architecture.md, Remote access gateway). The machine's engine runs it
//! while remote access is on (Windows, which has no machine engine: the engine remote access's logon task starts,
//! `serve --service`), so devices drive this process's sessions directly and the engine stays up for them. `serve
//! --gateway` runs it whatever the switch says (the phones' test gateways, apps/ios/scripts/gateway.sh). Binding the
//! control channel decides which process serves the data root: a draining engine gives the gateway up with its endpoint
//! and its successor takes it once the channel is free. A standalone `acpira-gateway` left by a build up to 2.0.6 is
//! stopped first.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use acpira_gateway::control::{self, Lifecycle};
use acpira_gateway::gateway::Opts;
use acpira_gateway::hosted::Hosted;
use acpira_gateway::store::Paths;
use acpira_shared::devices::{GatewayRequest, GatewayStatus};
use tokio::sync::mpsc;

use super::service::Service;
use crate::sidecar::server::Engine;
use crate::store::transcript_store::LogFn;

/// How often the switch is looked at: remote access turned on or off from another engine shows up within this
const RECHECK: Duration = Duration::from_secs(2);
/// While the gateway should run and cannot yet (a predecessor still lets go of the control channel)
const RETRY: Duration = Duration::from_millis(500);

/// The process's host, for the settings page of this same engine (`kick`)
static HOST: std::sync::OnceLock<Weak<Host>> = std::sync::OnceLock::new();

pub struct Host {
  engine: Weak<Engine>,
  root: PathBuf,
  user_home: PathBuf,
  /// `serve --gateway`: runs the gateway whether or not remote access is on
  forced: bool,
  log: LogFn,
  running: tokio::sync::Mutex<Option<Hosted>>,
  /// A control `stop`: no gateway until remote access is turned on again
  stopped: AtomicBool,
  /// Remote access was on at the last look: turning it on anew (from any engine) lifts a stop
  was_on: AtomicBool,
  /// The engine gave its endpoint up (drained, ending): the gateway goes for good
  released: AtomicBool,
  kick: tokio::sync::Notify,
}

impl Host {
  /// Runs the gateway in `engine` for as long as remote access is on (`forced`: always)
  pub fn start(engine: &Arc<Engine>, root: &Path, forced: bool, log: LogFn) -> Arc<Host> {
    let user_home = crate::store::data_dir::home_dir();
    let on = Service::new(root, &user_home).installed();
    let host = Arc::new(Host {
      engine: Arc::downgrade(engine),
      root: root.to_owned(),
      user_home,
      forced,
      log,
      running: Default::default(),
      stopped: AtomicBool::new(false),
      was_on: AtomicBool::new(on),
      released: AtomicBool::new(false),
      kick: tokio::sync::Notify::new(),
    });
    let _ = HOST.set(Arc::downgrade(&host));
    let (on_tx, on_rx) = mpsc::unbounded_channel();
    tokio::spawn(run(host.clone(), on_tx, on_rx));
    host
  }

  /// The engine gives its endpoint up: the gateway stops, and a successor serves the data root
  pub fn release(&self) {
    self.released.store(true, Ordering::Release);
    self.kick.notify_one();
  }

  fn log(&self, line: &str) {
    (self.log)(&format!("[gateway] {line}"));
  }

  /// Whether the gateway should run here now
  fn wanted(&self) -> bool {
    let on = Service::new(&self.root, &self.user_home).installed();
    if on && !self.was_on.swap(true, Ordering::AcqRel) {
      self.stopped.store(false, Ordering::Release);
    }
    if !on {
      self.was_on.store(false, Ordering::Release);
    }
    let draining = self.engine.upgrade().is_none_or(|e| e.draining());
    (on || self.forced) && !self.stopped.load(Ordering::Acquire) && !self.released.load(Ordering::Acquire) && !draining
  }

  /// Brings the gateway in line with `wanted`; false while it should run and could not start yet
  async fn reconcile(self: &Arc<Self>, on_tx: &mpsc::UnboundedSender<Lifecycle>) -> bool {
    let want = self.wanted();
    let mut running = self.running.lock().await;
    match (want, running.is_some()) {
      (true, false) => match self.start_gateway(on_tx).await {
        Some(h) => {
          *running = Some(h);
          drop(running);
          if let Some(e) = self.engine.upgrade() {
            e.set_hosting(true);
          }
          self.after_start();
          true
        }
        None => false,
      },
      (false, true) => {
        if let Some(h) = running.take() {
          h.stop().await;
        }
        drop(running);
        if let Some(e) = self.engine.upgrade() {
          e.set_hosting(false);
        }
        self.log("stopped");
        true
      }
      _ => true,
    }
  }

  async fn stop_gateway(&self) {
    if let Some(h) = self.running.lock().await.take() {
      h.stop().await;
      if let Some(e) = self.engine.upgrade() {
        e.set_hosting(false);
      }
    }
  }

  /// Binds the control channel and starts the gateway; none while another process holds the channel
  async fn start_gateway(self: &Arc<Self>, on_tx: &mpsc::UnboundedSender<Lifecycle>) -> Option<Hosted> {
    let paths = Paths::new(&self.root);
    let listener = match control::bind(&paths.socket).await {
      Ok(l) => l,
      Err(e) if e.downcast_ref::<control::AlreadyRunning>().is_some() => {
        self.stop_standalone(&paths.socket).await;
        return None;
      }
      Err(e) => {
        self.log(&format!("control channel: {e:#}"));
        return None;
      }
    };
    let engine = self.engine.clone();
    let link: acpira_gateway::engine::Link = Arc::new(move || match engine.upgrade() {
      Some(e) => e.link(),
      None => (acpira_gateway::engine::none())(),
    });
    let engine = self.engine.clone();
    let ssh = super::tunnel::SshHosts::new(move || engine.upgrade().and_then(|e| e.runtime()).map(|rt| rt.connections.clone()));
    let log = self.log.clone();
    let opts =
      Opts { home: self.root.clone(), engine: link, relays: None, bind: None, ssh: Some(ssh), log: Arc::new(move |l: &str| log(&format!("[gateway] {l}"))) };
    match Hosted::start(opts, listener, on_tx.clone()).await {
      Ok(h) => Some(h),
      Err(e) => {
        self.log(&format!("did not start: {e:#}"));
        None
      }
    }
  }

  /// Another process answers on the control channel. An engine's gateway (a predecessor letting go) is waited for; a
  /// standalone `acpira-gateway` from before the gateway moved into the engine is asked to stop, a clean exit its login
  /// service does not restart
  async fn stop_standalone(&self, socket: &Path) {
    let Ok(Ok(answer)) = tokio::time::timeout(Duration::from_secs(5), control::request(socket, &GatewayRequest::Status)).await else { return };
    let standalone = serde_json::from_value::<GatewayStatus>(answer).is_ok_and(|s| s.engine_pid.is_none());
    if standalone {
      self.log("stopping the standalone acpira-gateway of an earlier build");
      let _ = control::request(socket, &GatewayRequest::Stop).await;
    }
  }

  /// The login service's copy and definition follow this build (migrating one that ran a standalone gateway)
  fn after_start(self: &Arc<Self>) {
    let Ok(exe) = std::env::current_exe() else { return };
    let (root, user_home, log) = (self.root.clone(), self.user_home.clone(), self.log.clone());
    tokio::spawn(async move {
      if !Service::new(&root, &user_home).installed() {
        return;
      }
      if let Err(e) = super::refresh(&root, &user_home, &exe, &log).await {
        log(&format!("[gateway] remote access: updating its login service failed: {e:#}"));
      }
    });
  }
}

async fn run(host: Arc<Host>, on_tx: mpsc::UnboundedSender<Lifecycle>, mut on_rx: mpsc::UnboundedReceiver<Lifecycle>) {
  let me = Arc::downgrade(&host);
  drop(host);
  loop {
    let Some(h) = me.upgrade() else { return };
    let settled = h.reconcile(&on_tx).await;
    if h.released.load(Ordering::Acquire) && h.running.lock().await.is_none() {
      return;
    }
    let wait = if settled { RECHECK } else { RETRY };
    tokio::select! {
      () = h.kick.notified() => {}
      () = tokio::time::sleep(wait) => {}
      what = on_rx.recv() => match what {
        Some(Lifecycle::Stop) => {
          h.stopped.store(true, Ordering::Release);
          h.log("stop requested");
          // Windows: the next engine of the logon task (a newer build) serves the data root; this one ends once idle
          if cfg!(windows) && let Some(e) = h.engine.upgrade() {
            e.release_endpoint();
          }
        }
        Some(Lifecycle::Restart) => {
          h.log("restarting");
          h.stop_gateway().await;
        }
        None => {}
      },
    }
  }
}

/// How long `serve --service` waits for the engine already serving the machine to show it runs the gateway
#[cfg(unix)]
pub const SERVICE_CONFIRM_WAIT: Duration = Duration::from_secs(20);

/// `serve --service` finding the machine's socket served, before it takes it: None to go on and serve, or the exit code
/// to end with. An older build is asked to hand over, since one from before the gateway moved into the engine never runs
/// it; one of this build or newer runs it itself while remote access is on, and the service ends cleanly only once the
/// control channel shows it does. A failure exit has the login service try again later
#[cfg(unix)]
pub async fn before_service(socket: &Path, root: &Path, wait: Duration, log: &(dyn Fn(&str) + Send + Sync)) -> Option<i32> {
  use crate::machine_engine::{ensure, policy};
  let deadline = tokio::time::Instant::now() + wait;
  loop {
    match ensure::probe(socket).await {
      Ok(None) => return None,
      // Handing over already: whoever answers next is looked at again
      Ok(Some(p)) if p.draining => {}
      // The service's copy was put there by this machine's engine, so a local build counts as installed on purpose
      Ok(Some(p)) if policy::takes_over(acpira_shared::version::VERSION, &p.version, true) => {
        log(&format!("serve: engine {} (pid {}) is older: taking over", p.version, p.pid));
        return match ensure::drain(socket, true).await {
          Ok(()) => None,
          Err(e) => {
            log(&format!("serve: {e:#}"));
            Some(1)
          }
        };
      }
      Ok(Some(p)) => return Some(confirm_gateway(root, Some(p.pid), deadline, log).await),
      Err(e) => log(&format!("serve: {}: {e:#}", socket.display())),
    }
    if tokio::time::Instant::now() >= deadline {
      log(&format!("serve: no engine settled on {}", socket.display()));
      return Some(1);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
  }
}

/// 0 once the data root's gateway runs inside an engine (`pid`, or any when None), 1 when it does not by `deadline`
#[cfg(unix)]
pub async fn confirm_gateway(root: &Path, pid: Option<u32>, deadline: tokio::time::Instant, log: &(dyn Fn(&str) + Send + Sync)) -> i32 {
  let socket = Paths::new(root).socket;
  loop {
    let status = tokio::time::timeout(Duration::from_secs(2), control::request(&socket, &GatewayRequest::Status)).await;
    let host = status.ok().and_then(Result::ok).and_then(|v| serde_json::from_value::<GatewayStatus>(v).ok()).and_then(|s| s.engine_pid);
    if host.is_some_and(|h| pid.is_none_or(|p| p == h)) {
      log("serve: the engine serving this data root runs the gateway already");
      return 0;
    }
    if tokio::time::Instant::now() >= deadline {
      log("serve: the engine serving this data root does not run the gateway");
      return 1;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
  }
}

/// The settings page of this engine turned remote access on: a stop no longer holds, and the gateway starts now.
/// False when this process runs no host (an engine that never serves the gateway)
pub fn kick() -> bool {
  let Some(h) = HOST.get().and_then(Weak::upgrade) else { return false };
  h.stopped.store(false, Ordering::Release);
  h.kick.notify_one();
  true
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
  use super::*;
  use acpira_shared::devices::Relays;
  use serde_json::{Value, json};
  use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

  const PID: u32 = 4242;

  /// Answers every line on `socket` with `answer(line)`; the listener lives as long as the runtime
  fn serve_lines(socket: &Path, answer: impl Fn(Value) -> Value + Send + Sync + 'static) {
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    let l = tokio::net::UnixListener::bind(socket).unwrap();
    let answer = Arc::new(answer);
    tokio::spawn(async move {
      while let Ok((s, _)) = l.accept().await {
        let answer = answer.clone();
        tokio::spawn(async move {
          let (r, mut w) = s.into_split();
          let mut lines = BufReader::new(r).lines();
          while let Ok(Some(line)) = lines.next_line().await {
            let reply = answer(serde_json::from_str(&line).unwrap());
            let _ = w.write_all(format!("{reply}\n").as_bytes()).await;
          }
        });
      }
    });
  }

  /// An engine of `version` on the machine's socket; the flag is set once it is asked to drain
  fn engine(root: &Path, version: &'static str) -> (PathBuf, Arc<AtomicBool>) {
    let socket = crate::machine_engine::ensure::socket_path(root);
    let drained = Arc::new(AtomicBool::new(false));
    let flag = drained.clone();
    serve_lines(&socket, move |m| match m["type"].as_str() {
      Some("drain") => {
        flag.store(true, Ordering::Release);
        json!({ "type": "drainOk", "requestId": m["requestId"] })
      }
      _ => json!({ "type": "probeOk", "requestId": m["requestId"], "version": version, "pid": PID }),
    });
    (socket, drained)
  }

  /// A control channel reporting a gateway that runs in `engine_pid` (None: a standalone gateway)
  fn gateway(root: &Path, engine_pid: Option<u32>) {
    let status = GatewayStatus {
      id: "id".into(),
      name: "name".into(),
      version: "2.0.6".into(),
      relays: Relays::Off,
      relay: None,
      relay_connected: false,
      devices: vec![],
      pairing: None,
      awake_error: None,
      lan_blocked: false,
      engine_pid,
    };
    let status = serde_json::to_value(status).unwrap();
    serve_lines(&Paths::new(root).socket, move |_| status.clone());
  }

  fn quiet(_: &str) {}

  #[tokio::test]
  async fn the_service_takes_over_from_an_engine_that_predates_the_gateway() {
    let root = tempfile::tempdir().unwrap();
    let (socket, drained) = engine(root.path(), "0.0.1");
    assert_eq!(before_service(&socket, root.path(), Duration::from_secs(5), &quiet).await, None);
    assert!(drained.load(Ordering::Acquire));
  }

  #[tokio::test]
  async fn the_service_leaves_a_current_engine_only_once_it_runs_the_gateway() {
    let root = tempfile::tempdir().unwrap();
    let (socket, drained) = engine(root.path(), acpira_shared::version::VERSION);
    assert_eq!(before_service(&socket, root.path(), Duration::from_millis(600), &quiet).await, Some(1), "no gateway at all");
    gateway(root.path(), None);
    assert_eq!(before_service(&socket, root.path(), Duration::from_millis(600), &quiet).await, Some(1), "a standalone one");
    let other = tempfile::tempdir().unwrap();
    let (socket, _) = engine(other.path(), acpira_shared::version::VERSION);
    gateway(other.path(), Some(PID));
    assert_eq!(before_service(&socket, other.path(), Duration::from_secs(5), &quiet).await, Some(0));
    assert!(!drained.load(Ordering::Acquire));
  }
}
