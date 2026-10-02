//! Agent executable resolution and launch policy. Platform-specific argv construction lives in platform::command.

use std::path::Path;

use crate::platform::command::{self as native_command, Os};

pub trait Env: Sync {
  fn get(&self, key: &str) -> Option<String>;
}

pub struct ProcessEnv;

/// The sidecar's own environment, except that PATH includes what the login shell adds once that is known
impl Env for ProcessEnv {
  fn get(&self, key: &str) -> Option<String> {
    if key == "PATH" {
      return super::login_path::effective_path();
    }
    std::env::var(key).ok()
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
  tokio::task::spawn_blocking(move || unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0).await.unwrap_or(false)
}

#[cfg(not(unix))]
async fn is_executable(p: &Path) -> bool {
  tokio::fs::metadata(p).await.is_ok()
}

/// The executable, native argv and refreshed PATH used by both ACP sessions and account status probes.
pub fn command(binary: &str, args: &[String]) -> tokio::process::Command {
  let mut cmd = native_command::command(binary, args);
  if let Some(path) = super::login_path::merged() {
    cmd.env("PATH", path);
  }
  cmd
}
