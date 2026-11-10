//! What a turn changed on disk: the dirty files of the watched git work trees before and after it. Agents edit through
//! their own tools, shell commands and scripts alike, and only some of that shows up as an edit tool call, so the work
//! trees themselves are compared.
//!
//! A file counts by its content, not by its git status: whoever shares the work tree can commit or undo a commit
//! meanwhile, which moves files between dirty and clean without touching them

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A dirty file's size and modification time; None: deleted
type Stamp = Option<(u64, SystemTime)>;

/// The dirty files (modified, added, untracked, deleted) of every watched work tree
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
  files: BTreeMap<PathBuf, Stamp>,
  /// Each watched work tree's top level → its HEAD commit (none before the first commit): what its clean files held
  heads: BTreeMap<PathBuf, String>,
}

fn stamp(path: &Path) -> Stamp {
  let meta = std::fs::metadata(path).ok()?;
  Some((meta.len(), meta.modified().ok()?))
}

fn git_command(dir: &Path, args: &[&str]) -> tokio::process::Command {
  let mut cmd = tokio::process::Command::new("git");
  cmd.arg("-C").arg(dir).args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
  #[cfg(windows)]
  {
    // No console window flashing up for every turn
    cmd.creation_flags(0x0800_0000);
  }
  cmd
}

pub(crate) async fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
  let out = git_command(dir, args).output().await.ok()?;
  out.status.success().then_some(out.stdout)
}

