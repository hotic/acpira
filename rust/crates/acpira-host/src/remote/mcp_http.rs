//! Local loopback Streamable HTTP bridge for stdio MCP servers (`LocalRun::Exec`).
//!
//! Codex 0.170.0 rejects local stdio MCP servers whenever `CODEX_EXEC_SERVER_URL` replaces the thread's
//! local environment (`local stdio MCP server '<name>' requires a local environment`), while connecting
//! directly from this machine to Streamable HTTP MCP entries passed in `session/new` / `resume` / `load`.
//! Rewriting stdio entries (`acpira`, `acpira mcp-host` project shims, user stdio servers) onto a
//! session-owned loopback HTTP bridge keeps every MCP server working under Codex SSH local run.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{oneshot, watch};

use crate::store::transcript_store::LogFn;
use crate::util::{random_hex, random_uuid};

/// Maximum HTTP request head size (headers + request line)
const MAX_HEAD: usize = 64 * 1024;
/// Maximum HTTP request body or single stdio JSON-RPC response line (16 MiB covers large tool payloads)
const MAX_BODY: usize = 16 * 1024 * 1024;
/// Capped child stderr tail kept for startup / exit diagnostics
const MAX_STDERR: usize = 4096;

/// One stdio MCP server registered on the bridge
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StdioSpec {
  pub name: String,
  pub command: String,
  pub args: Vec<String>,
  pub env: Vec<(String, String)>,
}

/// Extract a stdio specification from an ACP `mcpServers` entry, or None for HTTP / SSE / ACP entries
pub fn stdio_spec(entry: &Value) -> Option<StdioSpec> {
  match entry.get("type").and_then(Value::as_str) {
    None | Some("stdio") => {}
    _ => return None,
  }
  let name = entry.get("name").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_owned();
  let command = entry.get("command").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_owned();
  let args = entry
    .get("args")
    .and_then(Value::as_array)
    .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
    .unwrap_or_default();
  let env = entry
    .get("env")
    .and_then(Value::as_array)
    .map(|arr| {
      arr
        .iter()
        .filter_map(|pair| {
          let k = pair.get("name").and_then(Value::as_str)?;
          let v = pair.get("value").and_then(Value::as_str)?;
          Some((k.to_owned(), v.to_owned()))
        })
        .collect()
    })
    .unwrap_or_default();
  Some(StdioSpec { name, command, args, env })
}

struct Pending {
  client_id: Value,
  tx: oneshot::Sender<Result<Value, String>>,
}

#[derive(Default)]
struct PendingState {
  map: HashMap<u64, Pending>,
  dead: Option<String>,
}

/// One spawned stdio MCP server process behind a Streamable HTTP session (`Mcp-Session-Id`)
struct StdioChild {
  name: String,
  stdin: tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
  state: parking_lot::Mutex<PendingState>,
  next_id: AtomicU64,
  stderr: Arc<parking_lot::Mutex<String>>,
  child: parking_lot::Mutex<Option<tokio::process::Child>>,
  tasks: parking_lot::Mutex<Vec<tokio::task::AbortHandle>>,
}

impl StdioChild {
  fn spawn(spec: &StdioSpec, cwd: &str, log: LogFn) -> Result<Arc<Self>, String> {
    let mut cmd = crate::acp::agents::launch::command(&spec.command, &spec.args);
    if !cwd.is_empty() && Path::new(cwd).is_dir() {
      cmd.current_dir(cwd);
    }
    for (k, v) in &spec.env {
      cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("failed to start MCP server `{}` ({}): {e}", spec.name, spec.command))?;
    let stdin = child.stdin.take().ok_or_else(|| format!("MCP server `{}` has no stdin pipe", spec.name))?;
    let stdout = child.stdout.take().ok_or_else(|| format!("MCP server `{}` has no stdout pipe", spec.name))?;
    let stderr = child.stderr.take().ok_or_else(|| format!("MCP server `{}` has no stderr pipe", spec.name))?;

    let stderr_tail = Arc::new(parking_lot::Mutex::new(String::new()));
    let me = Arc::new(StdioChild {
      name: spec.name.clone(),
      stdin: tokio::sync::Mutex::new(Some(stdin)),
      state: parking_lot::Mutex::new(PendingState::default()),
      next_id: AtomicU64::new(1),
      stderr: stderr_tail.clone(),
      child: parking_lot::Mutex::new(Some(child)),
      tasks: parking_lot::Mutex::new(Vec::new()),
    });

    let err_task = tokio::spawn(async move {
      let mut reader = stderr;
      let mut buf = [0u8; 2048];
      loop {
        match reader.read(&mut buf).await {
          Ok(0) | Err(_) => break,
          Ok(n) => {
            let chunk = String::from_utf8_lossy(&buf[..n]);
            let mut tail = stderr_tail.lock();
            tail.push_str(&chunk);
            if tail.len() > MAX_STDERR {
              let mut cut = tail.len() - (MAX_STDERR / 2);
              while !tail.is_char_boundary(cut) {
                cut += 1;
              }
              tail.drain(..cut);
            }
          }
        }
      }
    })
    .abort_handle();

    let weak = Arc::downgrade(&me);
    let name = spec.name.clone();
    let out_task = tokio::spawn(async move {
      let mut reader = BufReader::new(stdout);
      let mut buf = Vec::with_capacity(4096);
      loop {
        buf.clear();
        match read_bounded_line(&mut reader, &mut buf, MAX_BODY).await {
          Ok(0) => break,
          Ok(_) => {
            let line = String::from_utf8_lossy(&buf);
            let trimmed = line.trim();
            if trimmed.is_empty() {
              continue;
            }
            let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
              log(&format!("remote MCP `{name}`: ignored non-JSON stdout line"));
              continue;
            };
            let Some(me) = weak.upgrade() else { return };
            me.on_stdout(msg).await;
          }
          Err(e) => {
            log(&format!("remote MCP `{name}`: stdout read error: {e}"));
            break;
          }
        }
      }
      let Some(me) = weak.upgrade() else { return };
      // Give the stderr reader a brief moment to collect any final error lines emitted before exit
      tokio::time::sleep(std::time::Duration::from_millis(20)).await;
      let tail = last_non_empty_line(&me.stderr.lock());
      let why = if tail.is_empty() {
        format!("MCP server `{name}` closed stdout")
      } else {
        format!("MCP server `{name}` exited: {tail}")
      };
      log(&format!("remote MCP `{name}` ended{}", if tail.is_empty() { String::new() } else { format!(": {tail}") }));
      me.mark_dead(why);
    })
    .abort_handle();

