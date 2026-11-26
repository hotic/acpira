//! Engine staging directory layout, version classification and SSH session environment check.

use std::path::{Path, PathBuf};

use semver::Version;

/// Under ACPIRA_HOME
pub const DIR: &str = "engine";
/// The path the copy was landed from: a checkout's build finds other platforms' helpers through it.
pub const SOURCE: &str = ".source";
/// The landed binary's name, whatever the source was called.
pub const EXE: &str = if cfg!(windows) { "acpira.exe" } else { "acpira" };
/// Digest characters in a folder name.
pub const DIGEST_LEN: usize = 16;

/// Where a landed build came from, when recorded in `.source`.
pub fn source_of(dir: &Path) -> Option<PathBuf> {
  let bytes = std::fs::read(dir.join(SOURCE)).ok()?;
  #[cfg(unix)]
  {
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
  }
  #[cfg(not(unix))]
  {
    String::from_utf8(bytes).ok().map(PathBuf::from)
  }
}

/// A local package (`X.Y.Z-dev.<stamp>`): never published, so nothing can be downloaded for it.
pub fn is_local_package(version: &str) -> bool {
  Version::parse(version).is_ok_and(|v| v.pre.as_str().starts_with("dev."))
}

/// Whether the current process was started inside an SSH login session.
pub fn over_ssh() -> bool {
  ["SSH_CONNECTION", "SSH_CLIENT"].iter().any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
}
