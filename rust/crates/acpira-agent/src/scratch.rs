//! A session's scratch folder, `<cwd>/.acpira/tmp/<key>/`: TMPDIR, TEMP and TMP of its commands point there, so temp
//! files stay in the project, show which session made them and go with it. `.acpira/tmp/.gitignore` (`*`) keeps the
//! folder out of git without touching the project's own ignore files. Each folder holds an owner file (the session and
//! the data root); deleting and sweeping go by it alone, never by a name that merely looks like a key, so another data
//! root's sessions (the harness, a test home) are never touched. The engine deletes a session's folder with its log
//! (`remove_for_log`); the first use in a project sweeps the folders whose sessions are gone (`sweep`)

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

const DOT: &str = ".acpira";
const TMP: &str = "tmp";
const IGNORE: &str = ".gitignore";
const OWNER: &str = ".owner";
/// The key's length: the session id's first hex digits, short because the whole path counts (`MAX_BYTES`)
const KEY_LEN: usize = 8;
/// The longest scratch path taken. Programs make Unix sockets under TMPDIR, and a socket path holds 104 bytes with its
/// NUL on macOS: Python's multiprocessing adds about 32 (`pymp-…/listener-…`), ssh-agent about 27. A default chat's
/// folder comes to 65
pub const MAX_BYTES: usize = 70;
/// A folder whose session log is gone is kept this long after its last change: a session that just made its folder
/// may not have written its log yet
const ORPHAN_AGE: Duration = Duration::from_secs(24 * 3600);
/// Entries looked at when finding a folder's last change; a bigger folder counts as changed now
const WALK_LIMIT: usize = 5000;

/// The variables a scratch folder is handed in: Rust's temp_dir reads TMP and TEMP on Windows, Unix tools and Git Bash
/// TMPDIR
pub const VARS: [&str; 3] = ["TMPDIR", "TEMP", "TMP"];

/// Where a session's scratch folder goes; None where it takes none: the user's home and a filesystem root (`.acpira`
/// under home is the data root), or a path over `MAX_BYTES`
pub fn place(cwd: &Path, id: &str, user_home: Option<&Path>) -> Option<PathBuf> {
  // A root spelled another way (`/usr/..`, a link to `/`) counts as one too; a folder that cannot be resolved takes none
  if !cwd.is_absolute() || std::fs::canonicalize(cwd).map_or(true, |c| c.parent().is_none()) {
    return None;
  }
  if user_home.is_some_and(|h| same_dir(h, cwd)) {
    return None;
  }
  let path = tmp_root(cwd).join(key(id)?);
  (path.as_os_str().len() <= MAX_BYTES).then_some(path)
}

fn tmp_root(cwd: &Path) -> PathBuf {
  cwd.join(DOT).join(TMP)
}

fn key(id: &str) -> Option<String> {
  let k: String = id.chars().filter(char::is_ascii_hexdigit).take(KEY_LEN).collect::<String>().to_ascii_lowercase();
  (k.len() == KEY_LEN).then_some(k)
}

fn same_dir(a: &Path, b: &Path) -> bool {
  a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// A real directory, not a symbolic link or a Windows junction to one
fn real_dir(p: &Path) -> bool {
  std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_dir())
}

/// The data root as owner files name it
fn home_key(home: &Path) -> String {
  std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf()).to_string_lossy().into_owned()
}

/// The session id an owner file names, when it names this data root. Only an id of hex digits and dashes counts, as it
/// becomes part of a path
fn owner_in(dir: &Path, home: &str) -> Option<String> {
  let v: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(OWNER)).ok()?).ok()?;
  if v.get("home").and_then(Value::as_str) != Some(home) {
    return None;
  }
  v.get("session").and_then(Value::as_str).filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')).map(str::to_owned)
}

/// One session's scratch folder, made on first use (`ensure`)
#[derive(Default)]
pub struct Scratch {
  path: Option<PathBuf>,
  id: String,
  home: PathBuf,
  state: parking_lot::Mutex<State>,
}

#[derive(Default)]
struct State {
  /// Made at least once; a folder gone since (swept, deleted by hand) is made again
  made: bool,
  /// It could not be made: the session runs with the system's temp folder from then on
  failed: bool,
}

impl Scratch {
  /// `path` from `place`, None for a session that takes no folder
  pub fn new(path: Option<PathBuf>, id: &str, home: &Path) -> Scratch {
    Scratch { path, id: id.to_owned(), home: home.to_path_buf(), state: Default::default() }
  }

  /// The folder the prompt names, made or not
  pub fn path(&self) -> Option<&Path> {
    self.path.as_deref()
  }

