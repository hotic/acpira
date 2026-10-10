//! A scripted OpenAI-compatible model server for tests (feature `mock`): each request takes the next scripted reply,
//! and every request is recorded, so a test can check what the model was sent (prefix stability included). Plain
//! blocking HTTP/1.1 on a loopback port, one thread per connection, `Connection: close` responses

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

pub enum Reply {
  /// These chunks as SSE events, then `[DONE]`
  Sse(Vec<Value>),
  /// These chunks, then the connection closes without a finish or `[DONE]`
  Cut(Vec<Value>),
  /// The status line and headers, then nothing until the client hangs up or the server is dropped
  Hang,
  /// A plain answer with this status and body
  Status(u16, String),
}

#[derive(Debug, Clone)]
pub struct Recorded {
  pub path: String,
  pub headers: Vec<(String, String)>,
  pub body: Value,
}

impl Recorded {
  pub fn header(&self, name: &str) -> Option<&str> {
    self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
  }
}

struct State {
  script: parking_lot::Mutex<VecDeque<Reply>>,
  requests: parking_lot::Mutex<Vec<Recorded>>,
  stop: AtomicBool,
  /// Connections still waiting in `Hang`
  hanging: std::sync::atomic::AtomicUsize,
}

pub struct MockModel {
  port: u16,
  state: Arc<State>,
}

impl MockModel {
  pub fn start() -> MockModel {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback port");
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(State {
      script: Default::default(),
      requests: Default::default(),
      stop: AtomicBool::new(false),
      hanging: Default::default(),
    });
    let st = state.clone();
    std::thread::spawn(move || {
      for conn in listener.incoming() {
        if st.stop.load(Ordering::Acquire) {
          break;
        }
        let Ok(conn) = conn else { continue };
        let st = st.clone();
        std::thread::spawn(move || serve(conn, &st));
      }
    });
    MockModel { port, state }
  }

  /// The API root to put in providers.json
  pub fn base_url(&self) -> String {
    format!("http://127.0.0.1:{}/v1", self.port)
  }

  pub fn push(&self, reply: Reply) -> &Self {
    self.state.script.lock().push_back(reply);
    self
  }

  pub fn requests(&self) -> Vec<Recorded> {
    self.state.requests.lock().clone()
  }

  /// Connections parked in `Hang` right now (0 once the client dropped them)
  pub fn hanging(&self) -> usize {
    self.state.hanging.load(Ordering::Acquire)
  }
}

impl Drop for MockModel {
  fn drop(&mut self) {
    self.state.stop.store(true, Ordering::Release);
    // Wake the accept loop so it sees the flag
    let _ = TcpStream::connect(("127.0.0.1", self.port));
  }
}

fn serve(conn: TcpStream, st: &State) {
  let mut reader = BufReader::new(conn.try_clone().unwrap());
  let mut request_line = String::new();
  if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
    return;
  }
  let path = request_line.split_whitespace().nth(1).unwrap_or("").to_owned();
  let mut headers = vec![];
  let mut length = 0usize;
  loop {
    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
      return;
    }
    let line = line.trim_end();
    if line.is_empty() {
      break;
    }
    if let Some((k, v)) = line.split_once(':') {
      let (k, v) = (k.trim().to_owned(), v.trim().to_owned());
      if k.eq_ignore_ascii_case("content-length") {
        length = v.parse().unwrap_or(0);
      }
      headers.push((k, v));
    }
  }
  let mut body = vec![0u8; length];
  if reader.read_exact(&mut body).is_err() {
    return;
  }
  let body: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
  st.requests.lock().push(Recorded { path, headers, body });
  let reply = st.script.lock().pop_front().unwrap_or_else(|| Reply::Status(500, r#"{"error":{"message":"mock: no scripted reply"}}"#.into()));
  let mut out = conn;
  let sse_head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n";
  match reply {
    Reply::Status(code, text) => {
      let _ = write!(out, "HTTP/1.1 {code} Mock\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}", text.len());
    }
    Reply::Sse(chunks) => {
      let _ = out.write_all(sse_head.as_bytes());
      for c in chunks {
        let _ = write!(out, "data: {c}\n\n");
        let _ = out.flush();
      }
      let _ = out.write_all(b"data: [DONE]\n\n");
    }
    Reply::Cut(chunks) => {
      let _ = out.write_all(sse_head.as_bytes());
      for c in chunks {
        let _ = write!(out, "data: {c}\n\n");
      }
    }
    Reply::Hang => {
      let _ = out.write_all(sse_head.as_bytes());
      let _ = out.flush();
      st.hanging.fetch_add(1, Ordering::AcqRel);
      let _ = out.set_read_timeout(Some(Duration::from_millis(20)));
      let mut probe = [0u8; 1];
      while !st.stop.load(Ordering::Acquire) {
        match out.peek(&mut probe) {
          // The client hung up
          Ok(0) => break,
          Ok(_) => std::thread::sleep(Duration::from_millis(20)),
          Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
          Err(_) => break,
        }
      }
      st.hanging.fetch_sub(1, Ordering::AcqRel);
    }
  }
  let _ = out.flush();
  let _ = out.shutdown(std::net::Shutdown::Both);
}

/// Chunk builders in the OpenAI streaming shape
pub fn delta(d: Value) -> Value {
  json!({ "id": "mock", "object": "chat.completion.chunk", "choices": [{ "index": 0, "delta": d, "finish_reason": null }] })
}

pub fn finish(reason: &str) -> Value {
  json!({ "id": "mock", "object": "chat.completion.chunk", "choices": [{ "index": 0, "delta": {}, "finish_reason": reason }] })
}

/// The trailing usage chunk (`include_usage`), with `cached` prompt tokens read from cache
pub fn usage(input: u64, output: u64, cached: u64) -> Value {
  json!({ "id": "mock", "object": "chat.completion.chunk", "choices": [],
    "usage": { "prompt_tokens": input, "completion_tokens": output, "total_tokens": input + output, "prompt_tokens_details": { "cached_tokens": cached } } })
}

/// A plain answer in two deltas
pub fn text(answer: &str) -> Reply {
  let mid = answer.char_indices().nth(answer.chars().count() / 2).map(|(i, _)| i).unwrap_or(0);
  Reply::Sse(vec![
    delta(json!({ "role": "assistant", "content": &answer[..mid] })),
    delta(json!({ "content": &answer[mid..] })),
    finish("stop"),
    usage(100, 10, 0),
  ])
}

/// Tool calls (id, name, arguments), the arguments streamed in two pieces
pub fn tools(calls: &[(&str, &str, Value)]) -> Reply {
  let mut chunks = vec![];
  for (i, (id, name, args)) in calls.iter().enumerate() {
    let args = args.to_string();
    let mut mid = args.len() / 2;
    while !args.is_char_boundary(mid) {
      mid -= 1;
    }
    chunks.push(delta(json!({ "tool_calls": [{ "index": i, "id": id, "type": "function", "function": { "name": name, "arguments": &args[..mid] } }] })));
    chunks.push(delta(json!({ "tool_calls": [{ "index": i, "function": { "arguments": &args[mid..] } }] })));
  }
  chunks.push(finish("tool_calls"));
  chunks.push(usage(100, 10, 0));
  Reply::Sse(chunks)
}
