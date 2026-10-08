//! What a turn changed on disk: the dirty files of the watched git work trees before and after it. Agents edit through
//! their own tools, shell commands and scripts alike, and only some of that shows up as an edit tool call, so the work
//! trees themselves are compared

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A dirty file's size and modification time; None: deleted
type Stamp = Option<(u64, SystemTime)>;

/// The dirty files (modified, added, untracked, deleted) of every watched work tree
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
  files: BTreeMap<PathBuf, Stamp>,
}

fn stamp(path: &Path) -> Stamp {
  let meta = std::fs::metadata(path).ok()?;
  Some((meta.len(), meta.modified().ok()?))
}

async fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
  let mut cmd = tokio::process::Command::new("git");
  cmd.arg("-C").arg(dir).args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
  #[cfg(windows)]
  {
    // No console window flashing up for every turn
    cmd.creation_flags(0x0800_0000);
  }
  let out = cmd.output().await.ok()?;
  out.status.success().then_some(out.stdout)
}

/// Paths of `git status --porcelain -z` output, relative to the work tree's top level. A rename / copy entry is followed
/// by its source path, which is skipped (the source shows up as deleted on its own when it matters)
pub fn porcelain_paths(out: &[u8]) -> Vec<String> {
  let mut paths = vec![];
  let mut entries = out.split(|b| *b == 0).filter(|e| !e.is_empty());
  while let Some(entry) = entries.next() {
    if entry.len() < 4 {
      continue;
    }
    let (xy, path) = (&entry[..2], &entry[3..]);
    paths.push(String::from_utf8_lossy(path).into_owned());
    if xy.contains(&b'R') || xy.contains(&b'C') {
      entries.next();
    }
  }
  paths
}

/// One work tree's dirty files as absolute paths; empty when `dir` is not inside a git work tree
async fn dirty(dir: &Path) -> Vec<PathBuf> {
  let Some(top) = git(dir, &["rev-parse", "--show-toplevel"]).await else { return vec![] };
  let top = PathBuf::from(String::from_utf8_lossy(&top).trim());
  // Paths relative to the top level whatever the config says; the pathspec keeps a subdirectory watch to its subtree
  let args = ["-c", "status.relativePaths=false", "status", "--porcelain=v1", "-z", "--untracked-files=all", "--", "."];
  let Some(out) = git(dir, &args).await else { return vec![] };
  porcelain_paths(&out).into_iter().map(|p| top.join(p)).collect()
}

impl Snapshot {
  pub async fn take(watch: &[PathBuf]) -> Snapshot {
    let mut files = BTreeMap::new();
    for dir in watch {
      for path in dirty(dir).await {
        let s = stamp(&path);
        files.insert(path, s);
      }
    }
    Snapshot { files }
  }

  /// Files whose state differs between the two snapshots: newly dirty, touched again, or back to clean
  pub fn changed_since(&self, before: &Snapshot) -> Vec<PathBuf> {
    let keys: BTreeSet<&PathBuf> = self.files.keys().chain(before.files.keys()).collect();
    keys.into_iter().filter(|k| self.files.get(*k) != before.files.get(*k)).cloned().collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn porcelain_entries_and_renames_are_split() {
    let out = b" M src/a.rs\0?? new file.txt\0R  b.rs\0old b.rs\0 D gone.rs\0";
    assert_eq!(porcelain_paths(out), ["src/a.rs", "new file.txt", "b.rs", "gone.rs"]);
    assert!(porcelain_paths(b"").is_empty());
  }

  fn sh(dir: &Path, args: &[&str]) {
    let ok = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().map(|o| o.status.success());
    assert_eq!(ok.ok(), Some(true), "git {args:?}");
  }

  #[tokio::test]
  async fn a_turn_sees_new_touched_and_reverted_files_but_not_untouched_dirty_ones() {
    if std::process::Command::new("git").arg("--version").output().is_err() {
      return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    sh(root, &["init", "-q"]);
    sh(root, &["config", "user.email", "t@t"]);
    sh(root, &["config", "user.name", "t"]);
    for f in ["kept.txt", "touched.txt", "reverted.txt"] {
      std::fs::write(root.join(f), "base\n").unwrap();
    }
    sh(root, &["add", "."]);
    sh(root, &["commit", "-q", "-m", "base"]);
    // Dirty before the turn: someone else's work stays out of the turn's changes unless the turn touches it
    std::fs::write(root.join("kept.txt"), "theirs\n").unwrap();
    std::fs::write(root.join("touched.txt"), "theirs\n").unwrap();
    std::fs::write(root.join("reverted.txt"), "theirs\n").unwrap();
    let before = Snapshot::take(&[root.to_path_buf()]).await;
    std::fs::write(root.join("touched.txt"), "theirs and mine, longer\n").unwrap();
    std::fs::write(root.join("reverted.txt"), "base\n").unwrap();
    std::fs::create_dir(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/new.txt"), "x\n").unwrap();
    let after = Snapshot::take(&[root.to_path_buf()]).await;
    let names: Vec<String> = after
      .changed_since(&before)
      .iter()
      .map(|p| p.strip_prefix(dunce_top(root)).unwrap_or(p).to_string_lossy().replace('\\', "/"))
      .collect();
    assert_eq!(names, ["reverted.txt", "sub/new.txt", "touched.txt"]);
  }

  /// git prints the top level with symlinks resolved (macOS /var → /private/var) and `/` separators
  fn dunce_top(root: &Path) -> PathBuf {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(["rev-parse", "--show-toplevel"]).output().unwrap();
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
  }
}
