//! The engine binary the app starts: ACPIRA_SIDECAR_BIN when set, else the one the app bundle carries, else (a repository
//! checkout) the one `pnpm build:sidecar` or `cargo build` left behind. Mirror of packages/engine-client/sidecarLocator.ts

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One way to start the engine binary and a label for the log
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarCommand {
  pub command: PathBuf,
  pub args: Vec<String>,
  pub env: BTreeMap<String, String>,
  pub label: String,
}

impl SidecarCommand {
  pub fn binary(path: impl Into<PathBuf>) -> SidecarCommand {
    let command = path.into();
    SidecarCommand { label: format!("binary {}", command.display()), command, args: vec![], env: BTreeMap::new() }
  }
}

pub struct LocateOpts<'a> {
  /// Candidate binaries in preference order (the bundle's `Resources/bin/acpira`, then the checkout's builds)
  pub candidates: &'a [PathBuf],
  /// ACPIRA_SIDECAR_BIN
  pub override_bin: Option<&'a str>,
}

/// Empty when there is none: the app ships its engine, with no other one to fall back to
pub fn sidecar_commands(o: &LocateOpts) -> Vec<SidecarCommand> {
  if let Some(bin) = o.override_bin.map(str::trim).filter(|s| !s.is_empty()) {
    return vec![SidecarCommand::binary(bin)];
  }
  o.candidates.iter().find(|p| usable(p)).map(|p| vec![SidecarCommand::binary(p)]).unwrap_or_default()
}

/// scripts/release/sidecar-targets.mjs naming for this build: dist/sidecar/<dir>/acpira
pub fn platform_dir() -> Option<&'static str> {
  match (std::env::consts::OS, std::env::consts::ARCH) {
    ("macos", "aarch64") => Some("mac-arm64"),
    ("macos", "x86_64") => Some("mac-x86_64"),
    ("linux", "aarch64") => Some("linux-arm64"),
    ("linux", "x86_64") => Some("linux-x86_64"),
    ("windows", "aarch64") => Some("windows-arm64"),
    ("windows", "x86_64") => Some("windows-x86_64"),
    _ => None,
  }
}

pub const EXE: &str = if cfg!(windows) { "acpira.exe" } else { "acpira" };

/// Archives do not always keep the executable bit; restore it once rather than fail to start
fn usable(path: &Path) -> bool {
  let Ok(meta) = std::fs::metadata(path) else { return false };
  if !meta.is_file() {
    return false;
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 != 0 {
      return true;
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).is_ok()
  }
  #[cfg(not(unix))]
  true
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_override_wins_then_the_first_usable_candidate() {
    let d = tempfile::tempdir().unwrap();
    let a = d.path().join("a");
    let b = d.path().join("b");
    std::fs::write(&b, "").unwrap();
    let cands = [a.clone(), b.clone()];
    assert_eq!(
      sidecar_commands(&LocateOpts { candidates: &cands, override_bin: Some(" /x/acpira ") })[0].command,
      PathBuf::from("/x/acpira")
    );
    let found = sidecar_commands(&LocateOpts { candidates: &cands, override_bin: Some("") });
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].command, b);
    // The executable bit came back
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_ne!(std::fs::metadata(&b).unwrap().permissions().mode() & 0o111, 0);
    }
    assert!(sidecar_commands(&LocateOpts { candidates: &[a], override_bin: None }).is_empty());
  }
}
