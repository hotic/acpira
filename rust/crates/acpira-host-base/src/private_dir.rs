//! User-private socket/state directory creation and permission checks.

use std::path::Path;

/// Create `dir` (recursively) and verify on Unix that it is a real directory owned by the current user,
/// tightening mode to `0700` if group/other bits are set.
pub fn private_dir_io(dir: &Path) -> std::io::Result<()> {
  std::fs::create_dir_all(dir)?;
  #[cfg(unix)]
  {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let st = std::fs::symlink_metadata(dir)?;
    // SAFETY: geteuid has no preconditions and cannot fail.
    let me = unsafe { libc::geteuid() };
    if !st.is_dir() || st.uid() != me {
      return Err(std::io::Error::other(format!("{} must be a directory owned by this user", dir.display())));
    }
    if st.mode() & 0o077 != 0 {
      std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
  }
  Ok(())
}

/// Unix strict variant: creates missing directories directly with mode `0700` (no open window) and returns `String` errors.
#[cfg(unix)]
pub fn private_dir_strict(dir: &Path) -> Result<(), String> {
  use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
  std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
  let meta = std::fs::symlink_metadata(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
  // SAFETY: geteuid has no preconditions and cannot fail.
  let me = unsafe { libc::geteuid() };
  if !meta.is_dir() || meta.uid() != me {
    return Err(format!("{} must be a directory owned by this user", dir.display()));
  }
  if meta.mode() & 0o077 != 0 {
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| format!("{}: {e}", dir.display()))?;
  }
  Ok(())
}
