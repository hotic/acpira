//! The shell side of the envelope protocol for the desktop app: connects to the persistent engine (launching it when
//! nothing listens) or spawns it over stdio, handshakes, relays view traffic and platform requests, and reconnects with
//! backoff. Envelopes sent before helloOk wait in an outbox; a reconnect sends hello again and every attachView on the
//! session each view was showing. Each process or connection has a generation, so a replaced one's late output and exit
//! cannot touch its successor. Mirror of packages/engine-client/SidecarClient.ts, which the VS Code
//! extension uses

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use acpira_shared::sidecar::PlatformRequest;
use serde_json::value::RawValue;
use serde_json::{Map, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use crate::engine::EngineEndpoint;
use crate::locator::SidecarCommand;
use crate::wire::{HostMessage, Incoming, Initial, Outgoing, SIDECAR_PROTOCOL_VERSION};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidecarState {
  Starting,
  Ready,
  Failed,
  Stopped,
}

/// What hello carries besides its framing, read at every (re)start so a new engine gets the current settings
#[derive(Debug, Clone, Default)]
pub struct HelloPayload {
  pub client: Value,
  pub env: Value,
  pub settings: Map<String, Value>,
}

/// A view the shell renders (a page); its messages go out through `send`, the engine's host messages come back here
pub trait ShellView: Send + Sync {
  fn view_id(&self) -> &str;
  fn host(&self) -> &str;
  fn initial(&self) -> Option<Initial>;
  /// Where this view loads attachment blobs from; sent with every attach, so a re-attach keeps its own
  fn blob_base(&self) -> Option<String>;
  fn on_host_message(&self, m: HostMessage);
  /// Called once on attach with the current state, then on every change; a view that already initialized reloads on a
  /// later Ready
  fn on_state(&self, state: SidecarState, detail: Option<&str>);
}

pub type RequestFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;
pub type StateListener = Box<dyn Fn(SidecarState, Option<&str>) + Send + Sync>;
pub type EngineFor = Box<dyn Fn(&SidecarCommand) -> Option<EngineEndpoint> + Send + Sync>;

pub struct ClientOpts {
  /// Candidates in preference order: one that cannot be started, or ends before helloOk, hands over to the next
  pub commands: Box<dyn Fn() -> Vec<SidecarCommand> + Send + Sync>,
  pub cwd: Box<dyn Fn() -> Option<PathBuf> + Send + Sync>,
  pub hello: Box<dyn Fn() -> HelloPayload + Send + Sync>,
  /// IDE actions the engine asks for; the result (or error) answers RPC methods, notifications ignore it
  pub on_request: Box<dyn Fn(PlatformRequest) -> RequestFuture + Send + Sync>,
  pub log: Log,
  pub on_state: StateListener,
  /// Delay before the nth consecutive restart
  pub backoff: Box<dyn Fn(u32) -> Duration + Send + Sync>,
  /// The persistent engine for a command: connect to its socket (launching it detached when nothing listens) instead of
  /// spawning a child over stdio. None: a child over stdio (Windows)
  pub engine: EngineFor,
}

impl ClientOpts {
  /// Defaults for everything but the commands, hello and requests: no cwd, no state listener, the usual backoff, stdio
  pub fn new(
    commands: impl Fn() -> Vec<SidecarCommand> + Send + Sync + 'static,
    hello: impl Fn() -> HelloPayload + Send + Sync + 'static,
    on_request: impl Fn(PlatformRequest) -> RequestFuture + Send + Sync + 'static,
    log: Log,
  ) -> ClientOpts {
    ClientOpts {
      commands: Box::new(commands),
      cwd: Box::new(|| None),
      hello: Box::new(hello),
      on_request: Box::new(on_request),
      log,
      on_state: Box::new(|_, _| {}),
      backoff: Box::new(|n| Duration::from_millis((1000u64 << n.saturating_sub(1).min(5)).min(30_000))),
      engine: Box::new(|_| None),
    }
  }
}

/// The proxied page and its picker token, or why it cannot be shown
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewAnswer {
  Opened { url: String, token: String },
  Error(String),
}

/// A preview the shell keeps open: asked for again after every reconnect (the engine and its ports may be new)
pub struct PreviewLease {
  pub panel_id: String,
  pub url: String,
  pub picker: String,
  /// A session folder on an SSH host: the page is that host's dev server
  pub folder: Option<String>,
  pub on_opened: Box<dyn Fn(PreviewAnswer) + Send + Sync>,
}

const MAX_RESTARTS: u32 = 5;
/// A run longer than this starts a new burst of restarts instead of adding to the current one
const STABLE: Duration = Duration::from_secs(60);

struct Attached {
  view: Arc<dyn ShellView>,
  /// The session the view was last showing; a reconnect re-attaches on it
  last_session: Option<String>,
}

enum ConnKind {
  /// A child over stdio, ended by pid
  Child { pid: u32, exited: watch::Receiver<bool> },
  /// The persistent engine's socket: the sender stops its reader, which reports the connection closed
  Socket { close: Option<oneshot::Sender<()>>, done: watch::Receiver<bool> },
}

struct Conn {
  tx: mpsc::UnboundedSender<String>,
  kind: ConnKind,
}

struct State {
  conn: Option<Conn>,
  connecting: bool,
  generation: u64,
  ready: bool,
  outbox: Vec<Outgoing>,
  views: Vec<Attached>,
  state: SidecarState,
  detail: Option<String>,
  restarts: u32,
  last_start: Instant,
  candidate: usize,
  timer: Option<JoinHandle<()>>,
  disposed: bool,
  hello_seq: u64,
  previews: Vec<(u64, Arc<PreviewLease>)>,
  lease_seq: u64,
  preview_seq: u64,
  /// previewOpen request id → the panel and url it was for; an answer for a panel that moved on since is dropped
  preview_asks: HashMap<String, (String, String)>,
  /// The newest ask per panel: an older answer must not land on the page the panel shows now
  preview_latest: HashMap<String, String>,
  features: HashSet<String>,
  sessions_dir: Option<String>,
}

struct Inner {
  opts: ClientOpts,
  rt: tokio::runtime::Handle,
  st: Mutex<State>,
}

type StateChange = (Vec<Arc<dyn ShellView>>, SidecarState, Option<String>);

/// Callbacks gathered under the lock and run after it: a view or a listener may call back into the client
#[derive(Default)]
struct After {
  states: Option<StateChange>,
  answers: Vec<(Arc<PreviewLease>, PreviewAnswer)>,
  start: bool,
}

#[derive(Clone)]
pub struct SidecarClient {
  inner: Arc<Inner>,
}

impl SidecarClient {
  /// Tasks run on `rt` (the app's tokio runtime)
  pub fn new(opts: ClientOpts, rt: tokio::runtime::Handle) -> SidecarClient {
    let st = State {
      conn: None,
      connecting: false,
      generation: 0,
      ready: false,
      outbox: vec![],
      views: vec![],
      state: SidecarState::Stopped,
      detail: None,
      restarts: 0,
      last_start: Instant::now(),
      candidate: 0,
      timer: None,
      disposed: false,
      hello_seq: 0,
      previews: vec![],
      lease_seq: 0,
      preview_seq: 0,
      preview_asks: HashMap::new(),
      preview_latest: HashMap::new(),
      features: HashSet::new(),
      sessions_dir: None,
    };
    SidecarClient { inner: Arc::new(Inner { opts, rt, st: Mutex::new(st) }) }
  }

  pub fn current(&self) -> (SidecarState, Option<String>) {
    let s = self.inner.st.lock().unwrap();
    (s.state, s.detail.clone())
  }

  /// The engine's sessions directory, as its helloOk said
  pub fn sessions_dir(&self) -> Option<String> {
    self.inner.st.lock().unwrap().sessions_dir.clone()
  }

  pub fn start(&self) {
    Inner::start(&self.inner);
  }

  /// Retry after Failed: the consecutive-failure counter and the engine choice start over
  pub fn retry(&self) {
    {
      let mut s = self.inner.st.lock().unwrap();
      if s.disposed || s.live() {
        return;
      }
      s.restarts = 0;
      s.candidate = 0;
    }
    Inner::start(&self.inner);
  }

  pub fn attach(&self, view: Arc<dyn ShellView>) {
    let (state, detail) = {
      let mut s = self.inner.st.lock().unwrap();
      let a = Attached { view: view.clone(), last_session: None };
      if s.ready {
        let env = attach_envelope(&a);
        s.write(&env);
      }
      s.views.push(a);
      (s.state, s.detail.clone())
    };
    view.on_state(state, detail.as_deref());
  }

  pub fn detach(&self, view_id: &str) {
    let mut s = self.inner.st.lock().unwrap();
    let before = s.views.len();
    s.views.retain(|a| a.view.view_id() != view_id);
    if s.views.len() == before {
      return;
    }
    if s.ready {
      s.write(&Outgoing::DetachView { view_id: view_id.to_owned() });
    } else {
      s.outbox.retain(|m| m.view_id() != Some(view_id));
    }
  }

  /// A page message for a view (relayed verbatim)
  pub fn send(&self, view_id: &str, message: Box<RawValue>) {
    let mut s = self.inner.st.lock().unwrap();
    if !s.views.iter().any(|a| a.view.view_id() == view_id) {
      return;
    }
    s.post(Outgoing::WebviewMessage { view_id: view_id.to_owned(), message });
  }

  /// Facts about the shell changed (settings, focus). Before helloOk the event waits too: the hello already in flight may
  /// predate it, and replaying a snapshot the next hello also carries is harmless
  pub fn event(&self, event: Value) {
    self.inner.st.lock().unwrap().post(Outgoing::PlatformEvent { event });
  }

  /// Keeps a page open through the engine's preview proxy for this panel; again with another url navigates it. The
  /// handle closes it. Answers arrive through on_opened, once now and once after every reconnect
  pub fn preview(&self, lease: PreviewLease) -> PreviewHandle {
    let lease = Arc::new(lease);
    let (seq, answer) = {
      let mut s = self.inner.st.lock().unwrap();
      s.lease_seq += 1;
      let seq = s.lease_seq;
      s.previews.retain(|(_, l)| l.panel_id != lease.panel_id);
      s.previews.push((seq, lease.clone()));
      let answer = if s.ready { s.ask_preview(&lease) } else { None };
      (seq, answer)
    };
    if let Some(a) = answer {
      (lease.on_opened)(a);
    }
    PreviewHandle { client: Arc::downgrade(&self.inner), panel_id: lease.panel_id.clone(), seq }
  }

  /// A child process is shut down with its agents; a persistent engine is only disconnected from, so its turns keep
  /// running and the next launch picks them up
  pub async fn dispose(&self) {
    let conn = {
      let mut s = self.inner.st.lock().unwrap();
      if s.disposed {
        return;
      }
      s.disposed = true;
      if let Some(t) = s.timer.take() {
        t.abort();
      }
      s.generation += 1;
      s.connecting = false;
      s.ready = false;
      s.outbox.clear();
      s.views.clear();
      s.previews.clear();
      s.state = SidecarState::Stopped;
      s.conn.take()
    };
    let Some(conn) = conn else { return };
    match conn.kind {
      ConnKind::Socket { close, mut done } => {
        drop(conn.tx);
        // The writer flushes what was queued and closes its half; the reader stops
        if let Some(c) = close {
          let _ = c.send(());
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), done.wait_for(|d| *d)).await;
      }
      ConnKind::Child { pid, exited } => stop_child(conn.tx, pid, exited, Duration::from_secs(3)).await,
    }
  }

  #[cfg(test)]
  fn on_line_for_test(&self, generation: u64, line: &str) {
    Inner::on_line(&self.inner, generation, line);
  }

  #[cfg(test)]
  fn current_generation(&self) -> u64 {
    self.inner.st.lock().unwrap().generation
  }
}

