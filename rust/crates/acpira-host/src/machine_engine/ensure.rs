//! `acpira engine [--home DIR] [--no-dev-takeover] [--idle-grace S]`: makes sure the machine's one engine runs and is at least this build,
//! then prints where it listens as one JSON line (`{"socket", "version", "pid"}`). Every shell runs this before it
//! connects, with the engine binary it carries:
//! - nothing listens on the machine's socket: this build lands in `<home>/engine/` (`stage`) and starts from there;
//! - an older engine listens and this build may replace it (`policy::takes_over`): it is asked to drain (it hands the
//!   socket over, its windows reconnect here, it ends once its running turns have) and this build starts;
//! - an engine of this version or newer listens: nothing to do, the shell connects to it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};
#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use acpira_shared::version::VERSION;

use super::{policy, stage};

/// How long a probe or drain answer may take: an engine answers both before any runtime work
#[cfg(unix)]
const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// How long the whole thing may take: landing a large debug build, an old engine handing over, a fresh start
const ENSURE_WAIT: Duration = Duration::from_secs(40);
const LOG_ROTATE_BYTES: u64 = 4 * 1024 * 1024;

/// The engine the shell should connect to
#[derive(Debug, Clone, PartialEq)]
pub struct Reached {
  pub socket: PathBuf,
  pub version: String,
  pub pid: u32,
}

/// What the machine's socket answered
#[derive(Debug)]
pub struct Probed {
  pub version: String,
  pub pid: u32,
  pub draining: bool,
}

/// The machine's socket for `home`: `<home>/run/engine.sock`, or a private one under the temp dir when that path is
/// too long for a Unix socket (named after the data root, so two roots never share one)
pub fn socket_path(home: &Path) -> PathBuf {
  let own = home.join("run").join("engine.sock");
  if own.as_os_str().len() < 100 {
    return own;
  }
  // FNV-1a: stable across runs and builds, which is all a socket name needs
  let mut h: u64 = 0xcbf29ce484222325;
  for b in home.as_os_str().as_encoded_bytes() {
    h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
  }
  #[cfg(unix)]
  // SAFETY: geteuid has no preconditions and cannot fail
  let uid = unsafe { libc::geteuid() };
  #[cfg(not(unix))]
  let uid = 0;
  std::env::temp_dir().join(format!("acpira-{uid}")).join(format!("engine-{h:016x}.sock"))
}

/// The command line: prints the engine to connect to, exit 0; or the reason on stderr, exit 1
pub async fn cli(args: &[String], home: PathBuf) -> i32 {
  let allow_dev = !args.iter().any(|a| a == "--no-dev-takeover");
  // Handed to an engine this starts (tests end their engines quickly)
  let serve_args: Vec<String> = match args.iter().position(|a| a == "--idle-grace").and_then(|i| args.get(i + 1)) {
    Some(s) => vec!["--idle-grace".into(), s.clone()],
    None => vec![],
  };
  let log = |line: &str| eprintln!("[acpira engine] {line}");
  match ensure_with(&home, allow_dev, &serve_args, &log).await {
    Ok(r) => {
      println!("{}", json!({ "socket": r.socket, "version": r.version, "pid": r.pid }));
      0
    }
    Err(e) => {
      log(&format!("{e:#}"));
      1
    }
  }
}

/// Lands this build, then reaches the machine's engine: starting it, or taking over from an older one, when needed.
/// `allow_dev`: this build was installed here on purpose (`policy`); false for a build that came over SSH
pub async fn ensure(home: &Path, allow_dev: bool, log: &(dyn Fn(&str) + Send + Sync)) -> Result<Reached> {
  ensure_with(home, allow_dev, &[], log).await
}

