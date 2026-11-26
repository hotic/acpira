//! One agent subprocess = one long-lived ACP connection. Handlers are
//! rebindable so a warm (initialize-only) process can be handed to a session without a second spawn.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;

use crate::acp::launch;
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::legacy_models::LegacyModels;
use crate::acp::transport::rpc::{BoxFuture, Connection, Inbound, RpcError};
use crate::i18n::tp;

pub const CLIENT_NAME: &str = "acpira";
pub const PROTOCOL_VERSION: i64 = 1;
pub(crate) const KILL_GRACE: Duration = Duration::from_secs(5);
// 120 s: loaded Windows machines and cold Node CLIs under antivirus scanning can take well past 30 s to answer initialize
pub const INIT_TIMEOUT: Duration = Duration::from_secs(120);

pub fn client_version() -> &'static str {
  acpira_shared::version::VERSION
}

/// Pure launch specification for spawning an ACP subprocess without vendor or registry coupling.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
  pub command: String,
  pub binary: String,
  pub args: Vec<String>,
  pub cwd: String,
  pub env_remove: Vec<String>,
  pub env_set: BTreeMap<String, String>,
  pub initialize_params: Value,
  pub init_timeout: Option<Duration>,
  /// Optional Droid session store derived from the final child environment.
  pub droid_sessions: Option<PathBuf>,
  /// Optional CLI-owned directories derived from the final child environment.
  pub own_dirs: Vec<PathBuf>,
}

/// What the client side accepts from the agent. Standard ACP methods are routed directly;
/// vendor and host extensions flow through `on_custom_notification` (synchronous on the reader task)
/// and `on_custom_request`.
pub trait ClientHandlers: Send + Sync + 'static {
  fn on_update(&self, params: Value);
  fn on_permission(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>>;
  fn on_elicitation(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>>;
  /// Non-standard notification invoked synchronously on the reader loop to preserve ordering with `session/update`.
  fn on_custom_notification(&self, _method: &str, _params: Value) {}
  /// Non-standard server-to-client RPC request (vendor protocols, host tools, plugin hooks).
  fn on_custom_request(&self, method: String, _params: Value, _cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    Box::pin(async move { Err(RpcError::method_not_found(&method)) })
  }
  fn on_stderr(&self, _line: &str) {}
  fn on_exit(&self, _code: Option<i32>, _signal: Option<String>) {}
}

/// The executable would not spawn at all (ENOENT, EACCES): distinct from a process that started and then failed the handshake
#[derive(Debug)]
pub struct AgentSpawnError {
  err: std::io::Error,
  /// The folder the process was to start in, when that is what cannot be entered: the OS reports it with the same
  /// error as a missing or non-executable binary
  folder: Option<String>,
}

impl AgentSpawnError {
  fn new(err: std::io::Error, cwd: &str) -> Self {
    // Entering a folder needs what reading its `.` needs: that it is a folder, and search permission on it
    let closed = std::fs::metadata(std::path::Path::new(cwd).join(".")).is_err();
    AgentSpawnError { err, folder: closed.then(|| cwd.to_owned()) }
  }
}

impl std::fmt::Display for AgentSpawnError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let code = match self.err.kind() {
      std::io::ErrorKind::NotFound => "ENOENT",
      std::io::ErrorKind::PermissionDenied => "EACCES",
      _ => "",
    };
    if code.is_empty() { write!(f, "spawn failed: {}", self.err) } else { write!(f, "spawn {code}") }?;
    match &self.folder {
      Some(folder) => write!(f, ": the folder {folder} cannot be opened"),
      None => Ok(()),
    }
  }
}

impl std::error::Error for AgentSpawnError {}

type HandlerBox = Arc<parking_lot::RwLock<Arc<dyn ClientHandlers>>>;

struct Router {
  box_: HandlerBox,
}

impl Inbound for Router {
  fn notification(&self, method: &str, params: Value) {
    let h = self.box_.read().clone();
    if method == "session/update" {
      h.on_update(params);
    } else {
      h.on_custom_notification(method, params);
    }
  }

