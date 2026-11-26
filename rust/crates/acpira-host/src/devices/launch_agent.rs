//! macOS: remote access's LaunchAgent, which starts the machine's engine (and with it the gateway) at login without a
//! window open, from the service's copy (stage.rs). Generic launchd primitives live in `platform::launchd`.

use std::path::Path;

use crate::platform::launchd::xml;
pub use crate::platform::launchd::{install, kickstart, load, loaded, plist_path, uninstall};

/// The label for this data root: the default `~/.acpira` gets the plain one, any other root (the harness, tests) one
/// of its own, so two roots never replace each other's. The name is from when it ran a standalone gateway
pub fn label(root: &Path, user_home: &Path) -> String {
  super::service::label("com.hotic.acpira.gateway", ".", root, user_home)
}

/// The login job's program: the service copy itself, under `supervise` when engines start under a launcher, which
/// it then picks each time (`machine_engine/launcher.rs`)
fn supervised(launcher: Option<&Path>, exe: &Path) -> String {
  let mut parts = vec![format!("<string>{}</string>", xml(&exe.to_string_lossy()))];
  if launcher.is_some() {
    parts.push(format!("<string>{}</string>", crate::machine_engine::launcher::SUPERVISE));
  }
  parts.join("\n    ")
}

/// The LaunchAgent: started at login, restarted when it crashes or exits with an error.
pub fn plist(label: &str, launcher: Option<&Path>, exe: &Path, root: &Path) -> String {
  let log = super::service::log_path(root);
  let args: String = super::service::args(root).iter().map(|a| format!("\n    <string>{}</string>", xml(a))).collect();
  format!(
    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>{associated}
  <key>ProgramArguments</key>
  <array>
    {program}{args}
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
    program = supervised(launcher, exe),
    associated = crate::machine_engine::launchd::associated(launcher),
    log = xml(&log.to_string_lossy()),
  )
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn names() {
    let home = Path::new("/Users/u");
    assert_eq!(label(Path::new("/Users/u/.acpira"), home), "com.hotic.acpira.gateway");
    assert!(label(Path::new("/Users/u/.acpira/harness"), home).starts_with("com.hotic.acpira.gateway."));
    let p = plist("l", None, Path::new("/a&b/acpira"), Path::new("/r"));
    assert!(p.contains("<string>/a&amp;b/acpira</string>\n    <string>serve</string>"), "{p}");
    let log = super::super::service::log_path(Path::new("/r"));
    assert!(p.contains("<string>--service</string>") && p.contains(&format!("<string>{}</string>", log.display())), "{p}");
    let launched = plist("l", Some(Path::new("/r/engine/launcher/Acpira Engine")), Path::new("/s/acpira"), Path::new("/r"));
    assert!(launched.contains("<string>/s/acpira</string>\n    <string>supervise</string>\n    <string>serve</string>"), "{launched}");
    assert!(!launched.contains("AssociatedBundleIdentifiers"), "{launched}");
    let from_app = plist("l", Some(Path::new("/Applications/Acpira.app/Contents/MacOS/acpira-engine")), Path::new("/s/acpira"), Path::new("/r"));
    assert!(from_app.contains("<key>AssociatedBundleIdentifiers</key>\n  <array>\n    <string>cc.hotic.acpira.desktop</string>"), "{from_app}");
  }
}