    *me.tasks.lock() = vec![err_task, out_task];
    Ok(me)
  }

  async fn on_stdout(&self, msg: Value) {
    let has_id = msg.get("id").is_some_and(|v| !v.is_null());
    let is_response = has_id && (msg.get("result").is_some() || msg.get("error").is_some());
    if is_response {
      if let Some(wire_id) = msg.get("id").and_then(Value::as_u64) {
        let waiter = self.state.lock().map.remove(&wire_id);
        if let Some(pending) = waiter {
          let _ = pending.tx.send(Ok(msg));
        }
      }
      return;
    }
    // Server-to-client request: reply MethodNotFound so the child never stalls waiting on us
    if has_id && msg.get("method").is_some() {
      let id = msg.get("id").cloned().unwrap_or(Value::Null);
      let reply = json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": "Client does not handle server requests over this bridge" }
      });
      let _ = self.write_line(&reply).await;
    }
  }

  fn mark_dead(&self, why: String) {
    let waiters = {
      let mut st = self.state.lock();
      if st.dead.is_none() {
        st.dead = Some(why.clone());
      }
      std::mem::take(&mut st.map)
    };
    for (_, pending) in waiters {
      let _ = pending.tx.send(Err(why.clone()));
    }
  }

  fn is_dead(&self) -> bool {
    self.state.lock().dead.is_some()
  }

  async fn write_line(&self, msg: &Value) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(msg).map_err(|e| format!("failed to encode JSON-RPC message: {e}"))?;
    bytes.push(b'\n');
    let mut guard = self.stdin.lock().await;
    let Some(stdin) = guard.as_mut() else {
      return Err(format!("MCP server `{}` stdin is closed", self.name));
    };
    stdin.write_all(&bytes).await.map_err(|e| format!("failed to write to MCP server `{}`: {e}", self.name))?;
    stdin.flush().await.map_err(|e| format!("failed to flush MCP server `{}`: {e}", self.name))?;
    Ok(())
  }

  /// Send a JSON-RPC request and await its matching response; restores the caller's original `id`
  async fn call(&self, mut msg: Value) -> Result<Value, String> {
    let client_id = msg.get("id").cloned().ok_or_else(|| "missing JSON-RPC request id".to_owned())?;
    let wire_id = self.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    {
      let mut st = self.state.lock();
      if let Some(why) = &st.dead {
        return Err(why.clone());
      }
      st.map.insert(wire_id, Pending { client_id: client_id.clone(), tx });
    }
    struct Guard<'a> {
      child: &'a StdioChild,
      wire_id: u64,
    }
    impl Drop for Guard<'_> {
      fn drop(&mut self) {
        self.child.state.lock().map.remove(&self.wire_id);
      }
    }
    let _guard = Guard { child: self, wire_id };
    msg["id"] = json!(wire_id);
    self.write_line(&msg).await?;
    let mut reply = rx.await.map_err(|_| format!("MCP server `{}` ended before replying", self.name))??;
    reply["id"] = client_id;
    Ok(reply)
  }

  /// Forward a JSON-RPC notification to the child, translating `notifications/cancelled`'s requestId if in flight
  async fn notify(&self, mut msg: Value) -> Result<(), String> {
    if msg.get("method").and_then(Value::as_str) == Some("notifications/cancelled")
      && let Some(req_id) = msg.pointer("/params/requestId").cloned()
    {
      let mapped = self.state.lock().map.iter().find_map(|(wire, p)| (p.client_id == req_id).then_some(*wire));
      if let Some(wire) = mapped
        && let Some(params) = msg.get_mut("params").and_then(Value::as_object_mut)
      {
        params.insert("requestId".into(), json!(wire));
      }
    }
    self.write_line(&msg).await
  }
}