/// Closes a preview lease (the panel closed or moved off the proxy)
pub struct PreviewHandle {
  client: Weak<Inner>,
  panel_id: String,
  seq: u64,
}

impl PreviewHandle {
  pub fn release(&self) {
    let Some(inner) = self.client.upgrade() else { return };
    let mut s = inner.st.lock().unwrap();
    let Some(i) = s.previews.iter().position(|(q, l)| *q == self.seq && l.panel_id == self.panel_id) else { return };
    s.previews.remove(i);
    s.preview_latest.remove(&self.panel_id);
    if s.ready {
      s.write(&Outgoing::PreviewClose { panel_id: self.panel_id.clone() });
    }
  }
}

fn attach_envelope(a: &Attached) -> Outgoing {
  let initial = a.last_session.clone().map(Initial::Session).or_else(|| a.view.initial());
  Outgoing::AttachView { view_id: a.view.view_id().to_owned(), host: a.view.host().to_owned(), initial, blob_base: a.view.blob_base() }
}

impl State {
  fn live(&self) -> bool {
    self.conn.is_some() || self.connecting
  }

  fn write(&self, m: &Outgoing) {
    if let Some(c) = &self.conn {
      let _ = c.tx.send(m.line());
    }
  }

  fn post(&mut self, m: Outgoing) {
    if self.ready { self.write(&m) } else { self.outbox.push(m) }
  }

