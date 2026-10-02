//! Links keep shared directories live without requiring Windows Developer Mode or elevation.

use std::io;
use std::path::Path;

/// A symlink or junction, including dangling entries. Hard links are detected by file identity instead.
pub fn is_link(path: &Path) -> bool {
  if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
    return true;
  }
  #[cfg(windows)]
  return junction::exists(path).unwrap_or(false);
  #[cfg(not(windows))]
  false
}

/// `target` is absolute; `dest` may be relative to the link's parent for relocatable project links.
pub fn create_link(link: &Path, target: &Path, dest: &Path) -> io::Result<()> {
  #[cfg(unix)]
  {
    let _ = target;
    std::os::unix::fs::symlink(dest, link)
  }
  #[cfg(windows)]
  {
    let dir = std::fs::metadata(target)?.is_dir();
    let result = if dir { std::os::windows::fs::symlink_dir(dest, link) } else { std::os::windows::fs::symlink_file(dest, link) };
    if result.is_ok() {
      return result;
    }
    // Native APIs avoid cmd's second round of quoting and keep stdout reserved for sidecar envelopes.
    if dir { junction::create(target, link) } else { std::fs::hard_link(target, link) }
  }
}

/// Remove only the directory entry, never the contents behind a symlink or junction.
pub fn remove_link(link: &Path) -> io::Result<()> {
  let meta = std::fs::symlink_metadata(link)?;
  #[cfg(windows)]
  {
    use std::os::windows::fs::FileTypeExt;
    if meta.is_dir() || meta.file_type().is_symlink_dir() {
      return std::fs::remove_dir(link);
    }
  }
  if meta.is_dir() { std::fs::remove_dir(link) } else { std::fs::remove_file(link) }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn removing_a_directory_link_preserves_the_target() {
    let t = tempfile::tempdir().unwrap();
    let target = t.path().join("target 用户 & space");
    let link = t.path().join("link O'Brien");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("kept"), "x").unwrap();
    create_link(&link, &target, &target).unwrap();
    assert!(is_link(&link));
    assert!(same_file::is_same_file(&link, &target).unwrap());
    remove_link(&link).unwrap();
    assert!(target.join("kept").exists());
    assert!(std::fs::symlink_metadata(&link).is_err());
  }

  #[cfg(windows)]
  #[test]
  fn junctions_need_no_shell_or_symlink_privilege_and_dangling_ones_can_be_removed() {
    let t = tempfile::tempdir().unwrap();
    let target = t.path().join("target %PATH% & 用户");
    let link = t.path().join("junction %PATH% & 用户");
    std::fs::create_dir(&target).unwrap();
    junction::create(&target, &link).unwrap();
    assert!(is_link(&link));
    std::fs::remove_dir(&target).unwrap();
    assert!(is_link(&link));
    remove_link(&link).unwrap();
    assert!(std::fs::symlink_metadata(&link).is_err());
  }
}
