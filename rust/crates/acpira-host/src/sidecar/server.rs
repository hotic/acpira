//! The sidecar's state machine: one engine (the runtime, created by the first hello) behind any number of wires.
//!
//! A wire is one shell connection: hello (version-checked) → views. Control messages on a wire are processed strictly in
//! order. A view's WebviewMsg runs its synchronous prefix in order and continues concurrently, exactly like
//! `void core.handle(m)` in the TS host: a `send` claims its turn before a following `stop` is looked at, and the `stop`
//! never queues behind the whole turn.
//!
//! Two lifetimes:
//! - attached (`acpira` over stdio, one harness connection): exactly one wire, and the engine ends with it — EOF or
//!   `shutdown` tear everything down, agents included;
//! - persistent (`acpira serve --socket`): wires come and go (windows reloading, laptops sleeping, VS Code quitting) while
//!   sessions keep running. A wire's EOF or `shutdown` only detaches its views; the engine ends once `idle` sees no wire
//!   and no running turn for the grace period, or on a signal.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;

use acpira_shared::protocol::HostMsg;
use acpira_shared::sidecar::{Hello, PlatformEvent, SIDECAR_PROTOCOL_MIN, SIDECAR_PROTOCOL_VERSION, ShellMsg, SidecarInfo, SidecarMsg};

use super::platform::SidecarPlatform;
use crate::bridge_core::BridgeCore;
use crate::runtime::HostRuntime;

pub struct ServerOpts {
  pub version: String,
  pub home: PathBuf,
  pub log: Arc<dyn Fn(&str) + Send + Sync>,
  /// Browser harness: the connecting page must not supply executable command lines
  pub ignore_client_agents: bool,
  /// This binary: the built-in agent and the MCP server handed to agents run as it
  pub bridge_exe: Option<String>,
}

/// One wire's outgoing side as the engine sees it
#[derive(Clone)]
struct WireOut {
  out: mpsc::UnboundedSender<String>,
  /// Raised when the wire leaves: senders handed to its views go quiet
  closed: Arc<AtomicBool>,
}

#[derive(Default)]
struct Inner {
  runtime: Option<Arc<HostRuntime>>,
  platform: Option<Arc<SidecarPlatform>>,
  /// Wires past hello, by id; the platform serves IDE actions through `active`
  wires: BTreeMap<u64, WireOut>,
  /// The wire the user touched last (hello, window focus, a view message): toasts, terminals and file opens go there.
  /// Never a peer
  active: Option<u64>,
  /// Wires whose hello said `role: peer` (another engine): see `ClientInfo::role`
  peers: std::collections::BTreeSet<u64>,
  /// `idle` decided the engine ends: hellos are refused from here on (a wire that connected just before still gets an
  /// answer, and its shell reconnects to the next engine)
  sealed: bool,
  done: bool,
}

pub struct Engine {
  opts: ServerOpts,
  persistent: bool,
  /// Serializes runtime creation: two windows saying hello at once share one runtime
  init: tokio::sync::Mutex<()>,
  inner: parking_lot::Mutex<Inner>,
  /// Raised when teardown starts: every sender handed to views and the platform goes quiet, so nothing follows shutdownOk
  closing: Arc<AtomicBool>,
  next_wire: AtomicU64,
  /// Wires coming and going wake `idle`
  changed: tokio::sync::Notify,
  /// This process's `SSH_CONNECTION`: the SSH login, and so the macOS security session, the engine was started in
  ssh_connection: Option<String>,
  /// The browser panels' preview proxies, leased per wire
  previews: Arc<crate::preview::Previews>,
  /// A stretch the runtime is out of the watchdog's sight (being created, or torn down after `finish` took it), and
  /// since when: the watchdog bounds it by `PHASE_LIMIT`
  phase: parking_lot::Mutex<Option<(&'static str, std::time::Instant)>>,
  /// Gives up the endpoint (stops accepting, removes the socket, frees its lock); set by `acpira serve`, taken once by
  /// a drain or the end
  endpoint: parking_lot::Mutex<Option<Box<dyn FnOnce() + Send>>>,
  /// The socket a successor takes over after a drain, watched before the wires are let go
  endpoint_path: parking_lot::Mutex<Option<PathBuf>>,
  /// A newer build took the machine's endpoint over (`drain`): no new wires, sessions go as soon as they can, the engine
  /// ends once its running turns have
  draining: AtomicBool,
  /// Raised once the successor answers on the endpoint: every wire closes, so its shell reconnects there
  wires_released: tokio::sync::watch::Sender<bool>,
  /// The paired devices' gateway runs in this engine (`devices::host`): it stays up for them with no window connected
  hosting: AtomicBool,
}

// The watchdog's pace, and how long it waits to take one of the engine's locks before counting the engine as wedged. These
// locks are never held across an await, but some sync work runs under them (record serialization, an image read): the
// wait leaves a slow disk ample room, at the price of a minute before a wedged engine goes
const WATCH_EVERY: Duration = Duration::from_secs(10);
const WEDGED_AFTER: Duration = Duration::from_secs(60);
// How long creating the runtime or tearing it down may take before the engine counts as wedged
const PHASE_LIMIT: Duration = Duration::from_secs(90);
const EXIT_WEDGED: i32 = 3;

/// What one shell message did to its wire
enum Flow {
  Continue,
  /// The wire is done (rejected hello, shutdown); the code is its result, and for an attached engine the process's
  Leave(i32),
}

impl Engine {
  /// The engine of one wire: it ends with that wire
  pub fn attached(opts: ServerOpts) -> Arc<Engine> {
    Self::create(opts, false)
  }

