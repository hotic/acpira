//! File system primitives of the shared config: what sits where a link should go, making and removing links,
//! moving things aside into backups, git exclude entries and Claude's import line. Blocking std::fs throughout;
//! callers run these on the blocking pool

use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

/// What sits at the place a link to `target` should occupy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spot {
  /// Nothing (a dangling symlink counts as nothing to keep)
  Absent,
  /// A link resolving to the target
  Linked,
  /// A link resolving somewhere else
  Elsewhere,
  /// A real file / directory with the target's exact content
  Same,
  /// A real file / directory with other content
  Differs,
}

pub fn inspect(link: &Path, target: &Path) -> Spot {
  let Ok(meta) = std::fs::symlink_metadata(link) else { return Spot::Absent };
  let real_target = std::fs::canonicalize(target).ok();
  if meta.file_type().is_symlink() || is_junction(link) {
    return match std::fs::canonicalize(link) {
      Err(_) => Spot::Absent,
      Ok(p) if Some(&p) == real_target.as_ref() => Spot::Linked,
      Ok(_) => Spot::Elsewhere,
    };
  }
  // A Windows hard link is a real file that shares the target's content; equal content is all that can be checked
  if same_content(link, target) { Spot::Same } else { Spot::Differs }
}

#[cfg(windows)]
fn is_junction(p: &Path) -> bool {
  use std::os::windows::fs::MetadataExt;
  // FILE_ATTRIBUTE_REPARSE_POINT
  std::fs::symlink_metadata(p).is_ok_and(|m| m.file_attributes() & 0x400 != 0)
}

#[cfg(not(windows))]
fn is_junction(_: &Path) -> bool {
  false
}

/// Files compare byte for byte; directories by the same relative file list with the same bytes (dotfiles such as
/// .DS_Store left out). Large trees are not compared: past the limit they count as different
pub fn same_content(a: &Path, b: &Path) -> bool {
  const MAX_FILES: usize = 400;
  const MAX_BYTES: u64 = 16 * 1024 * 1024;
  let (Ok(ma), Ok(mb)) = (std::fs::metadata(a), std::fs::metadata(b)) else { return false };
  if ma.is_file() && mb.is_file() {
    return ma.len() == mb.len() && std::fs::read(a).ok() == std::fs::read(b).ok();
  }
  if !(ma.is_dir() && mb.is_dir()) {
    return false;
  }
  let (Some(fa), Some(fb)) = (list_files(a, MAX_FILES), list_files(b, MAX_FILES)) else { return false };
  if fa != fb {
    return false;
  }
  let mut total = 0u64;
  for rel in &fa {
    let (pa, pb) = (a.join(rel), b.join(rel));
    let (Ok(x), Ok(y)) = (std::fs::metadata(&pa), std::fs::metadata(&pb)) else { return false };
    total += x.len();
    if x.len() != y.len() || total > MAX_BYTES || std::fs::read(&pa).ok() != std::fs::read(&pb).ok() {
      return false;
    }
  }
  true
}

/// Sorted relative paths of the regular files under `dir`, following links; None past `max`
fn list_files(dir: &Path, max: usize) -> Option<Vec<PathBuf>> {
  let mut out = vec![];
  let mut stack = vec![PathBuf::new()];
  while let Some(rel) = stack.pop() {
    let Ok(rd) = std::fs::read_dir(dir.join(&rel)) else { continue };
    for e in rd.flatten() {
      let name = e.file_name();
      if name.to_string_lossy().starts_with('.') {
        continue;
      }
      let child = rel.join(&name);
      let Ok(m) = std::fs::metadata(dir.join(&child)) else { continue };
      if m.is_dir() {
        stack.push(child);
      } else if m.is_file() {
        out.push(child);
        if out.len() > max {
          return None;
        }
      }
    }
  }
  out.sort();
  Some(out)
}

/// `to` expressed relative to the directory `from_dir` (both absolute)
pub fn relative_to(to: &Path, from_dir: &Path) -> PathBuf {
  let a: Vec<Component> = to.components().collect();
  let b: Vec<Component> = from_dir.components().collect();
  let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
  let mut out = PathBuf::new();
  for _ in common..b.len() {
    out.push("..");
  }
  for c in &a[common..] {
    out.push(c.as_os_str());
  }
  out
}

/// Create a link at `link` to `target` (relative when asked, so a moved repository keeps its links).
/// Windows: a junction for a directory and a hard link for a file, neither needs elevation
pub fn make_link(link: &Path, target: &Path, relative: bool) -> io::Result<()> {
  if let Some(dir) = link.parent() {
    std::fs::create_dir_all(dir)?;
  }
  // A dangling link left behind is replaced
  if std::fs::symlink_metadata(link).is_ok() && std::fs::metadata(link).is_err() {
    std::fs::remove_file(link)?;
  }
  let dest = match (relative, link.parent()) {
    (true, Some(dir)) => relative_to(target, dir),
    _ => target.to_path_buf(),
  };
  platform_link(link, target, &dest)
}