  /// Sends previewOpen, or answers at once when the engine has no proxy
  fn ask_preview(&mut self, lease: &PreviewLease) -> Option<PreviewAnswer> {
    if !self.features.contains("preview") {
      return Some(PreviewAnswer::Error("engine-too-old".into()));
    }
    self.preview_seq += 1;
    let request_id = format!("p{}", self.preview_seq);
    self.preview_asks.insert(request_id.clone(), (lease.panel_id.clone(), lease.url.clone()));
    self.preview_latest.insert(lease.panel_id.clone(), request_id.clone());
    self.write(&Outgoing::PreviewOpen {
      request_id,
      panel_id: lease.panel_id.clone(),
      url: lease.url.clone(),
      picker: lease.picker.clone(),
      folder: lease.folder.clone(),
    });
    None
  }

  fn set_state(&mut self, after: &mut After, state: SidecarState, detail: Option<String>) {
    self.state = state;
    self.detail = detail.clone();
    after.states = Some((self.views.iter().map(|a| a.view.clone()).collect(), state, detail));
  }

  fn hello(&mut self, payload: HelloPayload) -> Outgoing {
    self.hello_seq += 1;
    Outgoing::Hello {
      protocol_version: SIDECAR_PROTOCOL_VERSION,
      request_id: format!("h{}", self.hello_seq),
      client: payload.client,
      env: payload.env,
      settings: payload.settings,
    }
  }
}

