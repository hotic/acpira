//! A session's project folder as the desktop tool panel reads it: one folder at a time, the work tree's git marks, a
//! file's text with its changed stretches, and the files the session's edits left changed. Every path is relative to
//! the project folder and must stay inside it once symlinks are resolved

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use acpira_shared::protocol::{ProjectChange, ProjectEditOp, ProjectEntry, ProjectHunk};
use acpira_shared::remote::RemoteDirEntry;
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
  /// A new or renamed entry would take a name in use
  Exists,
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
      "exists" => ProjectError::Exists,
      "binary" => ProjectError::Binary,
      "tooLarge" => ProjectError::TooLarge,
      other => ProjectError::Other(other.to_owned()),
    }
  }

  pub fn code(&self) -> String {
    match self {
      ProjectError::Outside => "outside".into(),
      ProjectError::Missing => "missing".into(),
      ProjectError::Exists => "exists".into(),
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

/// The file the viewer reads for `rel`: a path inside the project as `resolve` takes it, or an explicit absolute path
/// without `..` (a plan or a session-edited file outside the project) as written. Local folders and SSH hosts share it,
/// so a file the session edited elsewhere opens on either
pub fn resolve_read(root: &Path, rel: &str) -> Result<PathBuf, ProjectError> {
  let p = Path::new(rel);
  if p.is_absolute() && !p.components().any(|c| matches!(c, Component::ParentDir)) {
    return Ok(p.to_path_buf());
  }
  resolve(root, rel)
}

/// The entry `rel` names, for a change: its folder as `resolve` takes it, the entry itself as written, so a symlink is
/// renamed or deleted itself, never what it points at. The project folder is no entry of its own
fn resolve_entry(root: &Path, rel: &str) -> Result<PathBuf, ProjectError> {
  let path = Path::new(rel);
  let Some(Component::Normal(name)) = path.components().next_back() else { return Err(ProjectError::Outside) };
  let parent = path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
  Ok(resolve(root, &parent)?.join(name))
}

async fn exists(path: &Path) -> bool {
  tokio::fs::symlink_metadata(path).await.is_ok()
}

/// A change from the tool panel's tree. `trash`: a delete goes to the system's trash (this computer); without it the
/// entry is removed for good (an SSH host, where nobody would look into a trash)
pub async fn edit(root: &Path, edit: &ProjectEditOp, trash: bool) -> Result<(), ProjectError> {
  match edit {
    ProjectEditOp::Create { path, dir } => {
      let target = resolve_entry(root, path)?;
      // `create_dir` and `create_new` both refuse a name in use, a dangling symlink included
      let made = if *dir {
        tokio::fs::create_dir(&target).await
      } else {
        tokio::fs::OpenOptions::new().write(true).create_new(true).open(&target).await.map(drop)
      };
      made.map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists { ProjectError::Exists } else { io_error(e) })
    }
    ProjectEditOp::Rename { path, to } => {
      let (from, target) = (resolve_entry(root, path)?, resolve_entry(root, to)?);
      if !exists(&from).await {
        return Err(ProjectError::Missing);
      }
      // A rename replaces what holds the name. A change of case alone finds itself on a case-insensitive disk
      if exists(&target).await && path.to_lowercase() != to.to_lowercase() {
        return Err(ProjectError::Exists);
      }
      tokio::fs::rename(&from, &target).await.map_err(io_error)
    }
    ProjectEditOp::Delete { path } => {
      let target = resolve_entry(root, path)?;
      let meta = tokio::fs::symlink_metadata(&target).await.map_err(io_error)?;
      if trash {
        return tokio::task::spawn_blocking(move || to_trash(&target)).await.map_err(|e| ProjectError::Other(e.to_string()))?;
      }
      let removed = if meta.is_dir() { tokio::fs::remove_dir_all(&target).await } else { tokio::fs::remove_file(&target).await };
      removed.map_err(io_error)
    }
  }
}

fn to_trash(path: &Path) -> Result<(), ProjectError> {
  #[cfg_attr(not(target_os = "macos"), expect(unused_mut, reason = "only macOS picks a method"))]
  let mut ctx = trash::TrashContext::new();
  // The default asks Finder through AppleScript, which wants the Automation permission
  #[cfg(target_os = "macos")]
  trash::macos::TrashContextExtMacos::set_delete_method(&mut ctx, trash::macos::DeleteMethod::NsFileManager);
  ctx.delete(path).map_err(|e| ProjectError::Other(e.to_string()))
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

/// A folder of this computer for a paired device's add-project picker: its normalized path, its parent (None at a
/// root) and the folders in it by name without case, each marked when it holds `.git`. Folders only, dot-folders left
/// out: the picker chooses a project, it does not browse files. `path` None is the user's home; `~` leads expand to it
pub async fn local_folders(path: Option<&str>) -> Result<(String, Option<String>, Vec<RemoteDirEntry>), String> {
  let home = crate::store::data_dir::home_dir();
  let dir = match path.map(str::trim) {
    None | Some("") => home,
    Some("~") => home,
    Some(p) => match p.strip_prefix("~/").or_else(|| p.strip_prefix("~\\")) {
      Some(rest) => home.join(rest),
      None if Path::new(p).is_absolute() => PathBuf::from(p),
      None => return Err(format!("{p} is not an absolute path")),
    },
  };
  let dir = crate::store::data_dir::normalize(&dir);
  let mut read = tokio::fs::read_dir(&dir).await.map_err(|e| format!("{}: {e}", dir.display()))?;
  let mut entries = vec![];
  while let Some(entry) = read.next_entry().await.map_err(|e| e.to_string())? {
    let name = entry.file_name().to_string_lossy().into_owned();
    if name.starts_with('.') || HIDDEN.contains(&name.as_str()) {
      continue;
    }
    // A symlink counts as what it points at
    let is_dir = match entry.file_type().await {
      Ok(t) if t.is_symlink() => tokio::fs::metadata(entry.path()).await.is_ok_and(|m| m.is_dir()),
      Ok(t) => t.is_dir(),
      Err(_) => false,
    };
    if !is_dir {
      continue;
    }
    let git = tokio::fs::symlink_metadata(entry.path().join(".git")).await.is_ok();
    entries.push(RemoteDirEntry { name, dir: true, git });
    if entries.len() >= MAX_ENTRIES {
      break;
    }
  }
  entries.sort_by_key(|e| e.name.to_lowercase());
  let parent = dir.parent().map(|p| p.to_string_lossy().into_owned());
  Ok((dir.to_string_lossy().into_owned(), parent, entries))
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

  #[tokio::test]
  async fn local_folders_lists_visible_folders_only_and_marks_work_trees() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("beta/.git")).unwrap();
    std::fs::create_dir(root.join("Alpha")).unwrap();
    std::fs::create_dir(root.join(".hidden")).unwrap();
    std::fs::write(root.join("file.txt"), "x").unwrap();
    let (path, parent, entries) = local_folders(Some(&root.to_string_lossy())).await.unwrap();
    assert_eq!(path, crate::store::data_dir::normalize(root).to_string_lossy());
    assert_eq!(parent.as_deref(), root.parent().map(|p| p.to_string_lossy()).as_deref());
    let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.git)).collect();
    assert_eq!(names, [("Alpha", false), ("beta", true)]);
    assert!(local_folders(Some("relative/path")).await.is_err());
    assert!(local_folders(Some(&root.join("missing").to_string_lossy())).await.is_err());
  }

  #[tokio::test]
  async fn edits_stay_inside_and_never_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir(root.join("src")).unwrap();
    let create = |path: &str, dir| ProjectEditOp::Create { path: path.into(), dir };
    let rename = |path: &str, to: &str| ProjectEditOp::Rename { path: path.into(), to: to.into() };
    edit(root, &create("src/a.ts", false), false).await.unwrap();
    edit(root, &create("src/lib", true), false).await.unwrap();
    assert_eq!(edit(root, &create("src/a.ts", false), false).await, Err(ProjectError::Exists));
    assert_eq!(edit(root, &create("../out.ts", false), false).await, Err(ProjectError::Outside));
    assert_eq!(edit(root, &create("gone/a.ts", false), false).await, Err(ProjectError::Missing));
    assert_eq!(edit(root, &rename("src/a.ts", "src/lib"), false).await, Err(ProjectError::Exists));
    assert_eq!(edit(root, &rename("src/none.ts", "src/b.ts"), false).await, Err(ProjectError::Missing));
    edit(root, &rename("src/a.ts", "src/lib/b.ts"), false).await.unwrap();
    assert!(root.join("src/lib/b.ts").is_file() && !root.join("src/a.ts").exists());
    // The project folder itself is not an entry
    assert_eq!(edit(root, &ProjectEditOp::Delete { path: String::new() }, false).await, Err(ProjectError::Outside));
    // A link out of the project goes itself, what it points at stays
    #[cfg(unix)]
    {
      let outside = tempfile::tempdir().unwrap();
      std::fs::write(outside.path().join("keep.txt"), "x").unwrap();
      std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
      assert_eq!(edit(root, &create("link/new.txt", false), false).await, Err(ProjectError::Outside));
      edit(root, &ProjectEditOp::Delete { path: "link".into() }, false).await.unwrap();
      assert!(outside.path().join("keep.txt").is_file());
    }
    edit(root, &ProjectEditOp::Delete { path: "src".into() }, false).await.unwrap();
    assert!(!root.join("src").exists());
  }

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
    // The viewer's read takes an explicit absolute path outside the project, never a relative one climbing out
    let secret = tmp.path().join("secret");
    assert_eq!(resolve_read(&root, &secret.to_string_lossy()), Ok(secret));
    assert_eq!(resolve_read(&root, "../secret"), Err(ProjectError::Outside));
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
