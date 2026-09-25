//! ndjson JSON-RPC 2.0 over a child's stdio, the transport ACP runs on. Payloads stay `serde_json::Value`: vendor
//! extensions and `_meta` reach the normalizer untouched, nothing is dropped by a closed schema.
//!
//! Ordering: one reader task handles lines in arrival order. Notifications run synchronously on it, and a response
//! only wakes its waiter, so every `session/update` sent before a response is applied before the awaiting code resumes.
//! The reverse (a notification sent right after a response must see what the awaiting code did with it) holds only for
//! `request_ordered`: the reader waits for the caller to drop its `Handoff` before it reads on.
//! Incoming requests (permission, questions, file access) run as their own tasks and answer when they finish

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use super::cancel::Cancel;

// A single message above this is a broken peer, not a big image (inline images are capped far below)
const MAX_LINE: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
  pub code: i64,
  pub message: String,
  pub data: Option<Value>,
}

impl RpcError {
  pub fn new(code: i64, message: impl Into<String>) -> Self {
    RpcError { code, message: message.into(), data: None }
  }
  pub fn method_not_found(method: &str) -> Self {
    RpcError { code: -32601, message: format!("\"Method not found\": {method}"), data: Some(json!({ "method": method })) }
  }
  /// What the SDK answers when a handler throws a plain Error: "Internal error" with the text under data.details
  pub fn internal(details: impl Into<String>) -> Self {
    RpcError { code: -32603, message: "Internal error".into(), data: Some(json!({ "details": details.into() })) }
  }
  /// A request the peer or the connection cancelled (SDK RequestError.requestCancelled)
  pub fn cancelled() -> Self {
    RpcError::new(-32800, "Request cancelled")
  }
  pub fn connection_closed() -> Self {
    RpcError::new(-32099, "ACP connection closed")
  }
  fn to_json(&self) -> Value {
    let mut v = json!({ "code": self.code, "message": self.message });
    if let Some(d) = &self.data {
      v["data"] = d.clone();
    }
    v
  }
  fn from_json(v: &Value) -> Self {
    RpcError {
      code: v.get("code").and_then(Value::as_i64).unwrap_or(-32603),
      message: v.get("message").and_then(Value::as_str).unwrap_or("Unknown error").to_owned(),
      data: v.get("data").cloned(),
    }
  }
}

impl std::fmt::Display for RpcError {
  // The SDK's RequestError.message is the peer's message as sent
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.message)
  }
}

impl std::error::Error for RpcError {}

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// The client side's business: what to do with the agent's notifications and requests
pub trait Inbound: Send + Sync + 'static {
  fn notification(&self, method: &str, params: Value);
  fn request(&self, method: String, params: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>>;
}