  /// The engine of `acpira serve`: wires come and go, `idle` tells when it may end
  pub fn persistent(opts: ServerOpts) -> Arc<Engine> {
    Self::create(opts, true)
  }

  fn create(opts: ServerOpts, persistent: bool) -> Arc<Engine> {
    let previews = crate::preview::Previews::new(opts.log.clone());
    Arc::new(Engine {
      previews,
      phase: Default::default(),
      opts,
      persistent,
      init: tokio::sync::Mutex::new(()),
      inner: Default::default(),
      closing: Default::default(),
      next_wire: AtomicU64::new(1),
      changed: tokio::sync::Notify::new(),
      ssh_connection: std::env::var("SSH_CONNECTION").ok().filter(|s| !s.is_empty()),
      endpoint: Default::default(),
      endpoint_path: Default::default(),
      draining: AtomicBool::new(false),
      wires_released: tokio::sync::watch::Sender::new(false),
      hosting: AtomicBool::new(false),
    })
  }

  /// A wire inside this process (the gateway's devices, `acpira_gateway::engine::Link`): lines to the engine and lines
  /// back, the way back ended by an empty line once the wire is over (a running turn's view may outlive it with a sender)
  pub fn link(self: &Arc<Self>) -> (mpsc::UnboundedSender<String>, mpsc::UnboundedReceiver<String>) {
    let (line_tx, line_rx) = mpsc::unbounded_channel();
    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (engine, end) = (self.clone(), out_tx.clone());
    tokio::spawn(async move {
      engine.serve(out_tx, line_rx).await;
      let _ = end.send(String::new());
    });
    (line_tx, out_rx)
  }

  /// The gateway started or stopped in this engine
  pub fn set_hosting(&self, on: bool) {
    self.hosting.store(on, Ordering::Release);
    self.changed.notify_waiters();
  }

  pub fn hosting(&self) -> bool {
    self.hosting.load(Ordering::Acquire)
  }

  fn log(&self, line: &str) {
    (self.opts.log)(line);
  }

  /// Wires past hello right now
  pub fn wire_count(&self) -> usize {
    self.inner.lock().wires.len()
  }

  /// Some session is mid-turn. `inner` is never held while calling into the runtime: the manager's own locks are taken
  /// first there, and its platform calls come back through `route`, which takes `inner`
  pub fn busy(&self) -> bool {
    self.runtime().is_some_and(|rt| rt.manager.busy())
  }

  fn platform(&self) -> Option<Arc<SidecarPlatform>> {
    self.inner.lock().platform.clone()
  }

  pub fn runtime(&self) -> Option<Arc<HostRuntime>> {
    self.inner.lock().runtime.clone()
  }

  /// A persistent engine that wedges logs what is stuck and exits, so the next connection of a phone or window starts a
  /// fresh engine and the leases it held come free. A thread of its own: a wedged engine's tasks may never run again,
  /// and it no longer answers hellos or ends on SIGTERM (its teardown waits on the same locks)
  pub fn watch_for_wedge(self: &Arc<Self>) {
    let weak = Arc::downgrade(self);
    let spawned = std::thread::Builder::new().name("acpira-watchdog".into()).spawn(move || {
      loop {
        std::thread::sleep(WATCH_EVERY);
        let Some(engine) = weak.upgrade() else { return };
        if let Some(what) = engine.stuck() {
          engine.log(&format!("wedged: {what}; ending its agents and exiting so the next connection starts a fresh engine"));
          // Its leases come free with it: no agent of its may go on writing a native session the next engine resumes
          crate::acp::transport::process::kill_all_hard();
          std::process::exit(EXIT_WEDGED);
        }
      }
    });
    if let Err(e) = spawned {
      self.log(&format!("watchdog did not start: {e}"));
    }
  }