async fn ensure_with(home: &Path, allow_dev: bool, serve_args: &[String], log: &(dyn Fn(&str) + Send + Sync)) -> Result<Reached> {
  let source = std::env::current_exe().map_err(|e| anyhow!("this binary's path: {e}"))?;
  let (h, s) = (home.to_path_buf(), source.clone());
  let exe = tokio::task::spawn_blocking(move || stage::stage(&h, &s, VERSION)).await??;
  let socket = socket_path(home);
  #[cfg(unix)]
  if let Some(dir) = socket.parent() {
    crate::cli::private_dir(dir).map_err(|e| anyhow!(e))?;
  }
  let deadline = tokio::time::Instant::now() + ENSURE_WAIT;
  let mut started: Option<Started> = None;
  // Asked once per engine: whatever serves is compared again on every probe, since a concurrent start of an older
  // build may have won the socket after this one started its own
  let mut drained: Option<u32> = None;
  let mut wait = Duration::from_millis(5);
  loop {
    match probe(&socket).await {
      Ok(Some(p)) if p.draining => {}
      Ok(Some(p)) if policy::takes_over(VERSION, &p.version, allow_dev) && drained != Some(p.pid) => {
        log(&format!("engine {} (pid {}) is older than {VERSION}: taking over", p.version, p.pid));
        drained = Some(p.pid);
        drain(&socket, allow_dev).await?;
        started = Some(start(&exe, &socket, home, serve_args).await?);
      }
      Ok(Some(p)) => {
        let keep = vec![exe.clone()];
        let h = home.to_path_buf();
        // Copies nothing runs from any more go; a failure here is no reason to keep the shell waiting
        let _ = tokio::task::spawn_blocking(move || stage::prune(&h, &keep)).await;
        return Ok(Reached { socket, version: p.version, pid: p.pid });
      }
      Ok(None) => {
        // Nothing listens: start the newest of this build and the landed releases (a local build that is not this
        // machine's to start counts as older than any release), or again when the one started ended without taking
        // the socket (another start won the lock and then went away)
        let ended = match started.as_mut() {
          Some(s) => s.ended().await,
          None => true,
        };
        if ended {
          let pick = match stage::newest_release(home) {
            Some((v, landed)) if policy::takes_over(&v, VERSION, false) || (policy::is_dev(VERSION) && !allow_dev) => landed,
            _ => exe.clone(),
          };
          started = Some(start(&pick, &socket, home, serve_args).await?);
        }
      }
      Err(e) => log(&format!("{}: {e:#}", socket.display())),
    }
    if tokio::time::Instant::now() >= deadline {
      bail!("no engine answered on {} within {} s; see {}", socket.display(), ENSURE_WAIT.as_secs(), log_path(home).display());
    }
    // A starting engine listens within milliseconds; an old one handing over holds the lock up to its dispose grace
    tokio::time::sleep(wait).await;
    wait = (wait * 2).min(Duration::from_millis(100));
  }
}

/// Who answers on `socket`: None when nothing listens there
pub async fn probe(socket: &Path) -> Result<Option<Probed>> {
  let Some(answer) = ask(socket, json!({ "type": "probe", "requestId": "probe" }), &["probeOk"]).await? else { return Ok(None) };
  Ok(Some(Probed {
    version: answer.get("version").and_then(Value::as_str).unwrap_or_default().to_owned(),
    pid: answer.get("pid").and_then(Value::as_u64).and_then(|p| u32::try_from(p).ok()).unwrap_or(0),
    draining: answer.get("draining").and_then(Value::as_bool).unwrap_or(false),
  }))
}

/// Asks the engine on `socket` to hand it over to this build
pub(crate) async fn drain(socket: &Path, allow_dev: bool) -> Result<()> {
  let msg = json!({ "type": "drain", "requestId": "drain", "version": VERSION, "allowDev": allow_dev });
  match ask(socket, msg, &["drainOk", "drainRefused"]).await? {
    None => Ok(()),
    Some(a) if a.get("type").and_then(Value::as_str) == Some("drainOk") => Ok(()),
    Some(a) => bail!("the running engine refused to hand over: {}", a.get("reason").and_then(Value::as_str).unwrap_or("?")),
  }
}