  fn request(&self, method: String, params: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let h = self.box_.read().clone();
    match method.as_str() {
      "session/request_permission" => h.on_permission(params, cancel),
      "elicitation/create" => h.on_elicitation(params, cancel),
      _ => h.on_custom_request(method, params, cancel),
    }
  }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Life {
  Running,
  Exited,
}

pub struct AgentProcess {
  pub conn: Connection,
  /// The initialize response as the agent sent it
  pub init: Value,
  pub pid: Option<u32>,
  /// Droid's actual session store, after the launch env and account overrides have been applied.
  pub(crate) droid_sessions: Option<PathBuf>,
  /// The folders this child was told to keep the CLI's own files in (`OwnFiles::env_dirs`).
  pub(crate) own_dirs: Vec<PathBuf>,
  signals: Option<Signals>,
  box_: HandlerBox,
  life: watch::Receiver<Life>,
  killed: parking_lot::Mutex<bool>,
  /// The unstable `models` field shown as a model select (`legacy_models.rs`)
  legacy: LegacyModels,
}

/// A session start's requested sessionId, kept for `LegacyModels::adopt` without cloning every request's params
fn start_sid(method: &str, params: &Value) -> Option<String> {
  LegacyModels::starts(method).then(|| params.get("sessionId").and_then(Value::as_str).map(str::to_owned)).flatten()
}

impl AgentProcess {
  pub fn alive(&self) -> bool {
    *self.life.borrow() == Life::Running && !*self.killed.lock()
  }

  pub fn bind(&self, h: Arc<dyn ClientHandlers>) {
    *self.box_.write() = h;
  }

  pub async fn request(&self, method: &str, params: Value) -> Result<Value, RpcError> {
    if let Some((sid, model)) = self.legacy.intercept(method, &params) {
      self.conn.request("session/set_model", json!({ "sessionId": sid, "modelId": model })).await?;
      return Ok(self.legacy.picked(&sid, &model));
    }
    let requested = start_sid(method, &params);
    let mut r = self.conn.request(method, params).await?;
    self.legacy.adopt(method, requested.as_deref(), &mut r);
    Ok(r)
  }

  pub async fn request_ordered(&self, method: &str, params: Value) -> Result<(Value, crate::acp::transport::rpc::Handoff), RpcError> {
    let requested = start_sid(method, &params);
    let (mut r, handoff) = self.conn.request_ordered(method, params).await?;
    self.legacy.adopt(method, requested.as_deref(), &mut r);
    Ok((r, handoff))
  }

  pub fn notify(&self, method: &str, params: Value) {
    self.conn.notify(method, params);
  }

  /// agentCapabilities as advertised (Null when absent)
  pub fn caps(&self) -> &Value {
    self.init.get("agentCapabilities").unwrap_or(&Value::Null)
  }