  /// What keeps the engine wedged, if anything: a runtime creation or teardown past `PHASE_LIMIT`, else the first of its
  /// locks (its wires, then the session manager's) it could not take within `WEDGED_AFTER`
  fn stuck(&self) -> Option<String> {
    if let Some((what, since)) = *self.phase.lock()
      && since.elapsed() > PHASE_LIMIT
    {
      return Some(format!("{what} for over {}s", PHASE_LIMIT.as_secs()));
    }
    let runtime = match self.inner.try_lock_for(WEDGED_AFTER) {
      Some(inner) => inner.runtime.clone(),
      None => return Some(format!("the engine's wire table stayed locked for {}s", WEDGED_AFTER.as_secs())),
    };
    let what = runtime?.manager.stuck_lock(WEDGED_AFTER)?;
    Some(format!("{what} stayed locked for {}s", WEDGED_AFTER.as_secs()))
  }

  fn enter_phase(&self, what: &'static str) {
    *self.phase.lock() = Some((what, std::time::Instant::now()));
  }

  /// Serve one wire until it closes, asks to leave or the engine ends; resolves to the wire's exit code (2 = protocol
  /// rejected). For an attached engine that is also the end of the engine
  pub async fn serve(self: &Arc<Self>, out: mpsc::UnboundedSender<String>, mut lines: mpsc::UnboundedReceiver<String>) -> i32 {
    let id = self.next_wire.fetch_add(1, Ordering::Relaxed);
    let mut wire = Wire { id, peer: false, engine: self.clone(), me: WireOut { out, closed: Default::default() }, views: HashMap::new() };
    let mut released = self.wires_released.subscribe();
    let code = loop {
      let next = tokio::select! {
        line = lines.recv() => line,
        // A drained engine lets every wire go once its successor listens: the shell reconnects there
        _ = async { drop(released.wait_for(|r| *r).await) } => break self.wire_ended(&mut wire, "the engine handed over to a newer build", 0, false).await,
      };
      let Some(line) = next else {
        // Lines received before EOF were all honoured above
        break self.wire_ended(&mut wire, "wire closed", 0, false).await;
      };
      if let Flow::Leave(code) = wire.on_line(&line).await {
        break code;
      }
      if self.inner.lock().done {
        break 0;
      }
    };
    wire.me.closed.store(true, Ordering::Release);
    code
  }

  /// The wire is gone (EOF) or leaving (`ack`: it asked with shutdown). An attached engine ends with it; a persistent one
  /// only lets go of its views
  async fn wire_ended(self: &Arc<Self>, wire: &mut Wire, reason: &str, code: i32, ack: bool) -> i32 {
    if !self.persistent {
      return self.finish(reason, code, ack.then(|| wire.me.clone()), &mut wire.views).await;
    }
    self.leave(wire, reason);
    if ack {
      // Written after the views went quiet, so shutdownOk is the last envelope this wire gets
      write(&wire.me.out, &SidecarMsg::ShutdownOk);
    }
    code
  }

  /// A persistent engine's wire detaches: its views stop receiving and IDE actions move to the newest remaining wire; the
  /// sessions stay open (and leased) until the engine ends
  fn leave(&self, wire: &mut Wire, reason: &str) {
    wire.me.closed.store(true, Ordering::Release);
    let (runtime, platform, moved, joined) = {
      let mut inner = self.inner.lock();
      let joined = inner.wires.remove(&wire.id).is_some();
      inner.peers.remove(&wire.id);
      let mut moved = None;
      if inner.active == Some(wire.id) {
        let peers = inner.peers.clone();
        inner.active = inner.wires.keys().rev().find(|id| !peers.contains(id)).copied();
        // Switched under the wire table so the platform's active window never disagrees with `route`'s
        moved = inner.platform.as_ref().map(|p| p.set_active(inner.active));
      }
      (inner.runtime.clone(), inner.platform.clone(), moved, joined)
    };
    if let Some(rt) = &runtime {
      for (_, core) in wire.views.drain() {
        rt.detach_view(&core);
      }
    }
    if let Some(p) = &platform {
      p.drop_wire(wire.id);
      p.fail_wire(wire.id, reason);
      if let Some(moved) = moved {
        p.announce(moved);
      }
    }
    self.previews.release_wire(wire.id);
    // A probe or drain asks and goes without ever joining
    if joined {
      self.log(&format!("wire {} left ({reason}); {} still connected", wire.id, self.wire_count()));
    }
    self.changed.notify_waiters();
  }

