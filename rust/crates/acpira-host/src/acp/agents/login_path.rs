//! The login shell's PATH. A sidecar started by an IDE inherits whatever environment the IDE (or VS Code Server over
//! Remote-SSH) was started with, which misses directories that only the user's shell rc files add: an npm prefix such as
//! `~/.npm-global/bin`, nvm, pnpm, mise, volta … One interactive login shell run recovers them. The directories it adds
//! go after the inherited ones and serve both the executable lookup (`ProcessEnv`) and the agents' own environment, since
//! an npm-packaged adapter is a `#!/usr/bin/env node` script that also needs `node` on its PATH

use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::sync::OnceCell;

/// `0` skips the shell run (the vitest contract suites, which spawn many sidecars)
pub const LOGIN_PATH_ENV: &str = "ACPIRA_LOGIN_PATH";
/// Set for the shell run so rc files can skip slow or interactive parts
pub const RESOLVING_ENV: &str = "ACPIRA_RESOLVING_ENVIRONMENT";
/// A shell whose rc files hang (a prompt waiting for input, a slow network call) is abandoned after this
#[cfg(not(windows))]
const SHELL_TIMEOUT: Duration = Duration::from_secs(5);
const MARK: &str = "__ACPIRA_LOGIN_PATH__";

/// The login shell outcome, once the run finished
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LoginPath {
  /// The inherited PATH with the login shell's extra directories appended; None when the shell added nothing
  pub merged: Option<String>,
  /// The directories the login shell contributed, in its order
  pub added: Vec<String>,
}

static RESULT: parking_lot::RwLock<Option<LoginPath>> = parking_lot::RwLock::new(None);
static LOAD: OnceCell<()> = OnceCell::const_new();
/// When the last shell run finished, for `refresh`'s throttle
static LAST_RUN: parking_lot::Mutex<Option<Instant>> = parking_lot::Mutex::new(None);
/// Serializes refreshes so focus events arriving together share one shell run
static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Run the login shell once per process; later calls wait for the same run
pub async fn ready() {
  LOAD.get_or_init(|| async { store(load().await) }).await;
}

/// Run the shell again when the last run is older than `min_age`: an rc file edited after the sidecar started (an npm
/// prefix just added) then counts without reopening the window. Resolves to whether the merged PATH changed
pub async fn refresh(min_age: Duration) -> bool {
  ready().await;
  let _guard = REFRESHING.lock().await;
  if LAST_RUN.lock().is_some_and(|t| t.elapsed() < min_age) {
    return false;
  }
  let before = merged();
  store(load().await);
  before != merged()
}

fn store(found: LoginPath) {
  *RESULT.write() = Some(found);
  *LAST_RUN.lock() = Some(Instant::now());
}

/// The PATH agents are looked up in and launched with: the merged one once known, the inherited one before that
pub fn effective_path() -> Option<String> {
  merged().or_else(|| std::env::var("PATH").ok())
}

/// The merged PATH, only when the login shell added directories (spawns set it explicitly then)
pub fn merged() -> Option<String> {
  RESULT.read().as_ref().and_then(|r| r.merged.clone())
}

/// The directories the login shell added to the inherited PATH
pub fn added() -> Vec<String> {
  RESULT.read().as_ref().map(|r| r.added.clone()).unwrap_or_default()
}

async fn load() -> LoginPath {
  if std::env::var(LOGIN_PATH_ENV).is_ok_and(|v| v == "0") {
    return LoginPath::default();
  }
  #[cfg(windows)]
  {
    let inherited = std::env::var("PATH").unwrap_or_default();
    let installed = tokio::task::spawn_blocking(crate::platform::environment::installed_path).await.unwrap_or_default();
    merge_windows(&inherited, &installed)
  }
  #[cfg(not(windows))]
  {
    let Some(shell) = std::env::var("SHELL").ok().filter(|s| !s.is_empty()) else { return LoginPath::default() };
    let inherited = std::env::var("PATH").unwrap_or_default();
    match read_shell_path(&shell, SHELL_TIMEOUT).await {
      Some(login) => merge(&inherited, &login),
      None => LoginPath::default(),
    }
  }
}

/// Windows installers update the User / Machine registry PATH, not an already-running IDE's environment.
/// Retain the IDE's precedence, append new absolute entries, and compare names with Windows casing / separators.
#[cfg(any(windows, test))]
fn merge_windows(inherited: &str, installed: &str) -> LoginPath {
  let key = |s: &str| s.trim_matches('"').replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
  let mut dirs: Vec<String> = inherited.split(';').filter(|d| !d.is_empty()).map(String::from).collect();
  let mut known: std::collections::HashSet<String> = dirs.iter().map(|d| key(d)).collect();
  let mut added = vec![];
  for dir in installed.split(';').map(|d| d.trim().trim_matches('"')).filter(|d| !d.is_empty()) {
    let bytes = dir.as_bytes();
    let absolute =
      dir.starts_with(r"\\") || (bytes.len() > 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'/' | b'\\'));
    if absolute && known.insert(key(dir)) {
      dirs.push(dir.to_owned());
      added.push(dir.to_owned());
    }
  }
  if added.is_empty() { LoginPath::default() } else { LoginPath { merged: Some(dirs.join(";")), added } }
}

