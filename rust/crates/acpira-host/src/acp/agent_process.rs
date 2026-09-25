//! One agent subprocess = one long-lived ACP connection. Handlers are
//! rebindable so a warm (initialize-only) process can be handed to a session without a second spawn

use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

use super::agent_registry::AgentDef;
use super::cancel::Cancel;
use super::grok::{GROK_ASK_QUESTION, GROK_EXIT_PLAN};
use super::launch::{Os, ProcessEnv, spawn_spec};
use super::rpc::{BoxFuture, Connection, Inbound, RpcError};
use crate::i18n::tp;

pub const CLIENT_NAME: &str = "acpira";
pub const PROTOCOL_VERSION: i64 = 1;
const KILL_GRACE: Duration = Duration::from_secs(5);
pub const INIT_TIMEOUT: Duration = Duration::from_secs(30);

pub fn client_version() -> &'static str {
  env!("CARGO_PKG_VERSION")
}

/// What the client side accepts from the agent. Every handler here is advertised; the session and the idle stubs both
/// implement all of them
pub trait ClientHandlers: Send + Sync + 'static {
  fn on_update(&self, params: Value);
  fn on_permission(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>>;
  fn on_elicitation(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>>;
  fn on_grok_question(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>>;
  fn on_stderr(&self, _line: &str) {}
  fn on_exit(&self, _code: Option<i32>, _signal: Option<String>) {}
}

/// The executable would not spawn at all (ENOENT, EACCES): distinct from a process that started and then failed the handshake
#[derive(Debug)]
pub struct AgentSpawnError(pub std::io::Error);

impl std::fmt::Display for AgentSpawnError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let code = match self.0.kind() {
      std::io::ErrorKind::NotFound => "ENOENT",
      std::io::ErrorKind::PermissionDenied => "EACCES",
      _ => "",
    };
    if code.is_empty() { write!(f, "spawn failed: {}", self.0) } else { write!(f, "spawn {code}") }
  }
}

impl std::error::Error for AgentSpawnError {}

type HandlerBox = Arc<parking_lot::RwLock<Arc<dyn ClientHandlers>>>;

struct Router {
  box_: HandlerBox,
}

impl Inbound for Router {
  fn notification(&self, method: &str, params: Value) {
    if method == "session/update" {
      self.box_.read().clone().on_update(params);
    }
  }

