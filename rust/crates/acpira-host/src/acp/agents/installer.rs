//! In-app agent installs. The registry's install line runs as a background child of the engine instead of in an IDE
//! terminal, so it always lands on the machine the engine runs on (the remote host of an SSH / Remote window) and the
//! settings page shows its progress, result and log.
//!
//! On top of the vendor line:
//! - the network route of `net_proxy` (the local 7890 proxy by default), for curl / npm / PowerShell alike
//! - POSIX `npm install -g` whose global prefix is not writable (a system Node on a shared server) goes to
//!   `~/.local` instead of failing with EACCES; the copyable line shown in settings says so too
//! - a missing Node.js is reported before anything runs
//! - Windows runs the PowerShell source with UTF-8 output, TLS 1.2, no progress bars and the proxy on .NET's default
//!   web proxy (Windows PowerShell 5.1 ignores HTTPS_PROXY)
//!
//! Cancelling ends the whole process tree: a POSIX process group, a Windows job object.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use acpira_shared::protocol::{AgentInstallProgress, AgentInstallStatus};
use base64::Engine;
use parking_lot::Mutex;
use regex::Regex;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use crate::platform::command::Os;

/// Lines kept from an installer's output (the page shows the tail; a failure needs enough context)
const LOG_LINES: usize = 400;
/// Progress pushes per second at most while output streams
const PUSH_EVERY: Duration = Duration::from_millis(200);
/// How long a cancelled POSIX group gets after SIGTERM before SIGKILL
#[cfg(unix)]
const TERM_GRACE: Duration = Duration::from_secs(3);
/// Where npm puts packages when its global prefix is not writable (bins in `~/.local/bin`)
pub const USER_PREFIX: &str = "~/.local";

// ---- the install line ---------------------------------------------------------------------------------------------

/// Whether the line is an `npm install -g …` (the installs that need Node and a writable global prefix)
pub fn is_npm_global(line: &str) -> bool {
  let words: Vec<&str> = line.split_whitespace().collect();
  matches!(words.first(), Some(&"npm") | Some(&"npm.cmd"))
    && words.iter().any(|w| matches!(*w, "install" | "i" | "add"))
    && words.iter().any(|w| matches!(*w, "-g" | "--global"))
}

/// The copyable POSIX line with npm sent to the user prefix: `--prefix "$HOME/.local"` right after `-g`
pub fn with_user_prefix(line: &str) -> String {
  let mut out: Vec<String> = vec![];
  let mut done = false;
  for w in line.split_whitespace() {
    out.push(w.to_owned());
    if !done && (w == "-g" || w == "--global") {
      out.push("--prefix".into());
      out.push("\"$HOME/.local\"".into());
      done = true;
    }
  }
  out.join(" ")
}

/// npm's global prefix as last probed: Some(true) when it is not writable (POSIX installs go to USER_PREFIX)
static NPM_PREFIX_BLOCKED: Mutex<Option<bool>> = Mutex::new(None);

pub fn npm_prefix_blocked() -> bool {
  NPM_PREFIX_BLOCKED.lock().unwrap_or(false)
}

/// Ask npm where its global prefix is and whether this user may write there; None when npm is not installed. POSIX
/// only: Windows' default prefix is `%APPDATA%\npm`, always the user's own
pub async fn probe_npm_prefix(os: Os) -> Option<bool> {
  let npm = find_npm(os).await?;
  if os == Os::Windows {
    return Some(false);
  }
  let mut cmd = super::launch::command(&npm, &["prefix".into(), "-g".into()]);
  cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
  let out = tokio::time::timeout(Duration::from_secs(15), cmd.output()).await.ok()?.ok()?;
  let prefix = String::from_utf8_lossy(&out.stdout).trim().to_owned();
  if !out.status.success() || prefix.is_empty() {
    return None;
  }
  let blocked = !prefix_writable(Path::new(&prefix));
  *NPM_PREFIX_BLOCKED.lock() = Some(blocked);
  Some(blocked)
}