impl Inner {
  fn log(&self, line: &str) {
    (self.opts.log)(line);
  }

  fn run(self: &Arc<Self>, after: After) {
    if let Some((views, state, detail)) = after.states {
      for v in views {
        v.on_state(state, detail.as_deref());
      }
      (self.opts.on_state)(state, detail.as_deref());
    }
    for (lease, answer) in after.answers {
      (lease.on_opened)(answer);
    }
    if after.start {
      Inner::start(self);
    }
  }

  fn start(self: &Arc<Self>) {
    let candidates = (self.opts.commands)();
    let mut after = After::default();
    let (generation, cmd, endpoint) = {
      let mut s = self.st.lock().unwrap();
      if s.disposed || s.live() {
        return;
      }
      if let Some(t) = s.timer.take() {
        t.abort();
      }
      let Some(cmd) = candidates.get(s.candidate).cloned() else {
        s.set_state(&mut after, SidecarState::Failed, Some("no sidecar binary for this platform".into()));
        drop(s);
        self.run(after);
        return;
      };
      s.generation += 1;
      s.set_state(&mut after, SidecarState::Starting, None);
      s.last_start = Instant::now();
      let endpoint = (self.opts.engine)(&cmd);
      if endpoint.is_some() {
        s.connecting = true;
      }
      (s.generation, cmd, endpoint)
    };
    self.run(after);
    let total = candidates.len();
    match endpoint {
      Some(ep) => {
        let me = self.clone();
        self.rt.spawn(async move { me.connect(generation, cmd, ep, total).await });
      }
      None => self.spawn_child(generation, cmd, total),
    }
  }

