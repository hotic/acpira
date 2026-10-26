//! The engine's end of a helper channel: starts the command that runs `acpira remote --stdio` (ssh, or the binary
//! itself in tests), waits for the handshake past any login-shell noise, then multiplexes requests over the one stdio.
//!
//! A channel that dies takes its requests with it as `Disconnected`: the helper may or may not have carried them out,
//! and nothing here replays them. A new channel is a new client, so a late answer can never land on a request of the next

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};

use super::wire::{self, DEAD_AFTER_MS, Frame, Hello, PING_EVERY_MS, PROTOCOL, WireError};

/// Lines of noise before the handshake kept for the error message
const NOISE_KEPT: usize = 20;
/// How long a closed channel's transport may take to exit after its stdin closed
const CLOSE_GRACE: Duration = Duration::from_secs(3);
/// stderr lines kept for diagnostics (ssh's own errors land there)
const STDERR_KEPT: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
  /// The helper answered with an error
  Wire(WireError),
  /// The channel ended before the answer: the operation may or may not have happened on the host
  Disconnected(String),
}

impl RemoteError {
  pub fn code(&self) -> &str {
    match self {
      RemoteError::Wire(e) => &e.code,
      RemoteError::Disconnected(_) => "disconnected",
    }
  }
}

impl std::fmt::Display for RemoteError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      RemoteError::Wire(e) => write!(f, "{}", e.message),
      RemoteError::Disconnected(why) => write!(f, "the connection to the host ended before the answer ({why}); the outcome is unknown"),
    }
  }
}

impl std::error::Error for RemoteError {}

/// Why a channel could not start: the process's own words (ssh's stderr) and whatever it printed instead of a handshake
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartError {
  pub message: String,
  pub stderr: String,
  pub noise: Vec<String>,
  pub exit_code: Option<i32>,
}

impl std::fmt::Display for StartError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.message)
  }
}

/// What a request delivers: its bulk chunks in order, then its end
#[derive(Debug)]
pub enum Event {
  Chunk { stream: String, bytes: Vec<u8> },
  Done(Result<Value, RemoteError>),
}

/// A finished request with its bulk output gathered by stream
#[derive(Debug, Default)]
pub struct Reply {
  pub value: Value,
  pub data: Vec<u8>,
  pub out: Vec<u8>,
  pub err: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ClientOpts {
  pub handshake_timeout: Duration,
  pub ping_every: Duration,
  pub dead_after: Duration,
}

impl Default for ClientOpts {
  fn default() -> ClientOpts {
    ClientOpts {
      handshake_timeout: Duration::from_secs(30),
      ping_every: Duration::from_millis(PING_EVERY_MS),
      dead_after: Duration::from_millis(DEAD_AFTER_MS),
    }
  }
}

struct Inner {
  hello: Hello,
  lines: mpsc::UnboundedSender<String>,
  pending: Mutex<HashMap<u64, mpsc::UnboundedSender<Event>>>,
  next_id: AtomicU64,
  /// Why the channel ended, once it has
  closed: watch::Sender<Option<String>>,
  rtt_ms: AtomicU32,
  /// Ping number → when it went out
  pings: Mutex<HashMap<u64, tokio::time::Instant>>,
  last_seen: Mutex<tokio::time::Instant>,
  stderr: Arc<Mutex<VecDeque<String>>>,
  child: Mutex<Option<Child>>,
}

/// Held by every clone of a client (and every call in flight): when the last one goes, the channel closes the way
/// `close` does. The I/O tasks hold `Inner` themselves, so its own lifetime cannot tell
struct Owner(Arc<Inner>);

impl Drop for Owner {
  fn drop(&mut self) {
    if tokio::runtime::Handle::try_current().is_ok() {
      close(&self.0, "the last user let go", true);
    } else if let Some(mut child) = self.0.child.lock().take() {
      let _ = child.start_kill();
    }
  }
}

#[derive(Clone)]
pub struct RemoteClient {
  inner: Arc<Inner>,
  _owner: Arc<Owner>,
}

impl RemoteClient {
  /// Start `cmd` (stdio is set here) and wait for the helper's handshake
  pub async fn start(mut cmd: Command, opts: ClientOpts) -> Result<RemoteClient, StartError> {
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| StartError { message: format!("cannot start the connection: {e}"), stderr: String::new(), noise: vec![], exit_code: None })?;
    let (Some(stdin), Some(stdout), Some(stderr_pipe)) = (child.stdin.take(), child.stdout.take(), child.stderr.take()) else {
      let _ = child.start_kill();
      return Err(StartError { message: "cannot start the connection: no stdio pipes".to_owned(), stderr: String::new(), noise: vec![], exit_code: None });
    };
    let stderr: Arc<Mutex<VecDeque<String>>> = Arc::default();
    {
      let stderr = stderr.clone();
      tokio::spawn(async move {
        let mut lines = BufReader::new(stderr_pipe).lines();
        while let Ok(Some(l)) = lines.next_line().await {
          let mut s = stderr.lock();
          if s.len() >= STDERR_KEPT {
            s.pop_front();
          }
          s.push_back(l);
        }
      });
    }
    let stderr_text = |s: &Arc<Mutex<VecDeque<String>>>| s.lock().iter().cloned().collect::<Vec<_>>().join("\n");