/// npm writes `<prefix>/lib/node_modules` and `<prefix>/bin`: the deepest of those that exists must be writable
fn prefix_writable(prefix: &Path) -> bool {
  let mut probe = prefix.join("lib").join("node_modules");
  while !probe.exists() {
    match probe.parent() {
      Some(p) if p != probe => probe = p.to_path_buf(),
      _ => return false,
    }
  }
  writable(&probe) && (!prefix.join("bin").exists() || writable(&prefix.join("bin")))
}

#[cfg(unix)]
fn writable(p: &Path) -> bool {
  use std::os::unix::ffi::OsStrExt;
  let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
  // SAFETY: a NUL-terminated path; access(2) only reads it
  unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

#[cfg(not(unix))]
fn writable(p: &Path) -> bool {
  std::fs::metadata(p).is_ok_and(|m| !m.permissions().readonly())
}

async fn find_npm(os: Os) -> Option<String> {
  super::registry::resolve_command("npm", &[], os, &super::launch::ProcessEnv).await
}

// ---- the plan -----------------------------------------------------------------------------------------------------

/// What one install runs, and the facts the page shows next to its log
#[derive(Debug, Clone)]
pub struct Plan {
  pub program: String,
  pub args: Vec<String>,
  pub env: Vec<(String, String)>,
  pub proxy: Option<String>,
  pub prefix: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanError {
  /// An npm line on a machine without npm
  NodeMissing,
}

/// The process for `line` on this OS
pub async fn plan(line: &str, os: Os) -> Result<Plan, PlanError> {
  let proxy = crate::net_proxy::current();
  let mut env: Vec<(String, String)> = vec![
    // Plain output for a log, and installers that stay quiet about colours and prompts
    ("NO_COLOR".into(), "1".into()),
    ("TERM".into(), "dumb".into()),
    ("npm_config_progress".into(), "false".into()),
    ("npm_config_fund".into(), "false".into()),
    ("npm_config_audit".into(), "false".into()),
    ("npm_config_update_notifier".into(), "false".into()),
  ];
  if let Some(url) = &proxy {
    env.extend(crate::net_proxy::env_pairs(url));
    // `acpira install-agent` (a native release) takes the same route; it has no settings of its own
    env.push((crate::net_proxy::OVERRIDE_ENV.into(), url.clone()));
  }
  let mut prefix = None;
  if is_npm_global(line) {
    match probe_npm_prefix(os).await {
      None => return Err(PlanError::NodeMissing),
      Some(true) => {
        let dir = super::registry::expand_home(&format!("{USER_PREFIX}/"));
        let dir = dir.trim_end_matches('/').to_owned();
        env.push(("npm_config_prefix".into(), dir.clone()));
        prefix = Some(dir);
      }
      Some(false) => {}
    }
  }
  let (program, args) = match os {
    Os::Windows => powershell(line, proxy.as_deref()),
    Os::Posix => (posix_shell(), vec!["-c".into(), line.to_owned()]),
  };
  Ok(Plan { program, args, env, proxy, prefix })
}

/// The vendor lines are written for bash (`curl … | bash`); a machine without it still gets a POSIX shell
fn posix_shell() -> String {
  ["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash", "/opt/homebrew/bin/bash"]
    .iter()
    .find(|p| Path::new(p).exists())
    .map_or_else(|| "/bin/sh".into(), |p| (*p).into())
}

/// Windows PowerShell 5.1 source for `line`, UTF-16LE base64 like the terminal launch (no second quoting layer)
fn powershell(line: &str, proxy: Option<&str>) -> (String, Vec<String>) {
  let mut script = String::from(
    "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
     $ProgressPreference = 'SilentlyContinue'; \
     [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12; ",
  );
  if let Some(url) = proxy.filter(|u| u.starts_with("http://") || u.starts_with("https://")) {
    // Invoke-RestMethod / WebClient in 5.1 read this, not the environment; loopback stays direct
    script.push_str(&format!("[System.Net.WebRequest]::DefaultWebProxy = New-Object System.Net.WebProxy('{}', $true); ", url.replace('\'', "''")));
  }
  script.push_str(line);
  // A native command's failure (npm.cmd) does not end the script by itself
  script.push_str("\nif ($LASTEXITCODE) { exit $LASTEXITCODE }");
  let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
  (
    "powershell.exe".into(),
    vec![
      "-NoLogo".into(),
      "-NoProfile".into(),
      "-NonInteractive".into(),
      "-ExecutionPolicy".into(),
      "Bypass".into(),
      "-EncodedCommand".into(),
      base64::engine::general_purpose::STANDARD.encode(bytes),
    ],
  )
}

// ---- output -------------------------------------------------------------------------------------------------------

static ANSI: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]").expect("ansi"));

/// Printable text of one output line
pub fn clean(s: &str) -> String {
  ANSI.replace_all(s, "").chars().filter(|c| !c.is_control() || *c == '\t').collect::<String>().trim_end().to_owned()
}

/// One piece of a stream: a finished line, or a carriage-return update that the next piece of the same stream replaces
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
  Line(String),
  Progress(String),
}

/// Splits one stream's bytes into pieces; a multi-byte character cut by a read boundary waits for its other half, and so
/// does a `\r` until the next byte tells a Windows line end (`\r\n`) from a progress update
#[derive(Default)]
struct Splitter {
  pending: Vec<u8>,
  cr: bool,
}

impl Splitter {
  fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
    let mut out = vec![];
    for &b in bytes {
      if std::mem::take(&mut self.cr) && b != b'\n' {
        out.push(Piece::Progress(self.take()));
      }
      match b {
        b'\n' => out.push(Piece::Line(self.take())),
        b'\r' => self.cr = true,
        _ => self.pending.push(b),
      }
    }
    out
  }
  fn finish(&mut self) -> Option<Piece> {
    self.cr = false;
    (!self.pending.is_empty()).then(|| Piece::Line(self.take()))
  }
  fn take(&mut self) -> String {
    String::from_utf8_lossy(&std::mem::take(&mut self.pending)).into_owned()
  }
}