  #[cfg(unix)]
  async fn connect(self: Arc<Self>, generation: u64, cmd: SidecarCommand, ep: EngineEndpoint, total: usize) {
    let me = self.clone();
    let cancelled = move || {
      let s = me.st.lock().unwrap();
      s.generation != generation || s.disposed
    };
    let log = self.opts.log.clone();
    let cwd = (self.opts.cwd)();
    let stream = crate::engine::reach_engine(&cmd, &ep, cwd.as_deref(), &cancelled, &*log).await;
    let stream = match stream {
      Ok(s) => s,
      Err(e) => {
        {
          let mut s = self.st.lock().unwrap();
          if s.generation != generation {
            return;
          }
          s.connecting = false;
        }
        self.spawn_failed(generation, &cmd, &e.to_string(), total);
        return;
      }
    };
    let (read, mut write) = stream.into_split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let (close_tx, close_rx) = oneshot::channel::<()>();
    let (done_tx, done_rx) = watch::channel(false);
    {
      let mut s = self.st.lock().unwrap();
      if s.generation != generation || s.disposed {
        return;
      }
      s.connecting = false;
      s.conn = Some(Conn { tx, kind: ConnKind::Socket { close: Some(close_tx), done: done_rx } });
      let hello = s.hello((self.opts.hello)());
      s.write(&hello);
    }
    self.log(&format!("engine connected: {}", ep.socket.display()));
    self.rt.spawn(async move {
      while let Some(line) = rx.recv().await {
        if write.write_all(line.as_bytes()).await.is_err() {
          break;
        }
      }
      let _ = write.shutdown().await;
    });
    let me = self.clone();
    self.rt.spawn(async move {
      let mut lines = BufReader::new(read).lines();
      let mut close_rx = close_rx;
      loop {
        tokio::select! {
          line = lines.next_line() => match line {
            Ok(Some(l)) => Inner::on_line(&me, generation, &l),
            Ok(None) => break,
            Err(e) => { me.log(&format!("engine connection: {e}")); break }
          },
          _ = &mut close_rx => break,
        }
      }
      let _ = done_tx.send(true);
      {
        let mut s = me.st.lock().unwrap();
        if s.generation == generation {
          s.conn = None;
        }
      }
      me.on_exit(generation, "connection closed", total);
    });
  }

  #[cfg(not(unix))]
  async fn connect(self: Arc<Self>, generation: u64, cmd: SidecarCommand, _ep: EngineEndpoint, total: usize) {
    {
      self.st.lock().unwrap().connecting = false;
    }
    self.spawn_failed(generation, &cmd, "the persistent engine needs Unix domain sockets", total);
  }

  fn spawn_child(self: &Arc<Self>, generation: u64, cmd: SidecarCommand, total: usize) {
    let mut c = tokio::process::Command::new(&cmd.command);
    c.args(&cmd.args)
      .envs(&cmd.env)
      .stdin(std::process::Stdio::piped())
      .stdout(std::process::Stdio::piped())
      .stderr(std::process::Stdio::piped());
    if let Some(cwd) = (self.opts.cwd)() {
      c.current_dir(cwd);
    }
    // Spawning needs the runtime's reactor
    let _guard = self.rt.enter();
    let mut child = match c.spawn() {
      Ok(ch) => ch,
      Err(e) => {
        self.spawn_failed(generation, &cmd, &e.to_string(), total);
        return;
      }
    };
    let pid = child.id().unwrap_or(0);
    let (mut stdin, stdout, stderr) =
      (child.stdin.take().expect("piped"), child.stdout.take().expect("piped"), child.stderr.take().expect("piped"));
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let (exit_tx, exit_rx) = watch::channel(false);
    {
      let mut s = self.st.lock().unwrap();
      if s.generation != generation || s.disposed {
        drop(s);
        let _ = child.start_kill();
        return;
      }
      s.conn = Some(Conn { tx, kind: ConnKind::Child { pid, exited: exit_rx } });
      let hello = s.hello((self.opts.hello)());
      s.write(&hello);
    }
    self.log(&format!("sidecar starting: {}", cmd.label));
    let me = self.clone();
    self.rt.spawn(async move {
      while let Some(line) = rx.recv().await {
        // A sidecar that died leaves a broken pipe behind; the exit handler takes it from there
        if let Err(e) = stdin.write_all(line.as_bytes()).await {
          me.log(&format!("sidecar stdin: {e}"));
          break;
        }
      }
    });
    let me = self.clone();
    self.rt.spawn(async move {
      let mut lines = BufReader::new(stdout).lines();
      while let Ok(Some(l)) = lines.next_line().await {
        Inner::on_line(&me, generation, &l);
      }
    });
    let me = self.clone();
    self.rt.spawn(async move {
      let mut lines = BufReader::new(stderr).lines();
      while let Ok(Some(l)) = lines.next_line().await {
        me.log(&format!("sidecar: {l}"));
      }
    });
    let me = self.clone();
    self.rt.spawn(async move {
      let code = match child.wait().await {
        Ok(st) => st.code().map(|c| c.to_string()).unwrap_or_else(|| format!("{st}")),
        Err(e) => e.to_string(),
      };
      let _ = exit_tx.send(true);
      {
        let mut s = me.st.lock().unwrap();
        if s.generation == generation {
          s.conn = None;
        }
      }
      me.on_exit(generation, &code, total);
    });
  }

