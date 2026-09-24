//! Data root (mirror of the Rust-relevant part of src/host/store/dataDir.ts). Legacy globalStorage migration stays in
//! the VS Code shell, which alone can read the IDE's SecretStorage

use std::path::{Path, PathBuf};

/// ACPIRA_HOME when set, otherwise ~/.acpira
pub fn acpira_home() -> PathBuf {
  match std::env::var("ACPIRA_HOME").ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()) {
    Some(o) => absolute(Path::new(&o)),
    None => home_dir().join(".acpira"),
  }
}

pub fn home_dir() -> PathBuf {
  #[cfg(windows)]
  let v = std::env::var_os("USERPROFILE");
  #[cfg(not(windows))]
  let v = std::env::var_os("HOME");
  v.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

/// path.resolve: absolute against the current directory, `.` / `..` folded
pub fn absolute(p: &Path) -> PathBuf {
  let joined = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(p) };
  normalize(&joined)
}

pub fn normalize(p: &Path) -> PathBuf {
  use std::path::Component;
  let mut out = PathBuf::new();
  for c in p.components() {
    match c {
      Component::ParentDir => {
        out.pop();
      }
      Component::CurDir => {}
      other => out.push(other.as_os_str()),
    }
  }
  out
}