/// `git` answering one line per line of `input` (`cat-file --batch-check`, `hash-object --stdin-paths`); None when it
/// failed or the counts differ
async fn git_lines(dir: &Path, args: &[&str], input: &[String]) -> Option<Vec<String>> {
  use tokio::io::AsyncWriteExt;
  let mut cmd = git_command(dir, args);
  cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
  let mut child = cmd.spawn().ok()?;
  let mut stdin = child.stdin.take()?;
  let text: String = input.iter().map(|l| format!("{l}\n")).collect();
  // Fed while the output is read, so neither pipe fills up; dropping the handle ends the input
  let feed = async move {
    let _ = stdin.write_all(text.as_bytes()).await;
  };
  let (_, out) = tokio::join!(feed, child.wait_with_output());
  let out = out.ok().filter(|o| o.status.success())?;
  let lines: Vec<String> = String::from_utf8_lossy(&out.stdout).lines().map(str::to_owned).collect();
  (lines.len() == input.len()).then_some(lines)
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

/// The top level of the work tree `dir` lies in
async fn top_level(dir: &Path) -> Option<PathBuf> {
  let top = git(dir, &["rev-parse", "--show-toplevel"]).await?;
  Some(PathBuf::from(String::from_utf8_lossy(&top).trim()))
}

/// The dirty files under `dir`, as absolute paths in its work tree `top`
async fn dirty(dir: &Path, top: &Path) -> Vec<PathBuf> {
  // Paths relative to the top level whatever the config says; the pathspec keeps a subdirectory watch to its subtree
  let args = ["-c", "status.relativePaths=false", "status", "--porcelain=v1", "-z", "--untracked-files=all", "--", "."];
  let Some(out) = git(dir, &args).await else { return vec![] };
  porcelain_paths(&out).into_iter().map(|p| top.join(p)).collect()
}

/// Whether `dir` lies in a git work tree: outside one a snapshot sees nothing, whatever changed
pub async fn in_work_tree(dir: &Path) -> bool {
  top_level(dir).await.is_some()
}

/// `path` below `top` the way git names it
fn git_path(path: &Path, top: &Path) -> Option<String> {
  Some(path.strip_prefix(top).ok()?.to_string_lossy().replace('\\', "/"))
}

impl Snapshot {
  pub async fn take(watch: &[PathBuf]) -> Snapshot {
    let (mut files, mut heads) = (BTreeMap::new(), BTreeMap::new());
    for dir in watch {
      // Not inside a git work tree: nothing to see
      let Some(top) = top_level(dir).await else { continue };
      for path in dirty(dir, &top).await {
        let s = stamp(&path);
        files.insert(path, s);
      }
      if let Some(head) = git(&top, &["rev-parse", "--verify", "--quiet", "HEAD"]).await {
        heads.insert(top, String::from_utf8_lossy(&head).trim().to_owned());
      }
    }
    Snapshot { files, heads }
  }

  /// The snapshot for the wire (an SSH host's helper takes it there): `files` of `{ path, size, mtimeNs }`, size and
  /// time left out for a deleted file, and `heads` of `{ top, head }`
  pub fn to_json(&self) -> serde_json::Value {
    let files: Vec<serde_json::Value> = self
      .files
      .iter()
      .map(|(p, s)| match s {
        Some((size, t)) => {
          let ns = t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
          serde_json::json!({ "path": p, "size": size, "mtimeNs": ns })
        }
        None => serde_json::json!({ "path": p }),
      })
      .collect();
    let heads: Vec<serde_json::Value> = self.heads.iter().map(|(top, head)| serde_json::json!({ "top": top, "head": head })).collect();
    serde_json::json!({ "files": files, "heads": heads })
  }

  pub fn from_json(v: &serde_json::Value) -> Snapshot {
    let mut files = BTreeMap::new();
    for f in v.get("files").and_then(serde_json::Value::as_array).into_iter().flatten() {
      let Some(path) = f.get("path").and_then(serde_json::Value::as_str) else { continue };
      let stamp = match (f.get("size").and_then(serde_json::Value::as_u64), f.get("mtimeNs").and_then(serde_json::Value::as_u64)) {
        (Some(size), Some(ns)) => Some((size, SystemTime::UNIX_EPOCH + std::time::Duration::from_nanos(ns))),
        _ => None,
      };
      files.insert(PathBuf::from(path), stamp);
    }
    let mut heads = BTreeMap::new();
    for h in v.get("heads").and_then(serde_json::Value::as_array).into_iter().flatten() {
      if let (Some(top), Some(head)) = (h.get("top").and_then(serde_json::Value::as_str), h.get("head").and_then(serde_json::Value::as_str)) {
        heads.insert(PathBuf::from(top), head.to_owned());
      }
    }
    Snapshot { files, heads }
  }

  /// Files whose content differs between the two snapshots: newly dirty, touched again, or back to clean, minus the
  /// ones a commit (or its undoing) moved between dirty and clean as they stood. Looks at the work trees, so it runs
  /// on their host, right after `self` was taken
  pub async fn changed_since(&self, before: &Snapshot) -> Vec<PathBuf> {
    let keys: BTreeSet<&PathBuf> = self.files.keys().chain(before.files.keys()).collect();
    let mut changed = vec![];
    // Clean then, dirty now, in a work tree whose HEAD moved: (top, the HEAD then) → the files
    let mut appeared: BTreeMap<(&PathBuf, &String), Vec<&PathBuf>> = BTreeMap::new();
    for k in keys {
      match (before.files.get(k), self.files.get(k)) {
        (b, a) if b == a => {}
        // Dirty then, clean now: a commit took the file as it stood (committing leaves the file itself alone), unless
        // it was written again on the way (a revert, a formatter)
        (Some(b), None) if stamp(k) == *b => {}
        (None, Some(Some(_))) => match before.head_over(k) {
          Some((top, head)) if self.heads.get(top) != Some(head) => appeared.entry((top, head)).or_default().push(k),
          _ => changed.push(k.clone()),
        },
        _ => changed.push(k.clone()),
      }
    }
    for ((top, head), paths) in appeared {
      changed.extend(differing_from(top, head, paths).await);
    }
    changed.sort();
    changed
  }

  /// The innermost watched work tree holding `path`, and its HEAD
  fn head_over(&self, path: &Path) -> Option<(&PathBuf, &String)> {
    self.heads.iter().filter(|(top, _)| path.starts_with(top)).max_by_key(|(top, _)| top.components().count())
  }
}

/// The ones of `paths` (dirty files of the work tree `top`) that do not hold what the commit `head` has for them: the
/// commit's blobs against the files hashed the way git would store them. Anything that cannot be told counts as
/// differing
async fn differing_from(top: &Path, head: &str, paths: Vec<&PathBuf>) -> Vec<PathBuf> {
  // A line per file on stdin: a name with a line break, or anything but a plain file, is not asked about
  let named = |p: &&PathBuf| git_path(p, top).filter(|n| !n.contains('\n') && p.is_file());
  let (asked, mut differing): (Vec<&PathBuf>, Vec<&PathBuf>) = paths.into_iter().partition(|p| named(p).is_some());
  let names: Vec<String> = asked.iter().filter_map(named).collect();
  let revs: Vec<String> = names.iter().map(|n| format!("{head}:{n}")).collect();
  // A file the commit does not have answers "<name> missing", which equals no hash
  let then = git_lines(top, &["cat-file", "--batch-check=%(objectname)"], &revs).await;
  let now = git_lines(top, &["hash-object", "--stdin-paths"], &names).await;
  match (then, now) {
    (Some(then), Some(now)) => differing.extend(asked.into_iter().zip(then.iter().zip(&now)).filter(|(_, (t, n))| t != n).map(|(p, _)| p)),
    _ => differing.extend(asked),
  }
  differing.into_iter().cloned().collect()
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
    assert_eq!(changed(root, &before).await, ["reverted.txt", "sub/new.txt", "touched.txt"]);
  }

  /// What the work tree at `root` (and a nested one in `nested`) changed since `before`, relative to `root`
  async fn changed(root: &Path, before: &Snapshot) -> Vec<String> {
    let after = Snapshot::take(&watched(root)).await;
    // The wire keeps what the comparison needs
    assert_eq!(Snapshot::from_json(&after.to_json()), after);
    let top = dunce_top(root);
    after.changed_since(before).await.iter().map(|p| p.strip_prefix(&top).unwrap_or(p).to_string_lossy().replace('\\', "/")).collect()
  }

  fn watched(root: &Path) -> Vec<PathBuf> {
    let nested = root.join("nested");
    if nested.join(".git").exists() { vec![root.to_path_buf(), nested] } else { vec![root.to_path_buf()] }
  }

  fn repo(root: &Path, files: &[&str]) {
    std::fs::create_dir_all(root).unwrap();
    sh(root, &["init", "-q"]);
    sh(root, &["config", "user.email", "t@t"]);
    sh(root, &["config", "user.name", "t"]);
    for f in files {
      std::fs::write(root.join(f), "base\n").unwrap();
    }
    sh(root, &["add", "."]);
    sh(root, &["commit", "-q", "-m", "base"]);
  }

  /// Several sessions share one work tree: one of them committing its pending work during another's turn changes
  /// every committed file's git status and no file
  #[tokio::test]
  async fn a_commit_by_someone_else_is_not_a_change_of_the_turn() {
    if std::process::Command::new("git").arg("--version").output().is_err() {
      return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo(root, &["edited.txt", "removed.txt", "mine.txt", ".gitignore"]);
    std::fs::write(root.join(".gitignore"), "/nested/\n").unwrap();
    repo(&root.join("nested"), &["inner.txt", "inner-mine.txt"]);
    // Someone else's pending work when the turn starts: an edit, a new file, a deletion, in both repositories
    std::fs::write(root.join("edited.txt"), "theirs\n").unwrap();
    std::fs::write(root.join("added.txt"), "theirs\n").unwrap();
    std::fs::remove_file(root.join("removed.txt")).unwrap();
    std::fs::write(root.join("nested/inner.txt"), "theirs\n").unwrap();
    let before = Snapshot::take(&watched(root)).await;
    // They commit it all while the turn runs
    for r in [root.to_path_buf(), root.join("nested")] {
      sh(&r, &["add", "-A"]);
      sh(&r, &["commit", "-q", "-m", "theirs"]);
    }
    assert!(changed(root, &before).await.is_empty());
    // The turn's own edits still count, next to a commit and after one
    let before = Snapshot::take(&watched(root)).await;
    std::fs::write(root.join("edited.txt"), "theirs, pending again\n").unwrap();
    std::fs::write(root.join("mine.txt"), "mine\n").unwrap();
    std::fs::write(root.join("nested/inner-mine.txt"), "mine\n").unwrap();
    sh(root, &["commit", "-q", "-m", "theirs again", "--", "edited.txt"]);
    assert_eq!(changed(root, &before).await, ["mine.txt", "nested/inner-mine.txt"]);
  }

  /// The other way round: a commit taken back leaves its files dirty with what they held all along
  #[tokio::test]
  async fn an_undone_commit_is_not_a_change_of_the_turn() {
    if std::process::Command::new("git").arg("--version").output().is_err() {
      return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repo(root, &["edited.txt", "mine.txt"]);
    std::fs::write(root.join("edited.txt"), "theirs\n").unwrap();
    std::fs::write(root.join("added.txt"), "theirs\n").unwrap();
    std::fs::write(root.join("mine.txt"), "theirs\n").unwrap();
    sh(root, &["add", "-A"]);
    sh(root, &["commit", "-q", "-m", "theirs"]);
    let before = Snapshot::take(&watched(root)).await;
    sh(root, &["reset", "-q", "HEAD~1"]);
    std::fs::write(root.join("mine.txt"), "mine\n").unwrap();
    assert_eq!(changed(root, &before).await, ["mine.txt"]);
  }

  /// git prints the top level with symlinks resolved (macOS /var → /private/var) and `/` separators
  fn dunce_top(root: &Path) -> PathBuf {
    let out = std::process::Command::new("git").arg("-C").arg(root).args(["rev-parse", "--show-toplevel"]).output().unwrap();
    PathBuf::from(String::from_utf8_lossy(&out.stdout).trim())
  }
}
