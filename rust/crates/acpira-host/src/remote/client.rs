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

use super::wire::{self, BULK_WINDOW, CHUNK, DEAD_AFTER_MS, Frame, Hello, PING_EVERY_MS, PROTOCOL, WireError};

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
  /// Control frames, ordinary requests and interactive `stdin`: always sent ahead of `bulk`
  ctrl: mpsc::UnboundedSender<(String, usize)>,
  /// Background bulk `stdin` frames (`Call::send_bulk`): sent one chunk at a time while `BULK_WINDOW` has room
  bulk: mpsc::Sender<(u64, String, usize)>,
  acks: mpsc::UnboundedSender<usize>,
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
    Self::start_io(stdin, stdout, Some(child), stderr, stderr_text, opts).await
  }

  pub(super) async fn start_io<R, W>(
    stdin: W,
    stdout: R,
    mut child: Option<Child>,
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_text: impl Fn(&Arc<Mutex<VecDeque<String>>>) -> String,
    opts: ClientOpts,
  ) -> Result<RemoteClient, StartError>
  where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
  {
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
        let exit_code = match child.as_mut() {
          Some(c) => {
            let _ = c.start_kill();
            tokio::time::timeout(Duration::from_secs(2), c.wait()).await.ok().and_then(Result::ok).and_then(|s| s.code())
          }
          None => None,
        };
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
      if let Some(c) = child.as_mut() {
        let _ = c.start_kill();
      }
      return Err(StartError {
        message: format!("the host's helper speaks protocol {}, this engine {PROTOCOL}", hello.protocol),
        stderr: stderr_text(&stderr),
        noise,
        exit_code: None,
      });
    }

    let stdin_ack = hello.stdin_ack;
    let (ctrl_tx, mut ctrl_rx) = mpsc::unbounded_channel::<(String, usize)>();
    let (bulk_tx, mut bulk_rx) = mpsc::channel::<(u64, String, usize)>(2);
    let (ack_tx, mut ack_rx) = mpsc::unbounded_channel::<usize>();
    let (closed, _) = watch::channel(None);
    let inner = Arc::new(Inner {
      hello,
      ctrl: ctrl_tx,
      bulk: bulk_tx,
      acks: ack_tx,
      pending: Mutex::default(),
      next_id: AtomicU64::new(1),
      closed,
      rtt_ms: AtomicU32::new(0),
      pings: Mutex::default(),
      last_seen: Mutex::new(tokio::time::Instant::now()),
      stderr,
      child: Mutex::new(child),
    });

    // Writer: control frames and ordinary requests first; bulk `stdin` chunks only while `BULK_WINDOW` has room
    {
      let inner = inner.clone();
      let mut stdin = stdin;
      tokio::spawn(async move {
        let mut in_flight = 0usize;
        loop {
          let (line, n) = tokio::select! {
            biased;
            c = ctrl_rx.recv() => match c {
              Some(c) => c,
              None => break,
            },
            a = ack_rx.recv() => {
              if let Some(a) = a {
                in_flight = in_flight.saturating_sub(a);
              }
              continue;
            }
            b = bulk_rx.recv(), if !stdin_ack || in_flight < BULK_WINDOW => match b {
              Some((id, line, n)) => {
                // Already answered or cancelled while queued: save the wire bytes too
                if !inner.pending.lock().contains_key(&id) {
                  continue;
                }
                (line, n)
              }
              None => continue,
            },
          };
          // The end marker: dropping stdin is the helper's EOF, which it answers by cleaning up and exiting
          if line.is_empty() {
            break;
          }
          if stdin_ack {
            in_flight += n;
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

  /// `next` for a poll-based reader (`remote/tcp.rs`)
  pub fn poll_next(&mut self, cx: &mut std::task::Context<'_>) -> std::task::Poll<Option<Event>> {
    let e = std::task::ready!(self.rx.poll_recv(cx));
    if matches!(e, Some(Event::Done(_)) | None) {
      self.done.store(true, Ordering::Relaxed);
    }
    std::task::Poll::Ready(e)
  }

  /// Bytes for an exec's stdin
  pub fn stdin(&self, bytes: &[u8]) {
    send_counted(&self.client.inner, &Frame::Stdin { id: self.id, b: wire::b64(bytes), eof: false }, bytes.len());
  }

  pub fn stdin_eof(&self) {
    send(&self.client.inner, &Frame::Stdin { id: self.id, b: String::new(), eof: true });
  }

  /// Send `bytes` as this request's bulk `stdin` body, yielding to ordinary requests and control frames between chunks
  /// and keeping at most `BULK_WINDOW` bytes unacknowledged on helpers that advertise `stdinAck`
  pub async fn send_bulk(&self, bytes: &[u8]) -> Result<(), RemoteError> {
    if let Some(why) = self.client.closed_reason() {
      return Err(RemoteError::Disconnected(why));
    }
    if !self.client.inner.pending.lock().contains_key(&self.id) {
      return Ok(());
    }
    if bytes.is_empty() {
      self.stdin_eof();
      return Ok(());
    }
    let chunks: Vec<&[u8]> = bytes.chunks(CHUNK).collect();
    let last = chunks.len().saturating_sub(1);
    for (i, part) in chunks.into_iter().enumerate() {
      if let Some(why) = self.client.closed_reason() {
        return Err(RemoteError::Disconnected(why));
      }
      if !self.client.inner.pending.lock().contains_key(&self.id) {
        return Ok(());
      }
      let frame = Frame::Stdin { id: self.id, b: wire::b64(part), eof: i == last };
      let Ok(line) = serde_json::to_string(&frame) else { continue };
      if self.client.inner.bulk.send((self.id, line, part.len())).await.is_err() {
        return Err(RemoteError::Disconnected(self.client.closed_reason().unwrap_or_else(|| "closed".into())));
      }
    }
    Ok(())
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
  send_counted(inner, f, 0);
}

fn send_counted(inner: &Inner, f: &Frame, stdin_bytes: usize) {
  if let Ok(line) = serde_json::to_string(f) {
    let _ = inner.ctrl.send((line, stdin_bytes));
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
    Frame::Ack { n } => {
      let _ = inner.acks.send(n);
    }
    Frame::Req { .. } | Frame::Cancel { .. } | Frame::Stdin { .. } => {}
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
  // Wake the writer in both modes so `stdin` and `bulk_rx` drop right away
  let _ = inner.ctrl.send((String::new(), 0));
  let Some(mut child) = inner.child.lock().take() else { return };
  if !graceful {
    let _ = child.start_kill();
    return;
  }
  tokio::spawn(async move {
    if tokio::time::timeout(CLOSE_GRACE, child.wait()).await.is_err() {
      let _ = child.start_kill();
    }
  });
}

#[cfg(test)]
pub(super) async fn test_channel(slice_bytes: usize, pause: Duration) -> (RemoteClient, Arc< std::sync::atomic::AtomicUsize >) {
  use tokio::io::AsyncReadExt;
  use crate::remote::helper::{self, HelperOpts};
  let (client_w, mut pump_r) = tokio::io::duplex(16 * 1024);
  let (mut pump_w, helper_r) = tokio::io::duplex(16 * 1024);
  let (helper_w, client_r) = tokio::io::duplex(64 * 1024);
  let uplink = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let counted = uplink.clone();
  tokio::spawn(async move {
    let mut buf = vec![0u8; slice_bytes.max(1024)];
    while let Ok(n) = pump_r.read(&mut buf).await {
      if n == 0 {
        break;
      }
      counted.fetch_add(n, Ordering::Relaxed);
      if !pause.is_zero() {
        tokio::time::sleep(pause).await;
      }
      if pump_w.write_all(&buf[..n]).await.is_err() || pump_w.flush().await.is_err() {
        break;
      }
    }
  });
  tokio::spawn(helper::serve(
    helper_r,
    helper_w,
    HelperOpts { version: "test".into(), dead_after_ms: DEAD_AFTER_MS, ping_every_ms: PING_EVERY_MS, login_path: false, digest: String::new() },
  ));
  let client = RemoteClient::start_io(client_w, client_r, None, Arc::default(), |_| String::new(), ClientOpts::default()).await.unwrap();
  (client, uplink)
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::*;
  use crate::remote::host_store::call_with_body;

  #[tokio::test]
  async fn tool_requests_jump_ahead_of_bulk_uploads_on_a_throttled_link() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    // ~2 MiB/s uplink (16 KiB per 8 ms): a 3 MiB blob (~4 MiB base64 on the wire) takes ~2 s end-to-end
    let (client, uplink) = test_channel(16 * 1024, Duration::from_millis(8)).await;
    assert!(client.hello().stdin_ack);

    let blob: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let blob_name = crate::store::transcript_store::blob_name(".bin", &blob);
    let client_for_blob = client.clone();
    let home_str = home.to_string_lossy().into_owned();
    let name_for_blob = blob_name.clone();
    let payload = blob.clone();
    let mut upload = tokio::spawn(async move {
      call_with_body(&client_for_blob, "storeBlob", json!({ "home": home_str, "id": "s-bulk", "name": name_for_blob }), &payload).await
    });

    // Wait until the bulk upload has filled the window and is actively saturating the link
    while uplink.load(Ordering::Relaxed) < BULK_WINDOW {
      tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // An ordinary request (`info`, like `tool`) must jump the remaining ~3.7 MiB instead of waiting ~1.8 s behind it
    let t0 = tokio::time::Instant::now();
    let reply = client.call("info", json!({})).await.unwrap();
    let elapsed = t0.elapsed();
    let sent_so_far = uplink.load(Ordering::Relaxed);
    assert!(reply.value.get("path").is_some());
    assert!(elapsed < Duration::from_millis(750), "ordinary request waited {elapsed:?} behind bulk upload");
    assert!(sent_so_far < 2 * 1024 * 1024, "bulk upload was not still in flight ({sent_so_far} bytes sent)");
    assert!(!upload.is_finished(), "bulk upload finished before the priority check");

    (&mut upload).await.unwrap().unwrap();
    assert_eq!(std::fs::read(home.join("sessions/s-bulk").join(&blob_name)).unwrap(), blob);
    client.close();
  }

  #[tokio::test]
  async fn bulk_uploads_stop_once_the_helper_answers_early() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let store = crate::store::transcript_store::TranscriptStore::new(home.join("sessions"), Arc::new(|_| {}), None);
    let owned: crate::store::record::SessionRecord = serde_json::from_value(json!({
      "id": "owned", "agent": "acpira", "acpSessionId": "native", "cwd": "/work", "title": "owned",
      "updatedAt": "2026-01-01T00:00:00.000Z", "turns": []
    }))
    .unwrap();
    store.flush(Arc::new(owned)).await.unwrap();

    let (client, uplink) = test_channel(16 * 1024, Duration::from_millis(2)).await;
    let huge = vec![b'x'; 4 * 1024 * 1024];
    let reply = call_with_body(
      &client,
      "storeAgentLog",
      json!({ "home": home, "sessionId": "owned", "id": "native" }),
      &huge,
    )
    .await
    .unwrap();
    assert_eq!(reply["written"], false);
    assert_eq!(reply["takenOver"], true);
    let total = uplink.load(Ordering::Relaxed);
    assert!(total < 2 * BULK_WINDOW, "kept uploading {total} bytes after the helper rejected the write");
    client.close();
  }
}
