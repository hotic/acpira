//! Putting a staged bundle in the installed one's place, and the installs that cannot be updated in place. The swap is
//! `renamex_np(RENAME_SWAP)`: one atomic exchange of the two folders, after which the old bundle sits at the staged path
//! and is deleted. A running shell keeps its open files; the persistent engine keeps running its turns (its socket key
//! is the engine binary's path, size and mtime, so the new engine gets a new socket; docs/dev/desktop.md "Updates")

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Why this install only reports a new version and links to the release page instead of updating itself
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ManualReason {
  /// Not running from an app bundle (a development binary)
  NotBundled,
  /// Gatekeeper's App Translocation runs the app from a randomized read-only path
  Translocated,
  /// On a read-only volume (straight from the dmg)
  ReadOnly,
  /// The bundle or its folder is not writable by this user (another user's /Applications)
  NotWritable,
  /// Not signed with the release Developer ID (an ad hoc development build)
  Unsigned,
  /// This build carries no update key or Team ID
  Unconfigured,
  /// The system is older than the new version's minimum
  System,
}

/// The `.app` folder an executable runs from (`X.app/Contents/MacOS/exe`)
pub fn bundle_of(exe: &Path) -> Option<PathBuf> {
  let macos = exe.parent()?;
  let contents = macos.parent()?;
  let app = contents.parent()?;
  let named = |p: &Path, n: &str| p.file_name().is_some_and(|f| f == n);
  (named(macos, "MacOS") && named(contents, "Contents") && app.extension().is_some_and(|e| e == "app")).then(|| app.to_path_buf())
}

/// What keeps the bundle at `app` from being replaced in place, if anything
pub fn location_blocker(app: &Path) -> Option<ManualReason> {
  if app.to_string_lossy().contains("/AppTranslocation/") {
    return Some(ManualReason::Translocated);
  }
  if read_only_volume(app) {
    return Some(ManualReason::ReadOnly);
  }
  // The swap renames both the bundle (which moves to another folder) and an entry of its parent
  let parent = app.parent().unwrap_or(Path::new("/"));
  if !writable(app) || !writable(parent) {
    return Some(ManualReason::NotWritable);
  }
  None
}

#[cfg(unix)]
fn c_path(p: &Path) -> Option<std::ffi::CString> {
  use std::os::unix::ffi::OsStrExt;
  std::ffi::CString::new(p.as_os_str().as_bytes()).ok()
}

#[cfg(unix)]
fn writable(p: &Path) -> bool {
  // SAFETY: a valid NUL-terminated path for the duration of the call
  c_path(p).is_some_and(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) } == 0)
}

#[cfg(not(unix))]
fn writable(p: &Path) -> bool {
  std::fs::metadata(p).is_ok_and(|m| !m.permissions().readonly())
}

#[cfg(target_os = "macos")]
fn read_only_volume(p: &Path) -> bool {
  let Some(c) = c_path(p) else { return false };
  // SAFETY: statfs fills the zeroed struct for a valid path
  let mut s: libc::statfs = unsafe { std::mem::zeroed() };
  unsafe { libc::statfs(c.as_ptr(), &mut s) == 0 && s.f_flags & libc::MNT_RDONLY as u32 != 0 }
}

#[cfg(not(target_os = "macos"))]
fn read_only_volume(_p: &Path) -> bool {
  false
}

/// Exchanges the two folders in one step; afterwards `staged` holds the old bundle
pub fn swap(staged: &Path, installed: &Path) -> std::io::Result<()> {
  #[cfg(target_os = "macos")]
  {
    let (Some(a), Some(b)) = (c_path(staged), c_path(installed)) else {
      return Err(std::io::Error::other("path with a NUL byte"));
    };
    // SAFETY: valid NUL-terminated paths for the duration of the call
    if unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) } != 0 {
      return Err(std::io::Error::last_os_error());
    }
    Ok(())
  }
  #[cfg(not(target_os = "macos"))]
  {
    // No atomic exchange here: three renames, each undone if the next fails
    let aside = installed.with_extension("swap-old");
    std::fs::rename(installed, &aside)?;
    if let Err(e) = std::fs::rename(staged, installed) {
      let _ = std::fs::rename(&aside, installed);
      return Err(e);
    }
    std::fs::rename(&aside, staged)
  }
}

