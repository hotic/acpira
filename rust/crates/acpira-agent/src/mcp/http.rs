//! The streamable HTTP transport (MCP 2025-03-26 and later): every message is a POST, answered with plain JSON or with
//! an event stream that carries progress notifications before the response. `Mcp-Session-Id` from the `initialize`
//! answer goes on every later request, the negotiated `MCP-Protocol-Version` too. A 404 on a request that carried a
//! session id means the server forgot the session: the client initializes again and repeats the request once.
//! Blocking ureq, called from the blocking pool. A server on this machine or a private network is reached without the
//! user's proxy, as `webfetch` does

use std::io::BufReader;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

use super::Progress;
use crate::llm::sse::SseReader;

/// A JSON answer above this is refused (a tool result with a few screenshots is far below)
const MAX_BODY: u64 = 64 * 1024 * 1024;

pub struct Http {
  url: String,
  headers: Vec<(String, String)>,
  agent: ureq::Agent,
  session: parking_lot::Mutex<Option<String>>,
  protocol: parking_lot::Mutex<Option<String>>,
  /// The `initialize` params, kept to start a new session when the server forgot the old one
  init: parking_lot::Mutex<Option<Value>>,
  next_id: AtomicI64,
  stale: Arc<AtomicBool>,
}

/// Why a POST failed
enum Failure {
  /// 404 on a request with a session id: initialize again
  SessionGone,
  Other(String),
}

impl Http {
  pub fn new(url: &str, headers: &[(String, String)], stale: Arc<AtomicBool>) -> Http {
    let mut config = ureq::Agent::config_builder()
      .http_status_as_error(false)
      .timeout_connect(Some(Duration::from_secs(15)))
      // A tool call may stream for as long as it runs; the caller bounds the whole call
      .timeout_recv_body(None)
      .user_agent(format!("acpira/{}", env!("CARGO_PKG_VERSION")));
    if crate::tools::web::private_name(url) {
      config = config.proxy(None);
    }
    Http {
      url: url.to_owned(),
      headers: headers.to_vec(),
      agent: config.build().into(),
      session: Default::default(),
      protocol: Default::default(),
      init: Default::default(),
      next_id: AtomicI64::new(0),
      stale,
    }
  }

  pub fn set_protocol(&self, version: &str) {
    *self.protocol.lock() = Some(version.to_owned());
  }

  pub fn request(&self, method: &str, params: Value, progress: Option<Progress>) -> Result<Value, String> {
    if method == "initialize" {
      *self.init.lock() = Some(params.clone());
      *self.session.lock() = None;
    }
    match self.call(method, params.clone(), progress.clone()) {
      Err(Failure::SessionGone) => {
        self.reinitialize()?;
        self.call(method, params, progress).map_err(|f| match f {
          Failure::SessionGone => "the server dropped the session again".to_owned(),
          Failure::Other(e) => e,
        })
      }
      Err(Failure::Other(e)) => Err(e),
      Ok(v) => Ok(v),
    }
  }

  fn reinitialize(&self) -> Result<(), String> {
    let init = self.init.lock().clone().ok_or("the server dropped the session")?;
    *self.session.lock() = None;
    let answer = self.call("initialize", init, None).map_err(|_| "the server dropped the session and refused a new one".to_owned())?;
    if let Some(v) = answer.get("protocolVersion").and_then(Value::as_str) {
      self.set_protocol(v);
    }
    self.notify("notifications/initialized", json!({}));
    Ok(())
  }

  pub fn notify(&self, method: &str, params: Value) {
    let body = json!({ "jsonrpc": "2.0", "method": method, "params": params });
    if let Ok(mut res) = self.post(&body) {
      let _ = res.body_mut().with_config().limit(64 * 1024).read_to_string();
    }
  }

  /// End the server's session (best effort, off the caller's thread: closing must not wait on the network)
  pub fn close(&self) {
    let Some(id) = self.session.lock().take() else { return };
    let (agent, url) = (self.agent.clone(), self.url.clone());
    let headers = self.headers.clone();
    std::thread::spawn(move || {
      let mut req = agent.delete(&url).header("Mcp-Session-Id", &id);
      for (k, v) in &headers {
        req = req.header(k, v);
      }
      let _ = req.config().timeout_global(Some(Duration::from_secs(5))).build().call();
    });
  }