    let mut reader = BufReader::new(stdout);
    let mut noise = vec![];
    let hello = {
      let wait = async {
        let mut buf = Vec::new();
        loop {
          buf.clear();
          match wire::read_line(&mut reader, &mut buf).await {
            Ok(0) => return None,
            Ok(_) => {}
            Err(_) => return None,
          }
          let line = String::from_utf8_lossy(&buf).into_owned();
          if let Some(h) = wire::parse_hello(&line) {
            return Some(h);
          }
          if noise.len() < NOISE_KEPT {
            noise.push(line);
          }
        }
      };
      tokio::time::timeout(opts.handshake_timeout, wait).await
    };
    let hello = match hello {
      Ok(Some(h)) => h,
      other => {
        let timed_out = other.is_err();
        let _ = child.start_kill();
        let exit_code = tokio::time::timeout(Duration::from_secs(2), child.wait()).await.ok().and_then(Result::ok).and_then(|s| s.code());
        // Let the stderr reader take the last lines ssh wrote as it ended
        tokio::time::sleep(Duration::from_millis(50)).await;
        let stderr = stderr_text(&stderr);
        let message = if timed_out {
          format!("no handshake from the host within {} s", opts.handshake_timeout.as_secs())
        } else {
          stderr.lines().rev().find(|l| !l.trim().is_empty() && !l.starts_with("debug")).map(str::to_owned).unwrap_or_else(|| "the connection ended before the handshake".to_owned())
        };
        return Err(StartError { message, stderr, noise, exit_code });
      }
    };
    if hello.protocol != PROTOCOL {
      let _ = child.start_kill();
      return Err(StartError {
        message: format!("the host's helper speaks protocol {}, this engine {PROTOCOL}", hello.protocol),
        stderr: stderr_text(&stderr),
        noise,
        exit_code: None,
      });
    }

    let (lines_tx, mut lines_rx) = mpsc::unbounded_channel::<String>();
    let (closed, _) = watch::channel(None);
    let inner = Arc::new(Inner {
      hello,
      lines: lines_tx,
      pending: Mutex::default(),
      next_id: AtomicU64::new(1),
      closed,
      rtt_ms: AtomicU32::new(0),
      pings: Mutex::default(),
      last_seen: Mutex::new(tokio::time::Instant::now()),
      stderr,
      child: Mutex::new(Some(child)),
    });

