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
use acpira_shared::sidecar::{Hello, PlatformEvent, SIDECAR_PROTOCOL_VERSION, ShellMsg, SidecarInfo, SidecarMsg};

use super::platform::SidecarPlatform;
use crate::bridge_core::BridgeCore;
use crate::runtime::HostRuntime;

pub struct ServerOpts {
  pub version: String,
  pub home: PathBuf,
  pub log: Arc<dyn Fn(&str) + Send + Sync>,
  /// Browser harness: the connecting page must not supply executable command lines
  pub ignore_client_agents: bool,
  /// The executable the ChatGPT connection prompt names (this binary)
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
  /// The wire the user touched last (hello, window focus, a view message): toasts, terminals and file opens go there
  active: Option<u64>,
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
}

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
    Arc::new(Engine {
      opts,
      persistent,
      init: tokio::sync::Mutex::new(()),
      inner: Default::default(),
      closing: Default::default(),
      next_wire: AtomicU64::new(1),
      changed: tokio::sync::Notify::new(),
    })
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

  /// Serve one wire until it closes, asks to leave or the engine ends; resolves to the wire's exit code (2 = protocol
  /// rejected). For an attached engine that is also the end of the engine
  pub async fn serve(self: &Arc<Self>, out: mpsc::UnboundedSender<String>, mut lines: mpsc::UnboundedReceiver<String>) -> i32 {
    let id = self.next_wire.fetch_add(1, Ordering::Relaxed);
    let mut wire = Wire { id, engine: self.clone(), me: WireOut { out, closed: Default::default() }, views: HashMap::new() };
    let code = loop {
      let Some(line) = lines.recv().await else {
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
    let (runtime, platform) = {
      let mut inner = self.inner.lock();
      inner.wires.remove(&wire.id);
      if inner.active == Some(wire.id) {
        inner.active = inner.wires.keys().next_back().copied();
      }
      (inner.runtime.clone(), inner.platform.clone())
    };
    if let Some(rt) = &runtime {
      for (_, core) in wire.views.drain() {
        rt.detach_view(&core);
      }
    }
    if let Some(p) = &platform {
      p.fail_wire(wire.id, reason);
    }
    self.log(&format!("wire {} left ({reason}); {} still connected", wire.id, self.wire_count()));
    self.changed.notify_waiters();
  }

  /// Resolves once a persistent engine has had no wire and no running turn for `grace` in a row (or has ended); keeps
  /// session leases current meanwhile. The caller stops taking connections, then calls `stop`
  pub async fn idle(self: &Arc<Self>, grace: Duration) {
    let mut idle_since: Option<tokio::time::Instant> = None;
    loop {
      // Woken early when a wire comes or goes; a turn ending is caught by the next tick
      let _ = tokio::time::timeout(Duration::from_secs(1), self.changed.notified()).await;
      if self.inner.lock().done {
        return;
      }
      if let Some(rt) = self.runtime() {
        rt.manager.sync_leases();
      }
      if self.wire_count() > 0 || self.busy() {
        idle_since = None;
        continue;
      }
      if idle_since.get_or_insert_with(tokio::time::Instant::now).elapsed() >= grace {
        // Decided under `init`: a hello holds it from its check until its wire is registered, so no window can slip in
        // between this last look and the seal
        let _init = self.init.lock().await;
        if !self.busy() {
          let mut inner = self.inner.lock();
          if inner.wires.is_empty() {
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
      (inner.runtime.take(), inner.platform.clone())
    };
    self.log(&format!("sidecar finishing: {reason}"));
    self.closing.store(true, Ordering::Release);
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

  /// The platform's way out: the active wire, or none while no shell is connected
  fn route(self: &Arc<Self>) -> super::platform::Route {
    let weak = Arc::downgrade(self);
    Arc::new(move |m: SidecarMsg| {
      let engine = weak.upgrade()?;
      if engine.closing.load(Ordering::Acquire) {
        return None;
      }
      let (id, w) = {
        let inner = engine.inner.lock();
        let id = inner.active?;
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
    let mut inner = self.inner.lock();
    if inner.wires.contains_key(&wire) {
      inner.active = Some(wire);
    }
  }

  fn hello_ok(&self, request_id: String, sessions_dir: String) -> SidecarMsg {
    SidecarMsg::HelloOk {
      request_id,
      protocol_version: SIDECAR_PROTOCOL_VERSION,
      sidecar: SidecarInfo { version: self.opts.version.clone(), pid: std::process::id() },
      sessions_dir,
    }
  }
}

struct Wire {
  id: u64,
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
        let core = runtime.attach_view(host, initial, blob_base.or_else(|| platform.blob_base()), post);
        self.views.insert(view_id, core);
      }
      ShellMsg::DetachView { view_id } => {
        if let Some(core) = self.views.remove(&view_id) {
          runtime.detach_view(&core);
        }
      }
      ShellMsg::WebviewMessage { view_id, message } => match self.views.get(&view_id) {
        Some(core) => {
          // The window the user is typing in serves the IDE actions that follow (file opens, terminals)
          self.engine.activate(self.id);
          crate::util::run_prefix(core.clone().handle(message))
        }
        None => {
          let kind = message.get("type").and_then(Value::as_str).unwrap_or("undefined");
          self.log(&format!("webviewMessage for unknown view {view_id} ({kind}), ignored"));
        }
      },
      ShellMsg::PlatformResponse { request_id, result, error } => platform.on_response(&request_id, result, error),
      ShellMsg::PlatformEvent { event } => {
        if matches!(event, PlatformEvent::WindowFocus) {
          self.engine.activate(self.id);
        }
        platform.on_event(event)
      }
      ShellMsg::Hello(_) | ShellMsg::Shutdown => unreachable!("handled above"),
    }
    Flow::Continue
  }

  async fn hello(&mut self, m: Hello) -> Flow {
    let engine = self.engine.clone();
    if m.protocol_version != SIDECAR_PROTOCOL_VERSION {
      self.send(SidecarMsg::HelloReject {
        request_id: m.request_id,
        protocol_version: SIDECAR_PROTOCOL_VERSION,
        reason: format!("protocol version {} is not {SIDECAR_PROTOCOL_VERSION}", m.protocol_version),
      });
      let reason = format!("protocol version mismatch ({})", m.protocol_version);
      return Flow::Leave(engine.wire_ended(self, &reason, 2, false).await);
    }
    if engine.inner.lock().wires.contains_key(&self.id) {
      // A second hello on the same wire is a shell bug, but a harmless one
      if let Some(rt) = engine.runtime() {
        self.send(engine.hello_ok(m.request_id, rt.sessions_dir.to_string_lossy().into_owned()));
      }
      return Flow::Continue;
    }
    let created = {
      // Held until the wire is registered: `idle` seals the engine under the same lock, and `finish` waits for it
      let _init = engine.init.lock().await;
      let refused = {
        let inner = engine.inner.lock();
        inner.done || inner.sealed
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
        Some(rt) => {
          // A later window joins the running engine: its environment and settings snapshot become current. Bound first:
          // the change handlers it fires may call back into the platform, which takes `inner`
          let platform = engine.platform();
          if let Some(p) = platform {
            p.rebind(&m);
          }
          Ok(rt)
        }
        None => {
          let platform = SidecarPlatform::routed(engine.route(), &m, engine.opts.log.clone(), engine.opts.ignore_client_agents);
          engine.inner.lock().platform = Some(platform.clone());
          match HostRuntime::create(platform, engine.opts.home.clone(), engine.opts.bridge_exe.clone()).await {
            Ok(rt) => {
              engine.inner.lock().runtime = Some(rt.clone());
              Ok(rt)
            }
            Err(e) => Err(e),
          }
        }
      };
      if created.is_ok() {
        let mut inner = engine.inner.lock();
        inner.wires.insert(self.id, self.me.clone());
        inner.active = Some(self.id);
      }
      created
    };
    match created {
      Ok(rt) => {
        engine.changed.notify_waiters();
        if engine.persistent {
          engine.log(&format!("wire {} joined; {} connected", self.id, engine.wire_count()));
        }
        self.send(engine.hello_ok(m.request_id, rt.sessions_dir.to_string_lossy().into_owned()));
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
    ShellMsg::WebviewMessage { .. } => "webviewMessage",
    ShellMsg::PlatformResponse { .. } => "platformResponse",
    ShellMsg::PlatformEvent { .. } => "platformEvent",
    ShellMsg::Shutdown => "shutdown",
  }
}
