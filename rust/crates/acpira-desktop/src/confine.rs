//! A file under `root` named by a URL path (already split off its route prefix), or None when the name is not valid, does
//! not exist, or leaves the root once `..`, percent-escapes and symlinks are resolved. Every file the app protocol serves
//! (the webview bundle, attachment blobs) goes through here

use std::path::{Path, PathBuf};

/// `%XX` escapes decoded; None for a broken escape or bytes that are not UTF-8 (decodeURIComponent throws on both)
pub fn percent_decode(s: &str) -> Option<String> {
  let b = s.as_bytes();
  let mut out = Vec::with_capacity(b.len());
  let mut i = 0;
  while i < b.len() {
    if b[i] == b'%' {
      let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
      out.push(u8::from_str_radix(hex, 16).ok()?);
      i += 3;
    } else {
      out.push(b[i]);
      i += 1;
    }
  }
  String::from_utf8(out).ok()
}

pub fn confine(root: &Path, url_path: &str) -> Option<PathBuf> {
  let name = percent_decode(url_path)?;
  if name.is_empty() || name.contains('\0') {
    return None;
  }
  let base = std::fs::canonicalize(root).ok()?;
  // Joined as a relative name: a leading slash or a drive must not replace the root
  let rel = name.trim_start_matches(['/', '\\']);
  if rel.is_empty() {
    return None;
  }
  let real = std::fs::canonicalize(base.join(rel)).ok()?;
  (real != base && real.starts_with(&base)).then_some(real)
}

#[cfg(test)]
mod tests {
  use super::*;

  // Ported from the Electron app's confine.test.ts
  fn setup() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(tmp.path()).unwrap();
    let root = base.join("root");
    std::fs::create_dir_all(root.join("assets")).unwrap();
    std::fs::write(root.join("main.js"), "").unwrap();
    std::fs::write(root.join("assets/a b.css"), "").unwrap();
    std::fs::write(base.join("secret.txt"), "").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(base.join("secret.txt"), root.join("escape.txt")).unwrap();
    (tmp, root)
  }

  #[test]
  fn serves_files_under_the_root_percent_escapes_decoded() {
    let (_t, root) = setup();
    assert_eq!(confine(&root, "main.js"), Some(root.join("main.js")));
    assert_eq!(confine(&root, "assets/a%20b.css"), Some(root.join("assets").join("a b.css")));
  }

  #[test]
  fn refuses_parent_segments_symlinks_out_the_root_missing_files_and_bad_escapes() {
    let (_t, root) = setup();
    for p in ["../secret.txt", "%2e%2e/secret.txt", "assets/../../secret.txt", "escape.txt", "", ".", "missing.js", "%E0%A4%A", "a%00b"] {
      assert_eq!(confine(&root, p), None, "{p}");
    }
  }
}
