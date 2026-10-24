//! A session's project folder as the desktop tool panel reads it: one folder at a time, the work tree's git marks, a
//! file's text with its changed stretches, and the files the session's edits left changed. Every path is relative to
//! the project folder and must stay inside it once symlinks are resolved

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use acpira_shared::protocol::{ProjectChange, ProjectEntry, ProjectHunk};
use similar::{Algorithm, DiffTag, capture_diff_slices_deadline};

use crate::acp::transcript::diff::{DIFF_BUDGET, source_lines};
use crate::hooks::snapshot::git;

/// Larger files are not shown: the viewer lays out every line
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Entries listed per folder; a generated folder with more is cut there
const MAX_ENTRIES: usize = 5000;
/// Marks kept from one `git status`: `--ignored=matching` lists an ignored folder once, so this only caps odd trees
const MAX_MARKS: usize = 20_000;
/// Names an editor's explorer hides by default (VS Code's `files.exclude`)
const HIDDEN: &[&str] = &[".git", ".svn", ".hg", "CVS", ".DS_Store", "Thumbs.db"];

/// Why a listing or a read failed, as the page names it
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectError {
  /// The path leaves the project folder (`..`, an absolute path, a symlink out)
  Outside,
  Missing,
  Binary,
  TooLarge,
  Other(String),
}

impl ProjectError {
  /// The error a code names (`code`'s inverse; an unknown code is a message)
  pub fn from_code(code: &str) -> ProjectError {
    match code {
      "outside" => ProjectError::Outside,
      "missing" => ProjectError::Missing,
      "binary" => ProjectError::Binary,
      "tooLarge" => ProjectError::TooLarge,
      other => ProjectError::Other(other.to_owned()),
    }
  }

  pub fn code(&self) -> String {
    match self {
      ProjectError::Outside => "outside".into(),
      ProjectError::Missing => "missing".into(),
      ProjectError::Binary => "binary".into(),
      ProjectError::TooLarge => "tooLarge".into(),
      ProjectError::Other(m) => m.clone(),
    }
  }
}

fn io_error(e: std::io::Error) -> ProjectError {
  if e.kind() == std::io::ErrorKind::NotFound { ProjectError::Missing } else { ProjectError::Other(e.to_string()) }
}

/// `rel` under `root`, refused when it names anything outside: only normal components are taken, and the resolved path
/// (symlinks followed) must still lie under the resolved root. A path that does not exist resolves as written
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf, ProjectError> {
  let mut joined = root.to_path_buf();
  for part in Path::new(rel).components() {
    match part {
      Component::Normal(p) => joined.push(p),
      Component::CurDir => {}
      _ => return Err(ProjectError::Outside),
    }
  }
  let real_root = std::fs::canonicalize(root).map_err(io_error)?;
  match std::fs::canonicalize(&joined) {
    Ok(real) if real.starts_with(&real_root) => Ok(real),
    Ok(_) => Err(ProjectError::Outside),
    Err(e) => Err(io_error(e)),
  }
}

/// One folder: folders first, then files, each by name without case (as an editor's explorer orders them)
pub async fn list_dir(root: &Path, rel: &str) -> Result<Vec<ProjectEntry>, ProjectError> {
  let dir = resolve(root, rel)?;
  let mut read = tokio::fs::read_dir(&dir).await.map_err(io_error)?;
  let mut entries = vec![];
  while let Some(entry) = read.next_entry().await.map_err(io_error)? {
    let name = entry.file_name().to_string_lossy().into_owned();
    if HIDDEN.contains(&name.as_str()) {
      continue;
    }
    // A symlink counts as what it points at; one that points nowhere is listed as a file
    let dir = match entry.file_type().await {
      Ok(t) if t.is_symlink() => tokio::fs::metadata(entry.path()).await.is_ok_and(|m| m.is_dir()),
      Ok(t) => t.is_dir(),
      Err(_) => false,
    };
    entries.push(ProjectEntry { name, dir });
    if entries.len() >= MAX_ENTRIES {
      break;
    }
  }
  entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
  Ok(entries)
}

/// The project's git marks by path relative to `root`; empty outside a work tree. Paths come from the work tree's top
/// level, so the project's own prefix is taken off and anything outside it dropped
pub async fn git_marks(root: &Path) -> BTreeMap<String, String> {
  let mut marks = BTreeMap::new();
  let Some(prefix) = git(root, &["rev-parse", "--show-prefix"]).await else { return marks };
  let prefix = String::from_utf8_lossy(&prefix).trim().to_owned();
  let args = ["-c", "core.quotepath=off", "status", "--porcelain=v1", "-z", "--untracked-files=normal", "--ignored=matching", "--", "."];
  let Some(out) = git(root, &args).await else { return marks };
  for (xy, path) in porcelain(&out) {
    let Some(rel) = path.strip_prefix(&prefix) else { continue };
    if rel.is_empty() {
      continue;
    }
    marks.insert(rel.to_owned(), mark_of(&xy).to_owned());
    if marks.len() >= MAX_MARKS {
      break;
    }
  }
  marks
}

