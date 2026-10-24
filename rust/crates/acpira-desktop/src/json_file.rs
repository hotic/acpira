//! The app's small state files (window placement, the browser panel's recent pages, its own settings): read leniently,
//! written tmp + rename so a crash mid-write never leaves half a file

use std::path::Path;

use serde_json::Value;

pub fn read_json(file: &Path) -> Option<Value> {
  serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}

/// Pretty JSON with a trailing newline, tmp + rename; the error is the caller's to ignore or report
pub fn write_json_strict(file: &Path, value: &Value) -> std::io::Result<()> {
  if let Some(dir) = file.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let mut text = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
  text.push('\n');
  let tmp = file.with_file_name(format!("{}.{}.tmp", file.file_name().and_then(|n| n.to_str()).unwrap_or("state"), std::process::id()));
  std::fs::write(&tmp, text)?;
  std::fs::rename(&tmp, file)
}

/// Best effort: this state is a convenience, and a read-only profile must not break the app
pub fn write_json(file: &Path, value: &Value) {
  let _ = write_json_strict(file, value);
}