/// The merged log of both streams
#[derive(Default)]
pub struct Log {
  lines: VecDeque<String>,
  /// Lines dropped from the front so far: `progress` holds absolute positions
  dropped: usize,
  /// Per stream, where its carriage-return update sits: the stream's next piece rewrites that line in place, so a
  /// progress bar stays one line even when the other stream printed in between
  progress: [Option<usize>; 2],
}

impl Log {
  fn apply(&mut self, stream: u8, piece: Piece) {
    let (text, progress) = match piece {
      Piece::Line(t) => (clean(&t), false),
      Piece::Progress(t) => (clean(&t), true),
    };
    let slot = usize::from(stream.min(1));
    // A bare `\r` with nothing before it (a cursor reset) changes nothing
    if text.is_empty() && progress {
      return;
    }
    if let Some(at) = self.progress[slot].take().and_then(|abs| abs.checked_sub(self.dropped))
      && at < self.lines.len()
    {
      self.lines[at] = text;
      if progress {
        self.progress[slot] = Some(at + self.dropped);
      }
      return;
    }
    if text.is_empty() && self.lines.back().is_some_and(String::is_empty) {
      return;
    }
    self.push(text);
    if progress {
      self.progress[slot] = Some(self.dropped + self.lines.len() - 1);
    }
  }
  pub fn push(&mut self, line: String) {
    self.lines.push_back(line);
    while self.lines.len() > LOG_LINES {
      self.lines.pop_front();
      self.dropped += 1;
    }
  }
  pub fn lines(&self) -> Vec<String> {
    let mut v: Vec<String> = self.lines.iter().cloned().collect();
    while v.last().is_some_and(String::is_empty) {
      v.pop();
    }
    v
  }
}

// ---- running ------------------------------------------------------------------------------------------------------

/// How a run ended
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
  Success,
  Failed(Option<i32>),
  Cancelled,
  /// The process could not be started at all
  Spawn(String),
}

/// Stops a run from anywhere; the first call wins
#[derive(Clone, Default)]
pub struct Cancel(Arc<(Mutex<bool>, tokio::sync::Notify)>);

impl Cancel {
  pub fn cancel(&self) {
    *self.0.0.lock() = true;
    self.0.1.notify_waiters();
    self.0.1.notify_one();
  }
  fn is_cancelled(&self) -> bool {
    *self.0.0.lock()
  }
  async fn wait(&self) {
    if self.is_cancelled() {
      return;
    }
    self.0.1.notified().await;
  }
}