/// Held by the caller of `request_ordered` while it applies the response; the reader reads the next message once it is dropped
pub struct Handoff(#[allow(dead_code)] oneshot::Sender<()>);

// How long the reader waits for a Handoff; a caller holding one this long is a bug, and the wire must not stall on it
const HANDOFF_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

type Reply = Result<(Value, Option<Handoff>), RpcError>;

struct Waiter {
  tx: oneshot::Sender<Reply>,
  ordered: bool,
}

struct Shared {
  pending: parking_lot::Mutex<HashMap<i64, Waiter>>,
  incoming: parking_lot::Mutex<HashMap<String, Cancel>>,
  closed: AtomicBool,
  close_signal: Cancel,
}

#[derive(Clone)]
pub struct Connection {
  out: mpsc::UnboundedSender<String>,
  next_id: Arc<AtomicI64>,
  shared: Arc<Shared>,
}

impl Connection {
  /// Wire a connection over a reader / writer pair. `inbound` is resolved per message, so a warm process can be
  /// rebound to a session without a second spawn
  pub fn start<R, W>(
    reader: R,
    writer: W,
    inbound: Arc<dyn Fn() -> Arc<dyn Inbound> + Send + Sync>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
  ) -> Connection
  where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
  {
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let shared = Arc::new(Shared {
      pending: Default::default(),
      incoming: Default::default(),
      closed: AtomicBool::new(false),
      close_signal: Cancel::new(),
    });
    let conn = Connection { out: tx, next_id: Arc::new(AtomicI64::new(0)), shared: shared.clone() };

    let writer_shared = shared.clone();
    tokio::spawn(async move {
      let mut w = writer;
      while let Some(line) = rx.recv().await {
        if w.write_all(line.as_bytes()).await.is_err() || w.write_all(b"\n").await.is_err() || w.flush().await.is_err() {
          break;
        }
      }
      let _ = w.shutdown().await;
      drop(writer_shared);
    });

    let reader_conn = conn.clone();
    tokio::spawn(async move {
      let mut r = BufReader::with_capacity(64 * 1024, reader);
      let mut buf = Vec::with_capacity(64 * 1024);
      loop {
        buf.clear();
        match read_line(&mut r, &mut buf).await {
          Ok(0) => break,
          Ok(_) => {}
          Err(e) => {
            log(&format!("ACP read failed: {e}"));
            break;
          }
        }
        let text = trim_line(&buf);
        if text.is_empty() {
          continue;
        }
        let msg: Value = match serde_json::from_slice(text) {
          Ok(v) => v,
          Err(_) => {
            log(&format!("ACP stdout is not JSON-RPC: {}", String::from_utf8_lossy(&text[..text.len().min(200)])));
            continue;
          }
        };
        if let Some(ack) = reader_conn.dispatch(msg, &inbound)
          && tokio::time::timeout(HANDOFF_WAIT, ack).await.is_err()
        {
          log("ACP reader: an ordered response was held past the handoff wait");
        }
      }
      reader_conn.close();
    });
    conn
  }

  /// For the response to an ordered request, the signal the reader waits on before the next message
  fn dispatch(&self, mut msg: Value, inbound: &Arc<dyn Fn() -> Arc<dyn Inbound> + Send + Sync>) -> Option<oneshot::Receiver<()>> {
    let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
    let id = msg.get_mut("id").map(Value::take);
    match (method, id) {
      // A response to one of ours
      (None, Some(id)) => {
        let w = self.shared.pending.lock().remove(&id.as_i64()?)?;
        let res = match msg.get("error") {
          Some(e) if !e.is_null() => {
            let _ = w.tx.send(Err(RpcError::from_json(e)));
            return None;
          }
          _ => msg.get_mut("result").map(Value::take).unwrap_or(Value::Null),
        };
        if !w.ordered {
          let _ = w.tx.send(Ok((res, None)));
          return None;
        }
        let (ack_tx, ack_rx) = oneshot::channel();
        w.tx.send(Ok((res, Some(Handoff(ack_tx))))).ok().map(|_| ack_rx)
      }
      (Some(method), None) => {
        let params = msg.get_mut("params").map(Value::take).unwrap_or(Value::Null);
        if method == "$/cancel_request" {
          if let Some(key) = params.get("requestId").map(Value::to_string)
            && let Some(c) = self.shared.incoming.lock().get(&key)
          {
            c.cancel();
          }
          return None;
        }
        inbound().notification(&method, params);
        None
      }
      (Some(method), Some(id)) => {
        let params = msg.get_mut("params").map(Value::take).unwrap_or(Value::Null);
        let key = id.to_string();
        let cancel = self.shared.close_signal.child();
        self.shared.incoming.lock().insert(key.clone(), cancel.clone());
        let fut = inbound().request(method, params, cancel);
        let conn = self.clone();
        tokio::spawn(async move {
          let res = fut.await;
          conn.shared.incoming.lock().remove(&key);
          let line = match res {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(e) => json!({ "jsonrpc": "2.0", "id": id, "error": e.to_json() }),
          };
          conn.send_line(line.to_string());
        });
        None
      }
      (None, None) => None,
    }
  }

  fn send_line(&self, line: String) {
    if !self.shared.closed.load(Ordering::Acquire) {
      let _ = self.out.send(line);
    }
  }

  pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
    self.send_request(method, params, false).await.map(|(v, _)| v)
  }

  /// A request whose response the caller applies before the reader handles anything after it: keep the Handoff until the
  /// state change is done (no awaits in between), then drop it
  pub async fn request_ordered(&self, method: &str, params: Value) -> Result<(Value, Handoff), RpcError> {
    let (v, h) = self.send_request(method, params, true).await?;
    Ok((v, h.expect("an ordered reply carries its handoff")))
  }

  async fn send_request(&self, method: &str, params: Value, ordered: bool) -> Reply {
    let id = self.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    {
      // Checked under the pending lock: close() raises the flag before it drains, so a waiter is either refused here or drained there
      let mut pending = self.shared.pending.lock();
      if self.shared.closed.load(Ordering::Acquire) {
        return Err(RpcError::connection_closed());
      }
      pending.insert(id, Waiter { tx, ordered });
    }
    let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
    if self.out.send(line).is_err() {
      self.shared.pending.lock().remove(&id);
      return Err(RpcError::connection_closed());
    }
    rx.await.unwrap_or_else(|_| Err(RpcError::connection_closed()))
  }

  pub fn notify(&self, method: &str, params: Value) {
    self.send_line(json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string());
  }

  /// Every waiter fails and every incoming request's signal fires; idempotent
  pub fn close(&self) {
    if self.shared.closed.swap(true, Ordering::AcqRel) {
      return;
    }
    self.shared.close_signal.cancel();
    let waiters: Vec<Waiter> = self.shared.pending.lock().drain().map(|(_, w)| w).collect();
    for w in waiters {
      let _ = w.tx.send(Err(RpcError::connection_closed()));
    }
  }

  pub fn is_closed(&self) -> bool {
    self.shared.closed.load(Ordering::Acquire)
  }
}

