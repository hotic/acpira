//! Safe mode from the menu (docs/dev/plugins.md): every engine on the machine unloads the user's plugins. The switch goes
//! through the engine binary's `acpira safe-mode on|off`, which writes `<ACPIRA_HOME>/plugins/state.json`, so it works
//! while the engine itself is stuck or down; running engines follow within a second

use std::path::Path;
use std::process::Command;

/// Whether safe mode is on, from the plugin state (false when there is none or it cannot be read)
pub fn is_on(home: &Path) -> bool {
  std::fs::read(home.join("plugins").join("state.json"))
    .ok()
    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
    .is_some_and(|v| v["safeMode"] == true)
}

/// Switch safe mode with the engine binary; the error is its stderr. Blocks until the command ends
pub fn set(binary: &Path, home: &Path, on: bool) -> Result<(), String> {
  let mut cmd = Command::new(binary);
  cmd.args(["safe-mode", if on { "on" } else { "off" }, "--home"]).arg(home);
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(crate::process::CREATE_NO_WINDOW);
  }
  let out = cmd.output().map_err(|e| format!("{}: {e}", binary.display()))?;
  if out.status.success() {
    return Ok(());
  }
  let err = String::from_utf8_lossy(&out.stderr).trim().to_owned();
  Err(if err.is_empty() { format!("{} safe-mode: {}", binary.display(), out.status) } else { err })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_the_plugin_state() {
    let Ok(dir) = tempfile::tempdir() else { return };
    assert!(!is_on(dir.path()));
    let _ = std::fs::create_dir_all(dir.path().join("plugins"));
    let _ = std::fs::write(dir.path().join("plugins/state.json"), r#"{"generation":3,"safeMode":true}"#);
    assert!(is_on(dir.path()));
    let _ = std::fs::write(dir.path().join("plugins/state.json"), "not json");
    assert!(!is_on(dir.path()));
  }
}