  fn post(&self, body: &Value) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let mut req = self.agent.post(&self.url).header("Content-Type", "application/json").header("Accept", "application/json, text/event-stream");
    if let Some(id) = self.session.lock().clone() {
      req = req.header("Mcp-Session-Id", &id);
    }
    if let Some(v) = self.protocol.lock().clone() {
      req = req.header("MCP-Protocol-Version", &v);
    }
    for (k, v) in &self.headers {
      req = req.header(k, v);
    }
    req.send(body.to_string())
  }

  fn call(&self, method: &str, mut params: Value, progress: Option<Progress>) -> Result<Value, Failure> {
    let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let token = progress.as_ref().map(|_| format!("acpira-{id}"));
    if let Some(t) = &token {
      params["_meta"]["progressToken"] = Value::String(t.clone());
    }
    let had_session = self.session.lock().is_some();
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let mut res = self.post(&body).map_err(|e| Failure::Other(e.to_string()))?;
    let status = res.status();
    if status == 404 && had_session && method != "initialize" {
      return Err(Failure::SessionGone);
    }
    if let Some(sid) = res.headers().get("mcp-session-id").and_then(|v| v.to_str().ok()) {
      *self.session.lock() = Some(sid.to_owned());
    }
    let is_stream = res.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.contains("text/event-stream"));
    if !status.is_success() {
      let text = res.body_mut().with_config().limit(64 * 1024).read_to_string().unwrap_or_default();
      let detail = crate::llm::error_message(&text);
      return Err(Failure::Other(if status == 401 || status == 403 {
        format!("HTTP {status}: the server refused the credentials ({detail})")
      } else {
        format!("HTTP {status}: {detail}")
      }));
    }
    let reply = if is_stream {
      self.read_stream(res, id, token.as_deref(), progress.as_ref())?
    } else {
      let text = res.body_mut().with_config().limit(MAX_BODY).read_to_string().map_err(|e| Failure::Other(e.to_string()))?;
      let parsed: Value = serde_json::from_str(text.trim()).map_err(|_| Failure::Other(format!("not a JSON-RPC answer: {}", crate::budget::cut(text.trim(), 300))))?;
      // A batch answer holds ours among others
      match parsed {
        Value::Array(all) => all.into_iter().find(|m| m.get("id").and_then(Value::as_i64) == Some(id)).ok_or_else(|| Failure::Other("the answer did not include this request".into()))?,
        one => one,
      }
    };
    match reply.get("error").filter(|e| !e.is_null()) {
      Some(e) => Err(Failure::Other(e.get("message").and_then(Value::as_str).unwrap_or("unknown error").to_owned())),
      None => Ok(reply.get("result").cloned().unwrap_or(Value::Null)),
    }
  }

  /// The events up to our response: progress goes to the caller, a changed tool list marks the list stale
  fn read_stream(&self, res: ureq::http::Response<ureq::Body>, id: i64, token: Option<&str>, progress: Option<&Progress>) -> Result<Value, Failure> {
    let mut events = SseReader::new(BufReader::new(res.into_body().into_reader()));
    loop {
      let ev = match events.next_event() {
        Ok(Some(ev)) => ev,
        Ok(None) => return Err(Failure::Other("the stream ended without an answer".into())),
        Err(e) => return Err(Failure::Other(format!("the stream broke: {e}"))),
      };
      let Ok(msg) = serde_json::from_str::<Value>(&ev.data) else { continue };
      match msg.get("method").and_then(Value::as_str) {
        Some("notifications/progress") => {
          let theirs = msg.pointer("/params/progressToken").map(|t| t.as_str().map(str::to_owned).unwrap_or_else(|| t.to_string()));
          if let (Some(p), Some(t)) = (progress, token)
            && theirs.as_deref() == Some(t)
          {
            let text = super::progress_text(&msg["params"]);
            if !text.is_empty() {
              p(text);
            }
          }
        }
        Some("notifications/tools/list_changed") => self.stale.store(true, Ordering::Release),
        Some(_) => {}
        None if msg.get("id").and_then(Value::as_i64) == Some(id) => return Ok(msg),
        None => {}
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::{BufRead, Read, Write};
  use std::net::TcpListener;

  type Reply = (u16, &'static str, String, Option<&'static str>);

  /// A one-connection-per-request server: `reply(body, head)` answers each request; it stops after `last` requests
  fn serve(last: usize, reply: impl Fn(&Value, &str) -> Reply + Send + 'static) -> (String, std::thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
      let mut seen = vec![];
      for stream in listener.incoming().take(last) {
        let mut stream = stream.unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let (mut head, mut len) = (String::new(), 0usize);
        loop {
          let mut line = String::new();
          reader.read_line(&mut line).unwrap();
          if line.trim().is_empty() {
            break;
          }
          if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap();
          }
          head.push_str(&line.to_ascii_lowercase());
        }
        let mut body = vec![0; len];
        reader.read_exact(&mut body).unwrap();
        let body: Value = serde_json::from_slice(&body).unwrap();
        let sid = head.lines().find_map(|l| l.strip_prefix("mcp-session-id:")).map(str::trim).unwrap_or("-").to_owned();
        seen.push(format!("{} {sid}", body["method"].as_str().unwrap_or("")));
        let (status, kind, text, new_sid) = reply(&body, &head);
        let new_sid = new_sid.map(|s| format!("Mcp-Session-Id: {s}\r\n")).unwrap_or_default();
        write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\n{new_sid}Content-Length: {}\r\nConnection: close\r\n\r\n{text}", text.len()).unwrap();
      }
      seen
    });
    (url, handle)
  }

  #[test]
  fn a_forgotten_session_is_started_again_and_progress_arrives_from_the_stream() {
    let restarted = Arc::new(AtomicBool::new(false));
    let r = restarted.clone();
    let (url, server) = serve(6, move |body, head| {
      let id = body["id"].clone();
      match body["method"].as_str().unwrap_or("") {
        "initialize" => {
          let result = json!({ "jsonrpc": "2.0", "id": id, "result": { "protocolVersion": "2025-03-26", "capabilities": { "tools": {} } } });
          (200, "application/json", result.to_string(), Some(if r.load(Ordering::SeqCst) { "s2" } else { "s1" }))
        }
        "notifications/initialized" => (202, "application/json", String::new(), None),
        "tools/call" if head.contains("mcp-session-id: s1") => {
          r.store(true, Ordering::SeqCst);
          (404, "text/plain", "gone".into(), None)
        }
        _ => {
          assert!(head.contains("mcp-protocol-version: 2025-03-26"), "{head}");
          let token = body.pointer("/params/_meta/progressToken").cloned().unwrap();
          let progress = json!({ "jsonrpc": "2.0", "method": "notifications/progress", "params": { "progressToken": token, "progress": 1, "message": "half way" } });
          let done = json!({ "jsonrpc": "2.0", "id": id, "result": { "content": [{ "type": "text", "text": "ok" }] } });
          (200, "text/event-stream", format!("event: message\ndata: {progress}\n\nevent: message\ndata: {done}\n\n"), None)
        }
      }
    });
    let http = Http::new(&url, &[("Authorization".into(), "Bearer t".into())], Default::default());
    let init = http.request("initialize", json!({ "protocolVersion": "2025-06-18" }), None).unwrap();
    http.set_protocol(init["protocolVersion"].as_str().unwrap());
    http.notify("notifications/initialized", json!({}));
    let heard = Arc::new(parking_lot::Mutex::new(vec![]));
    let h = heard.clone();
    let progress: Progress = Arc::new(move |t| h.lock().push(t));
    let result = http.request("tools/call", json!({ "name": "x", "arguments": {} }), Some(progress)).unwrap();
    assert_eq!(result["content"][0]["text"], "ok");
    assert_eq!(*heard.lock(), vec!["half way".to_owned()]);
    let seen = server.join().unwrap();
    assert_eq!(seen, ["initialize -", "notifications/initialized s1", "tools/call s1", "initialize -", "notifications/initialized s2", "tools/call s2"]);
  }

  #[test]
  fn an_error_answer_and_a_refused_key_read_as_messages() {
    let (url, server) = serve(2, |body, _| match body["method"].as_str() {
      Some("tools/list") => (200, "application/json", json!({ "jsonrpc": "2.0", "id": body["id"], "error": { "code": -32601, "message": "no tools here" } }).to_string(), None),
      _ => (401, "application/json", json!({ "error": { "message": "bad key" } }).to_string(), None),
    });
    let http = Http::new(&url, &[], Default::default());
    assert_eq!(http.request("tools/list", json!({}), None).unwrap_err(), "no tools here");
    let e = http.request("initialize", json!({}), None).unwrap_err();
    assert!(e.contains("401") && e.contains("refused the credentials") && e.contains("bad key"), "{e}");
    server.join().unwrap();
  }
}
