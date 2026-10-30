//! macOS: the gateway as a LaunchAgent, so it runs from login on without a window open (the copies it runs: stage.rs).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// The label for this data root: the default `~/.acpira` gets the plain one, any other root (the harness, tests) one
/// of its own, so two roots never replace each other's gateway
pub fn label(root: &Path, user_home: &Path) -> String {
  super::service::label("com.hotic.acpira.gateway", ".", root, user_home)
}

pub fn plist_path(label: &str, user_home: &Path) -> PathBuf {
  user_home.join("Library").join("LaunchAgents").join(format!("{label}.plist"))
}

fn xml(s: &str) -> String {
  s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The LaunchAgent: started at login, restarted when it crashes or exits with an error (not after `acpira-gateway stop`)
pub fn plist(label: &str, gateway: &Path, root: &Path) -> String {
  let log = root.join("devices").join("gateway.log");
  format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{gateway}</string>
    <string>run</string>
    <string>--home</string>
    <string>{root}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
    label = xml(label),
    gateway = xml(&gateway.to_string_lossy()),
    root = xml(&root.to_string_lossy()),
    log = xml(&log.to_string_lossy()),
  )
}

fn domain() -> String {
  // SAFETY: getuid has no preconditions
  #[cfg(unix)]
  let uid = unsafe { libc::getuid() };
  // launchd exists only on macOS; other platforms never reach launchctl, this only keeps them compiling
  #[cfg(not(unix))]
  let uid = 0;
  format!("gui/{uid}")
}

async fn launchctl(args: &[&str]) -> Result<std::process::Output> {
  tokio::process::Command::new("/bin/launchctl").args(args).output().await.context("launchctl")
}

/// How long launchd may take to let go of a job after `bootout`: it waits for the process to exit, up to the job's
/// `ExitTimeOut` (20 s by default) before a SIGKILL
const GONE_WAIT: Duration = Duration::from_secs(25);

/// Whether launchd has the label loaded, running or not
pub async fn loaded(label: &str) -> bool {
  launchctl(&["print", &format!("{}/{label}", domain())]).await.is_ok_and(|o| o.status.success())
}

/// Stop it, unload it, and wait until launchd has let go of it: `bootout` answers while the job is still listed
/// (`state = SIGTERMed`), and a bootstrap in that window fails with `5: Input/output error`
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

/// Load the plist at `plist` and start it; the label must not be loaded (`unload` first to replace it)
pub async fn load(label: &str, plist: &Path) -> Result<()> {
  let out = launchctl(&["bootstrap", &domain(), &plist.to_string_lossy()]).await?;
  // 5 also answers a label already loaded: another engine sharing the data root loaded it meanwhile
  if out.status.success() || loaded(label).await {
    return Ok(());
  }
  let err = String::from_utf8_lossy(&out.stderr);
  let mut msg = format!("launchctl bootstrap: {}", err.lines().next().unwrap_or("").trim());
  if out.status.code() == Some(5) {
    // Neither loaded nor being torn down: launchd answers 5 for a job switched off too
    msg.push_str(" (is acpira-gateway switched off under System Settings > General > Login Items?)");
  }
  bail!(msg)
}

/// Stop it and unload it; not loaded is not an error
pub async fn bootout(label: &str) -> Result<()> {
  let out = launchctl(&["bootout", &format!("{}/{label}", domain())]).await?;
  // 3: no such process; 113: not loaded (`Could not find service`)
  if !out.status.success() && !matches!(out.status.code(), Some(3) | Some(113)) {
    bail!("launchctl bootout: {}", String::from_utf8_lossy(&out.stderr).trim());
  }
  Ok(())
}

/// Start it; with `kill`, stop a running one first
pub async fn kickstart(label: &str, kill: bool) -> Result<()> {
  let target = format!("{}/{label}", domain());
  let args: &[&str] = if kill { &["kickstart", "-k", &target] } else { &["kickstart", &target] };
  let out = launchctl(args).await?;
  if !out.status.success() {
    bail!("launchctl kickstart: {}", String::from_utf8_lossy(&out.stderr).trim());
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names() {
    let home = Path::new("/Users/u");
    assert_eq!(label(Path::new("/Users/u/.acpira"), home), "com.hotic.acpira.gateway");
    assert!(label(Path::new("/Users/u/.acpira/harness"), home).starts_with("com.hotic.acpira.gateway."));
    assert!(plist("l", Path::new("/a&b/acpira-gateway"), Path::new("/r")).contains("<string>/a&amp;b/acpira-gateway</string>"));
  }
}