impl Drop for StdioChild {
  fn drop(&mut self) {
    for task in std::mem::take(&mut *self.tasks.lock()) {
      task.abort();
    }
    // Closing stdin first lets cooperating shims (`acpira mcp`, `acpira mcp-host`) notice EOF immediately
    if let Ok(mut guard) = self.stdin.try_lock() {
      guard.take();
    }
    if let Some(mut child) = self.child.lock().take() {
      let _ = child.start_kill();
    }
    self.mark_dead(format!("MCP server `{}` closed", self.name));
  }
}

#[derive(Default)]
struct Sessions {
  closed: bool,
  active: HashMap<(usize, String), Arc<StdioChild>>,
}

struct BridgeState {
  routes: Vec<StdioSpec>,
  cwd: String,
  token: String,
  log: LogFn,
  sessions: parking_lot::Mutex<Sessions>,
}

impl BridgeState {
  fn authorized(&self, head: &HttpHead) -> bool {
    head
      .get("authorization")
      .and_then(|v| v.strip_prefix("Bearer "))
      .is_some_and(|tok| tok == self.token)
  }

  fn parse_route(&self, target: &str) -> Option<usize> {
    let path = target.split_once('?').map_or(target, |(p, _)| p);
    let idx: usize = path.strip_prefix("/mcp/")?.parse().ok()?;
    (idx < self.routes.len()).then_some(idx)
  }

  fn get_session(&self, idx: usize, sid: Option<&str>) -> SessionLookup {
    let guard = self.sessions.lock();
    if let Some(sid) = sid {
      return match guard.active.get(&(idx, sid.to_owned())) {
        Some(child) => SessionLookup::Found(sid.to_owned(), child.clone()),
        None => SessionLookup::NotFound,
      };
    }
    // Stateless fallback: if exactly one session exists for this route, reuse it
    let mut matching = guard.active.iter().filter(|((r, _), _)| *r == idx);
    match (matching.next(), matching.next()) {
      (Some(((_, sid), child)), None) => SessionLookup::Found(sid.clone(), child.clone()),
      _ => SessionLookup::MissingHeader,
    }
  }

  fn insert_session(&self, idx: usize, sid: String, child: Arc<StdioChild>) -> bool {
    let mut guard = self.sessions.lock();
    if guard.closed {
      return false;
    }
    guard.active.insert((idx, sid), child);
    true
  }

  fn remove_session(&self, idx: usize, sid: &str) -> bool {
    self.sessions.lock().active.remove(&(idx, sid.to_owned())).is_some()
  }

  fn close_all(&self) {
    let drained = {
      let mut guard = self.sessions.lock();
      guard.closed = true;
      std::mem::take(&mut guard.active)
    };
    drop(drained);
  }

  async fn handle_request(&self, head: &HttpHead, body: Vec<u8>) -> HttpReply {
    if !self.authorized(head) {
      return HttpReply::plain(401, "Unauthorized", "Unauthorized");
    }
    let Some(idx) = self.parse_route(&head.target) else {
      return HttpReply::plain(404, "Not Found", "Unknown MCP route");
    };
    match head.method.as_str() {
      "POST" => self.handle_post(idx, head, body).await,
      "DELETE" => {
        if let Some(sid) = head.get("mcp-session-id") {
          self.remove_session(idx, sid);
        }
        HttpReply::empty(200, "OK")
      }
      "GET" => HttpReply::method_not_allowed(),
      _ => HttpReply::method_not_allowed(),
    }
  }

  async fn handle_post(&self, idx: usize, head: &HttpHead, body: Vec<u8>) -> HttpReply {
    let msg: Value = match serde_json::from_slice(&body) {
      Ok(v) => v,
      Err(e) => return HttpReply::plain(400, "Bad Request", &format!("Invalid JSON-RPC payload: {e}")),
    };
    let sid_hdr = head.get("mcp-session-id").map(str::trim).filter(|s| !s.is_empty());
    match msg {
      Value::Object(_) => self.handle_single(idx, sid_hdr, msg).await,
      Value::Array(items) => {
        if items.is_empty() {
          return HttpReply::plain(400, "Bad Request", "Empty JSON-RPC batch");
        }
        let mut replies = Vec::new();
        let mut out_sid = sid_hdr.map(str::to_owned);
        for item in items {
          match self.handle_single(idx, out_sid.as_deref(), item).await {
            r if r.status == 202 => {}
            r if r.status == 200 => {
              if let Some(s) = r.session_id {
                out_sid = Some(s);
              }
              if let Ok(v) = serde_json::from_slice::<Value>(&r.body) {
                replies.push(v);
              }
            }
            err => return err,
          }
        }
        if replies.is_empty() {
          HttpReply::empty(202, "Accepted")
        } else {
          HttpReply::json(Value::Array(replies), out_sid)
        }
      }
      _ => HttpReply::plain(400, "Bad Request", "JSON-RPC payload must be an object or array"),
    }
  }

