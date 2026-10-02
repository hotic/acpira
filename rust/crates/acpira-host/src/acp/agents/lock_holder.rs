//! The process that holds a native session lock, when it is an agent another Acpira sidecar started.
//!
//! A VS Code Remote extension host outlives a dropped SSH connection (the server keeps it for the reconnection grace
//! period); a reconnect that lands in a fresh extension host starts a second sidecar whose resume then meets Devin's
//! `session_locked` while the first sidecar's agent still holds the session. The lock names the holder's pid; when that
//! process is a direct child of another `acpira` binary it is one of ours and the user may take the session over.

use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

use crate::acp::session::errors::rpc_of;
#[cfg(unix)]
use crate::acp::transport::process::KILL_GRACE;
use crate::store::file_lock::pid_alive;

static HOLDER_PID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\bPID[:\s]+(\d+)").unwrap());

/// The holder pid a locked restore names: Devin words it `… already open in another process (PID 1411462) …`
pub fn holder_pid(e: &anyhow::Error) -> Option<u32> {
  let data = rpc_of(e).and_then(|r| r.data.as_ref()).map(|d| d.to_string()).unwrap_or_default();
  HOLDER_PID.captures(&format!("{e} {data}")).and_then(|c| c[1].parse().ok()).filter(|p| *p > 1)
}

/// `ps -o ppid=,comm=` for one pid: the parent pid and the executable (a full path on macOS, the 15-char name on Linux)
#[cfg(any(unix, test))]
fn parse_ps(line: &str) -> Option<(u32, String)> {
  let line = line.trim();
  let (ppid, comm) = line.split_once(char::is_whitespace)?;
  Some((ppid.parse().ok()?, comm.trim().to_owned()))
}

fn is_sidecar(comm: &str) -> bool {
  Path::new(comm).file_name().and_then(|n| n.to_str()).is_some_and(|n| {
    if cfg!(windows) {
      n.eq_ignore_ascii_case("acpira.exe") || n.eq_ignore_ascii_case("acpira")
    } else {
      n.trim_end_matches(".exe") == "acpira"
    }
  })
}

#[cfg(unix)]
async fn ps(pid: u32) -> Option<(u32, String)> {
  let out = tokio::time::timeout(
    Duration::from_millis(1500),
    tokio::process::Command::new("ps").args(["-o", "ppid=,comm=", "-p", &pid.to_string()]).output(),
  )
  .await
  .ok()?
  .ok()?;
  if !out.status.success() {
    return None;
  }
  parse_ps(String::from_utf8_lossy(&out.stdout).lines().next()?)
}

#[cfg(windows)]
async fn ps(pid: u32) -> Option<(u32, String)> {
  crate::platform::windows_process::process_info(pid)
}

/// The pid is alive and a direct child of an `acpira` process other than this one (never our own agents)
pub async fn held_by_sibling(pid: u32) -> bool {
  if pid == std::process::id() || !pid_alive(Some(pid.into())) {
    return false;
  }
  let Some((parent, _)) = ps(pid).await else { return false };
  if parent <= 1 || parent == std::process::id() {
    return false;
  }
  ps(parent).await.is_some_and(|(_, comm)| is_sidecar(&comm))
}

/// SIGTERM, SIGKILL after the agent grace period; resolves once the pid is gone or a last second has passed
#[cfg(unix)]
pub async fn terminate(pid: u32) {
  let gone = |limit: Duration| async move {
    let deadline = tokio::time::Instant::now() + limit;
    while tokio::time::Instant::now() < deadline {
      if !pid_alive(Some(pid.into())) {
        return true;
      }
      tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
  };
  // SAFETY: plain kill(2); the caller verified the pid is an agent another acpira sidecar spawned
  unsafe {
    libc::kill(pid as libc::pid_t, libc::SIGTERM);
  }
  if gone(KILL_GRACE).await {
    return;
  }
  // SAFETY: as above
  unsafe {
    libc::kill(pid as libc::pid_t, libc::SIGKILL);
  }
  gone(Duration::from_secs(1)).await;
}

/// Hold the original process handle across the ownership check, so PID reuse cannot redirect the termination.
#[cfg(windows)]
pub async fn terminate(pid: u32) {
  use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
  use windows_sys::Win32::Foundation::WAIT_TIMEOUT;
  use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject};
  // SAFETY: opens only the candidate process; the handle is owned before any await or early return.
  let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, 0, pid) };
  if raw.is_null() {
    return;
  }
  let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
  if !held_by_sibling(pid).await {
    return;
  }
  // SAFETY: the live handle still identifies the checked process, not a later occupant of the same PID.
  if unsafe { TerminateProcess(handle.as_raw_handle(), 1) } == 0 {
    return;
  }
  let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
  while unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } == WAIT_TIMEOUT && tokio::time::Instant::now() < deadline {
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_holder_pid_comes_out_of_the_lock_message() {
    let e = anyhow::anyhow!(
      "Session 'bristle-charger' is already open in another process (PID 1411462). Close the other instance before opening it here."
    );
    assert_eq!(holder_pid(&e), Some(1411462));
    assert_eq!(holder_pid(&anyhow::anyhow!("Session is locked")), None);
    assert_eq!(holder_pid(&anyhow::anyhow!("held by pid: 1")), None);
  }

  #[test]
  fn ps_lines_and_sidecar_names_parse_on_both_platforms() {
    assert_eq!(parse_ps("  8876 /Users/x/.vscode-server/extensions/hotic.acpira-1.7.1-darwin-arm64/bin/acpira\n").map(|p| p.0), Some(8876));
    assert_eq!(parse_ps("7793 /Applications/Visual Studio Code.app/x").map(|p| p.1), Some("/Applications/Visual Studio Code.app/x".into()));
    assert_eq!(parse_ps(""), None);
    assert!(is_sidecar("/Users/x/.vscode-server/extensions/hotic.acpira-1.7.1-darwin-arm64/bin/acpira"));
    assert!(is_sidecar("acpira"));
    assert!(is_sidecar(r"acpira.exe"));
    assert!(!is_sidecar("/Users/x/.local/bin/devin"));
    assert!(!is_sidecar("acpira-helper"));
  }
}