  fn request(&self, method: String, params: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let h = self.box_.read().clone();
    match method.as_str() {
      "session/request_permission" => h.on_permission(params, cancel),
      "elicitation/create" => h.on_elicitation(params, cancel),
      GROK_ASK_QUESTION => match super::grok::parse_question(&params) {
        Ok(req) => h.on_grok_question(req, cancel),
        Err(e) => Box::pin(async move { Err(e) }),
      },
      GROK_EXIT_PLAN => match super::grok::parse_exit_plan(&params) {
        Ok(req) => Box::pin(super::grok::approve_plan(req, cancel, h)),
        Err(e) => Box::pin(async move { Err(e) }),
      },
      _ => Box::pin(async move { Err(RpcError::method_not_found(&method)) }),
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Life {
  Running,
  Exited,
}

pub struct AgentProcess {
  pub def: AgentDef,
  pub conn: Connection,
  /// The initialize response as the agent sent it
  pub init: Value,
  pub pid: Option<u32>,
  box_: HandlerBox,
  life: watch::Receiver<Life>,
  killed: parking_lot::Mutex<bool>,
}

impl AgentProcess {
  pub fn alive(&self) -> bool {
    *self.life.borrow() == Life::Running && !*self.killed.lock()
  }

  pub fn bind(&self, h: Arc<dyn ClientHandlers>) {
    *self.box_.write() = h;
  }

  pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
    self.conn.request(method, params).await
  }

  pub async fn request_ordered(&self, method: &str, params: Value) -> Result<(Value, super::rpc::Handoff), RpcError> {
    self.conn.request_ordered(method, params).await
  }

  pub fn notify(&self, method: &str, params: Value) {
    self.conn.notify(method, params);
  }

  /// agentCapabilities as advertised (Null when absent)
  pub fn caps(&self) -> &Value {
    self.init.get("agentCapabilities").unwrap_or(&Value::Null)
  }

  /// `extra_env`: variables the account layer injects per identity, on top of the definition's env
  pub async fn spawn(
    def: &AgentDef,
    binary: &str,
    cwd: &str,
    h: Arc<dyn ClientHandlers>,
    extra_env: Option<&acpira_shared::transcript::StrMap>,
    init_timeout: Option<Duration>,
  ) -> Result<Arc<AgentProcess>> {
    let box_: HandlerBox = Arc::new(parking_lot::RwLock::new(h));
    let spec = spawn_spec(binary, &def.args, Os::current(), &ProcessEnv);
    let mut cmd = Command::new(&spec.command);
    #[cfg(windows)]
    if spec.verbatim {
      for a in &spec.args {
        cmd.raw_arg(a);
      }
    } else {
      cmd.args(&spec.args);
    }
    #[cfg(not(windows))]
    cmd.args(&spec.args);
    cmd.current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(false);
    for (k, v) in def.env.iter().flatten() {
      cmd.env(k, v);
    }
    for (k, v) in extra_env.into_iter().flatten() {
      cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| anyhow::Error::new(AgentSpawnError(e)))?;
    let pid = child.id();
    let stdin = child.stdin.take().expect("piped");
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");

    let err_box = box_.clone();
    tokio::spawn(async move {
      let mut lines = BufReader::new(stderr).lines();
      while let Ok(Some(line)) = lines.next_line().await {
        err_box.read().clone().on_stderr(&line);
      }
    });

    let router_box = box_.clone();
    let inbound: Arc<dyn Fn() -> Arc<dyn Inbound> + Send + Sync> = {
      let router: Arc<dyn Inbound> = Arc::new(Router { box_: router_box });
      Arc::new(move || router.clone())
    };
    let log_box = box_.clone();
    let conn = Connection::start(stdout, stdin, inbound, Arc::new(move |l: &str| log_box.read().clone().on_stderr(l)));

    let (life_tx, life_rx) = watch::channel(Life::Running);
    let exit_box = box_.clone();
    let exit_conn = conn.clone();
    let (exit_info_tx, exit_info_rx) = watch::channel::<Option<(Option<i32>, Option<String>)>>(None);
    tokio::spawn(async move {
      let status = child.wait().await;
      let (code, signal) = match status {
        Ok(s) => (s.code(), signal_name(&s)),
        Err(_) => (None, None),
      };
      let _ = exit_info_tx.send(Some((code, signal.clone())));
      let _ = life_tx.send(Life::Exited);
      exit_conn.close();
      exit_box.read().clone().on_exit(code, signal);
    });

    let init_req = initialize_request(def);
    let timeout = init_timeout.unwrap_or(INIT_TIMEOUT);
    let mut exit_rx = exit_info_rx;
    let outcome = tokio::select! {
      r = conn.request("initialize", init_req) => r.map_err(anyhow::Error::new),
      seen = exit_rx.wait_for(|v| v.is_some()) => {
        let (code, signal) = seen.ok().and_then(|v| v.clone()).unwrap_or((None, None));
        let code = code.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
        Err(anyhow!(tp("host.spawnExited", &[("command", &def.command), ("code", &code), ("signal", signal.as_deref().unwrap_or("-"))])))
      }
      _ = tokio::time::sleep(timeout) => Err(anyhow!(tp("host.initTimeout", &[("command", &def.command), ("seconds", &timeout.as_secs().to_string())]))),
    };
    match outcome {
      Ok(init) => {
        Ok(Arc::new(AgentProcess { def: def.clone(), conn, init, pid, box_, life: life_rx, killed: parking_lot::Mutex::new(false) }))
      }
      Err(e) => {
        // A CLI that answered initialize with an error is still running: never leave it behind as an orphan
        conn.close();
        terminate(pid, life_rx);
        Err(e)
      }
    }
  }

  fn kill_now(&self) {
    *self.killed.lock() = true;
    self.conn.close();
    terminate(self.pid, self.life.clone());
  }

  /// SIGKILL at once, for a host that is about to exit and cannot wait out the grace period
  pub fn kill_hard(&self) {
    *self.killed.lock() = true;
    self.conn.close();
    if let Some(pid) = self.pid
      && *self.life.borrow() != Life::Exited
    {
      send_signal(pid, true);
    }
  }

  /// SIGTERM, then SIGKILL after the grace period; resolves when the process has exited
  pub async fn kill(&self) {
    self.kill_now();
    let mut life = self.life.clone();
    let _ = life.wait_for(|l| *l == Life::Exited).await;
  }
}

fn terminate(pid: Option<u32>, life: watch::Receiver<Life>) {
  if *life.borrow() == Life::Exited {
    return;
  }
  let Some(pid) = pid else { return };
  send_signal(pid, false);
  let mut life = life;
  tokio::spawn(async move {
    if tokio::time::timeout(KILL_GRACE, life.wait_for(|l| *l == Life::Exited)).await.is_err() {
      send_signal(pid, true);
    }
  });
}

#[cfg(unix)]
fn send_signal(pid: u32, force: bool) {
  // SAFETY: plain kill(2) on our own child's pid
  unsafe {
    libc::kill(pid as libc::pid_t, if force { libc::SIGKILL } else { libc::SIGTERM });
  }
}

#[cfg(not(unix))]
fn send_signal(pid: u32, _force: bool) {
  let _ =
    std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

#[cfg(unix)]
fn signal_name(s: &std::process::ExitStatus) -> Option<String> {
  use std::os::unix::process::ExitStatusExt;
  let sig = s.signal()?;
  Some(
    match sig {
      libc::SIGTERM => "SIGTERM",
      libc::SIGKILL => "SIGKILL",
      libc::SIGINT => "SIGINT",
      libc::SIGHUP => "SIGHUP",
      libc::SIGSEGV => "SIGSEGV",
      libc::SIGABRT => "SIGABRT",
      libc::SIGPIPE => "SIGPIPE",
      _ => return Some(format!("SIG{sig}")),
    }
    .to_owned(),
  )
}

#[cfg(not(unix))]
fn signal_name(_: &std::process::ExitStatus) -> Option<String> {
  None
}

/// The initialize request, capability by capability as the TS host advertises it
pub fn initialize_request(def: &AgentDef) -> Value {
  let mut caps = json!({
    "fs": { "readTextFile": false, "writeTextFile": false },
    "terminal": false,
  });
  // The host can reproduce the agent's invocation in an interactive terminal; an agent whose ACP process ignores the local login opts out
  if def.terminal_auth {
    caps["auth"] = json!({ "terminal": true });
  }
  caps["elicitation"] = json!({ "form": {} });
  // ACP boolean session config options (RFD boolean-config-option). `compaction` opts into structured
  // compaction_update: without it claude-agent-acp 0.81.0 reports compaction as a `think` tool call
  caps["session"] = json!({ "configOptions": { "boolean": {} }, "compaction": {} });
  if def.subagents {
    caps["subagents"] = json!({});
  }
  let mut air = vec![];
  if def.subagents {
    air.push("nativeSubagentSessions");
  }
  air.extend(["sessionFailure", "asyncTasks", "recommendedValue"]);
  caps["_meta"] = json!({
    "terminal_output_delta": true,
    "jetbrains": { "air": { "version": 1, "capabilities": air } },
  });
  json!({ "protocolVersion": PROTOCOL_VERSION, "clientInfo": { "name": CLIENT_NAME, "version": client_version() }, "clientCapabilities": caps })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn initialize_advertises_structured_compaction() {
    let req = initialize_request(&AgentDef::default());
    assert!(req["clientCapabilities"]["session"]["compaction"].is_object());
  }
}