  async fn handle_single(&self, idx: usize, sid_hdr: Option<&str>, msg: Value) -> HttpReply {
    let method = msg.get("method").and_then(Value::as_str);
    let client_id = msg.get("id").filter(|v| !v.is_null()).cloned();
    // Client response (no method)
    let Some(method) = method else {
      return HttpReply::empty(202, "Accepted");
    };
    // Notification (no id)
    let Some(client_id) = client_id else {
      match self.get_session(idx, sid_hdr) {
        SessionLookup::Found(_, child) => {
          let _ = child.notify(msg).await;
          return HttpReply::empty(202, "Accepted");
        }
        SessionLookup::NotFound => return HttpReply::plain(404, "Not Found", "Unknown MCP session"),
        SessionLookup::MissingHeader => return HttpReply::empty(202, "Accepted"),
      }
    };
    // Initialize starts a fresh stdio server process for this session
    if method == "initialize" {
      let Some(spec) = self.routes.get(idx) else {
        return HttpReply::plain(404, "Not Found", "Unknown MCP route");
      };
      let child = match StdioChild::spawn(spec, &self.cwd, self.log.clone()) {
        Ok(c) => c,
        Err(why) => {
          (self.log)(&format!("remote MCP `{}`: {why}", spec.name));
          return HttpReply::json(
            json!({ "jsonrpc": "2.0", "id": client_id, "error": { "code": -32000, "message": why } }),
            None,
          );
        }
      };
      let reply = match child.call(msg).await {
        Ok(r) => r,
        Err(why) => {
          (self.log)(&format!("remote MCP `{}` initialize failed: {why}", spec.name));
          return HttpReply::json(
            json!({ "jsonrpc": "2.0", "id": client_id, "error": { "code": -32000, "message": why } }),
            None,
          );
        }
      };
      let sid = random_uuid();
      if !self.insert_session(idx, sid.clone(), child) {
        return HttpReply::plain(503, "Service Unavailable", "MCP bridge closed");
      }
      return HttpReply::json(reply, Some(sid));
    }
    let (sid, child) = match self.get_session(idx, sid_hdr) {
      SessionLookup::Found(sid, child) => (sid, child),
      SessionLookup::NotFound => return HttpReply::plain(404, "Not Found", "Unknown MCP session"),
      SessionLookup::MissingHeader => return HttpReply::plain(400, "Bad Request", "Missing Mcp-Session-Id"),
    };
    match child.call(msg).await {
      Ok(reply) => HttpReply::json(reply, Some(sid)),
      Err(why) => {
        if child.is_dead() {
          self.remove_session(idx, &sid);
        }
        HttpReply::json(
          json!({ "jsonrpc": "2.0", "id": client_id, "error": { "code": -32000, "message": why } }),
          Some(sid),
        )
      }
    }
  }
}

enum SessionLookup {
  Found(String, Arc<StdioChild>),
  NotFound,
  MissingHeader,
}

/// Loopback Streamable HTTP bridge owned by one `AcpSession` while its CLI runs.
/// Dropped, its listener stops and every spawned stdio MCP child is terminated.
pub struct McpHttpBridge {
  state: Arc<BridgeState>,
  stop: watch::Sender<bool>,
  accept: tokio::task::AbortHandle,
}

impl McpHttpBridge {
  /// Rewrites every stdio entry in `servers` onto a fresh loopback Streamable HTTP bridge.
  /// Returns `(servers, None)` without binding a socket when there are no stdio entries.
  pub async fn rewrite(servers: Vec<Value>, cwd: String, log: LogFn) -> Result<(Vec<Value>, Option<Self>, Vec<String>)> {
    let mut specs = Vec::new();
    let mut positions = Vec::new();
    for (i, entry) in servers.iter().enumerate() {
      if let Some(spec) = stdio_spec(entry) {
        positions.push((i, specs.len()));
        specs.push(spec);
      }
    }
    if specs.is_empty() {
      return Ok((servers, None, Vec::new()));
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.context("cannot open a loopback port for the MCP HTTP bridge")?;
    let port = listener.local_addr()?.port();
    let token = random_hex(32);
    let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
    let mut rewritten = servers;
    for (server_idx, route_idx) in positions {
      if let (Some(slot), Some(spec)) = (rewritten.get_mut(server_idx), specs.get(route_idx)) {
        *slot = json!({
          "type": "http",
          "name": spec.name,
          "url": format!("http://127.0.0.1:{port}/mcp/{route_idx}"),
          "headers": [{ "name": "Authorization", "value": format!("Bearer {token}") }],
        });
      }
    }
    let state = Arc::new(BridgeState {
      routes: specs,
      cwd,
      token,
      log,
      sessions: parking_lot::Mutex::new(Sessions::default()),
    });
    let (stop_tx, stop_rx) = watch::channel(false);
    let accept = tokio::spawn(accept_loop(listener, state.clone(), stop_rx)).abort_handle();
    Ok((rewritten, Some(McpHttpBridge { state, stop: stop_tx, accept }), names))
  }
}

impl Drop for McpHttpBridge {
  fn drop(&mut self) {
    let _ = self.stop.send(true);
    self.accept.abort();
    self.state.close_all();
  }
}

async fn accept_loop(listener: TcpListener, state: Arc<BridgeState>, mut stop: watch::Receiver<bool>) {
  loop {
    tokio::select! {
      _ = stop.changed() => return,
      conn = listener.accept() => {
        let Ok((sock, _)) = conn else { continue };
        let _ = sock.set_nodelay(true);
        let (state, mut stop) = (state.clone(), stop.clone());
        tokio::spawn(async move {
          tokio::select! {
            _ = stop.changed() => {}
            _ = serve_connection(sock, &state) => {}
          }
        });
      }
    }
  }
}

struct HttpHead {
  method: String,
  target: String,
  version: String,
  headers: Vec<(String, String)>,
}

impl HttpHead {
  fn get(&self, name: &str) -> Option<&str> {
    self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
  }