  fn on_line(self: &Arc<Self>, generation: u64, line: &str) {
    if line.trim().is_empty() {
      return;
    }
    if self.st.lock().unwrap().generation != generation {
      return;
    }
    let m: Incoming = match serde_json::from_str(line) {
      Ok(m) => m,
      Err(_) => {
        self.log(&format!("sidecar stdout is not an envelope: {}", line.chars().take(200).collect::<String>()));
        return;
      }
    };
    let mut after = After::default();
    match m.kind.as_str() {
      "helloOk" => {
        let mut s = self.st.lock().unwrap();
        if s.generation != generation {
          return;
        }
        s.ready = true;
        s.candidate = 0;
        s.sessions_dir = m.sessions_dir.clone();
        s.features = m.features.unwrap_or_default().into_iter().collect();
        let info = m.sidecar.unwrap_or(crate::wire::SidecarInfo { version: String::new(), pid: 0 });
        self.log(&format!("sidecar ready: pid {}, version {}, sessions {}", info.pid, info.version, m.sessions_dir.unwrap_or_default()));
        // Queued facts first: a view reads the environment when it attaches
        let queued = std::mem::take(&mut s.outbox);
        for q in queued.iter().filter(|q| matches!(q, Outgoing::PlatformEvent { .. })) {
          s.write(q);
        }
        for a in &s.views {
          s.write(&attach_envelope(a));
        }
        for q in queued.iter().filter(|q| !matches!(q, Outgoing::PlatformEvent { .. })) {
          s.write(q);
        }
        s.preview_asks.clear();
        let leases: Vec<Arc<PreviewLease>> = s.previews.iter().map(|(_, l)| l.clone()).collect();
        for lease in leases {
          if let Some(a) = s.ask_preview(&lease) {
            after.answers.push((lease, a));
          }
        }
        s.set_state(&mut after, SidecarState::Ready, None);
      }
      "helloReject" => {
        let reason = m.reason.unwrap_or_default();
        self.log(&format!("sidecar rejected hello: {reason}"));
        let mut s = self.st.lock().unwrap();
        let socket = matches!(s.conn.as_ref().map(|c| &c.kind), Some(ConnKind::Socket { .. }));
        let close = |s: &mut State| {
          if let Some(Conn { kind: ConnKind::Socket { close, .. }, .. }) = s.conn.as_mut()
            && let Some(c) = close.take()
          {
            let _ = c.send(());
          }
        };
        // A persistent engine that is just ending: dropping the connection reconnects (and launches the next engine)
        if socket && reason.contains("shutting down") {
          close(&mut s);
          return;
        }
        s.set_state(&mut after, SidecarState::Failed, Some(reason));
        close(&mut s);
        let child = match s.conn.as_ref() {
          Some(Conn { tx, kind: ConnKind::Child { pid, exited } }) => Some((tx.clone(), *pid, exited.clone())),
          _ => None,
        };
        if let Some((tx, pid, exited)) = child {
          self.rt.spawn(stop_child(tx, pid, exited, Duration::from_millis(500)));
        }
      }
      "hostMessage" => {
        let (Some(view_id), Some(raw)) = (m.view_id, m.message) else { return };
        let msg = HostMessage::from_raw(raw);
        let view = {
          let mut s = self.st.lock().unwrap();
          let Some(a) = s.views.iter_mut().find(|a| a.view.view_id() == view_id) else { return };
          if let Some(id) = &msg.session {
            a.last_session = Some(id.clone());
          }
          a.view.clone()
        };
        view.on_host_message(msg);
        return;
      }
      "platformRequest" => {
        let me = self.clone();
        let (request_id, request) = (m.request_id, m.request.unwrap_or(Value::Null));
        self.rt.spawn(async move { me.on_request(generation, request_id, request).await });
        return;
      }
      "previewOpened" => {
        let Some(request_id) = m.request_id else { return };
        let mut s = self.st.lock().unwrap();
        let Some((panel, url)) = s.preview_asks.remove(&request_id) else { return };
        let Some(lease) = s.previews.iter().find(|(_, l)| l.panel_id == panel).map(|(_, l)| l.clone()) else { return };
        if lease.url != url || s.preview_latest.get(&panel) != Some(&request_id) {
          return;
        }
        let answer = match (m.url, m.token) {
          (Some(url), Some(token)) if !url.is_empty() && !token.is_empty() => PreviewAnswer::Opened { url, token },
          _ => PreviewAnswer::Error(m.error.unwrap_or_else(|| "no answer".into())),
        };
        after.answers.push((lease, answer));
      }
      "shutdownOk" => return,
      other => {
        self.log(&format!("sidecar sent an unknown envelope: {other}"));
        return;
      }
    }
    self.run(after);
  }