/// One request on its own connection, answered by one of `kinds`; None when nothing listens
#[cfg(unix)]
async fn ask(socket: &Path, msg: Value, kinds: &[&str]) -> Result<Option<Value>> {
  let stream = match tokio::net::UnixStream::connect(socket).await {
    Ok(s) => s,
    Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => return Ok(None),
    Err(e) => return Err(e.into()),
  };
  let (read, mut write) = stream.into_split();
  write.write_all(format!("{msg}\n").as_bytes()).await?;
  let mut lines = BufReader::new(read).lines();
  let answer = tokio::time::timeout(ANSWER_WAIT, async {
    while let Some(line) = lines.next_line().await? {
      let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
      if v.get("type").and_then(Value::as_str).is_some_and(|t| kinds.contains(&t)) {
        return Ok::<_, anyhow::Error>(Some(v));
      }
    }
    // Closed without an answer: an engine ending right now
    Ok(None)
  })
  .await
  .map_err(|_| anyhow!("no answer within {} s", ANSWER_WAIT.as_secs()))??;
  Ok(answer)
}

#[cfg(not(unix))]
async fn ask(_socket: &Path, _msg: Value, _kinds: &[&str]) -> Result<Option<Value>> {
  bail!("the machine engine has no endpoint on this platform yet")
}

fn log_path(home: &Path) -> PathBuf {
  home.join("run").join("engine.log")
}

/// An engine this run started
struct Started {
  how: How,
  /// The landed copy it runs from stays out of `prune`'s reach until the engine holds it itself
  _pinned: Option<std::fs::File>,
}

enum How {
  Child(Detached),
  /// A launchd job of this label
  Job(String),
}

impl Started {
  /// It exited (a start that found the lock taken exits 0), or a launchd job has no process: a kickstart while the
  /// previous process of the same build was still ending started nothing
  async fn ended(&mut self) -> bool {
    match &mut self.how {
      How::Child(Detached(c)) => c.as_mut().is_none_or(Spawned::exited),
      How::Job(label) => !crate::devices::launch_agent::running(label).await,
    }
  }
}

/// `exe serve --socket` detached from this process: in the user's macOS GUI session when this runs over SSH and one is
/// there (the login keychain is unlocked only there), else in a session of its own. On macOS either way under the
/// launcher (launcher.rs). Output goes to `run/engine.log`
async fn start(exe: &Path, socket: &Path, home: &Path, serve_args: &[String]) -> Result<Started> {
  let log = log_path(home);
  if let Some(dir) = log.parent() {
    std::fs::create_dir_all(dir)?;
  }
  if std::fs::metadata(&log).is_ok_and(|m| m.len() > LOG_ROTATE_BYTES) {
    let _ = std::fs::rename(&log, log.with_extension("log.1"));
  }
  let pinned = stage::pin(exe);
  if serve_args.is_empty() && crate::cli::over_ssh() && crate::devices::launch_agent::can_use_gui(home).await {
    let build = exe.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match crate::devices::launch_agent::start_machine_engine(&build, exe, socket, home, &log).await {
      Ok(label) => return Ok(Started { how: How::Job(label), _pinned: pinned }),
      Err(e) => append(&log, &format!("launchd start fell back to a detached process: {e:#}")),
    }
  }
  let child = spawn_detached(exe, socket, home, serve_args, &log)?;
  Ok(Started { how: How::Child(Detached(Some(child))), _pinned: pinned })
}

/// A process this one started
enum Spawned {
  #[cfg_attr(not(unix), allow(dead_code, reason = "Windows has no detached start yet (`spawn_detached`)"))]
  Child(std::process::Child),
  /// Started without `std::process` (launcher::spawn_disclaimed)
  #[cfg(target_os = "macos")]
  Pid(libc::pid_t),
}