  fn wants_close(&self) -> bool {
    if self.version.eq_ignore_ascii_case("HTTP/1.0") {
      return !self.get("connection").is_some_and(|v| v.to_ascii_lowercase().contains("keep-alive"));
    }
    self.get("connection").is_some_and(|v| v.to_ascii_lowercase().contains("close"))
  }

  fn is_chunked(&self) -> bool {
    self.get("transfer-encoding").is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
  }
}

struct HttpReply {
  status: u16,
  reason: &'static str,
  content_type: Option<&'static str>,
  session_id: Option<String>,
  extra_headers: &'static str,
  body: Vec<u8>,
}

impl HttpReply {
  fn empty(status: u16, reason: &'static str) -> Self {
    HttpReply { status, reason, content_type: None, session_id: None, extra_headers: "", body: Vec::new() }
  }

  fn plain(status: u16, reason: &'static str, msg: &str) -> Self {
    HttpReply { status, reason, content_type: Some("text/plain; charset=utf-8"), session_id: None, extra_headers: "", body: msg.as_bytes().to_vec() }
  }

  fn method_not_allowed() -> Self {
    HttpReply { status: 405, reason: "Method Not Allowed", content_type: None, session_id: None, extra_headers: "Allow: POST, DELETE\r\n", body: Vec::new() }
  }

  fn json(value: Value, session_id: Option<String>) -> Self {
    let body = serde_json::to_vec(&value).unwrap_or_else(|_| b"{}".to_vec());
    HttpReply { status: 200, reason: "OK", content_type: Some("application/json"), session_id, extra_headers: "", body }
  }
}

async fn serve_connection(sock: TcpStream, state: &BridgeState) {
  let mut reader = BufReader::new(sock);
  loop {
    let head = match read_head(&mut reader).await {
      Ok(Some(h)) => h,
      Ok(None) => return,
      Err(ReadHeadError::TooLarge) => {
        let _ = write_reply(reader.get_mut(), &HttpReply::plain(431, "Request Header Fields Too Large", "Headers too large"), true).await;
        return;
      }
      Err(ReadHeadError::Malformed) => {
        let _ = write_reply(reader.get_mut(), &HttpReply::plain(400, "Bad Request", "Malformed HTTP request"), true).await;
        return;
      }
      Err(ReadHeadError::Io) => return,
    };
    if head.get("expect").is_some_and(|v| v.trim().eq_ignore_ascii_case("100-continue"))
      && reader.get_mut().write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await.is_err()
    {
      return;
    }
    let body = match read_body(&mut reader, &head).await {
      Ok(b) => b,
      Err(ReadBodyError::TooLarge) => {
        let _ = write_reply(reader.get_mut(), &HttpReply::plain(413, "Payload Too Large", "Payload too large"), true).await;
        return;
      }
      Err(ReadBodyError::Malformed) => {
        let _ = write_reply(reader.get_mut(), &HttpReply::plain(400, "Bad Request", "Malformed HTTP body"), true).await;
        return;
      }
      Err(ReadBodyError::Io) => return,
    };
    let close = head.wants_close();
    let reply = state.handle_request(&head, body).await;
    if write_reply(reader.get_mut(), &reply, close).await.is_err() || close {
      return;
    }
  }
}

enum ReadHeadError {
  TooLarge,
  Malformed,
  Io,
}

async fn read_head<R: AsyncRead + Unpin>(reader: &mut BufReader<R>) -> Result<Option<HttpHead>, ReadHeadError> {
  let mut line = Vec::with_capacity(256);
  let mut total = 0usize;
  let n = read_bounded_line(reader, &mut line, MAX_HEAD).await.map_err(|e| if e.kind() == std::io::ErrorKind::InvalidData { ReadHeadError::TooLarge } else { ReadHeadError::Io })?;
  if n == 0 {
    return Ok(None);
  }
  total += n;
  let req_line = std::str::from_utf8(&line).map_err(|_| ReadHeadError::Malformed)?.trim_end_matches(['\r', '\n']);
  let mut parts = req_line.split_whitespace();
  let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
    return Err(ReadHeadError::Malformed);
  };
  let (method, target, version) = (method.to_owned(), target.to_owned(), version.to_owned());
  let mut headers = Vec::new();
  loop {
    line.clear();
    let rem = MAX_HEAD.saturating_sub(total);
    if rem == 0 {
      return Err(ReadHeadError::TooLarge);
    }
    let n = read_bounded_line(reader, &mut line, rem).await.map_err(|e| if e.kind() == std::io::ErrorKind::InvalidData { ReadHeadError::TooLarge } else { ReadHeadError::Io })?;
    if n == 0 {
      return Err(ReadHeadError::Malformed);
    }
    total += n;
    let raw = std::str::from_utf8(&line).map_err(|_| ReadHeadError::Malformed)?.trim_end_matches(['\r', '\n']);
    if raw.is_empty() {
      break;
    }
    let Some((k, v)) = raw.split_once(':') else {
      return Err(ReadHeadError::Malformed);
    };
    headers.push((k.trim().to_owned(), v.trim().to_owned()));
  }
  Ok(Some(HttpHead { method, target, version, headers }))
}