#[cfg(unix)]
fn platform_link(link: &Path, _target: &Path, dest: &Path) -> io::Result<()> {
  std::os::unix::fs::symlink(dest, link)
}

#[cfg(windows)]
fn platform_link(link: &Path, target: &Path, dest: &Path) -> io::Result<()> {
  let dir = std::fs::metadata(target)?.is_dir();
  let symlink = if dir { std::os::windows::fs::symlink_dir(dest, link) } else { std::os::windows::fs::symlink_file(dest, link) };
  if symlink.is_ok() {
    return Ok(());
  }
  if !dir {
    return std::fs::hard_link(target, link);
  }
  let status = std::process::Command::new("cmd").arg("/c").arg("mklink").arg("/J").arg(link).arg(target).status()?;
  if status.success() { Ok(()) } else { Err(io::Error::other("mklink /J failed")) }
}

/// Remove a link (never follows it)
pub fn remove_link(link: &Path) -> io::Result<()> {
  let meta = std::fs::symlink_metadata(link)?;
  if meta.file_type().is_symlink() || !meta.is_dir() {
    return std::fs::remove_file(link);
  }
  // A junction is a directory entry; remove_dir drops the junction, not its target
  std::fs::remove_dir(link)
}

/// Move `path` into `backups` under a name derived from its full path; returns where it went
pub fn move_aside(path: &Path, backups: &Path) -> io::Result<PathBuf> {
  std::fs::create_dir_all(backups)?;
  let flat: String = path.to_string_lossy().chars().map(|c| if matches!(c, '/' | '\\' | ':') { '_' } else { c }).collect();
  let mut dest = backups.join(flat.trim_start_matches('_'));
  let mut n = 1;
  while dest.exists() {
    n += 1;
    dest = backups.join(format!("{}.{n}", flat.trim_start_matches('_')));
  }
  match std::fs::rename(path, &dest) {
    Ok(()) => Ok(dest),
    Err(e) if e.kind() == ErrorKind::CrossesDevices || e.raw_os_error() == Some(18) => {
      copy_tree(path, &dest)?;
      remove_tree(path)?;
      Ok(dest)
    }
    Err(e) => Err(e),
  }
}

/// Move `from` to `to` (creating the parent), across volumes too
pub fn move_to(from: &Path, to: &Path) -> io::Result<()> {
  if let Some(dir) = to.parent() {
    std::fs::create_dir_all(dir)?;
  }
  match std::fs::rename(from, to) {
    Ok(()) => Ok(()),
    Err(e) if e.kind() == ErrorKind::CrossesDevices || e.raw_os_error() == Some(18) => {
      copy_tree(from, to)?;
      remove_tree(from)
    }
    Err(e) => Err(e),
  }
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
  let meta = std::fs::metadata(from)?;
  if meta.is_file() {
    std::fs::copy(from, to)?;
    return Ok(());
  }
  std::fs::create_dir_all(to)?;
  for e in std::fs::read_dir(from)? {
    let e = e?;
    copy_tree(&e.path(), &to.join(e.file_name()))?;
  }
  Ok(())
}

fn remove_tree(p: &Path) -> io::Result<()> {
  if std::fs::symlink_metadata(p)?.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) }
}

/// The `info/exclude` of the repository at `root` (a worktree's `.git` file points at its git dir, whose `commondir`
/// leads to the shared one)
fn exclude_file(root: &Path) -> Option<PathBuf> {
  let dot = root.join(".git");
  let meta = std::fs::metadata(&dot).ok()?;
  let git_dir = if meta.is_dir() {
    dot
  } else {
    let text = std::fs::read_to_string(&dot).ok()?;
    let rel = text.lines().find_map(|l| l.strip_prefix("gitdir:"))?.trim().to_owned();
    let gd = if Path::new(&rel).is_absolute() { PathBuf::from(rel) } else { root.join(rel) };
    match std::fs::read_to_string(gd.join("commondir")) {
      Ok(c) => {
        let c = c.trim();
        if Path::new(c).is_absolute() { PathBuf::from(c) } else { gd.join(c) }
      }
      Err(_) => gd,
    }
  };
  Some(git_dir.join("info").join("exclude"))
}

/// Keep a link Acpira made in a repository out of `git status` without touching the tracked `.gitignore`
pub fn git_exclude(root: &Path, rel: &str) -> io::Result<()> {
  let Some(file) = exclude_file(root) else { return Ok(()) };
  let line = format!("/{}", rel.trim_start_matches('/'));
  let text = std::fs::read_to_string(&file).unwrap_or_default();
  if text.lines().any(|l| l.trim() == line) {
    return Ok(());
  }
  if let Some(dir) = file.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
  std::fs::write(&file, format!("{text}{sep}{line}\n"))
}