  /// Spawn an agent subprocess from a fully resolved `SpawnSpec` and run the ACP `initialize` handshake.
  pub async fn spawn_spec(spec: SpawnSpec, h: Arc<dyn ClientHandlers>) -> Result<Arc<AgentProcess>> {
    let box_: HandlerBox = Arc::new(parking_lot::RwLock::new(h));
    let mut cmd = launch::command(&spec.binary, &spec.args);
    cmd.current_dir(&spec.cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(false);
    for key in &spec.env_remove {
      cmd.env_remove(key);
    }
    for (k, v) in &spec.env_set {
      cmd.env(k, v);
    }
    #[cfg(windows)]
    let (mut child, job) = crate::platform::windows_process::spawn(&mut cmd).await.map_err(|e| anyhow::Error::new(AgentSpawnError::new(e, &spec.cwd)))?;
    #[cfg(not(windows))]
    let mut child = cmd.spawn().map_err(|e| anyhow::Error::new(AgentSpawnError::new(e, &spec.cwd)))?;
    let pid = child.id();
    let signals = pid.map(|_pid| Signals {
      #[cfg(unix)]
      pid: _pid,
      #[cfg(windows)]
      job,
      reaped: Arc::new(parking_lot::Mutex::new(false)),
    });
    if let (Some(pid), Some(sig)) = (pid, &signals) {
      LIVE_AGENTS.lock().insert(pid, sig.clone());
    }
    let stdin = child.stdin.take().ok_or_else(|| anyhow!("agent stdin is not piped"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("agent stdout is not piped"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("agent stderr is not piped"))?;

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
      let status = child.wait().await;
      if let Some(sig) = &reaper {
        #[cfg(windows)]
        sig.job.terminate();
        *sig.reaped.lock() = true;
        #[cfg(unix)]
        LIVE_AGENTS.lock().remove(&sig.pid);
      }
      #[cfg(not(unix))]
      if let Some(pid) = pid {
        LIVE_AGENTS.lock().remove(&pid);
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

    let timeout = spec.init_timeout.unwrap_or(INIT_TIMEOUT);
    let mut exit_rx = exit_info_rx;
    let outcome = tokio::select! {
      r = conn.request("initialize", spec.initialize_params) => r.map_err(anyhow::Error::new),
      seen = exit_rx.wait_for(|v| v.is_some()) => {
        let (code, signal) = seen.ok().and_then(|v| v.clone()).unwrap_or((None, None));
        let code = code.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
        Err(anyhow!(tp("host.spawnExited", &[("command", &spec.command), ("code", &code), ("signal", signal.as_deref().unwrap_or("-"))])))
      }
      _ = tokio::time::sleep(timeout) => Err(anyhow!(tp("host.initTimeout", &[("command", &spec.command), ("seconds", &timeout.as_secs().to_string())]))),
    };
    match outcome {
      Ok(init) => Ok(Arc::new(AgentProcess {
        conn,
        init,
        pid,
        droid_sessions: spec.droid_sessions,
        own_dirs: spec.own_dirs,
        signals,
        box_,
        life: life_rx,
        killed: parking_lot::Mutex::new(false),
        legacy: LegacyModels::default(),
      })),
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

/// Every agent process this engine started and has not reaped yet, by pid. Kept apart from the sessions, whose locks a
/// wedged engine may never release (`kill_all_hard`)
static LIVE_AGENTS: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashMap<u32, Signals>>> = std::sync::LazyLock::new(Default::default);

/// SIGKILL to every agent this engine started (its pid, or its Job on Windows), for an engine about to exit
/// without its normal teardown: its leases come free with it, and no agent may still write a native session that the
/// next engine resumes
pub fn kill_all_hard() {
  let all: Vec<Signals> = LIVE_AGENTS.lock().values().cloned().collect();
  for sig in all {
    sig.send(true);
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

/// Where an agent's signals go. Nothing is sent once the child was reaped: its pid may belong to someone else by then
#[derive(Clone)]
struct Signals {
  #[cfg(unix)]
  pid: u32,
  #[cfg(windows)]
  job: Arc<crate::platform::windows_process::Job>,
  reaped: Arc<parking_lot::Mutex<bool>>,
}

impl Signals {
  fn send(&self, force: bool) {
    let reaped = self.reaped.lock();
    if !*reaped {
      #[cfg(unix)]
      send_signal(self.pid, force);
      #[cfg(windows)]
      {
        let _ = force;
        self.job.terminate();
      }
    }
  }
}

#[cfg(unix)]
fn send_signal(pid: u32, force: bool) {
  // SAFETY: plain kill(2) on our own unreaped child's pid
  unsafe {
    libc::kill(pid as libc::pid_t, if force { libc::SIGKILL } else { libc::SIGTERM });
  }
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

/// Build the ACP initialize request from capability flags without depending on `AgentDef`.
pub fn initialize_request_caps(terminal_auth: bool, subagents: bool) -> Value {
  let mut caps = json!({
    "fs": { "readTextFile": false, "writeTextFile": false },
    "terminal": false,
  });
  if terminal_auth {
    caps["auth"] = json!({ "terminal": true });
  }
  caps["elicitation"] = json!({ "form": {} });
  caps["session"] = json!({ "configOptions": { "boolean": {} }, "compaction": {}, "notices": {} });
  if subagents {
    caps["subagents"] = json!({});
  }
  let mut air = vec![];
  if subagents {
    air.push("nativeSubagentSessions");
  }
  air.extend(["sessionFailure", "asyncTasks", "recommendedValue"]);
  caps["_meta"] = json!({
    "terminal_output_delta": true,
    "jetbrains": { "air": { "version": 1, "capabilities": air } },
    "acpira/sidekick": true,
  });
  json!({ "protocolVersion": PROTOCOL_VERSION, "clientInfo": { "name": CLIENT_NAME, "version": client_version() }, "clientCapabilities": caps })
}