enum ReadBodyError {
  TooLarge,
  Malformed,
  Io,
}

async fn read_body<R: AsyncRead + Unpin>(reader: &mut BufReader<R>, head: &HttpHead) -> Result<Vec<u8>, ReadBodyError> {
  if head.is_chunked() {
    let mut out = Vec::new();
    let mut line = Vec::with_capacity(64);
    loop {
      line.clear();
      let n = read_bounded_line(reader, &mut line, 1024).await.map_err(|_| ReadBodyError::Io)?;
      if n == 0 {
        return Err(ReadBodyError::Malformed);
      }
      let size_str = std::str::from_utf8(&line).map_err(|_| ReadBodyError::Malformed)?.trim_end_matches(['\r', '\n']);
      let hex_part = size_str.split_once(';').map_or(size_str, |(h, _)| h).trim();
      let size = usize::from_str_radix(hex_part, 16).map_err(|_| ReadBodyError::Malformed)?;
      if size == 0 {
        // Consume optional trailers through the terminating empty line
        loop {
          line.clear();
          let n = read_bounded_line(reader, &mut line, 4096).await.map_err(|_| ReadBodyError::Io)?;
          if n == 0 || line == b"\r\n" || line == b"\n" {
            break;
          }
        }
        return Ok(out);
      }
      if out.len().saturating_add(size) > MAX_BODY {
        return Err(ReadBodyError::TooLarge);
      }
      let start = out.len();
      out.resize(start + size, 0);
      reader.read_exact(&mut out[start..]).await.map_err(|_| ReadBodyError::Io)?;
      let mut crlf = [0u8; 2];
      reader.read_exact(&mut crlf).await.map_err(|_| ReadBodyError::Io)?;
      if crlf != *b"\r\n" {
        return Err(ReadBodyError::Malformed);
      }
    }
  }
  let Some(len_str) = head.get("content-length") else {
    return Ok(Vec::new());
  };
  let len: usize = len_str.trim().parse().map_err(|_| ReadBodyError::Malformed)?;
  if len > MAX_BODY {
    return Err(ReadBodyError::TooLarge);
  }
  let mut buf = vec![0u8; len];
  reader.read_exact(&mut buf).await.map_err(|_| ReadBodyError::Io)?;
  Ok(buf)
}

async fn write_reply(sock: &mut TcpStream, reply: &HttpReply, close: bool) -> std::io::Result<()> {
  let mut head = format!(
    "HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: {}\r\n{}",
    reply.status,
    reply.reason,
    reply.body.len(),
    if close { "close" } else { "keep-alive" },
    reply.extra_headers,
  );
  if let Some(ct) = reply.content_type {
    head.push_str(&format!("Content-Type: {ct}\r\n"));
  }
  if let Some(sid) = &reply.session_id {
    head.push_str(&format!("Mcp-Session-Id: {sid}\r\n"));
  }
  head.push_str("\r\n");
  sock.write_all(head.as_bytes()).await?;
  if !reply.body.is_empty() {
    sock.write_all(&reply.body).await?;
  }
  sock.flush().await
}

/// Reads bytes up to and including `\n` into `buf`, capping at `max` bytes
async fn read_bounded_line<R: AsyncRead + Unpin>(reader: &mut BufReader<R>, buf: &mut Vec<u8>, max: usize) -> std::io::Result<usize> {
  let mut total = 0usize;
  loop {
    let available = reader.fill_buf().await?;
    if available.is_empty() {
      return Ok(total);
    }
    let (take, done) = match available.iter().position(|&b| b == b'\n') {
      Some(i) => (i + 1, true),
      None => (available.len(), false),
    };
    if total.saturating_add(take) > max {
      return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "line exceeds limit"));
    }
    buf.extend_from_slice(&available[..take]);
    reader.consume(take);
    total += take;
    if done {
      return Ok(total);
    }
  }
}

