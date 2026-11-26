//! macOS `gui/<uid>` launchd jobs for engines started over SSH (`machine_engine/ensure.rs`).

use std::path::Path;

use anyhow::{Result, bail};

use crate::platform::launchd::{self, xml};

const ENGINE_LABEL: &str = "com.hotic.acpira.engine";

/// The LaunchAgent label of the peer engine for `version` (`remote/helper/streams.rs`).
pub fn peer_label(version: &str) -> String {
  let clean: String = version.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '-' }).collect();
  format!("com.hotic.acpira.peer.{clean}")
}

/// What a job runs: `exe` itself, or the launcher starting it (`machine_engine/launcher.rs`).
fn program(launcher: Option<&Path>, exe: &Path) -> String {
  let mut parts = vec![];
  if let Some(l) = launcher {
    parts.push(xml(&l.to_string_lossy()));
    parts.push(super::launcher::COMMAND.to_owned());
  }
  parts.push(xml(&exe.to_string_lossy()));
  parts.iter().map(|p| format!("<string>{p}</string>")).collect::<Vec<_>>().join("\n    ")
}

/// A job started from the desktop app's bundle names the app, so Login Items lists it under Acpira.app.
pub(crate) fn associated(launcher: Option<&Path>) -> String {
  if !launcher.is_some_and(super::launcher::in_app) {
    return String::new();
  }
  format!(
    "\n  <key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>{}</string>\n  </array>",
    super::launcher::APP_ID
  )
}

/// The LaunchAgent plist for a peer/SSH-started engine on macOS: runs in `Aqua` (`gui/<uid>`) without `KeepAlive`.
pub fn peer_plist(label: &str, launcher: Option<&Path>, exe: &Path, socket: &Path, home: &Path, log: &Path) -> String {
  format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>{associated}
  <key>LimitLoadToSessionType</key>
  <string>Aqua</string>
  <key>ProgramArguments</key>
  <array>
    {program}
    <string>serve</string>
    <string>--socket</string>
    <string>{socket}</string>
    <string>--home</string>
    <string>{home}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
    label = xml(label),
    program = program(launcher, exe),
    associated = associated(launcher),
    socket = xml(&socket.to_string_lossy()),
    home = xml(&home.to_string_lossy()),
    log = xml(&log.to_string_lossy()),
  )
}

/// Start (or kickstart after an idle exit) the peer engine for `version` in `gui/<uid>`.
pub async fn start_peer(version: &str, exe: &Path, socket: &Path, home: &Path, log: &Path) -> Result<()> {
  start_serve(&peer_label(version), &format!("peer-{version}"), exe, socket, home, log).await
}

/// Start the machine's engine in `gui/<uid>` for a specific landed build, cleaning up old non-running build jobs.
pub async fn start_machine_engine(build: &str, exe: &Path, socket: &Path, home: &Path, log: &Path) -> Result<String> {
  let label = format!("{ENGINE_LABEL}.{build}");
  let stem = format!("engine-{build}");
  let run = home.join("run");
  if let Ok(entries) = std::fs::read_dir(&run) {
    for e in entries.flatten() {
      let name = e.file_name().to_string_lossy().into_owned();
      let Some(other) = name.strip_prefix("engine-").and_then(|n| n.strip_suffix(".plist")) else { continue };
      if other == build {
        continue;
      }
      let other_label = format!("{ENGINE_LABEL}.{other}");
      if !launchd::running(&other_label).await {
        let _ = launchd::bootout(&other_label).await;
        let _ = std::fs::remove_file(e.path());
      }
    }
  }
  start_serve(&label, &stem, exe, socket, home, log).await?;
  Ok(label)
}

async fn start_serve(label: &str, stem: &str, exe: &Path, socket: &Path, home: &Path, log: &Path) -> Result<()> {
  for p in [exe, socket, home, log] {
    if p.to_string_lossy().bytes().any(|b| b < 0x20) {
      bail!("path contains control characters: {}", p.display());
    }
  }
  let run = home.join("run");
  let plist_path = run.join(format!("{stem}.plist"));
  let lock_path = run.join(format!("{stem}.plist.lock"));
  let launcher = super::launcher::ensure(home, exe);
  let text = peer_plist(label, launcher.as_deref(), exe, socket, home, log);
  launchd::start_locked_plist(label, &plist_path, &lock_path, &text).await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn peer_names_and_plist() {
    assert_eq!(peer_label("2.0.5"), "com.hotic.acpira.peer.2.0.5");
    let peer = peer_plist("com.hotic.acpira.peer.2.0.5", None, Path::new("/a'b/acpira"), Path::new("/r/peer.sock"), Path::new("/r"), Path::new("/r/peer.log"));
    assert!(peer.contains("<string>Aqua</string>") && peer.contains("/a&apos;b/acpira") && !peer.contains("KeepAlive"));
    assert!(!launchd::is_default_user_home(Path::new("/tmp/not-real-user-home/.acpira")));
  }
}
