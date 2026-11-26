//! Platform path spelling primitives shared by host crates.
//! Keep verbatim paths for filesystem access; normalize only at external protocol boundaries.

use std::path::{Path, PathBuf};

/// Strip Windows verbatim prefixes (`\\?\` / `\\?\UNC\`) to ordinary drive or UNC spelling.
pub fn for_cli(path: PathBuf) -> PathBuf {
  #[cfg(windows)]
  {
    use std::ffi::OsString;
    use std::path::{Component, Prefix};
    let mut parts = path.components();
    let Some(Component::Prefix(prefix)) = parts.next() else { return path };
    let mut ordinary = match prefix.kind() {
      Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
      Prefix::VerbatimUNC(server, share) => {
        let mut root = OsString::from(r"\\");
        root.push(server);
        root.push(r"\");
        root.push(share);
        root.push(r"\");
        PathBuf::from(root)
      }
      _ => return path,
    };
    if parts.next() != Some(Component::RootDir) {
      return path;
    }
    ordinary.push(parts.as_path());
    ordinary
  }
  #[cfg(not(windows))]
  path
}

/// A path in the platform's own separators: on Windows every `/` becomes `\`, so a home given as `S:/x` or a
/// template tail like `.codex/AGENTS.md` cannot leave a mixed `S:/x\.codex/AGENTS.md` behind. Verbatim paths (`\\?\`)
/// are left alone, since a `/` in them is a literal character. Elsewhere the path is returned unchanged.
pub fn native(path: &Path) -> PathBuf {
  #[cfg(windows)]
  {
    let s = path.to_string_lossy();
    if s.starts_with(r"\\?\") || !s.contains('/') {
      return path.to_path_buf();
    }
    PathBuf::from(s.replace('/', "\\"))
  }
  #[cfg(not(windows))]
  path.to_path_buf()
}

/// `native` for a path held as a string (the shells' home and cwd).
pub fn native_str(path: &str) -> String {
  native(Path::new(path)).to_string_lossy().into_owned()
}

/// A path as it goes over the wire to the webview: native separators, so the pages show one spelling per platform and
/// the webview never has to repair it. Every path a view struct carries is built through this.
pub fn wire_path(path: &Path) -> String {
  native(path).to_string_lossy().into_owned()
}

/// A `/`-separated template tail (`.codex/AGENTS.md`) as path components, for joining onto a native base: `join` alone
/// keeps the template's `/` inside the joined path on Windows.
pub fn native_tail(rest: &str) -> PathBuf {
  rest.split('/').filter(|c| !c.is_empty()).collect()
}

#[cfg(test)]
mod spelling_tests {
  use super::*;

  #[test]
  fn native_spelling_joins_template_tails_with_the_platform_separator() {
    let sep = std::path::MAIN_SEPARATOR;
    assert_eq!(wire_path(&Path::new("h").join(native_tail(".codex/AGENTS.md"))), format!("h{sep}.codex{sep}AGENTS.md"));
    assert_eq!(native_tail("a//b/").components().count(), 2);
    #[cfg(windows)]
    {
      assert_eq!(wire_path(Path::new(r"C:\Users\me\.codex/AGENTS.md")), r"C:\Users\me\.codex\AGENTS.md");
      assert_eq!(native_str("S:/tmp/home"), r"S:\tmp\home");
      assert_eq!(wire_path(Path::new(r"\\?\C:\a/b")), r"\\?\C:\a/b");
    }
    #[cfg(not(windows))]
    assert_eq!(wire_path(Path::new("/a\\b/c")), "/a\\b/c");
  }
}