impl Spawned {
  fn exited(&mut self) -> bool {
    match self {
      Spawned::Child(c) => c.try_wait().ok().flatten().is_some(),
      // 0: still running; the pid once reaped, or -1 when there is nothing left to reap
      #[cfg(target_os = "macos")]
      // SAFETY: waitpid on this process's own child, writing into a local
      Spawned::Pid(pid) => unsafe {
        let mut status = 0;
        libc::waitpid(*pid, &mut status, libc::WNOHANG) != 0
      },
    }
  }

  fn wait(self) {
    match self {
      Spawned::Child(mut c) => {
        let _ = c.wait();
      }
      #[cfg(target_os = "macos")]
      // SAFETY: as in `exited`; an already reaped pid answers ECHILD at once
      Spawned::Pid(pid) => unsafe {
        let mut status = 0;
        libc::waitpid(pid, &mut status, 0);
      },
    }
  }
}

/// A started engine outlives this process; a long-lived caller (the SSH helper) must not keep it as a zombie once it
/// ends, so whatever has not been reaped by then is waited for on a thread of its own
struct Detached(Option<Spawned>);

impl Drop for Detached {
  fn drop(&mut self) {
    if let Some(child) = self.0.take() {
      std::thread::spawn(move || child.wait());
    }
  }
}

fn append(log: &Path, line: &str) {
  use std::io::Write;
  if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
    let _ = writeln!(f, "[acpira engine] {line}");
  }
}

#[cfg(unix)]
fn spawn_detached(exe: &Path, socket: &Path, home: &Path, serve_args: &[String], log: &Path) -> Result<Spawned> {
  use std::os::unix::process::CommandExt;
  // Not the window's workspace: the engine serves every window, and nothing in it reads its own working directory
  let cwd = crate::store::data_dir::home_dir();
  #[cfg(target_os = "macos")]
  if let Some(program) = super::launcher::ensure(home, exe) {
    let mut args: Vec<std::ffi::OsString> = vec![super::launcher::COMMAND.into(), exe.into(), "serve".into(), "--socket".into()];
    args.extend([socket.into(), "--home".into(), home.into()]);
    args.extend(serve_args.iter().map(Into::into));
    match super::launcher::spawn_disclaimed(&program, &args, &cwd, log) {
      Ok(pid) => return Ok(Spawned::Pid(pid)),
      Err(e) => append(log, &format!("starting under {} failed, starting the engine itself: {e}", program.display())),
    }
  }
  let out = std::fs::OpenOptions::new().create(true).append(true).open(log)?;
  let err = out.try_clone()?;
  let mut cmd = std::process::Command::new(exe);
  cmd
    .arg("serve")
    .arg("--socket")
    .arg(socket)
    .arg("--home")
    .arg(home)
    .args(serve_args)
    .current_dir(cwd)
    .stdin(std::process::Stdio::null())
    .stdout(out)
    .stderr(err);
  // SAFETY: setsid is async-signal-safe and touches no memory of this process
  unsafe {
    cmd.pre_exec(|| {
      libc::setsid();
      Ok(())
    });
  }
  Ok(Spawned::Child(cmd.spawn()?))
}

#[cfg(not(unix))]
fn spawn_detached(_exe: &Path, _socket: &Path, _home: &Path, _serve_args: &[String], _log: &Path) -> Result<Spawned> {
  bail!("the machine engine has no endpoint on this platform yet")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_socket_is_per_data_root_and_short_enough() {
    let a = socket_path(Path::new("/Users/u/.acpira"));
    assert_eq!(a, Path::new("/Users/u/.acpira/run/engine.sock"));
    let deep = "/x".repeat(60);
    let b = socket_path(Path::new(&deep));
    assert!(b.as_os_str().len() < 104 && b.file_name().is_some_and(|n| n.to_string_lossy().starts_with("engine-")));
    assert_ne!(b, socket_path(Path::new(&format!("{deep}/y"))));
  }
}