/// Starts the bundle at `app` once process `pid` (this shell) has exited: the single-instance lock would otherwise
/// hand the new launch's arguments to the old process and end it. `direct` runs the bundle's executable with this
/// environment instead of `open` (LaunchServices starts an app with its own environment; the smoke run needs its temp
/// home carried over)
pub fn relaunch_after_exit(pid: u32, app: &Path, direct: bool) -> std::io::Result<()> {
  let start = if direct { r#"exec "$1/Contents/MacOS/Acpira""# } else { r#"exec /usr/bin/open "$1""# };
  let script = format!(r#"while kill -0 "$0" 2>/dev/null; do sleep 0.1; done; {start}"#);
  let mut cmd = std::process::Command::new("/bin/sh");
  cmd.arg("-c").arg(script).arg(pid.to_string()).arg(app);
  cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
  crate::process::detach(&mut cmd);
  // Not reaped: this process exits right after, and the helper outlives it
  cmd.spawn().map(drop)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn the_bundle_is_found_from_its_executable() {
    assert_eq!(bundle_of(Path::new("/Applications/Acpira.app/Contents/MacOS/Acpira")), Some(PathBuf::from("/Applications/Acpira.app")));
    assert_eq!(bundle_of(Path::new("/repo/apps/desktop/target/debug/acpira-desktop-tauri")), None);
    assert_eq!(bundle_of(Path::new("/x/Acpira/Contents/MacOS/Acpira")), None);
  }

  #[test]
  fn translocated_and_unwritable_installs_are_manual() {
    assert_eq!(location_blocker(Path::new("/private/var/folders/x/AppTranslocation/ABC/d/Acpira.app")), Some(ManualReason::Translocated));
    let d = tempfile::tempdir().unwrap();
    let app = d.path().join("Acpira.app");
    std::fs::create_dir_all(&app).unwrap();
    assert_eq!(location_blocker(&app), None);
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      // Root ignores permission bits; the check is meaningless there
      if unsafe { libc::geteuid() } != 0 {
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
        assert_eq!(location_blocker(&app), Some(ManualReason::NotWritable));
        std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
      }
    }
  }

  #[cfg(target_os = "macos")]
  #[test]
  fn the_root_volume_reads_as_read_only() {
    // macOS keeps the system volume sealed and mounted read-only
    assert!(read_only_volume(Path::new("/System")));
    assert!(!read_only_volume(&std::env::temp_dir()));
  }

  #[test]
  fn a_swap_exchanges_the_two_bundles() {
    let d = tempfile::tempdir().unwrap();
    let (installed, staged) = (d.path().join("Applications/Acpira.app"), d.path().join("cache/1.1/Acpira.app"));
    for (dir, v) in [(&installed, "old"), (&staged, "new")] {
      std::fs::create_dir_all(dir.join("Contents")).unwrap();
      std::fs::write(dir.join("Contents/version"), v).unwrap();
    }
    swap(&staged, &installed).unwrap();
    assert_eq!(std::fs::read_to_string(installed.join("Contents/version")).unwrap(), "new");
    assert_eq!(std::fs::read_to_string(staged.join("Contents/version")).unwrap(), "old");
    // Nothing to swap with: an error, both sides untouched
    std::fs::remove_dir_all(&staged).unwrap();
    assert!(swap(&staged, &installed).is_err());
    assert_eq!(std::fs::read_to_string(installed.join("Contents/version")).unwrap(), "new");
  }
}
