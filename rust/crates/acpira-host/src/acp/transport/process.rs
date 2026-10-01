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

use crate::acp::agents::registry::AgentDef;
use crate::acp::transport::cancel::Cancel;
use crate::acp::vendors::grok::{GROK_ASK_QUESTION, GROK_EXIT_PLAN};
use crate::acp::agents::launch::{Os, ProcessEnv, spawn_spec};
use crate::acp::transport::rpc::{BoxFuture, Connection, Inbound, RpcError};
use crate::i18n::tp;

pub const CLIENT_NAME: &str = "acpira";
pub const PROTOCOL_VERSION: i64 = 1;
pub(crate) const KILL_GRACE: Duration = Duration::from_secs(5);
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
      GROK_ASK_QUESTION => match crate::acp::vendors::grok::parse_question(&params) {
        Ok(req) => h.on_grok_question(req, cancel),
        Err(e) => Box::pin(async move { Err(e) }),
      },
      GROK_EXIT_PLAN => match crate::acp::vendors::grok::parse_exit_plan(&params) {
        Ok(req) => Box::pin(crate::acp::vendors::grok::approve_plan(req, cancel, h)),
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
  signals: Option<Signals>,
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

  pub async fn request_ordered(&self, method: &str, params: Value) -> Result<(Value, crate::acp::transport::rpc::Handoff), RpcError> {
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
    // A native release finds its helpers next to its own path, which a symlink on PATH would hide
    let real = def.release.and_then(|_| std::fs::canonicalize(binary).ok()).map(|p| p.to_string_lossy().into_owned());
    let binary = real.as_deref().unwrap_or(binary);
    let group = def.release.is_some() && cfg!(unix);
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
    #[cfg(unix)]
    if group {
      cmd.process_group(0);
    }
    // The directories the login shell adds reach the agent too (a node-script adapter needs `node` from them);
    // the definition's own env may still set PATH explicitly
    if let Some(path) = crate::acp::agents::login_path::merged() {
      cmd.env("PATH", path);
    }
    for (k, v) in def.env.iter().flatten() {
      cmd.env(k, v);
    }
    for (k, v) in extra_env.into_iter().flatten() {
      cmd.env(k, v);
    }
    let mut child = cmd.spawn().map_err(|e| anyhow::Error::new(AgentSpawnError(e)))?;
    let pid = child.id();
    let signals = pid.map(|pid| Signals { pid, group, reaped: Arc::new(parking_lot::Mutex::new(false)) });
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
    let reaper = signals.clone();
    tokio::spawn(async move {
      // A group leader is left unreaped until its group is swept: while it is a zombie its pid, and so the group id, cannot
      // be reused, and helpers it left behind (crashed or not) go with it
      if let Some(sig) = reaper.as_ref().filter(|s| s.group)
        && leader_exited(sig.pid).await
      {
        let mut reaped = sig.reaped.lock();
        sig.kill_group();
        *reaped = true;
      }
      let status = child.wait().await;
      if let Some(sig) = &reaper {
        *sig.reaped.lock() = true;
      }
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
        Ok(Arc::new(AgentProcess { def: def.clone(), conn, init, pid, signals, box_, life: life_rx, killed: parking_lot::Mutex::new(false) }))
      }
      Err(e) => {
        // A CLI that answered initialize with an error is still running: never leave it behind as an orphan
        conn.close();
        terminate(signals, life_rx);
        Err(e)
      }
    }
  }

  fn kill_now(&self) {
    *self.killed.lock() = true;
    self.conn.close();
    terminate(self.signals.clone(), self.life.clone());
  }

  /// SIGKILL at once, for a host that is about to exit and cannot wait out the grace period
  pub fn kill_hard(&self) {
    *self.killed.lock() = true;
    self.conn.close();
    if let Some(sig) = &self.signals {
      sig.send(true);
    }
  }

  /// SIGTERM, then SIGKILL after the grace period; resolves when the process has exited
  pub async fn kill(&self) {
    self.kill_now();
    let mut life = self.life.clone();
    let _ = life.wait_for(|l| *l == Life::Exited).await;
  }
}

fn terminate(signals: Option<Signals>, life: watch::Receiver<Life>) {
  if *life.borrow() == Life::Exited {
    return;
  }
  let Some(sig) = signals else { return };
  sig.send(false);
  let mut life = life;
  tokio::spawn(async move {
    if tokio::time::timeout(KILL_GRACE, life.wait_for(|l| *l == Life::Exited)).await.is_err() {
      sig.send(true);
    }
  });
}

/// Where an agent's signals go. `group`: the pid leads a process group this process created (`process_group(0)` at
/// spawn), so the whole group is signalled. Nothing is sent once the child was reaped: its pid may belong to someone
/// else by then
#[derive(Clone)]
struct Signals {
  pid: u32,
  group: bool,
  reaped: Arc<parking_lot::Mutex<bool>>,
}

impl Signals {
  fn send(&self, force: bool) {
    let reaped = self.reaped.lock();
    if !*reaped {
      send_signal(self.pid, self.group, force);
    }
  }

  /// SIGKILL to the group, called by the reaper with the leader still unreaped
  fn kill_group(&self) {
    send_signal(self.pid, true, true);
  }
}

#[cfg(unix)]
fn send_signal(pid: u32, group: bool, force: bool) {
  let target = if group { -(pid as libc::pid_t) } else { pid as libc::pid_t };
  // SAFETY: plain kill(2) on our own unreaped child's pid or the group it leads
  unsafe {
    libc::kill(target, if force { libc::SIGKILL } else { libc::SIGTERM });
  }
}

#[cfg(not(unix))]
fn send_signal(pid: u32, _group: bool, _force: bool) {
  let _ =
    std::process::Command::new("taskkill").args(["/PID", &pid.to_string(), "/T", "/F"]).stdout(Stdio::null()).stderr(Stdio::null()).spawn();
}

/// Resolves once the child has exited, without reaping it (`waitid` with `WNOWAIT`), so the zombie keeps its pid. False
/// when that could not be watched (no thread): the group is then left alone rather than swept while it may still run
#[cfg(unix)]
async fn leader_exited(pid: u32) -> bool {
  let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
  let spawned = std::thread::Builder::new().name(format!("acpira-wait-{pid}")).spawn(move || {
    let exited = loop {
      // SAFETY: waitid fills a zeroed siginfo_t for our own child; WNOWAIT leaves it waitable for tokio
      let r = unsafe {
        let mut info: libc::siginfo_t = std::mem::zeroed();
        libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOWAIT)
      };
      if r == 0 {
        break true;
      }
      if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
        break false;
      }
    };
    let _ = tx.send(exited);
  });
  spawned.is_ok() && rx.await.unwrap_or(false)
}

#[cfg(not(unix))]
async fn leader_exited(_pid: u32) -> bool {
  false
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