  /// The folder, made with its ignore and owner files when it is not there; None when the session takes none or it
  /// cannot be made (the command then runs with the system's temp folder). The first success sweeps the project's
  /// other folders on a thread of its own
  pub fn ensure(&self) -> Option<PathBuf> {
    let path = self.path.as_ref()?;
    let mut st = self.state.lock();
    if st.failed {
      return None;
    }
    // Still there and still this session's: a folder swept meanwhile may have been made again by another session
    // whose id starts the same way, which `create` refuses
    if st.made && real_dir(path) && owner_in(path, &home_key(&self.home)).as_deref() == Some(self.id.as_str()) {
      return Some(path.clone());
    }
    match create(path, &self.id, &self.home) {
      Ok(()) => {
        if !st.made {
          let (root, home, own) = (path.parent().map(Path::to_path_buf)?, self.home.clone(), path.clone());
          std::thread::spawn(move || sweep(&root, &home, &own, SystemTime::now()));
        }
        st.made = true;
        Some(path.clone())
      }
      Err(e) => {
        st.failed = true;
        eprintln!("[acpira agent] scratch folder {} not used: {e}", path.display());
        None
      }
    }
  }
}

/// Set the scratch variables on a command about to start
pub fn apply(dir: Option<&Path>, mut set: impl FnMut(&str, &Path)) {
  if let Some(d) = dir {
    for k in VARS {
      set(k, d);
    }
  }
}

fn create(path: &Path, id: &str, home: &Path) -> std::io::Result<()> {
  let root = path.parent().ok_or_else(|| std::io::Error::other("no parent"))?;
  let mut made = false;
  // Twice at most: a sweep in another process may take an emptied `tmp` away between the two steps
  for _ in 0..2 {
    std::fs::create_dir_all(root)?;
    if !real_dir(root) || root.parent().is_none_or(|d| !real_dir(d)) {
      return Err(std::io::Error::other(".acpira/tmp is not a plain folder"));
    }
    match std::fs::create_dir(path) {
      Ok(()) => {
        made = true;
        break;
      }
      Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => break,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
      Err(e) => return Err(e),
    }
  }
  // After the folder, so a sweep that just emptied `tmp` never leaves a folder without it (`prune`)
  write_new(&root.join(IGNORE), "*\n")?;
  let home = home_key(home);
  if made {
    write_new(&path.join(OWNER), &json!({ "session": id, "home": home }).to_string())?;
  } else if !real_dir(path) || owner_in(path, &home).as_deref() != Some(id) {
    return Err(std::io::Error::other("the folder belongs to another session"));
  }
  Ok(())
}

/// Write a file only where none is
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
  match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
    Ok(mut f) => f.write_all(text.as_bytes()),
    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
    Err(e) => Err(e),
  }
}

/// Delete a session's folder in `cwd` when its owner file names this session and data root, then `.acpira` if
/// nothing else is left in it. True when a folder went
pub fn remove(cwd: &Path, id: &str, home: &Path) -> std::io::Result<bool> {
  let Some(k) = key(id) else { return Ok(false) };
  let root = tmp_root(cwd);
  let dir = root.join(k);
  if !real_dir(&cwd.join(DOT)) || !real_dir(&root) || !real_dir(&dir) || owner_in(&dir, &home_key(home)).as_deref() != Some(id) {
    return Ok(false);
  }
  delete(&dir)?;
  prune(&root);
  Ok(true)
}

/// Remove a scratch folder with its owner file last: a failure (a file Windows keeps open, a job writing meanwhile)
/// leaves the owner file in place, so a later sweep can try again
fn delete(dir: &Path) -> std::io::Result<()> {
  let owner = std::fs::read(dir.join(OWNER))?;
  for e in std::fs::read_dir(dir)?.flatten() {
    if e.file_name() == OWNER {
      continue;
    }
    let p = e.path();
    // `file_type` does not follow links: a link goes as itself (a directory link on Windows needs remove_dir)
    if e.file_type().is_ok_and(|t| t.is_dir()) {
      std::fs::remove_dir_all(&p)?;
    } else {
      std::fs::remove_file(&p).or_else(|_| std::fs::remove_dir(&p))?;
    }
  }
  std::fs::remove_file(dir.join(OWNER))?;
  std::fs::remove_dir(dir).inspect_err(|_| {
    let _ = std::fs::write(dir.join(OWNER), &owner);
  })
}

/// `remove` for the session a built-in agent log belongs to, its folder read off the log's header line. Remote sessions
/// made no folder here, so nothing on this disk names them as owner
pub fn remove_for_log(log: &Path, home: &Path) -> std::io::Result<bool> {
  let Some(first) = std::fs::read_to_string(log).ok().and_then(|t| t.lines().next().map(str::to_owned)) else { return Ok(false) };
  let Ok(header) = serde_json::from_str::<Value>(&first) else { return Ok(false) };
  match (header.get("id").and_then(Value::as_str), header.get("cwd").and_then(Value::as_str)) {
    (Some(id), Some(cwd)) if !cwd.is_empty() => remove(Path::new(cwd), id, home),
    _ => Ok(false),
  }
}