  /// Resolves once a persistent engine has had no wire and no running turn for `grace` in a row (or has ended); keeps
  /// session leases current meanwhile. The caller stops taking connections, then calls `stop`
  pub async fn idle(self: &Arc<Self>, grace: Duration) {
    let mut idle_since: Option<tokio::time::Instant> = None;
    loop {
      // Woken early when a wire comes or goes; a turn ending is caught by the next tick
      let _ = tokio::time::timeout(Duration::from_secs(1), self.changed.notified()).await;
      {
        let inner = self.inner.lock();
        // Sealed by a hello that retired this engine (`retires_for`)
        if inner.done || inner.sealed {
          return;
        }
      }
      let draining = self.draining();
      if let Some(rt) = self.runtime() {
        rt.manager.sync_leases();
        // Sessions nobody can be shown here any more go now, so the successor opens them; running ones when they end
        if draining {
          rt.manager.release_idle(Duration::ZERO, 0).await;
        }
      }
      let grace = if draining { Duration::ZERO } else { grace };
      let wires = if draining && *self.wires_released.borrow() { 0 } else { self.wire_count() };
      // A handed-over engine ends with no window, so it waits for every unfinished piece of work, not just turns. One
      // running the gateway waits for its devices, which may connect at any time
      let busy = |e: &Self| e.busy() || e.hosting() || (draining && e.runtime().is_some_and(|rt| rt.manager.in_flight()));
      if wires > 0 || busy(self) {
        idle_since = None;
        continue;
      }
      if idle_since.get_or_insert_with(tokio::time::Instant::now).elapsed() >= grace {
        // Decided under `init`: a hello holds it from its check until its wire is registered, so no window can slip in
        // between this last look and the seal
        let _init = self.init.lock().await;
        if !busy(self) {
          let mut inner = self.inner.lock();
          if inner.wires.is_empty() || (draining && *self.wires_released.borrow()) {
            inner.sealed = true;
            return;
          }
        }
        idle_since = None;
      }
    }
  }

  /// End the engine now (a signal): every session closes and its agent ends
  pub async fn stop(&self, reason: &str) -> i32 {
    self.finish(reason, 0, None, &mut HashMap::new()).await
  }

  /// `acpira serve` hands over how to give up its endpoint (`path`: the Unix socket a successor binds)
  pub fn set_endpoint(&self, path: Option<PathBuf>, release: Box<dyn FnOnce() + Send>) {
    *self.endpoint_path.lock() = path;
    *self.endpoint.lock() = Some(release);
  }

  /// Gives up the endpoint now, if it still has it
  pub fn release_endpoint(&self) {
    let release = self.endpoint.lock().take();
    if let Some(r) = release {
      r();
    }
  }

  pub fn draining(&self) -> bool {
    self.draining.load(Ordering::Acquire)
  }

