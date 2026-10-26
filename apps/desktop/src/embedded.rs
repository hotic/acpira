//! The page bundle and the picker script built into the Windows exe (build.rs fills the tables; elsewhere they are empty
//! and the page comes from the bundle's resources or the checkout's dist/)

include!(concat!(env!("OUT_DIR"), "/embedded_page.rs"));

/// Whether this exe carries the page
pub fn has_page() -> bool {
  !WEBVIEW.is_empty()
}

/// A bundle file by its path under webview/: its bytes and whether they are zstd-compressed
pub fn file(rel: &str) -> Option<(&'static [u8], bool)> {
  WEBVIEW.iter().find(|(p, _, _)| *p == rel).map(|(_, bytes, packed)| (*bytes, *packed))
}

/// The engine reads the picker from a file: the built-in copy is written under `dir` (once per content) and its path
/// returned; None when this exe carries no picker
pub fn picker_file(dir: &std::path::Path) -> Option<std::path::PathBuf> {
  let bytes = PICKER?;
  let path = dir.join("picker.js");
  if std::fs::read(&path).ok().as_deref() == Some(bytes) {
    return Some(path);
  }
  std::fs::create_dir_all(dir).ok()?;
  let tmp = dir.join(format!("picker.js.{}.tmp", std::process::id()));
  std::fs::write(&tmp, bytes).ok()?;
  std::fs::rename(&tmp, &path).ok()?;
  Some(path)
}