/// `$SHELL -i -l -c` printing PATH between markers, since rc files may print banners of their own. bash, zsh and fish
/// all take the three flags; a shell that does not (nushell) just yields nothing
pub async fn read_shell_path(shell: &str, timeout: Duration) -> Option<String> {
  let script = format!("printf '\\n%s%s%s\\n' '{MARK}' \"$PATH\" '{MARK}'");
  let mut cmd = Command::new(shell);
  cmd
    .args(["-i", "-l", "-c", &script])
    .env(RESOLVING_ENV, "1")
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::null())
    .kill_on_drop(true);
  // A new session without a controlling terminal: an interactive shell otherwise tries to take over the terminal the
  // sidecar was started from (the harness) and stops on SIGTTOU / SIGTTIN until the timeout
  #[cfg(unix)]
  unsafe {
    cmd.pre_exec(|| {
      libc::setsid();
      Ok(())
    });
  }
  let child = cmd.spawn().ok()?;
  let pid = child.id();
  match tokio::time::timeout(timeout, child.wait_with_output()).await {
    Ok(out) => parse_marked(&String::from_utf8_lossy(&out.ok()?.stdout)),
    Err(_) => {
      // The shell leads its own session: take down whatever its rc files started along with it
      #[cfg(unix)]
      if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
        unsafe { libc::kill(-pid, libc::SIGKILL) };
      }
      let _ = pid;
      None
    }
  }
}

/// The text between the last pair of markers
pub fn parse_marked(stdout: &str) -> Option<String> {
  let end = stdout.rfind(MARK)?;
  let start = stdout[..end].rfind(MARK)? + MARK.len();
  let path = stdout[start..end].trim();
  (!path.is_empty()).then(|| path.to_owned())
}

/// Inherited directories keep their order and precedence; the login shell's absolute directories not already present
/// are appended
pub fn merge(inherited: &str, login: &str) -> LoginPath {
  let mut dirs: Vec<&str> = inherited.split(':').filter(|d| !d.is_empty()).collect();
  let mut added = vec![];
  for d in login.split(':') {
    if d.starts_with('/') && !dirs.contains(&d) {
      dirs.push(d);
      added.push(d.to_owned());
    }
  }
  if added.is_empty() {
    return LoginPath::default();
  }
  LoginPath { merged: Some(dirs.join(":")), added }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn windows_installs_extend_the_existing_path_without_case_duplicates() {
    let result = merge_windows(r"C:\Windows\System32;C:\Node", r"c:/node/;D:\用户\bin;\\server\share\bin;relative;C:relative");
    assert_eq!(result.added, [r"D:\用户\bin", r"\\server\share\bin"]);
    assert_eq!(result.merged.as_deref(), Some(r"C:\Windows\System32;C:\Node;D:\用户\bin;\\server\share\bin"));
    assert_eq!(merge_windows(r"C:\Node", r#""c:\node\""#), LoginPath::default());
  }

  #[test]
  fn parses_the_last_marked_path_among_rc_noise() {
    let out = format!("welcome!\n{MARK}/stale{MARK}\nmotd\n{MARK}/a/bin:/b/bin{MARK}\n");
    assert_eq!(parse_marked(&out).as_deref(), Some("/a/bin:/b/bin"));
    assert_eq!(parse_marked("no markers"), None);
    assert_eq!(parse_marked(&format!("{MARK}{MARK}")), None);
  }

  #[test]
  fn merge_appends_only_new_absolute_dirs() {
    let r = merge("/usr/bin:/bin", "/home/u/.npm-global/bin:/usr/bin:relative:/bin:/opt/x");
    assert_eq!(r.merged.as_deref(), Some("/usr/bin:/bin:/home/u/.npm-global/bin:/opt/x"));
    assert_eq!(r.added, vec!["/home/u/.npm-global/bin", "/opt/x"]);
    assert_eq!(merge("/usr/bin", "/usr/bin"), LoginPath::default());
  }

  #[cfg(unix)]
  #[tokio::test]
  async fn reads_path_from_a_shell_and_gives_up_on_a_hung_one() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    // A stand-in shell: ignores its flags and runs the script the way `sh -c` would, after some rc noise
    let fake = dir.path().join("fake-shell");
    std::fs::write(&fake, "#!/bin/sh\necho banner\nPATH=/login/bin:$PATH\neval \"$4\"\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = read_shell_path(fake.to_str().unwrap(), Duration::from_secs(5)).await.unwrap();
    assert!(path.starts_with("/login/bin:"), "{path}");

    let hung = dir.path().join("hung-shell");
    std::fs::write(&hung, "#!/bin/sh\nsleep 30\n").unwrap();
    std::fs::set_permissions(&hung, std::fs::Permissions::from_mode(0o755)).unwrap();
    let t = std::time::Instant::now();
    assert_eq!(read_shell_path(hung.to_str().unwrap(), Duration::from_millis(300)).await, None);
    assert!(t.elapsed() < Duration::from_secs(5));
  }
}