  async fn on_request(self: Arc<Self>, generation: u64, request_id: Option<String>, request: Value) {
    let method = request.get("method").and_then(Value::as_str).unwrap_or("?").to_owned();
    let result = match serde_json::from_value::<PlatformRequest>(request) {
      Ok(r) => (self.opts.on_request)(r).await,
      Err(_) => Err(format!("{method} is not supported by the desktop app")),
    };
    let reply = match result {
      Ok(v) => request_id.map(|id| Outgoing::PlatformResponse { request_id: id, result: Some(v), error: None }),
      Err(e) => {
        self.log(&format!("platform request {method} failed: {e}"));
        request_id.map(|id| Outgoing::PlatformResponse { request_id: id, result: None, error: Some(e) })
      }
    };
    let s = self.st.lock().unwrap();
    if let Some(r) = reply
      && s.generation == generation
    {
      s.write(&r);
    }
  }

  fn spawn_failed(self: &Arc<Self>, generation: u64, cmd: &SidecarCommand, err: &str, total: usize) {
    let mut after = After::default();
    {
      let mut s = self.st.lock().unwrap();
      if s.generation != generation {
        return;
      }
      s.conn = None;
      self.log(&format!("sidecar could not start ({}): {err}", cmd.label));
      if s.candidate + 1 < total {
        s.candidate += 1;
        after.start = true;
      } else {
        s.set_state(&mut after, SidecarState::Failed, Some(err.to_owned()));
      }
    }
    self.run(after);
  }

  fn on_exit(self: &Arc<Self>, generation: u64, code: &str, total: usize) {
    let mut after = After::default();
    {
      let mut s = self.st.lock().unwrap();
      if s.generation != generation {
        return;
      }
      let reached_hello = s.ready;
      s.conn = None;
      s.ready = false;
      // The outbox is only ever non-empty before a handshake, so what it holds was never delivered: it goes to the next
      // process
      if s.disposed || s.state == SidecarState::Failed {
        return;
      }
      self.log(&format!("sidecar exited ({code})"));
      // An engine that never got through the handshake is not worth retrying while another one is available
      if !reached_hello && s.candidate + 1 < total {
        s.candidate += 1;
        self.log("sidecar exited before the handshake; trying the next engine");
        after.start = true;
      } else {
        s.restarts = if s.last_start.elapsed() > STABLE { 1 } else { s.restarts + 1 };
        if s.restarts > MAX_RESTARTS {
          s.set_state(&mut after, SidecarState::Failed, Some(format!("the host process keeps exiting (last: {code})")));
        } else {
          let delay = (self.opts.backoff)(s.restarts);
          s.set_state(&mut after, SidecarState::Starting, Some(format!("exited ({code}), restarting in {}s", delay.as_secs_f64().round())));
          let me = self.clone();
          s.timer = Some(self.rt.spawn(async move {
            tokio::time::sleep(delay).await;
            me.st.lock().unwrap().timer = None;
            Inner::start(&me);
          }));
        }
      }
    }
    self.run(after);
  }
}

/// Ask nicely, then insist: shutdown → wait → SIGTERM → wait → SIGKILL
async fn stop_child(tx: mpsc::UnboundedSender<String>, pid: u32, mut exited: watch::Receiver<bool>, grace: Duration) {
  if *exited.borrow() {
    return;
  }
  let _ = tx.send(Outgoing::Shutdown.line());
  drop(tx);
  if tokio::time::timeout(grace, exited.wait_for(|e| *e)).await.is_ok() {
    return;
  }
  #[cfg(unix)]
  {
    // SAFETY: a signal to the sidecar this client started
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if tokio::time::timeout(Duration::from_secs(2), exited.wait_for(|e| *e)).await.is_ok() {
      return;
    }
    // SAFETY: as above
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
  }
  #[cfg(not(unix))]
  let _ = pid;
}

#[cfg(test)]
mod tests;