async fn read_line<R: AsyncBufReadExt + Unpin>(r: &mut R, buf: &mut Vec<u8>) -> std::io::Result<usize> {
  loop {
    let (done, used) = {
      let available = r.fill_buf().await?;
      if available.is_empty() {
        return Ok(buf.len());
      }
      match available.iter().position(|b| *b == b'\n') {
        Some(i) => {
          buf.extend_from_slice(&available[..=i]);
          (true, i + 1)
        }
        None => {
          buf.extend_from_slice(available);
          (false, available.len())
        }
      }
    };
    r.consume(used);
    if buf.len() > MAX_LINE {
      return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "message exceeds the line limit"));
    }
    if done {
      return Ok(buf.len());
    }
  }
}

fn trim_line(b: &[u8]) -> &[u8] {
  let start = b.iter().position(|c| !c.is_ascii_whitespace()).unwrap_or(b.len());
  let end = b.iter().rposition(|c| !c.is_ascii_whitespace()).map(|i| i + 1).unwrap_or(start);
  &b[start..end.max(start)]
}

#[cfg(test)]
mod tests {
  use super::*;

  struct Echo;
  impl Inbound for Echo {
    fn notification(&self, _: &str, _: Value) {}
    fn request(&self, method: String, params: Value, _: Cancel) -> BoxFuture<Result<Value, RpcError>> {
      Box::pin(async move { if method == "echo" { Ok(params) } else { Err(RpcError::method_not_found(&method)) } })
    }
  }

  #[tokio::test]
  async fn request_response_both_ways() {
    let (a_r, b_w) = tokio::io::duplex(1 << 16);
    let (b_r, a_w) = tokio::io::duplex(1 << 16);
    let log: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|_| {});
    let echo: Arc<dyn Fn() -> Arc<dyn Inbound> + Send + Sync> = Arc::new(|| Arc::new(Echo));
    let a = Connection::start(a_r, a_w, echo.clone(), log.clone());
    let b = Connection::start(b_r, b_w, echo, log);
    assert_eq!(a.request("echo", json!({ "x": 1 })).await.unwrap(), json!({ "x": 1 }));
    assert_eq!(b.request("nope", json!({})).await.unwrap_err().code, -32601);
    a.close();
    assert_eq!(a.request("echo", json!({})).await.unwrap_err().code, -32099);
  }

  // A notification sent right behind the response to an ordered request is handled only after the caller dropped its Handoff
  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn an_ordered_response_is_applied_before_the_next_notification() {
    struct Seen(Arc<AtomicBool>, Arc<parking_lot::Mutex<Option<bool>>>);
    impl Inbound for Seen {
      fn notification(&self, _: &str, _: Value) {
        *self.1.lock() = Some(self.0.load(Ordering::SeqCst));
      }
      fn request(&self, _: String, _: Value, _: Cancel) -> BoxFuture<Result<Value, RpcError>> {
        Box::pin(async { Ok(Value::Null) })
      }
    }
    let applied = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(parking_lot::Mutex::new(None));
    let (r, mut peer_w) = tokio::io::duplex(1 << 16);
    let (peer_r, w) = tokio::io::duplex(1 << 16);
    let inbound: Arc<dyn Inbound> = Arc::new(Seen(applied.clone(), seen.clone()));
    let conn = Connection::start(r, w, Arc::new(move || inbound.clone()), Arc::new(|_| {}));
    tokio::spawn(async move {
      let mut lines = BufReader::new(peer_r).lines();
      let req: Value = serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
      let both = format!(
        "{}\n{}\n",
        json!({ "jsonrpc": "2.0", "id": req["id"], "result": { "ok": true } }),
        json!({ "jsonrpc": "2.0", "method": "session/update", "params": {} })
      );
      peer_w.write_all(both.as_bytes()).await.unwrap();
      std::future::pending::<()>().await;
    });
    let (res, handoff) = conn.request_ordered("session/new", json!({})).await.unwrap();
    assert_eq!(res, json!({ "ok": true }));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(seen.lock().is_none(), "the reader waits for the handoff");
    applied.store(true, Ordering::SeqCst);
    drop(handoff);
    for _ in 0..100 {
      if seen.lock().is_some() {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(*seen.lock(), Some(true));
  }

  // A request racing close() must settle: either refused up front or failed by the drain, never parked on a waiter nobody drains
  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_request_racing_close_always_settles() {
    let log: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(|_| {});
    let echo: Arc<dyn Fn() -> Arc<dyn Inbound> + Send + Sync> = Arc::new(|| Arc::new(Echo));
    for _ in 0..2_000 {
      // The peer never answers, so only close() can settle the request
      let (r, _peer_w) = tokio::io::duplex(1 << 16);
      let (_peer_r, w) = tokio::io::duplex(1 << 16);
      let conn = Connection::start(r, w, echo.clone(), log.clone());
      let closer = conn.clone();
      let req = tokio::spawn(async move { conn.request("echo", json!({})).await });
      tokio::spawn(async move { closer.close() });
      let res = tokio::time::timeout(std::time::Duration::from_secs(2), req).await.expect("request settles after close").unwrap();
      assert_eq!(res.unwrap_err().code, -32099);
    }
  }
}