/// `git status --porcelain=v1 -z` entries as (XY, path); a rename / copy's source path that follows it is skipped
fn porcelain(out: &[u8]) -> Vec<(String, String)> {
  let mut entries = out.split(|b| *b == 0).filter(|e| !e.is_empty());
  let mut found = vec![];
  while let Some(entry) = entries.next() {
    if entry.len() < 4 {
      continue;
    }
    let xy = String::from_utf8_lossy(&entry[..2]).into_owned();
    if xy.contains('R') || xy.contains('C') {
      entries.next();
    }
    found.push((xy, String::from_utf8_lossy(&entry[3..]).into_owned()));
  }
  found
}

fn mark_of(xy: &str) -> &'static str {
  match xy {
    "??" => "U",
    "!!" => "I",
    _ if xy.contains('D') => "D",
    _ if xy.contains('A') || xy.contains('R') || xy.contains('C') => "A",
    _ => "M",
  }
}

/// A file's text, refused when it is too large to lay out or looks binary (a NUL in its first 8 KiB, as git decides)
pub async fn read_text(path: &Path) -> Result<String, ProjectError> {
  let meta = tokio::fs::metadata(path).await.map_err(io_error)?;
  if meta.is_dir() {
    return Err(ProjectError::Other("is a directory".into()));
  }
  if meta.len() > MAX_FILE_BYTES {
    return Err(ProjectError::TooLarge);
  }
  let bytes = tokio::fs::read(path).await.map_err(io_error)?;
  if bytes[..bytes.len().min(8192)].contains(&0) {
    return Err(ProjectError::Binary);
  }
  Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The file as the last commit has it: Some("") for a file the commit does not have, None outside a work tree
pub async fn head_text(root: &Path, rel: &str) -> Option<String> {
  git(root, &["rev-parse", "--is-inside-work-tree"]).await?;
  // `./` keeps the path relative to the project folder rather than the work tree's top level
  let spec = format!("HEAD:./{rel}");
  Some(git(root, &["show", &spec]).await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default())
}

/// The stretches of `now` that differ from `base`, as the viewer marks them: added lines by position in `now`, removed
/// ones attached to the line they stood before
pub fn hunks(base: &str, now: &str) -> Vec<ProjectHunk> {
  let a = source_lines(base);
  let b = source_lines(now);
  let ops = capture_diff_slices_deadline(Algorithm::Histogram, &a, &b, Some(Instant::now() + DIFF_BUDGET));
  let mut out: Vec<ProjectHunk> = vec![];
  for op in ops {
    let (tag, old, new) = op.as_tag_tuple();
    if tag == DiffTag::Equal {
      continue;
    }
    let at = new.start as u32 + 1;
    let del: Vec<String> = old.map(|i| a[i].to_owned()).collect();
    let add = new.len() as u32;
    // Delete + insert at the same place come out as two ops: one hunk
    match out.last_mut() {
      Some(last) if last.at + last.add == at && (last.add == 0 || del.is_empty()) => {
        last.add += add;
        last.del.extend(del);
      }
      _ => out.push(ProjectHunk { at, add, del }),
    }
  }
  out
}

/// Added and removed line counts between two texts
pub fn line_stat(base: &str, now: &str) -> (u32, u32) {
  hunks(base, now).iter().fold((0, 0), |(a, d), h| (a + h.add, d + h.del.len() as u32))
}

/// `path` (absolute, as an edit names it) relative to `root` with `/` separators, or as it is when outside
pub fn relative(root: &Path, path: &str) -> String {
  match Path::new(path).strip_prefix(root) {
    Ok(rel) => rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"),
    Err(_) => path.to_owned(),
  }
}

/// The session's edited files that still differ from what they held before its first edit; a file whose base the record
/// lost, or that is too large or binary now, is listed without counts
pub async fn session_changes(root: &Path, bases: Vec<(String, Option<String>)>) -> Vec<ProjectChange> {
  let mut out = vec![];
  for (path, base) in bases {
    let rel = relative(root, &path);
    let now = read_text(Path::new(&path)).await;
    let change = match (base, now) {
      (Some(base), Ok(now)) => {
        let (add, del) = tokio::task::spawn_blocking(move || line_stat(&base, &now)).await.unwrap_or_default();
        if add == 0 && del == 0 {
          continue;
        }
        ProjectChange { path: rel, add, del, gone: false }
      }
      (Some(base), Err(ProjectError::Missing)) => {
        // A file the session created and then removed again left nothing behind
        if base.is_empty() {
          continue;
        }
        ProjectChange { path: rel, add: 0, del: source_lines(&base).len() as u32, gone: true }
      }
      (_, Err(ProjectError::Missing)) => ProjectChange { path: rel, add: 0, del: 0, gone: true },
      _ => ProjectChange { path: rel, add: 0, del: 0, gone: false },
    };
    out.push(change);
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hunks_place_added_and_removed_lines() {
    let base = "a\nb\nc\nd\n";
    // b replaced by B1 B2, d removed at the end, x added at the start
    let now = "x\na\nB1\nB2\nc\n";
    let h = hunks(base, now);
    assert_eq!(h, vec![
      ProjectHunk { at: 1, add: 1, del: vec![] },
      ProjectHunk { at: 3, add: 2, del: vec!["b".into()] },
      ProjectHunk { at: 6, add: 0, del: vec!["d".into()] },
    ]);
    assert_eq!(line_stat(base, now), (3, 2));
    assert!(hunks(base, base).is_empty());
  }

  #[test]
  fn porcelain_marks_and_rename_sources() {
    let out = b"?? new.txt\0 M src/a.rs\0R  b.rs\0old.rs\0!! target/\0 D gone.md\0A  added.rs\0";
    let marks: Vec<(String, &str)> = porcelain(out).into_iter().map(|(xy, p)| (p, mark_of(&xy))).collect();
    assert_eq!(marks, vec![
      ("new.txt".into(), "U"),
      ("src/a.rs".into(), "M"),
      ("b.rs".into(), "A"),
      ("target/".into(), "I"),
      ("gone.md".into(), "D"),
      ("added.rs".into(), "A"),
    ]);
  }

  #[test]
  fn resolve_stays_inside_the_project() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("p");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/a.rs"), "x").unwrap();
    std::fs::write(tmp.path().join("secret"), "s").unwrap();
    assert!(resolve(&root, "src/a.rs").is_ok());
    assert!(resolve(&root, "").is_ok());
    assert_eq!(resolve(&root, "../secret"), Err(ProjectError::Outside));
    assert_eq!(resolve(&root, "/etc/hosts"), Err(ProjectError::Outside));
    assert_eq!(resolve(&root, "src/none.rs"), Err(ProjectError::Missing));
    #[cfg(unix)]
    {
      std::os::unix::fs::symlink(tmp.path().join("secret"), root.join("link")).unwrap();
      assert_eq!(resolve(&root, "link"), Err(ProjectError::Outside));
    }
  }

  #[tokio::test]
  async fn lists_folders_first_and_hides_vcs_folders() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("zeta")).unwrap();
    std::fs::create_dir_all(root.join("Alpha")).unwrap();
    std::fs::write(root.join("b.txt"), "").unwrap();
    std::fs::write(root.join("A.md"), "").unwrap();
    let names: Vec<(String, bool)> = list_dir(root, "").await.unwrap().into_iter().map(|e| (e.name, e.dir)).collect();
    assert_eq!(names, vec![("Alpha".into(), true), ("zeta".into(), true), ("A.md".into(), false), ("b.txt".into(), false)]);
  }

  #[tokio::test]
  async fn refuses_binary_and_large_files() {
    let tmp = tempfile::tempdir().unwrap();
    let bin = tmp.path().join("x.bin");
    std::fs::write(&bin, [1u8, 0, 2]).unwrap();
    assert_eq!(read_text(&bin).await, Err(ProjectError::Binary));
    let big = tmp.path().join("big.txt");
    std::fs::write(&big, vec![b'a'; MAX_FILE_BYTES as usize + 1]).unwrap();
    assert_eq!(read_text(&big).await, Err(ProjectError::TooLarge));
  }

  #[tokio::test]
  async fn session_changes_skip_reverted_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let edited = root.join("src/a.rs");
    std::fs::create_dir_all(edited.parent().unwrap()).unwrap();
    std::fs::write(&edited, "one\nTWO\nthree\n").unwrap();
    let reverted = root.join("same.rs");
    std::fs::write(&reverted, "x\n").unwrap();
    let removed = root.join("gone.rs");
    let changes = session_changes(root, vec![
      (edited.to_string_lossy().into_owned(), Some("one\ntwo\nthree\n".into())),
      (reverted.to_string_lossy().into_owned(), Some("x\n".into())),
      (removed.to_string_lossy().into_owned(), Some("a\nb\n".into())),
      ("/elsewhere/c.rs".into(), None),
    ]).await;
    assert_eq!(changes, vec![
      ProjectChange { path: "src/a.rs".into(), add: 1, del: 1, gone: false },
      ProjectChange { path: "gone.rs".into(), add: 0, del: 2, gone: true },
      ProjectChange { path: "/elsewhere/c.rs".into(), add: 0, del: 0, gone: true },
    ]);
  }
}