  /// A newer build (`machine_engine::ensure`) takes the machine's endpoint over. Refused unless it is newer by the same
  /// rule the asker applied (`policy::takes_over`), so a confused older client cannot end a newer engine. The endpoint
  /// goes at once; the wires go once the successor answers there (so a shell reconnecting meanwhile does not start
  /// another engine), and the engine ends when its running turns have (`idle`)
  pub fn drain(self: &Arc<Self>, successor: &str, allow_dev: bool) -> Result<(), String> {
    if !self.persistent {
      return Err("this engine is not the machine's".into());
    }
    if !crate::machine_engine::policy::takes_over(successor, &self.opts.version, allow_dev) {
      return Err(format!("{successor} does not replace {}", self.opts.version));
    }
    if self.draining.swap(true, Ordering::AcqRel) {
      return Ok(());
    }
    self.log(&format!("draining: {successor} takes the endpoint over"));
    self.release_endpoint();
    let (engine, path) = (self.clone(), self.endpoint_path.lock().clone());
    tokio::spawn(async move {
      // The successor starts within a second; past the wait the wires go anyway and their shells start what they carry
      let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
      while let Some(p) = &path
        && tokio::time::Instant::now() < deadline
      {
        if crate::machine_engine::ensure::probe(p).await.ok().flatten().is_some_and(|a| a.pid != std::process::id()) {
          break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
      }
      engine.log("draining: letting the wires go to the successor");
      engine.wires_released.send_replace(true);
      engine.changed.notify_waiters();
    });
    self.changed.notify_waiters();
    Ok(())
  }

  /// Gives the machine up with no successor in sight (its launcher is gone, `machine_engine::launcher::watch`): the
  /// endpoint and every wire go now, so each shell reconnects and starts a fresh engine, and this one ends once its
  /// work has (`idle`)
  pub fn retire(&self, reason: &str) {
    if !self.persistent || self.draining.swap(true, Ordering::AcqRel) {
      return;
    }
    self.log(&format!("draining: {reason}"));
    self.release_endpoint();
    self.wires_released.send_replace(true);
    self.changed.notify_waiters();
  }

  /// Tear everything down once: pending platform RPCs fail, sessions flush and their agents end, `ack` gets shutdownOk
  async fn finish(&self, reason: &str, code: i32, ack: Option<WireOut>, views: &mut HashMap<String, Arc<BridgeCore>>) -> i32 {
    // A hello creating the runtime right now finishes first, so the runtime it creates is the one disposed below
    let _init = self.init.lock().await;
    let (runtime, platform) = {
      let mut inner = self.inner.lock();
      if inner.done {
        return code;
      }
      // Refuses wires that say hello from here on
      inner.done = true;
      // From here the runtime is out of `inner`: the watchdog bounds the teardown by time instead
      self.enter_phase("tearing down");
      (inner.runtime.take(), inner.platform.clone())
    };
    self.log(&format!("sidecar finishing: {reason}"));
    self.closing.store(true, Ordering::Release);
    self.previews.close_all();
    if let Some(p) = &platform {
      p.dispose(reason);
    }
    views.clear();
    if let Some(rt) = runtime {
      rt.dispose().await;
    }
    if let Some(w) = ack {
      write(&w.out, &SidecarMsg::ShutdownOk);
    }
    self.changed.notify_waiters();
    code
  }

  /// A wire's sender: quiet once the engine closes or the wire leaves
  fn sender_of(&self, w: &WireOut) -> Arc<dyn Fn(SidecarMsg) + Send + Sync> {
    let (out, closed, closing) = (w.out.clone(), w.closed.clone(), self.closing.clone());
    Arc::new(move |m: SidecarMsg| {
      if !closing.load(Ordering::Acquire) && !closed.load(Ordering::Acquire) {
        write(&out, &m);
      }
    })
  }

  /// The platform's way out: the wire (window or peer) whose view caused the request, else the active window, else the
  /// newest peer (a host only peers use); none while nothing is connected
  fn route(self: &Arc<Self>) -> super::platform::Route {
    let weak = Arc::downgrade(self);
    Arc::new(move |m: SidecarMsg| {
      let engine = weak.upgrade()?;
      if engine.closing.load(Ordering::Acquire) {
        return None;
      }
      let (id, w) = {
        let inner = engine.inner.lock();
        let origin = super::platform::origin().filter(|o| inner.wires.contains_key(o));
        let id = origin.or(inner.active).or_else(|| inner.peers.iter().next_back().copied())?;
        (id, inner.wires.get(&id)?.clone())
      };
      if w.closed.load(Ordering::Acquire) {
        return None;
      }
      write(&w.out, &m);
      Some(id)
    })
  }

  fn activate(&self, wire: u64) {
    let moved = {
      let mut inner = self.inner.lock();
      if !inner.wires.contains_key(&wire) || inner.peers.contains(&wire) || inner.active == Some(wire) {
        return;
      }
      inner.active = Some(wire);
      // Switched under the wire table, announced after it: the listeners call back through `route`
      inner.platform.as_ref().map(|p| (p.clone(), p.set_active(Some(wire))))
    };
    if let Some((p, moved)) = moved {
      p.announce(moved);
    }
  }

  /// A window from another SSH login (or a peer on macOS when the user's `gui/<uid>` session can run the engine instead
  /// of an SSH session) joins a persistent engine whose keychain is locked. Unlock state belongs to the security
  /// session: with no other wire on it and no turn running it retires, so the next connection starts the engine in the
  /// window's or `gui/<uid>`'s session. Called under `init`, before the wire joins.
  /// Observed 2026-10-10 on acpira 1.9.0 over Remote-SSH: an engine left by an earlier connection stayed locked while
  /// the window's terminal unlocked the keychain, and the unlock notice waited out its 5 minutes
  async fn retires_for(&self, hello: &Hello, rt: &HostRuntime) -> bool {
    // Retiring would take the gateway, and with it every device, down with nothing to start it again
    if !self.persistent || self.hosting() {
      return false;
    }
    let another_session = if hello.client.is_peer() {
      self.ssh_connection.is_some() && crate::devices::launch_agent::can_use_gui(&self.opts.home).await
    } else {
      other_login(self.ssh_connection.as_deref(), hello.client.ssh_connection.as_deref())
    };
    if !another_session || !self.inner.lock().wires.is_empty() || self.busy() || !rt.credentials_locked().await {
      return false;
    }
    let mut inner = self.inner.lock();
    // A wire that joined meanwhile keeps the engine
    if !inner.wires.is_empty() {
      return false;
    }
    inner.sealed = true;
    drop(inner);
    self.log("retiring: a window or peer joined while the keychain is locked for this engine's SSH login");
    self.changed.notify_waiters();
    true
  }

  fn hello_ok(&self, request_id: String, rt: &HostRuntime) -> SidecarMsg {
    SidecarMsg::HelloOk {
      request_id,
      protocol_version: SIDECAR_PROTOCOL_VERSION,
      sidecar: SidecarInfo { version: self.opts.version.clone(), pid: std::process::id() },
      sessions_dir: rt.sessions_dir.to_string_lossy().into_owned(),
      features: vec![
        "preview".into(),
        "projects".into(),
        "remote".into(),
        "plugins".into(),
        "remoteAccess".into(),
        // A paired device may browse this computer's folders (`localDir`) and edit the built-in agent's model sources
        "localDir".into(),
        "providers".into(),
      ],
      safe_mode: rt.plugins.loaded().safe_mode,
    }
  }
}

struct Wire {
  id: u64,
  /// Its hello said `role: peer`
  peer: bool,
  engine: Arc<Engine>,
  me: WireOut,
  views: HashMap<String, Arc<BridgeCore>>,
}

impl Wire {
  fn send(&self, m: SidecarMsg) {
    (self.engine.sender_of(&self.me))(m);
  }