/// A `tmp` holding only its ignore file goes, and `.acpira` with it when empty. Should a session make a folder
/// meanwhile, the ignore file comes back
fn prune(root: &Path) {
  let only_ignore = std::fs::read_dir(root).is_ok_and(|rd| rd.flatten().all(|e| e.file_name() == IGNORE));
  if !only_ignore {
    return;
  }
  let _ = std::fs::remove_file(root.join(IGNORE));
  if std::fs::remove_dir(root).is_err() {
    let _ = write_new(&root.join(IGNORE), "*\n");
    return;
  }
  if let Some(dot) = root.parent() {
    let _ = std::fs::remove_dir(dot);
  }
}

/// Delete the folders in `root` (a project's `.acpira/tmp`) that this data root's sessions left: the session's log is
/// gone and the folder quiet for `ORPHAN_AGE` (an edit's fork, a crash, a file Windows kept open). A session whose log
/// is still there may be reopened by another process at any time, so its folder stays however old. `own` is skipped;
/// folders of another data root are never touched
pub fn sweep(root: &Path, home: &Path, own: &Path, now: SystemTime) {
  if !real_dir(root) {
    return;
  }
  let (owner_home, logs) = (home_key(home), crate::acp::sessions_root(home));
  let Ok(rd) = std::fs::read_dir(root) else { return };
  for dir in rd.flatten().map(|e| e.path()).filter(|p| p != own && real_dir(p)) {
    let Some(session) = owner_in(&dir, &owner_home) else { continue };
    if logs.join(format!("{session}.jsonl")).exists() || age(now, newest_change(&dir)) <= ORPHAN_AGE {
      continue;
    }
    if let Err(e) = delete(&dir) {
      eprintln!("[acpira agent] scratch folder {} not swept: {e}", dir.display());
    }
  }
}

fn age(now: SystemTime, then: SystemTime) -> Duration {
  now.duration_since(then).unwrap_or_default()
}

/// The latest change under `dir`, links not followed; a folder too big to walk counts as changed now
fn newest_change(dir: &Path) -> SystemTime {
  let mut newest = std::fs::symlink_metadata(dir).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
  let (mut stack, mut seen) = (vec![dir.to_path_buf()], 0usize);
  while let Some(d) = stack.pop() {
    let Ok(rd) = std::fs::read_dir(&d) else { continue };
    for e in rd.flatten() {
      seen += 1;
      if seen > WALK_LIMIT {
        return SystemTime::now();
      }
      let Ok(m) = std::fs::symlink_metadata(e.path()) else { continue };
      if let Ok(t) = m.modified() {
        newest = newest.max(t);
      }
      if m.file_type().is_dir() {
        stack.push(e.path());
      }
    }
  }
  newest
}

#[cfg(test)]
mod tests {
  use super::*;

  const ID: &str = "0123abcd-1111-2222-3333-444455556666";