/// Run `plan` to its end, handing the log to `update` (throttled) as it grows
pub async fn run(plan: &Plan, cancel: Cancel, log: Arc<Mutex<Log>>, update: impl Fn() + Send + Sync) -> Outcome {
  let mut cmd = crate::platform::command::command(&plan.program, &plan.args);
  if let Some(path) = super::login_path::merged() {
    cmd.env("PATH", path);
  }
  for (k, v) in &plan.env {
    cmd.env(k, v);
  }
  cmd.current_dir(crate::store::data_dir::home_dir()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
  #[cfg(unix)]
  cmd.process_group(0);
  #[cfg(windows)]
  let spawned = crate::platform::windows_process::spawn(&mut cmd).await;
  #[cfg(not(windows))]
  let spawned = cmd.spawn().map(|child| (child, ()));
  let (mut child, job) = match spawned {
    Ok(c) => c,
    Err(e) => return Outcome::Spawn(format!("{}: {e}", plan.program)),
  };
  #[cfg(not(windows))]
  let _ = &job;
  let (tx, mut rx) = mpsc::unbounded_channel::<(u8, Piece)>();
  let readers: Vec<_> = [child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>), child.stderr.take().map(|s| Box::new(s) as _)]
    .into_iter()
    .enumerate()
    .filter_map(|(i, s)| s.map(|s| (i as u8, s)))
    .map(|(stream, mut src)| {
      let tx = tx.clone();
      tokio::spawn(async move {
        let mut split = Splitter::default();
        let mut buf = vec![0u8; 8192];
        loop {
          match src.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
              for p in split.feed(&buf[..n]) {
                let _ = tx.send((stream, p));
              }
            }
          }
        }
        if let Some(p) = split.finish() {
          let _ = tx.send((stream, p));
        }
      })
    })
    .collect();
  drop(tx);
  #[cfg(unix)]
  let pid = child.id();
  let mut last_push = tokio::time::Instant::now() - PUSH_EVERY;
  let mut streams_open = true;
  let mut cancelled = false;
  let status = loop {
    tokio::select! {
      piece = rx.recv(), if streams_open => match piece {
        Some((stream, piece)) => {
          log.lock().apply(stream, piece);
          if last_push.elapsed() >= PUSH_EVERY {
            last_push = tokio::time::Instant::now();
            update();
          }
        }
        None => streams_open = false,
      },
      status = child.wait() => break status.ok(),
      _ = cancel.wait(), if !cancelled => {
        cancelled = true;
        #[cfg(unix)]
        if let Some(pid) = pid {
          kill_group(pid).await;
        }
        #[cfg(windows)]
        job.terminate();
        let _ = child.start_kill();
      }
    }
  };
  // Whatever the readers still hold (a grandchild may keep a pipe open: give it a moment, not forever)
  let drain = async {
    while let Some((stream, piece)) = rx.recv().await {
      log.lock().apply(stream, piece);
    }
  };
  let _ = tokio::time::timeout(Duration::from_secs(2), drain).await;
  for r in readers {
    r.abort();
  }
  #[cfg(unix)]
  if let Some(pid) = pid {
    // Helpers the installer left running in its group (a daemon it started) go with it
    // SAFETY: signalling the process group this run created
    unsafe {
      libc::killpg(pid as i32, libc::SIGKILL);
    }
  }
  update();
  if cancelled || cancel.is_cancelled() {
    return Outcome::Cancelled;
  }
  match status {
    Some(s) if s.success() => Outcome::Success,
    Some(s) => Outcome::Failed(s.code()),
    None => Outcome::Failed(None),
  }
}

#[cfg(unix)]
async fn kill_group(pid: u32) {
  // SAFETY: signalling the process group this run created (its leader's pid)
  unsafe {
    libc::killpg(pid as i32, libc::SIGTERM);
  }
  tokio::time::sleep(TERM_GRACE).await;
  unsafe {
    libc::killpg(pid as i32, libc::SIGKILL);
  }
}

// ---- the engine's installs ----------------------------------------------------------------------------------------

struct Entry {
  progress: AgentInstallProgress,
  log: Arc<Mutex<Log>>,
  cancel: Option<Cancel>,
}