  fn log(&self, line: &str) {
    self.engine.log(line);
  }

  /// A line that is not a JSON envelope is noise on the channel: logged and skipped, never fatal
  async fn on_line(&mut self, line: &str) -> Flow {
    let text = line.trim();
    if text.is_empty() {
      return Flow::Continue;
    }
    let head: String = text.chars().take(120).collect();
    let parsed: Value = match serde_json::from_str(text) {
      Ok(v) => v,
      Err(_) => {
        self.log(&format!("ignoring non-JSON line from the shell: {head}"));
        return Flow::Continue;
      }
    };
    let kind = parsed.get("type").and_then(Value::as_str).map(str::to_owned);
    let Some(kind) = kind else {
      self.log(&format!("ignoring envelope without a type: {head}"));
      return Flow::Continue;
    };
    let m: ShellMsg = match serde_json::from_value(parsed) {
      Ok(m) => m,
      Err(e) => {
        self.log(&format!("{kind} failed: {e}"));
        return Flow::Continue;
      }
    };
    self.handle(m).await
  }

  async fn handle(&mut self, m: ShellMsg) -> Flow {
    match m {
      ShellMsg::Hello(h) => return self.hello(*h).await,
      ShellMsg::Shutdown => {
        let engine = self.engine.clone();
        return Flow::Leave(engine.wire_ended(self, "shutdown requested", 0, true).await);
      }
      // `acpira engine`: answered without joining, so asking never starts the runtime or counts as a window
      ShellMsg::Probe { request_id } => {
        self.send(SidecarMsg::ProbeOk {
          request_id,
          version: self.engine.opts.version.clone(),
          pid: std::process::id(),
          protocol_version: SIDECAR_PROTOCOL_VERSION,
          protocol_min: SIDECAR_PROTOCOL_MIN,
          // Also once it is ending (idle, or retired): a shell must not hand out a socket about to go away
          draining: self.engine.draining() || {
            let inner = self.engine.inner.lock();
            inner.done || inner.sealed
          },
        });
        return Flow::Continue;
      }
      ShellMsg::Drain { request_id, version, allow_dev } => {
        self.send(match self.engine.drain(&version, allow_dev) {
          Ok(()) => SidecarMsg::DrainOk { request_id },
          Err(reason) => SidecarMsg::DrainRefused { request_id, reason },
        });
        return Flow::Continue;
      }
      _ => {}
    }
    let joined = self.engine.inner.lock().wires.contains_key(&self.id);
    let (Some(runtime), Some(platform)) = (self.engine.runtime(), self.engine.platform()) else {
      self.log(&format!("{} before hello, ignored", shell_kind(&m)));
      return Flow::Continue;
    };
    if !joined {
      self.log(&format!("{} before hello, ignored", shell_kind(&m)));
      return Flow::Continue;
    }
    match m {
      ShellMsg::AttachView { view_id, host, initial, blob_base } => {
        if self.views.contains_key(&view_id) {
          self.log(&format!("attachView: {view_id} already attached, ignored"));
          return Flow::Continue;
        }
        let send = self.engine.sender_of(&self.me);
        let vid = view_id.clone();
        let post = Arc::new(move |message: HostMsg| send(SidecarMsg::HostMessage { view_id: vid.clone(), message }));
        // The view reads its own window's facts (workspace, project mode, session scope, blob base)
        let core = runtime.attach_view(host, initial, blob_base.or_else(|| platform.blob_base_of(Some(self.id))), post, Some(self.id));
        self.views.insert(view_id, core);
      }
      ShellMsg::DetachView { view_id } => {
        if let Some(core) = self.views.remove(&view_id) {
          runtime.detach_view(&core);
        }
      }
      ShellMsg::SetSessionList { view_id, scope } => {
        if let Some(core) = self.views.get(&view_id) {
          runtime.manager.set_session_list(&core.viewer, scope);
        }
      }
      ShellMsg::WebviewMessage { view_id, message } => match self.views.get(&view_id) {
        Some(core) => {
          // The window the user is typing in serves the IDE actions that follow (file opens, terminals)
          if !self.peer {
            self.engine.activate(self.id);
          }
          // The platform requests its handling makes go back to this wire, under its own capabilities
          crate::util::run_prefix(super::platform::ORIGIN.scope(self.id, core.clone().handle(message)))
        }
        None => {
          let kind = message.get("type").and_then(Value::as_str).unwrap_or("undefined");
          self.log(&format!("webviewMessage for unknown view {view_id} ({kind}), ignored"));
        }
      },
      ShellMsg::PlatformResponse { request_id, result, error } => platform.on_response(&request_id, result, error),
      ShellMsg::PreviewOpen { request_id, panel_id, url, picker, folder } => {
        // Answered on its own task: binding and reading the script must not hold up the wire's next message
        // The open takes its place in line here, so a close or a later open sent after it wins even if this one binds last
        // A tab of a session in a folder on an SSH host shows that host's dev server
        let remote = folder
          .as_deref()
          .and_then(acpira_shared::remote::parse_remote_folder)
          .and_then(|f| self.engine.runtime().map(|rt| (f.alias, rt.connections.clone())));
        let (opening, send) = (self.engine.previews.open(self.id, &panel_id, &url, picker, remote), self.engine.sender_of(&self.me));
        tokio::spawn(async move {
          let reply = match opening.await {
            Ok(o) => SidecarMsg::PreviewOpened { request_id, url: Some(o.url), token: Some(o.token), error: None },
            Err(e) => SidecarMsg::PreviewOpened { request_id, url: None, token: None, error: Some(e) },
          };
          send(reply);
        });
      }
      ShellMsg::PreviewClose { panel_id } => self.engine.previews.close(self.id, &panel_id),
      // A peer's focus, settings and environment are its own machine's, never this one's
      ShellMsg::PlatformEvent { .. } if self.peer => {}
      ShellMsg::PlatformEvent { event } => {
        if matches!(event, PlatformEvent::WindowFocus) {
          self.engine.activate(self.id);
        }
        platform.on_wire_event(Some(self.id), event)
      }
      // Both returned at the top of this function
      ShellMsg::Hello(_) | ShellMsg::Shutdown | ShellMsg::Probe { .. } | ShellMsg::Drain { .. } => {}
    }
    Flow::Continue
  }

