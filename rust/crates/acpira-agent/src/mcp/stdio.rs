//! The stdio transport: the server is a child process speaking newline-delimited JSON-RPC, the same framing ACP uses,
//! so the connection is `acpira-rpc`'s. The child stays in the agent's process group (the engine's group kill, or its
//! Job object on Windows, takes it down with the agent) and is killed when the session's hub closes. Its stderr goes to
//! the agent's log line by line, prefixed with the server's name

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio as Pipe;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use serde_json::Value;
use tokio::io::AsyncBufReadExt;

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Connection, Inbound, RpcError};

use super::Progress;

/// Calls waiting for progress, by token
type Waiting = Arc<parking_lot::Mutex<HashMap<String, Progress>>>;

pub struct Stdio {
  conn: Connection,
  child: parking_lot::Mutex<Option<tokio::process::Child>>,
  progress: Waiting,
  next_token: AtomicU64,
}

/// The server's requests and notifications
struct Peer {
  name: String,
  cwd: PathBuf,
  progress: Waiting,
  stale: Arc<AtomicBool>,
}

impl Inbound for Peer {
  fn notification(&self, method: &str, params: Value) {
    match method {
      "notifications/progress" => {
        let token = params.get("progressToken").map(token_key).unwrap_or_default();
        let call = self.progress.lock().get(&token).cloned();
        if let Some(call) = call {
          let text = super::progress_text(&params);
          if !text.is_empty() {
            call(text);
          }
        }
      }
      "notifications/tools/list_changed" => self.stale.store(true, Ordering::Release),
      "notifications/message" => {
        let data = params.get("data").map(|d| d.as_str().map(str::to_owned).unwrap_or_else(|| d.to_string())).unwrap_or_default();
        eprintln!("[mcp {}] {}", self.name, crate::budget::cut(&data, 500));
      }
      _ => {}
    }
  }

  fn request(&self, method: String, _: Value, _: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let answer = super::answer(&method, &self.cwd);
    Box::pin(async move { answer })
  }
}

/// A progress token as a map key: servers echo it back as sent, string or number
fn token_key(v: &Value) -> String {
  v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())
}

impl Stdio {
  pub fn spawn(name: &str, command: &str, args: &[String], env: &[(String, String)], cwd: &Path, stale: Arc<AtomicBool>) -> Result<Stdio, String> {
    let mut cmd = tokio::process::Command::new(command);
    cmd.args(args).current_dir(cwd).stdin(Pipe::piped()).stdout(Pipe::piped()).stderr(Pipe::piped()).kill_on_drop(true);
    for (k, v) in env {
      cmd.env(k, v);
    }
    #[cfg(windows)]
    {
      // No console window flashes up for a console program started from the agent
      const CREATE_NO_WINDOW: u32 = 0x0800_0000;
      cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start {command}: {e}"))?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
      return Err("the process has no stdio".into());
    };
    if let Some(stderr) = child.stderr.take() {
      let name = name.to_owned();
      tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
          eprintln!("[mcp {name}] {line}");
        }
      });
    }
    let progress: Waiting = Default::default();
    let peer: Arc<dyn Inbound> = Arc::new(Peer { name: name.to_owned(), cwd: cwd.to_owned(), progress: progress.clone(), stale });
    let label = name.to_owned();
    let conn = Connection::start(stdout, stdin, Arc::new(move || peer.clone()), Arc::new(move |line: &str| eprintln!("[mcp {label}] {line}")));
    Ok(Stdio { conn, child: parking_lot::Mutex::new(Some(child)), progress, next_token: AtomicU64::new(0) })
  }

  pub async fn request(&self, method: &str, mut params: Value, progress: Option<Progress>) -> Result<Value, String> {
    // A progress token only when someone listens: `acpira mcp` streams an `ask_agent` round's steps this way
    let token = progress.map(|p| {
      let token = format!("acpira-{}", self.next_token.fetch_add(1, Ordering::Relaxed) + 1);
      self.progress.lock().insert(token.clone(), p);
      params["_meta"]["progressToken"] = Value::String(token.clone());
      token
    });
    let _forget = Forget { waiting: &self.progress, token };
    self.conn.request(method, params).await.map_err(|e| if e.code == RpcError::connection_closed().code { "the server process exited".to_owned() } else { e.message })
  }

  pub fn notify(&self, method: &str, params: Value) {
    self.conn.notify(method, params);
  }

  pub fn close(&self) {
    self.conn.close();
    if let Some(mut child) = self.child.lock().take() {
      let _ = child.start_kill();
    }
  }
}

/// Drops a call's progress listener however the call ends (an answer, an error, the turn cancelled)
struct Forget<'a> {
  waiting: &'a Waiting,
  token: Option<String>,
}

impl Drop for Forget<'_> {
  fn drop(&mut self) {
    if let Some(t) = &self.token {
      self.waiting.lock().remove(t);
    }
  }
}