/// Every agent's latest install on this engine; `emit` pushes the list to every webview
pub struct Installs {
  entries: Mutex<HashMap<String, Entry>>,
  order: Mutex<Vec<String>>,
  emit: Arc<dyn Fn(Vec<AgentInstallProgress>) + Send + Sync>,
}

/// Texts the page shows for a failure, localized by the caller
pub struct Messages {
  pub node_missing: String,
  pub failed: Box<dyn Fn(Option<i32>) -> String + Send + Sync>,
  pub spawn: Box<dyn Fn(&str) -> String + Send + Sync>,
}

impl Installs {
  pub fn new(emit: Arc<dyn Fn(Vec<AgentInstallProgress>) + Send + Sync>) -> Arc<Self> {
    Arc::new(Installs { entries: Default::default(), order: Default::default(), emit })
  }

  pub fn list(&self) -> Vec<AgentInstallProgress> {
    let entries = self.entries.lock();
    self.order.lock().iter().filter_map(|id| entries.get(id)).map(|e| {
      let mut p = e.progress.clone();
      p.log = e.log.lock().lines();
      p
    }).collect()
  }

  pub fn running(&self, agent: &str) -> bool {
    self.entries.lock().get(agent).is_some_and(|e| e.progress.status == AgentInstallStatus::Running)
  }

  fn push(&self) {
    (self.emit)(self.list());
  }

  fn set(&self, agent: &str, f: impl FnOnce(&mut Entry)) {
    if let Some(e) = self.entries.lock().get_mut(agent) {
      f(e);
    }
    self.push();
  }

  /// Start `line` for `agent` unless one is running already; `done(success)` runs after the end has been pushed
  pub fn start(
    self: &Arc<Self>,
    agent: &str,
    line: String,
    os: Os,
    messages: Messages,
    done: impl FnOnce(bool) + Send + 'static,
  ) -> bool {
    {
      let mut entries = self.entries.lock();
      if entries.get(agent).is_some_and(|e| e.progress.status == AgentInstallStatus::Running) {
        return false;
      }
      let mut log = Log::default();
      log.push(format!("$ {}", first_line(&line)));
      let cancel = Cancel::default();
      entries.insert(
        agent.to_owned(),
        Entry {
          progress: AgentInstallProgress { agent: agent.to_owned(), status: AgentInstallStatus::Running, log: vec![], proxy: None, prefix: None, error: None },
          log: Arc::new(Mutex::new(log)),
          cancel: Some(cancel),
        },
      );
      let mut order = self.order.lock();
      order.retain(|a| a != agent);
      order.push(agent.to_owned());
    }
    self.push();
    let me = self.clone();
    let agent = agent.to_owned();
    tokio::spawn(async move {
      let (log, cancel) = {
        let entries = me.entries.lock();
        let e = &entries[&agent];
        (e.log.clone(), e.cancel.clone().unwrap_or_default())
      };
      let plan = match plan(&line, os).await {
        Ok(p) => p,
        Err(PlanError::NodeMissing) => {
          me.set(&agent, |e| {
            e.progress.status = AgentInstallStatus::Failed;
            e.progress.error = Some(messages.node_missing.clone());
            e.cancel = None;
          });
          done(false);
          return;
        }
      };
      me.set(&agent, |e| {
        e.progress.proxy = plan.proxy.clone();
        e.progress.prefix = plan.prefix.clone();
      });
      let push = {
        let me = me.clone();
        move || me.push()
      };
      let outcome = if cancel.is_cancelled() { Outcome::Cancelled } else { run(&plan, cancel, log, push).await };
      let ok = outcome == Outcome::Success;
      me.set(&agent, |e| {
        e.cancel = None;
        match &outcome {
          Outcome::Success => e.progress.status = AgentInstallStatus::Success,
          Outcome::Cancelled => e.progress.status = AgentInstallStatus::Cancelled,
          Outcome::Failed(code) => {
            e.progress.status = AgentInstallStatus::Failed;
            e.progress.error = Some((messages.failed)(*code));
          }
          Outcome::Spawn(why) => {
            e.progress.status = AgentInstallStatus::Failed;
            e.progress.error = Some((messages.spawn)(why));
          }
        }
      });
      done(ok);
    });
    true
  }

