//! Low-level macOS `launchctl` / LaunchAgent primitives shared by machine_engine and devices::service.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

pub fn plist_path(label: &str, user_home: &Path) -> PathBuf {
  user_home.join("Library").join("LaunchAgents").join(format!("{label}.plist"))
}

pub fn xml(s: &str) -> String {
  s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[cfg(unix)]
fn passwd_home() -> Option<PathBuf> {
  // SAFETY: geteuid has no preconditions; getpwuid_r writes into caller-owned buffers and sets `result` on success.
  unsafe {
    let uid = libc::geteuid();
    let mut pwd: libc::passwd = std::mem::zeroed();
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    if libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) != 0 || result.is_null() || pwd.pw_dir.is_null() {
      return None;
    }
    let dir = std::ffi::CStr::from_ptr(pwd.pw_dir).to_str().ok()?.trim();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
  }
}

/// Whether `home` is this OS user's real `~/.acpira` with no test or custom environment overrides.
pub fn is_default_user_home(home: &Path) -> bool {
  #[cfg(unix)]
  {
    if ["ACPIRA_HOME", "ACPIRA_REMOTE_ROOT_MAP", "ACPIRA_REMOTE_MACHINE_ID"].iter().any(|k| std::env::var_os(k).is_some()) {
      return false;
    }
    passwd_home().is_some_and(|u| home == u.join(".acpira"))
  }
  #[cfg(not(unix))]
  {
    let _ = home;
    false
  }
}

pub fn domain() -> String {
  // SAFETY: getuid has no preconditions.
  #[cfg(unix)]
  let uid = unsafe { libc::getuid() };
  #[cfg(not(unix))]
  let uid = 0;
  format!("gui/{uid}")
}

async fn launchctl(args: &[&str]) -> Result<std::process::Output> {
  tokio::process::Command::new("/bin/launchctl").args(args).output().await.context("launchctl")
}

/// Whether the user has an active macOS GUI (`Aqua`) login session (`gui/<uid>`).
pub async fn gui_available() -> bool {
  if !cfg!(target_os = "macos") {
    return false;
  }
  launchctl(&["print", &domain()]).await.is_ok_and(|o| o.status.success())
}

/// Whether a job for `home` can run in the user's macOS GUI (`gui/<uid>`) login session.
pub async fn can_use_gui(home: &Path) -> bool {
  cfg!(target_os = "macos") && is_default_user_home(home) && gui_available().await
}

/// Whether the job has a running process right now.
pub async fn running(label: &str) -> bool {
  launchctl(&["print", &format!("{}/{label}", domain())])
    .await
    .is_ok_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).lines().any(|l| l.trim_start().starts_with("pid = ")))
}

const GONE_WAIT: Duration = Duration::from_secs(25);

/// Whether launchd has the label loaded, running or not.
pub async fn loaded(label: &str) -> bool {
  launchctl(&["print", &format!("{}/{label}", domain())]).await.is_ok_and(|o| o.status.success())
}

/// Stop it, unload it, and wait until launchd has let go of it.
pub async fn unload(label: &str) -> Result<()> {
  bootout(label).await?;
  let deadline = Instant::now() + GONE_WAIT;
  while loaded(label).await {
    if Instant::now() >= deadline {
      bail!("launchd did not let go of {label}");
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
  }
  Ok(())
}

/// Load the plist at `plist` and start it; the label must not be loaded (`unload` first to replace it).
pub async fn load(label: &str, plist: &Path) -> Result<()> {
  let out = launchctl(&["bootstrap", &domain(), &plist.to_string_lossy()]).await?;
  if out.status.success() || loaded(label).await {
    return Ok(());
  }
  let err = String::from_utf8_lossy(&out.stderr);
  let mut msg = format!("launchctl bootstrap: {}", err.lines().next().unwrap_or("").trim());
  if out.status.code() == Some(5) {
    msg.push_str(" (is Acpira switched off under System Settings > General > Login Items?)");
  }
  bail!(msg)
}

/// Stop it and unload it; not loaded is not an error.
pub async fn bootout(label: &str) -> Result<()> {
  let out = launchctl(&["bootout", &format!("{}/{label}", domain())]).await?;
  if !out.status.success() && !matches!(out.status.code(), Some(3) | Some(113)) {
    bail!("launchctl bootout: {}", String::from_utf8_lossy(&out.stderr).trim());
  }
  Ok(())
}

/// Start it; with `kill`, stop a running one first.
pub async fn kickstart(label: &str, kill: bool) -> Result<()> {
  let target = format!("{}/{label}", domain());
  let args: &[&str] = if kill { &["kickstart", "-k", &target] } else { &["kickstart", &target] };
  let out = launchctl(args).await?;
  if !out.status.success() && (kill || out.status.code() != Some(37)) {
    bail!("launchctl kickstart: {}", String::from_utf8_lossy(&out.stderr).trim());
  }
  Ok(())
}

/// Write `text` as the label's plist under `~/Library/LaunchAgents/` and have launchd run it, leaving a running job as is.
pub async fn install(label: &str, user_home: &Path, text: &str) -> Result<()> {
  let path = plist_path(label, user_home);
  if let Some(dir) = path.parent() {
    tokio::fs::create_dir_all(dir).await?;
  }
  let changed = !tokio::fs::read_to_string(&path).await.is_ok_and(|t| t == text);
  if changed {
    acpira_host_base::file_lock::write_atomic(&path, text.as_bytes(), Some(0o644)).await?;
  }
  if !loaded(label).await {
    return load(label, &path).await;
  }
  if running(label).await {
    return Ok(());
  }
  if changed {
    unload(label).await?;
    return load(label, &path).await;
  }
  kickstart(label, false).await
}

/// Remove the label's plist from `~/Library/LaunchAgents/`; unload only if not running.
pub async fn uninstall(label: &str, user_home: &Path) -> Result<()> {
  if !running(label).await {
    unload(label).await?;
  }
  let _ = tokio::fs::remove_file(plist_path(label, user_home)).await;
  Ok(())
}

/// Atomically write and bootstrap/kickstart a locked per-run plist under `<home>/run/<stem>.plist`.
pub async fn start_locked_plist(label: &str, plist_path: &Path, lock_path: &Path, text: &str) -> Result<()> {
  if !gui_available().await {
    bail!("no GUI login session ({})", domain());
  }
  acpira_host_base::file_lock::with_file_lock(lock_path, || async {
    let same = tokio::fs::read_to_string(plist_path).await.is_ok_and(|t| t == text);
    if !same {
      acpira_host_base::file_lock::write_atomic(plist_path, text.as_bytes(), Some(0o600)).await?;
    }
    if loaded(label).await {
      if !same {
        unload(label).await?;
        return load(label, plist_path).await;
      }
      return kickstart(label, false).await;
    }
    load(label, plist_path).await
  })
  .await
}