    // Writer: requests and control frames, in order
    {
      let inner = inner.clone();
      let mut stdin = stdin;
      tokio::spawn(async move {
        while let Some(line) = lines_rx.recv().await {
          // The end marker: dropping stdin is the helper's EOF, which it answers by cleaning up and exiting
          if line.is_empty() {
            break;
          }
          if stdin.write_all(line.as_bytes()).await.is_err() || stdin.write_all(b"\n").await.is_err() || stdin.flush().await.is_err() {
            close(&inner, "the connection stopped taking input", false);
            break;
          }
        }
      });
    }
    // Reader
    {
      let inner = inner.clone();
      tokio::spawn(async move {
        let mut buf = Vec::new();
        let why = loop {
          buf.clear();
          match wire::read_line(&mut reader, &mut buf).await {
            Ok(0) => break "the host closed the connection".to_owned(),
            Ok(_) => {}
            Err(e) => break e.to_string(),
          }
          *inner.last_seen.lock() = tokio::time::Instant::now();
          let Ok(frame) = serde_json::from_slice::<Frame>(&buf) else { continue };
          on_frame(&inner, frame);
        };
        close(&inner, &why, false);
      });
    }
    // Heartbeat
    {
      let weak = Arc::downgrade(&inner);
      tokio::spawn(async move {
        let mut n = 0u64;
        let mut tick = tokio::time::interval(opts.ping_every);
        tick.tick().await;
        loop {
          tick.tick().await;
          let Some(inner) = weak.upgrade() else { break };
          if inner.closed.borrow().is_some() {
            break;
          }
          if inner.last_seen.lock().elapsed() > opts.dead_after {
            close(&inner, &format!("no answer from the host for {} s", opts.dead_after.as_secs()), false);
            break;
          }
          n += 1;
          inner.pings.lock().insert(n, tokio::time::Instant::now());
          send(&inner, &Frame::Ping { n });
        }
      });
    }
    Ok(RemoteClient { _owner: Arc::new(Owner(inner.clone())), inner })
  }

  pub fn hello(&self) -> &Hello {
    &self.inner.hello
  }

  /// The last measured round trip, 0 before the first pong
  pub fn rtt_ms(&self) -> u32 {
    self.inner.rtt_ms.load(Ordering::Relaxed)
  }

  /// Why the channel ended; None while it is up
  pub fn closed_reason(&self) -> Option<String> {
    self.inner.closed.borrow().clone()
  }

  /// Resolves once the channel has ended, with the reason
  pub async fn closed(&self) -> String {
    let mut rx = self.inner.closed.subscribe();
    loop {
      if let Some(why) = rx.borrow().clone() {
        return why;
      }
      if rx.changed().await.is_err() {
        return "closed".to_owned();
      }
    }
  }

  /// What the transport printed on stderr lately (ssh's warnings, the helper's own log)
  pub fn stderr_tail(&self) -> String {
    self.inner.stderr.lock().iter().cloned().collect::<Vec<_>>().join("\n")
  }

  /// End the channel: stdin closes, so the helper stops its commands and exits; the transport is killed only if it
  /// is still there after a grace
  pub fn close(&self) {
    close(&self.inner, "closed here", true);
  }

  /// Start a request; its events arrive on the returned call
  pub fn request(&self, op: &str, a: Value) -> Call {
    let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = mpsc::unbounded_channel();
    if let Some(why) = self.closed_reason() {
      let _ = tx.send(Event::Done(Err(RemoteError::Disconnected(why))));
    } else {
      self.inner.pending.lock().insert(id, tx);
      send(&self.inner, &Frame::Req { id, op: op.to_owned(), a });
    }
    Call { id, rx, client: self.clone(), done: AtomicBool::new(false) }
  }

  /// A request run to its end, its chunks gathered
  pub async fn call(&self, op: &str, a: Value) -> Result<Reply, RemoteError> {
    self.request(op, a).collect().await
  }
}

/// One request in flight. Dropped before its end, it cancels the request on the host
pub struct Call {
  id: u64,
  rx: mpsc::UnboundedReceiver<Event>,
  client: RemoteClient,
  done: AtomicBool,
}

impl Call {
  pub fn id(&self) -> u64 {
    self.id
  }

