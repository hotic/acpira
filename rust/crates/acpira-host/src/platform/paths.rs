//! Paths written into another CLI's configuration use its ordinary drive / UNC spelling.
//! Keep verbatim paths for filesystem access; only normalize at the external protocol boundary.

use std::path::PathBuf;

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

#[cfg(all(test, windows))]
mod tests {
  use super::*;

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
    let output = std::process::Command::new("node")
      .args(["-e", "process.stdout.write(require('node:fs').realpathSync(process.argv[1]))"])
      .arg(dir.path())
      .output()
      .unwrap();
    assert!(output.status.success());
    assert_eq!(for_cli(std::fs::canonicalize(dir.path()).unwrap()), PathBuf::from(String::from_utf8(output.stdout).unwrap()));
  }
}