pub fn git_unexclude(root: &Path, rel: &str) -> io::Result<()> {
  let Some(file) = exclude_file(root) else { return Ok(()) };
  let line = format!("/{}", rel.trim_start_matches('/'));
  let Ok(text) = std::fs::read_to_string(&file) else { return Ok(()) };
  if !text.lines().any(|l| l.trim() == line) {
    return Ok(());
  }
  let kept: Vec<&str> = text.lines().filter(|l| l.trim() != line).collect();
  std::fs::write(&file, if kept.is_empty() { String::new() } else { format!("{}\n", kept.join("\n")) })
}

/// Whether `text` has an import line for one of `forms` (`@~/.agents/AGENTS.md`, `@/abs/…`)
pub fn has_import(text: &str, forms: &[String]) -> bool {
  text.lines().any(|l| forms.iter().any(|f| l.trim() == f))
}

/// Put `line` on top of the file (creating it); a file that already has it is left alone
pub fn add_import(file: &Path, line: &str) -> io::Result<()> {
  let text = std::fs::read_to_string(file).unwrap_or_default();
  if has_import(&text, &[line.to_owned()]) {
    return Ok(());
  }
  if let Some(dir) = file.parent() {
    std::fs::create_dir_all(dir)?;
  }
  let body = if text.trim().is_empty() { format!("{line}\n") } else { format!("{line}\n\n{text}") };
  std::fs::write(file, body)
}

/// Take the import line out again; a file left empty is removed
pub fn remove_import(file: &Path, line: &str) -> io::Result<()> {
  let Ok(text) = std::fs::read_to_string(file) else { return Ok(()) };
  let mut lines: Vec<&str> = text.lines().collect();
  let Some(i) = lines.iter().position(|l| l.trim() == line) else { return Ok(()) };
  lines.remove(i);
  // The blank line add_import put after it
  if lines.get(i).is_some_and(|l| l.trim().is_empty()) && i == 0 {
    lines.remove(i);
  }
  if lines.iter().all(|l| l.trim().is_empty()) {
    return std::fs::remove_file(file);
  }
  std::fs::write(file, format!("{}\n", lines.join("\n")))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn relative_paths() {
    assert_eq!(relative_to(Path::new("/r/.agents/skills/a"), Path::new("/r/.claude/skills")), PathBuf::from("../../.agents/skills/a"));
    assert_eq!(relative_to(Path::new("/r/x"), Path::new("/r")), PathBuf::from("x"));
  }

  #[cfg(unix)]
  #[test]
  fn inspects_links_copies_and_conflicts() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src/a");
    std::fs::create_dir_all(src.join("scripts")).unwrap();
    std::fs::write(src.join("SKILL.md"), "x").unwrap();
    std::fs::write(src.join("scripts/run.sh"), "echo").unwrap();
    let link = t.path().join("claude/a");
    assert_eq!(inspect(&link, &src), Spot::Absent);
    make_link(&link, &src, true).unwrap();
    assert_eq!(inspect(&link, &src), Spot::Linked);
    assert_eq!(std::fs::read_link(&link).unwrap(), PathBuf::from("../src/a"));
    remove_link(&link).unwrap();
    assert!(src.join("SKILL.md").exists());
    copy_tree(&src, &link).unwrap();
    std::fs::write(link.join(".DS_Store"), "junk").unwrap();
    assert_eq!(inspect(&link, &src), Spot::Same);
    std::fs::write(link.join("SKILL.md"), "y").unwrap();
    assert_eq!(inspect(&link, &src), Spot::Differs);
    let moved = move_aside(&link, &t.path().join("backups")).unwrap();
    assert!(moved.join("SKILL.md").exists() && !link.exists());
  }

  #[test]
  fn import_lines_and_excludes() {
    let t = tempfile::tempdir().unwrap();
    let f = t.path().join("CLAUDE.md");
    add_import(&f, "@~/.agents/AGENTS.md").unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "@~/.agents/AGENTS.md\n");
    remove_import(&f, "@~/.agents/AGENTS.md").unwrap();
    assert!(!f.exists());
    std::fs::write(&f, "# mine\n").unwrap();
    add_import(&f, "@AGENTS.md").unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "@AGENTS.md\n\n# mine\n");
    remove_import(&f, "@AGENTS.md").unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "# mine\n");

    std::fs::create_dir_all(t.path().join(".git/info")).unwrap();
    git_exclude(t.path(), ".claude/skills/a").unwrap();
    git_exclude(t.path(), ".claude/skills/a").unwrap();
    assert_eq!(std::fs::read_to_string(t.path().join(".git/info/exclude")).unwrap(), "/.claude/skills/a\n");
    git_unexclude(t.path(), ".claude/skills/a").unwrap();
    assert_eq!(std::fs::read_to_string(t.path().join(".git/info/exclude")).unwrap(), "");
  }
}