  pub async fn next(&mut self) -> Option<Event> {
    let e = self.rx.recv().await;
    if matches!(e, Some(Event::Done(_)) | None) {
      self.done.store(true, Ordering::Relaxed);
    }
    e
  }

  /// Bytes for an exec's stdin
  pub fn stdin(&self, bytes: &[u8]) {
    send(&self.client.inner, &Frame::Stdin { id: self.id, b: wire::b64(bytes), eof: false });
  }

  pub fn stdin_eof(&self) {
    send(&self.client.inner, &Frame::Stdin { id: self.id, b: String::new(), eof: true });
  }

  pub async fn collect(mut self) -> Result<Reply, RemoteError> {
    let mut reply = Reply::default();
    loop {
      match self.next().await {
        Some(Event::Chunk { stream, bytes }) => match stream.as_str() {
          "out" => reply.out.extend(bytes),
          "err" => reply.err.extend(bytes),
          _ => reply.data.extend(bytes),
        },
        Some(Event::Done(r)) => {
          reply.value = r?;
          return Ok(reply);
        }
        None => return Err(RemoteError::Disconnected(self.client.closed_reason().unwrap_or_else(|| "closed".into()))),
      }
    }
  }
}

impl Drop for Call {
  fn drop(&mut self) {
    if !self.done.load(Ordering::Relaxed) && self.client.inner.pending.lock().remove(&self.id).is_some() {
      send(&self.client.inner, &Frame::Cancel { id: self.id });
    }
  }
}

fn send(inner: &Inner, f: &Frame) {
  if let Ok(line) = serde_json::to_string(f) {
    let _ = inner.lines.send(line);
  }
}

fn on_frame(inner: &Inner, frame: Frame) {
  match frame {
    Frame::Chunk { id, s, b } => {
      let bytes = wire::unb64(&b).unwrap_or_default();
      // Acked as it lands, whether anyone still waits for it: the window is the helper's, not the request's
      send(inner, &Frame::Ack { n: bytes.len() });
      if let Some(tx) = inner.pending.lock().get(&id) {
        let _ = tx.send(Event::Chunk { stream: s, bytes });
      }
    }
    Frame::Res { id, ok, err } => {
      if let Some(tx) = inner.pending.lock().remove(&id) {
        let r = match (ok, err) {
          (_, Some(e)) => Err(RemoteError::Wire(e)),
          (ok, None) => Ok(ok.unwrap_or(Value::Null)),
        };
        let _ = tx.send(Event::Done(r));
      }
    }
    Frame::Ping { n } => send(inner, &Frame::Pong { n }),
    Frame::Pong { n } => {
      if let Some(at) = inner.pings.lock().remove(&n) {
        inner.rtt_ms.store(at.elapsed().as_millis().min(u32::MAX as u128) as u32, Ordering::Relaxed);
      }
    }
    Frame::Req { .. } | Frame::Cancel { .. } | Frame::Stdin { .. } | Frame::Ack { .. } => {}
  }
}

/// Mark the channel ended (first reason wins), fail every request still waiting, and take the transport down: at once
/// when the other end is gone or stuck, after a grace for EOF to do its work when `graceful`
fn close(inner: &Inner, why: &str, graceful: bool) {
  let first = inner.closed.send_if_modified(|c| {
    if c.is_some() {
      return false;
    }
    *c = Some(why.to_owned());
    true
  });
  if !first {
    return;
  }
  for (_, tx) in inner.pending.lock().drain() {
    let _ = tx.send(Event::Done(Err(RemoteError::Disconnected(why.to_owned()))));
  }
  let Some(mut child) = inner.child.lock().take() else { return };
  if !graceful {
    let _ = child.start_kill();
    return;
  }
  let _ = inner.lines.send(String::new());
  tokio::spawn(async move {
    if tokio::time::timeout(CLOSE_GRACE, child.wait()).await.is_err() {
      let _ = child.start_kill();
    }
  });
}