  pub fn cancel(&self, agent: &str) {
    if let Some(c) = self.entries.lock().get(agent).and_then(|e| e.cancel.clone()) {
      c.cancel();
    }
  }

  /// Stop every running install (engine shutdown)
  pub fn cancel_all(&self) {
    for c in self.entries.lock().values().filter_map(|e| e.cancel.clone()) {
      c.cancel();
    }
  }
}

/// The first line of a multi-line PowerShell block, for the log's command echo
fn first_line(line: &str) -> String {
  let mut lines = line.lines().map(str::trim).filter(|l| !l.is_empty());
  let first = lines.next().unwrap_or_default().to_owned();
  if lines.next().is_some() { format!("{first} …") } else { first }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn npm_global_lines_are_recognized_and_get_the_user_prefix() {
    assert!(is_npm_global("npm install -g --include=optional @agentclientprotocol/codex-acp@2.1.1"));
    assert!(is_npm_global("npm i --global pi-acp"));
    assert!(!is_npm_global("npm install pi-acp"));
    assert!(!is_npm_global("curl -fsSL https://opencode.ai/install | bash"));
    assert_eq!(
      with_user_prefix("npm install -g --ignore-scripts @earendil-works/pi-coding-agent pi-acp"),
      "npm install -g --prefix \"$HOME/.local\" --ignore-scripts @earendil-works/pi-coding-agent pi-acp"
    );
  }

  #[test]
  fn output_is_cleaned_and_carriage_return_progress_collapses() {
    assert_eq!(clean("\x1b[32m✓\x1b[0m Installed\x1b]8;;https://x\x07link\x1b]8;;\x07  "), "✓ Installedlink");
    let mut log = Log::default();
    let mut out = Splitter::default();
    let mut err = Splitter::default();
    for p in out.feed(b"Downloading\n 10%\r 50%\r") {
      log.apply(0, p);
    }
    for p in err.feed(b"warn: slow\n") {
      log.apply(1, p);
    }
    for p in out.feed(b"100%\r\ndone\n") {
      log.apply(0, p);
    }
    // The progress line is rewritten in place even though stderr printed after it
    assert_eq!(log.lines(), vec!["Downloading", "100%", "warn: slow", "done"]);
    // A UTF-8 character split across two reads arrives whole
    let mut s = Splitter::default();
    let bytes = "安装完成\n".as_bytes();
    let mut pieces = s.feed(&bytes[..4]);
    pieces.extend(s.feed(&bytes[4..]));
    assert_eq!(pieces, vec![Piece::Line("安装完成".into())]);
  }

  #[test]
  fn the_log_keeps_only_its_tail() {
    let mut log = Log::default();
    for i in 0..(LOG_LINES + 10) {
      log.apply(0, Piece::Line(format!("line {i}")));
    }
    let lines = log.lines();
    assert_eq!(lines.len(), LOG_LINES);
    assert_eq!(lines[0], "line 10");
  }

  #[test]
  fn the_windows_plan_carries_utf8_tls_the_proxy_and_the_exit_code() {
    let (program, args) = powershell("npm install -g pi-acp", Some("http://127.0.0.1:7890"));
    assert_eq!(program, "powershell.exe");
    assert!(args.contains(&"-NonInteractive".to_owned()) && args.contains(&"Bypass".to_owned()));
    let bytes = base64::engine::general_purpose::STANDARD.decode(args.last().unwrap()).unwrap();
    let src = String::from_utf16(&bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>()).unwrap();
    assert!(src.contains("OutputEncoding"));
    assert!(src.contains("Tls12"));
    assert!(src.contains("New-Object System.Net.WebProxy('http://127.0.0.1:7890', $true)"));
    assert!(src.contains("npm install -g pi-acp\nif ($LASTEXITCODE) { exit $LASTEXITCODE }"));
    // A SOCKS proxy is left to the environment: .NET's WebProxy speaks HTTP only
    let (_, args) = powershell("irm x | iex", Some("socks5://127.0.0.1:1080"));
    let bytes = base64::engine::general_purpose::STANDARD.decode(args.last().unwrap()).unwrap();
    let src = String::from_utf16(&bytes.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect::<Vec<_>>()).unwrap();
    assert!(!src.contains("WebProxy"));
  }

  #[cfg(unix)]
  #[test]
  fn an_unwritable_prefix_is_detected() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let mine = dir.path().join("mine");
    std::fs::create_dir_all(mine.join("lib/node_modules")).unwrap();
    assert!(prefix_writable(&mine));
    // A prefix whose lib does not exist yet is judged by the deepest existing parent
    assert!(prefix_writable(&dir.path().join("fresh")));
    let locked = dir.path().join("locked");
    std::fs::create_dir_all(locked.join("lib/node_modules")).unwrap();
    std::fs::set_permissions(locked.join("lib/node_modules"), std::fs::Permissions::from_mode(0o555)).unwrap();
    // root writes anywhere: the check is only meaningful for an ordinary user
    if unsafe { libc::geteuid() } != 0 {
      assert!(!prefix_writable(&locked));
    }
    std::fs::set_permissions(locked.join("lib/node_modules"), std::fs::Permissions::from_mode(0o755)).unwrap();
  }

  #[cfg(unix)]
  fn posix_plan(line: &str) -> Plan {
    Plan { program: posix_shell(), args: vec!["-c".into(), line.into()], env: vec![], proxy: None, prefix: None }
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn a_run_streams_both_outputs_and_reports_the_exit_code() {
    let log = Arc::new(Mutex::new(Log::default()));
    let ok = run(&posix_plan("echo out; echo err >&2; printf '1%%\\r2%%\\r'; echo done"), Cancel::default(), log.clone(), || {}).await;
    assert_eq!(ok, Outcome::Success);
    let lines = log.lock().lines();
    for want in ["out", "err", "done"] {
      assert!(lines.iter().any(|l| l == want), "{want} in {lines:?}");
    }
    assert!(!lines.iter().any(|l| l == "1%"), "{lines:?}");
    let log = Arc::new(Mutex::new(Log::default()));
    assert_eq!(run(&posix_plan("echo nope; exit 3"), Cancel::default(), log, || {}).await, Outcome::Failed(Some(3)));
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn cancelling_ends_the_whole_group() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("child.pid");
    // The installer starts a grandchild that would outlive a plain kill of the shell
    let line = format!("sleep 30 & echo $! > '{}'; wait", marker.display());
    let cancel = Cancel::default();
    let c = cancel.clone();
    tokio::spawn(async move {
      tokio::time::sleep(Duration::from_millis(300)).await;
      c.cancel();
    });
    let log = Arc::new(Mutex::new(Log::default()));
    let started = std::time::Instant::now();
    assert_eq!(run(&posix_plan(&line), cancel, log, || {}).await, Outcome::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(10));
    let pid: i32 = std::fs::read_to_string(&marker).unwrap().trim().parse().unwrap();
    // SAFETY: signal 0 only checks whether the pid exists
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    assert!(!alive, "grandchild {pid} survived the cancel");
  }

  #[tokio::test]
  async fn installs_push_progress_and_end_with_the_outcome() {
    let pushed: Arc<Mutex<Vec<Vec<AgentInstallProgress>>>> = Default::default();
    let p = pushed.clone();
    let installs = Installs::new(Arc::new(move |list| p.lock().push(list)));
    let messages = || Messages {
      node_missing: "node".into(),
      failed: Box::new(|code| format!("failed {code:?}")),
      spawn: Box::new(|why| format!("spawn {why}")),
    };
    let (tx, rx) = tokio::sync::oneshot::channel();
    let line = if cfg!(windows) { "Write-Output hello; exit 2" } else { "echo hello; exit 2" };
    assert!(installs.start("mine", line.into(), Os::current(), messages(), move |ok| {
      let _ = tx.send(ok);
    }));
    // A second click while it runs starts nothing
    assert!(!installs.start("mine", "echo again".into(), Os::current(), messages(), |_| {}));
    assert!(!rx.await.unwrap());
    let last = installs.list().pop().unwrap();
    assert_eq!(last.status, AgentInstallStatus::Failed);
    assert_eq!(last.error.as_deref(), Some("failed Some(2)"));
    assert!(last.log.iter().any(|l| l == "hello"), "{:?}", last.log);
    assert_eq!(pushed.lock().first().unwrap()[0].status, AgentInstallStatus::Running);
  }
}