fn last_non_empty_line(text: &str) -> String {
  text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim().to_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn noop_log() -> LogFn {
    Arc::new(|_| {})
  }

  #[tokio::test]
  async fn non_stdio_servers_pass_through_without_binding_a_bridge() {
    let servers = vec![json!({ "type": "http", "name": "remote", "url": "https://example.com/mcp", "headers": [] })];
    let (out, bridge, names) = McpHttpBridge::rewrite(servers.clone(), String::new(), noop_log()).await.unwrap();
    assert!(bridge.is_none());
    assert!(names.is_empty());
    assert_eq!(out, servers);
  }

  #[test]
  fn stdio_spec_distinguishes_stdio_from_http_and_sse() {
    let s1 = stdio_spec(&json!({ "name": "acpira", "command": "/bin/acpira", "args": ["mcp"], "env": [{ "name": "A", "value": "1" }] })).unwrap();
    assert_eq!(s1, StdioSpec { name: "acpira".into(), command: "/bin/acpira".into(), args: vec!["mcp".into()], env: vec![("A".into(), "1".into())] });
    let s2 = stdio_spec(&json!({ "type": "stdio", "name": "x", "command": "node" })).unwrap();
    assert_eq!(s2.command, "node");
    assert!(stdio_spec(&json!({ "type": "http", "name": "h", "url": "http://127.0.0.1:1/mcp" })).is_none());
    assert!(stdio_spec(&json!({ "type": "sse", "name": "s", "url": "http://127.0.0.1:1/sse" })).is_none());
    assert!(stdio_spec(&json!({ "type": "acp", "name": "a", "serverId": "1" })).is_none());
  }

  #[cfg(unix)]
  async fn http_exchange(sock: &mut BufReader<TcpStream>, raw: &[u8]) -> (u16, Vec<(String, String)>, Vec<u8>) {
    sock.get_mut().write_all(raw).await.unwrap();
    sock.get_mut().flush().await.unwrap();
    let mut line = Vec::new();
    read_bounded_line(sock, &mut line, MAX_HEAD).await.unwrap();
    let status_line = std::str::from_utf8(&line).unwrap();
    let status: u16 = status_line.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut headers = Vec::new();
    let mut content_len = 0usize;
    loop {
      line.clear();
      read_bounded_line(sock, &mut line, MAX_HEAD).await.unwrap();
      let s = std::str::from_utf8(&line).unwrap().trim_end_matches(['\r', '\n']);
      if s.is_empty() {
        break;
      }
      let (k, v) = s.split_once(':').unwrap();
      if k.eq_ignore_ascii_case("content-length") {
        content_len = v.trim().parse().unwrap();
      }
      headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
    }
    let mut body = vec![0u8; content_len];
    if content_len > 0 {
      sock.read_exact(&mut body).await.unwrap();
    }
    (status, headers, body)
  }

  /// End-to-end Streamable HTTP bridge test over a real stdio child process:
  /// auth checks, GET 405, keep-alive initialize + notifications/initialized + tools/list + chunked tools/call,
  /// DELETE cleanup, and Drop shutdown.
  #[cfg(unix)]
  #[tokio::test]
  async fn bridges_stdio_mcp_over_streamable_http_with_keepalive_chunked_and_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("child.pid");
    // Minimal POSIX sh stdio JSON-RPC echo server (no Node dependency required)
    let script = dir.path().join("mcp.sh");
    std::fs::write(
      &script,
      format!(
        r#"#!/bin/sh
echo "$$" > "{pid}"
while IFS= read -r line; do
  case "$line" in
    *'"method":"initialize"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"mock","version":"1"}}}}}}\n' "$id"
      ;;
    *'"method":"tools/list"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"tools":[{{"name":"ping","inputSchema":{{"type":"object"}}}}]}}}}\n' "$id"
      ;;
    *'"method":"tools/call"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"content":[{{"type":"text","text":"pong:%s"}}]}}}}\n' "$id" "$MCP_TAG"
      ;;
  esac
done
"#,
        pid = pid_file.display()
      ),
    )
    .unwrap();

    let servers = vec![
      json!({ "type": "http", "name": "keep-http", "url": "https://example.com/mcp", "headers": [] }),
      json!({
        "name": "mock-stdio",
        "command": "/bin/sh",
        "args": [script.to_string_lossy()],
        "env": [{ "name": "MCP_TAG", "value": "from-env" }]
      }),
    ];
    let (out, bridge, names) = McpHttpBridge::rewrite(servers, dir.path().to_string_lossy().into_owned(), noop_log()).await.unwrap();
    let bridge = bridge.expect("bridge started");
    assert_eq!(names, vec!["mock-stdio"]);
    assert_eq!(out[0]["type"], "http");
    assert_eq!(out[0]["url"], "https://example.com/mcp");
    assert_eq!(out[1]["type"], "http");
    assert_eq!(out[1]["name"], "mock-stdio");
    let url = out[1]["url"].as_str().unwrap();
    let auth = out[1]["headers"][0]["value"].as_str().unwrap();
    let rest = url.strip_prefix("http://127.0.0.1:").unwrap();
    let (port_str, path_tail) = rest.split_once('/').unwrap();
    let port: u16 = port_str.parse().unwrap();
    let path = format!("/{path_tail}");

    let mut sock = BufReader::new(TcpStream::connect(("127.0.0.1", port)).await.unwrap());

    // 1. Missing auth -> 401
    let (st, _, _) = http_exchange(&mut sock, format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 2\r\n\r\n{{}}").as_bytes()).await;
    assert_eq!(st, 401);

    // 2. GET with auth -> 405 Method Not Allowed (matches Codex SSE probe)
    let (st, _, _) = http_exchange(&mut sock, format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\n\r\n").as_bytes()).await;
    assert_eq!(st, 405);

    // 3. POST initialize -> 200 OK with Mcp-Session-Id and original JSON-RPC id restored
    let init_body = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#;
    let (st, hdrs, body) = http_exchange(
      &mut sock,
      format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{init_body}",
        init_body.len()
      )
      .as_bytes(),
    )
    .await;
    assert_eq!(st, 200);
    let sid = hdrs.iter().find(|(k, _)| k == "mcp-session-id").map(|(_, v)| v.clone()).expect("Mcp-Session-Id");
    let init_json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(init_json["id"], json!(0));
    assert_eq!(init_json["result"]["serverInfo"]["name"], "mock");

    // 4. POST notifications/initialized on the same keep-alive connection -> 202 Accepted
    let notif = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    let (st, _, _) = http_exchange(
      &mut sock,
      format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nMcp-Session-Id: {sid}\r\nContent-Length: {}\r\n\r\n{notif}",
        notif.len()
      )
      .as_bytes(),
    )
    .await;
    assert_eq!(st, 202);

    // 5. POST tools/list -> 200 OK
    let list_body = r#"{"jsonrpc":"2.0","id":"req-list","method":"tools/list","params":{}}"#;
    let (st, _, body) = http_exchange(
      &mut sock,
      format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nMcp-Session-Id: {sid}\r\nContent-Length: {}\r\n\r\n{list_body}",
        list_body.len()
      )
      .as_bytes(),
    )
    .await;
    assert_eq!(st, 200);
    let list_json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(list_json["id"], json!("req-list"));
    assert_eq!(list_json["result"]["tools"][0]["name"], "ping");

    // 6. POST tools/call with Transfer-Encoding: chunked -> 200 OK with env var propagated
    let call_body = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"ping","arguments":{}}}"#;
    let chunked = format!("{:x}\r\n{call_body}\r\n0\r\n\r\n", call_body.len());
    let (st, _, body) = http_exchange(
      &mut sock,
      format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nMcp-Session-Id: {sid}\r\nTransfer-Encoding: chunked\r\n\r\n{chunked}"
      )
      .as_bytes(),
    )
    .await;
    assert_eq!(st, 200);
    let call_json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(call_json["id"], json!(7));
    assert_eq!(call_json["result"]["content"][0]["text"], "pong:from-env");

    // 7. DELETE terminates the stdio child and subsequent requests get 404
    let pid = std::fs::read_to_string(&pid_file).unwrap().trim().to_owned();
    let (st, _, _) = http_exchange(
      &mut sock,
      format!("DELETE {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nMcp-Session-Id: {sid}\r\n\r\n").as_bytes(),
    )
    .await;
    assert_eq!(st, 200);
    let (st, _, _) = http_exchange(
      &mut sock,
      format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nMcp-Session-Id: {sid}\r\nContent-Length: {}\r\n\r\n{list_body}",
        list_body.len()
      )
      .as_bytes(),
    )
    .await;
    assert_eq!(st, 404);

    // Wait briefly for the terminated child to exit
    for _ in 0..40 {
      if !std::process::Command::new("kill").args(["-0", &pid]).stderr(Stdio::null()).status().unwrap().success() {
        break;
      }
      tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert!(!std::process::Command::new("kill").args(["-0", &pid]).stderr(Stdio::null()).status().unwrap().success(), "child pid {pid} should exit after DELETE");

    drop(bridge);
  }

  /// Spawn failures and early exits before initialize return a JSON-RPC error with diagnostics (avoiding HTTP 502 retry storms).
  #[cfg(unix)]
  #[tokio::test]
  async fn startup_errors_and_early_exit_return_jsonrpc_error_with_diagnostics() {
    let servers = vec![
      json!({ "name": "missing-bin", "command": "/nonexistent/mcp-binary-12345", "args": [], "env": [] }),
      json!({ "name": "early-exit", "command": "/bin/sh", "args": ["-c", "echo 'boom on stderr' >&2; exit 1"], "env": [] }),
    ];
    let (out, bridge, _) = McpHttpBridge::rewrite(servers, String::new(), noop_log()).await.unwrap();
    let _bridge = bridge.expect("bridge started");
    let auth = out[0]["headers"][0]["value"].as_str().unwrap();
    let url0 = out[0]["url"].as_str().unwrap();
    let port: u16 = url0.strip_prefix("http://127.0.0.1:").unwrap().split_once('/').unwrap().0.parse().unwrap();
    let init_body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;

    let mut sock = BufReader::new(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
    let (st, _, body) = http_exchange(
      &mut sock,
      format!("POST /mcp/0 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{init_body}", init_body.len()).as_bytes(),
    )
    .await;
    assert_eq!(st, 200);
    let err0: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(err0["id"], json!(1));
    assert!(err0["error"]["message"].as_str().unwrap().contains("missing-bin"));

    let (st, _, body) = http_exchange(
      &mut sock,
      format!("POST /mcp/1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: {auth}\r\nContent-Length: {}\r\n\r\n{init_body}", init_body.len()).as_bytes(),
    )
    .await;
    assert_eq!(st, 200);
    let err1: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(err1["id"], json!(1));
    assert!(err1["error"]["message"].as_str().unwrap().contains("boom on stderr"), "got: {err1}");
  }
}
