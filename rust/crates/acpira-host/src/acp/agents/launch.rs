//! Agent executable resolution and launch policy. Platform-specific argv construction lives in platform::command.

use std::path::Path;

use crate::platform::command::{self as native_command, Os};

pub trait Env: Sync {
  fn get(&self, key: &str) -> Option<String>;
}

pub struct ProcessEnv;

/// The sidecar's own environment, except that PATH includes what the login shell adds once that is known, plus Acpira's
/// private portable Node.js runtime at lowest priority when installed
impl Env for ProcessEnv {
  fn get(&self, key: &str) -> Option<String> {
    if key == "PATH" {
      return effective_path();
    }
    std::env::var(key).ok()
  }
}

/// Effective PATH for discovery and child processes: inherited PATH, login shell / registry additions, then Acpira's
/// private Node.js bin dir if present (never ahead of a user-installed Node.js)
pub fn effective_path() -> Option<String> {
  let managed = super::node_runtime::managed_bin_dir();
  super::node_runtime::append_managed_bin(super::login_path::effective_path(), managed.as_deref(), Os::current())
}

/// The PATH override to set explicitly on child processes when login_path or the private Node.js runtime adds entries
pub fn spawn_path() -> Option<String> {
  let managed = super::node_runtime::managed_bin_dir();
  let merged = super::login_path::merged();
  match (merged, managed.as_deref()) {
    (Some(p), m) => super::node_runtime::append_managed_bin(Some(p), m, Os::current()),
    (None, Some(m)) => super::node_runtime::append_managed_bin(std::env::var("PATH").ok(), Some(m), Os::current()),
    (None, None) => None,
  }
}

/// The path as resolved for spawn, or None. POSIX: must exist as a file and be executable. Windows: native / batch
/// extensions are required; PATHEXT suffixes are tried in order, each as given and lower-cased.
pub async fn resolve_executable(p: &str, os: Os, env: &dyn Env) -> Option<String> {
  if os == Os::Posix {
    return is_executable(Path::new(p)).await.then(|| p.to_owned());
  }
  let pathext = env.get("PATHEXT").filter(|v| !v.is_empty()).unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
  // npm also writes an extensionless POSIX shell shim beside its .cmd launcher. Passing that shim directly to
  // CreateProcess fails with ERROR_BAD_EXE_FORMAT (193), even though the file exists. File associations for
  // .js / .ps1 are not usable here either: only native executables and the batch files spawn_spec handles count.
  let launchable = |ext: &str| matches!(ext.to_ascii_lowercase().as_str(), ".com" | ".exe" | ".bat" | ".cmd");
  let explicit = Path::new(p).extension().and_then(|e| e.to_str()).is_some_and(|e| launchable(&format!(".{e}")));
  let mut variants = if explicit { vec![p.to_owned()] } else { vec![] };
  for ext in pathext.split(';').filter(|e| launchable(e)) {
    variants.push(format!("{p}{ext}"));
    if ext != ext.to_lowercase() {
      variants.push(format!("{p}{}", ext.to_lowercase()));
    }
  }
  for c in variants {
    if tokio::fs::metadata(&c).await.is_ok_and(|m| m.is_file()) {
      return Some(c);
    }
  }
  None
}

#[cfg(unix)]
async fn is_executable(p: &Path) -> bool {
  use std::os::unix::ffi::OsStrExt;
  let Ok(c) = std::ffi::CString::new(p.as_os_str().as_bytes()) else { return false };
  // access(2) X_OK, as fs.access(p, X_OK) does (a directory with the execute bit passes there too)
  // SAFETY: `c` is a NUL-terminated CString the closure owns for the whole call
  tokio::task::spawn_blocking(move || unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0).await.unwrap_or(false)
}

#[cfg(not(unix))]
async fn is_executable(p: &Path) -> bool {
  tokio::fs::metadata(p).await.is_ok()
}

/// The executable, native argv, refreshed PATH and network proxy used by both ACP sessions and account status probes.
/// The proxy goes in first: a definition's or account's own env set afterwards still wins
pub fn command(binary: &str, args: &[String]) -> tokio::process::Command {
  let mut cmd = native_command::command(binary, args);
  if let Some(path) = spawn_path() {
    cmd.env("PATH", path);
  }
  crate::net_proxy::apply(&mut cmd);
  cmd
}

/// A probe's process group, killed when dropped unless the probe was reaped (cleared).
#[cfg(unix)]
pub(crate) struct ProbeGroup(pub(crate) Option<u32>);

#[cfg(unix)]
impl Drop for ProbeGroup {
  fn drop(&mut self) {
    if let Some(pid) = self.0.and_then(|p| i32::try_from(p).ok()) {
      // SAFETY: the negated pid names the group created by this probe's unreaped child.
      unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
  }
}