  async fn hello(&mut self, m: Hello) -> Flow {
    let engine = self.engine.clone();
    // Every protocol from the oldest still served to this engine's own: changes in between are additive
    if !(SIDECAR_PROTOCOL_MIN..=SIDECAR_PROTOCOL_VERSION).contains(&m.protocol_version) {
      self.send(SidecarMsg::HelloReject {
        request_id: m.request_id,
        protocol_version: SIDECAR_PROTOCOL_VERSION,
        reason: format!("protocol version {} is outside {SIDECAR_PROTOCOL_MIN}..={SIDECAR_PROTOCOL_VERSION}", m.protocol_version),
      });
      let reason = format!("protocol version mismatch ({})", m.protocol_version);
      return Flow::Leave(engine.wire_ended(self, &reason, 2, false).await);
    }
    if engine.inner.lock().wires.contains_key(&self.id) {
      // A second hello on the same wire is a shell bug, but a harmless one
      if let Some(rt) = engine.runtime() {
        self.send(engine.hello_ok(m.request_id, &rt));
      }
      return Flow::Continue;
    }
    let created = {
      // Held until the wire is registered: `idle` seals the engine under the same lock, and `finish` waits for it
      let _init = engine.init.lock().await;
      let refused = {
        let inner = engine.inner.lock();
        // A draining engine takes no new window: the shell reconnects on "shutting down" and reaches the successor
        inner.done || inner.sealed || engine.draining()
      };
      if refused {
        self.send(SidecarMsg::HelloReject {
          request_id: m.request_id,
          protocol_version: SIDECAR_PROTOCOL_VERSION,
          reason: "the engine is shutting down".into(),
        });
        return Flow::Leave(1);
      }
      let created = match engine.runtime() {
        Some(rt) if engine.retires_for(&m, &rt).await => {
          // The shell reconnects on "shutting down" and launches the next engine once this one has gone
          self.send(SidecarMsg::HelloReject {
            request_id: m.request_id,
            protocol_version: SIDECAR_PROTOCOL_VERSION,
            reason: "the engine is shutting down (keychain locked for its SSH login)".into(),
          });
          return Flow::Leave(1);
        }
        Some(rt) => {
          // A later window joins the running engine with facts of its own, current once it is made active below. A peer
          // changes nothing
          let platform = engine.platform();
          if let Some(p) = platform {
            if m.client.is_peer() {
              p.add_peer(self.id, &m);
            } else {
              p.add_window(self.id, &m);
            }
          }
          Ok(rt)
        }
        None => {
          // A peer first: the platform starts from its hello (until a window is active), but the peer stays a peer
          let platform = SidecarPlatform::routed(engine.route(), &m, engine.opts.log.clone(), engine.opts.ignore_client_agents);
          if m.client.is_peer() {
            platform.add_peer(self.id, &m);
          } else {
            platform.add_window(self.id, &m);
          }
          engine.inner.lock().platform = Some(platform.clone());
          engine.enter_phase("creating its runtime");
          let created = HostRuntime::create(platform, engine.opts.home.clone(), engine.opts.bridge_exe.clone()).await;
          *engine.phase.lock() = None;
          match created {
            Ok(rt) => {
              engine.inner.lock().runtime = Some(rt.clone());
              Ok(rt)
            }
            Err(e) => Err(e),
          }
        }
      };
      if created.is_ok() {
        let moved = {
          let mut inner = engine.inner.lock();
          inner.wires.insert(self.id, self.me.clone());
          if m.client.is_peer() {
            inner.peers.insert(self.id);
            self.peer = true;
            None
          } else {
            inner.active = Some(self.id);
            // Listeners hear about every key whose value differs from the previous window's (the settings may also have
            // changed while no shell was connected); they call back into the platform, which takes `inner`
            inner.platform.as_ref().map(|p| (p.clone(), p.set_active(Some(self.id))))
          }
        };
        if let Some((p, moved)) = moved {
          p.announce(moved);
        }
      }
      created
    };
    match created {
      Ok(rt) => {
        engine.changed.notify_waiters();
        if engine.persistent {
          let role = if self.peer { " (peer)" } else { "" };
          engine.log(&format!("wire {}{role} joined; {} connected", self.id, engine.wire_count()));
        }
        self.send(engine.hello_ok(m.request_id, &rt));
        Flow::Continue
      }
      Err(e) => {
        self.send(SidecarMsg::HelloReject {
          request_id: m.request_id,
          protocol_version: SIDECAR_PROTOCOL_VERSION,
          reason: format!("runtime failed to start: {e}"),
        });
        // A persistent engine without a runtime has nothing to keep: the next hello tries again, `idle` ends it otherwise
        engine.inner.lock().platform = None;
        Flow::Leave(engine.wire_ended(self, &format!("runtime failed: {e}"), 1, false).await)
      }
    }
  }
}

/// The window and the engine run under two different SSH logins, so in two security sessions. A side that reports no
/// login (a local window, an older shell, a test) never counts as another one
fn other_login(engine: Option<&str>, window: Option<&str>) -> bool {
  matches!((engine, window), (Some(e), Some(w)) if e != w)
}

fn write(out: &mpsc::UnboundedSender<String>, m: &SidecarMsg) {
  if let Ok(line) = serde_json::to_string(m) {
    let _ = out.send(line);
  }
}

fn shell_kind(m: &ShellMsg) -> &'static str {
  match m {
    ShellMsg::Hello(_) => "hello",
    ShellMsg::AttachView { .. } => "attachView",
    ShellMsg::DetachView { .. } => "detachView",
    ShellMsg::SetSessionList { .. } => "setSessionList",
    ShellMsg::WebviewMessage { .. } => "webviewMessage",
    ShellMsg::PlatformResponse { .. } => "platformResponse",
    ShellMsg::PlatformEvent { .. } => "platformEvent",
    ShellMsg::PreviewOpen { .. } => "previewOpen",
    ShellMsg::PreviewClose { .. } => "previewClose",
    ShellMsg::Shutdown => "shutdown",
    ShellMsg::Probe { .. } => "probe",
    ShellMsg::Drain { .. } => "drain",
  }
}

#[cfg(test)]
mod tests {
  use super::other_login;

  #[test]
  fn only_two_different_ssh_logins_count_as_another_login() {
    let (a, b) = (Some("10.0.0.2 52502 10.0.0.1 22"), Some("10.0.0.2 61822 10.0.0.1 22"));
    assert!(other_login(a, b));
    assert!(!other_login(a, a));
    assert!(!other_login(a, None));
    assert!(!other_login(None, b));
    assert!(!other_login(None, None));
  }
}
