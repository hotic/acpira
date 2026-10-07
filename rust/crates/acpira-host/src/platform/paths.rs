//! Paths written into another CLI's configuration use its ordinary drive / UNC spelling.
//! Keep verbatim paths for filesystem access; only normalize at the external protocol boundary.

use std::path::{Path, PathBuf};

/// Resolve links with Node's realpathSync spelling, preserving Windows case and 8.3 names.
pub fn canonical_for_cli(path: &Path) -> std::io::Result<PathBuf> {
  #[cfg(not(windows))]
  {
    std::fs::canonicalize(path)
  }
  #[cfg(windows)]
  {
    use std::path::Component;

    let mut path = std::path::absolute(for_cli(path.to_path_buf()))?;
    for _ in 0..40 {
      let mut resolved = PathBuf::new();
      let mut parts = path.components();
      let mut linked = false;
      while let Some(part) = parts.next() {
        resolved.push(part);
        if matches!(part, Component::Prefix(_)) {
          continue;
        }
        if std::fs::symlink_metadata(&resolved)?.file_type().is_symlink() {
          let target = for_cli(std::fs::read_link(&resolved)?);
          let target = if target.is_absolute() { target } else { resolved.parent().unwrap_or(Path::new("")).join(target) };
          path = std::path::absolute(target.join(parts.as_path()))?;
          linked = true;
          break;
        }
      }
      if !linked {
        return Ok(resolved);
      }
    }
    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "too many symbolic links"))
  }
}

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
/// are left alone, since a `/` in them is a literal character. Elsewhere the path is returned unchanged
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

/// `native` for a path held as a string (the shells' home and cwd)
pub fn native_str(path: &str) -> String {
  native(Path::new(path)).to_string_lossy().into_owned()
}

/// A path as it goes over the wire to the webview: native separators, so the pages show one spelling per platform and
/// the webview never has to repair it. Every path a view struct carries is built through this
pub fn wire_path(path: &Path) -> String {
  native(path).to_string_lossy().into_owned()
}

/// A `/`-separated template tail (`.codex/AGENTS.md`) as path components, for joining onto a native base: `join` alone
/// keeps the template's `/` inside the joined path on Windows
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

#[cfg(all(test, windows))]
mod tests {
  use super::*;

  fn node_realpath(path: &Path) -> PathBuf {
    let output = std::process::Command::new("node")
      .args(["-e", "process.stdout.write(require('node:fs').realpathSync(process.argv[1]))"])
      .arg(path)
      .output()
      .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    PathBuf::from(String::from_utf8(output.stdout).unwrap())
  }

  fn short_path(path: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let input: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let len = unsafe { GetShortPathNameW(input.as_ptr(), std::ptr::null_mut(), 0) };
    assert_ne!(len, 0, "{}", std::io::Error::last_os_error());
    let mut output = vec![0u16; len as usize];
    let written = unsafe { GetShortPathNameW(input.as_ptr(), output.as_mut_ptr(), len) };
    assert!(written > 0 && written < len, "{}", std::io::Error::last_os_error());
    PathBuf::from(std::ffi::OsString::from_wide(&output[..written as usize]))
  }

  #[test]
  fn canonical_cli_keys_match_node_drive_and_unc_paths() {
    for (input, expected) in [
      (r"\\?\C:\Users\Spark\work", r"C:\Users\Spark\work"),
      (r"\\?\UNC\server\share\项目", r"\\server\share\项目"),
      (r"C:\Users\Spark\work", r"C:\Users\Spark\work"),
    ] {
      assert_eq!(for_cli(input.into()), PathBuf::from(expected));
    }
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(canonical_for_cli(dir.path()).unwrap(), node_realpath(dir.path()));
  }

  #[test]
  fn cli_realpath_preserves_short_names_and_case_and_resolves_junctions() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("long project directory");
    let target = dir.path().join("junction target");
    std::fs::create_dir_all(root.join("nested")).unwrap();
    std::fs::create_dir_all(target.join("child")).unwrap();
    junction::create(&target, root.join("linked")).unwrap();
    let short = short_path(&root);
    for path in [
      root.clone(),
      short.clone(),
      PathBuf::from(short.to_string_lossy().to_lowercase()),
      short.join("nested").join("..").join("linked").join("child"),
    ] {
      assert_eq!(canonical_for_cli(&path).unwrap(), node_realpath(&path), "{}", path.display());
    }
    assert!(canonical_for_cli(&root.join("missing")).is_err());
  }
}