  fn tmp() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
  }

  fn project(t: &tempfile::TempDir) -> PathBuf {
    let p = t.path().join("p");
    std::fs::create_dir_all(&p).unwrap();
    p
  }

  /// `place` without the length rule: a system temp dir is long already
  fn at(cwd: &Path, id: &str) -> Option<PathBuf> {
    Some(tmp_root(cwd).join(key(id)?))
  }

  #[cfg(unix)]
  #[test]
  fn takes_no_folder_at_home_at_a_root_or_past_the_length() {
    // Real folders under a short root: `place` resolves the folder to spot a root spelled another way
    let t = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let home = t.path().join("u");
    let dir = |rel: &str| {
      let d = t.path().join(rel);
      std::fs::create_dir_all(&d).unwrap();
      d
    };
    let (app, long, chat) = (dir("u/app"), dir(&"x".repeat(60)), dir(".acpira/chats/2026-11-04-a1b2c3"));
    assert_eq!(place(&home, ID, Some(&home)), None);
    assert_eq!(place(Path::new("/"), ID, Some(&home)), None);
    assert_eq!(place(Path::new("/usr/.."), ID, Some(&home)), None);
    assert_eq!(place(&long, ID, Some(&home)), None);
    assert_eq!(place(&t.path().join("missing"), ID, Some(&home)), None);
    assert_eq!(place(&app, ID, Some(&home)), Some(app.join(".acpira/tmp/0123abcd")));
    // A default chat (`<data root>/chats/<day>-<hex>`) fits
    assert!(place(&chat, ID, Some(&home)).is_some());
  }

  #[test]
  fn makes_the_folder_ignored_and_owned_and_removes_it_without_a_shell() {
    let (t, data) = (tmp(), tmp());
    let cwd = project(&t);
    let s = Scratch::new(at(&cwd, ID), ID, data.path());
    let dir = s.ensure().unwrap();
    assert_eq!(std::fs::read_to_string(cwd.join(".acpira/tmp/.gitignore")).unwrap(), "*\n");
    assert!(dir.join(OWNER).is_file());
    std::fs::write(dir.join("a.txt"), "x").unwrap();
    // Another data root's session with the same id owns nothing here
    assert!(!remove(&cwd, ID, t.path()).unwrap());
    assert!(remove(&cwd, ID, data.path()).unwrap());
    assert!(!cwd.join(".acpira").exists());
  }

  #[test]
  fn a_folder_taken_away_meanwhile_is_made_again() {
    let (t, data) = (tmp(), tmp());
    let cwd = project(&t);
    let s = Scratch::new(at(&cwd, ID), ID, data.path());
    let dir = s.ensure().unwrap();
    assert!(remove(&cwd, ID, data.path()).unwrap());
    assert_eq!(s.ensure(), Some(dir.clone()));
    assert!(dir.join(OWNER).is_file() && cwd.join(".acpira/tmp/.gitignore").is_file());
  }

  #[test]
  fn a_key_taken_by_another_session_is_not_used() {
    let (t, data) = (tmp(), tmp());
    let cwd = project(&t);
    let other = "0123abcd-9999-9999-9999-999999999999";
    assert!(Scratch::new(at(&cwd, other), other, data.path()).ensure().is_some());
    assert!(Scratch::new(at(&cwd, ID), ID, data.path()).ensure().is_none());
    assert!(!remove(&cwd, ID, data.path()).unwrap());
  }

  #[cfg(unix)]
  #[test]
  fn a_linked_tmp_is_neither_used_nor_removed() {
    let (t, data, elsewhere) = (tmp(), tmp(), tmp());
    let cwd = project(&t);
    std::fs::create_dir_all(cwd.join(".acpira")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), cwd.join(".acpira/tmp")).unwrap();
    assert!(Scratch::new(at(&cwd, ID), ID, data.path()).ensure().is_none());
    std::fs::create_dir_all(elsewhere.path().join("0123abcd")).unwrap();
    assert!(!remove(&cwd, ID, data.path()).unwrap());
    assert!(elsewhere.path().join("0123abcd").exists());
  }

  #[test]
  fn sweeps_only_this_roots_folders_whose_sessions_are_gone() {
    let (t, data, harness) = (tmp(), tmp(), tmp());
    let cwd = project(&t);
    let ids = ["aaaaaaaa-0000-0000-0000-000000000001", "bbbbbbbb-0000-0000-0000-000000000002", "cccccccc-0000-0000-0000-000000000003"];
    let logs = crate::acp::sessions_root(data.path());
    std::fs::create_dir_all(&logs).unwrap();
    // The first one's session is gone, the second one's still has its log, the third is the harness's
    std::fs::write(logs.join(format!("{}.jsonl", ids[1])), "{}").unwrap();
    let dirs: Vec<PathBuf> = ids
      .iter()
      .zip([data.path(), data.path(), harness.path()])
      .map(|(id, home)| Scratch::new(at(&cwd, id), id, home).ensure().unwrap())
      .collect();
    let own = cwd.join(".acpira/tmp/dddddddd");
    let later = SystemTime::now() + ORPHAN_AGE + Duration::from_secs(60);
    sweep(&cwd.join(".acpira/tmp"), data.path(), &own, later);
    assert_eq!(dirs.iter().map(|d| d.exists()).collect::<Vec<_>>(), [false, true, true]);
    // Within a day of its last change even a folder without a log stays
    let fresh = Scratch::new(at(&cwd, ids[0]), ids[0], data.path());
    let dir = fresh.ensure().unwrap();
    sweep(&cwd.join(".acpira/tmp"), data.path(), &own, SystemTime::now());
    assert!(dir.exists());
  }

  #[test]
  fn removes_by_a_logs_header() {
    let (t, data) = (tmp(), tmp());
    let cwd = project(&t);
    Scratch::new(at(&cwd, ID), ID, data.path()).ensure().unwrap();
    let log = data.path().join("s.jsonl");
    std::fs::write(&log, format!("{}\n{{}}\n", json!({ "type": "session", "id": ID, "cwd": cwd }))).unwrap();
    assert!(remove_for_log(&log, data.path()).unwrap());
    assert!(!cwd.join(".acpira").exists());
  }
}
