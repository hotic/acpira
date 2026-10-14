//! The Storage tab's file-system side: what the data root holds by owner (`scan`), the leftovers nothing needs any more
//! and their removal (`clean`). Blocking std::fs throughout, run on the blocking pool. Sessions themselves are never
//! deleted here: that goes through `SessionManager` (`session_manager/storage.rs`), with its leases and undo window.
//!
//! Ownership follows what the records reference, never what a file name looks like: a file the rules cannot place is
//! counted under "other" rather than offered for cleaning

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::Deserialize;
use serde_json::Value;

use acpira_shared::storage::{StorageEntry, StorageResidue, StorageResidueKind, StorageSegment, StorageSegmentKind, StorageView};
use acpira_shared::transcript::SessionSummary;

use super::record::SessionRecord;
use super::transcript_store::is_session_id;
use crate::acp::agents::registry::SELF_AGENT_ID;

/// Younger than this, an unreferenced agent log or attachment may belong to a session whose record is still on its way
/// (another window's first write, a prompt being staged), and a stopped engine's log may be wanted for a crash report
pub const FRESH: Duration = Duration::from_secs(24 * 3600);
/// Shared tab backups are kept at least this long, and always while the ledger can still put one back
pub const BACKUP_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
/// The extension's globalStorage folder name (`publisher.name`), the only legacy folder ever removed
const LEGACY_DIR: &str = "hotic.acpira";

pub struct ScanInput {
  pub root: PathBuf,
  /// The transcript store's directory, `<root>/sessions` outside tests
  pub sessions_dir: PathBuf,
  /// The reconciled session list
  pub sessions: Vec<SessionSummary>,
  pub now: SystemTime,
}

/// What one session takes on disk
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SessionUsage {
  pub record: u64,
  pub blobs: u64,
  /// The built-in agent's log and folder
  pub log: u64,
}

impl SessionUsage {
  pub fn total(&self) -> u64 {
    self.record + self.blobs + self.log
  }
}

/// A leftover and the paths removing it takes
#[derive(Debug, Clone)]
pub struct Residue {
  pub item: StorageResidue,
  pub targets: Vec<PathBuf>,
  /// An unreadable record goes through the transcript store (it remembers unreadable files)
  pub session: Option<String>,
}

pub struct Scan {
  pub view: StorageView,
  pub usage: HashMap<String, SessionUsage>,
  pub residue: Vec<Residue>,
}

#[derive(Debug, Default, Clone, Copy)]
struct Du {
  bytes: u64,
  newest: Option<SystemTime>,
}

impl Du {
  fn add(&mut self, o: Du) {
    self.bytes += o.bytes;
    self.newest = self.newest.max(o.newest);
  }
}

/// Size and newest file mtime of a path (a folder's own mtime is left out: it moves whenever an entry is added or
/// removed). Symlinks count as themselves and are never followed: a link out of the root must not pull a whole other
/// tree into the numbers, and is never followed by a removal either
fn du(path: &Path) -> Du {
  let Ok(meta) = std::fs::symlink_metadata(path) else { return Du::default() };
  let mut out = if meta.is_dir() { Du::default() } else { Du { bytes: meta.len(), newest: meta.modified().ok() } };
  if meta.is_dir()
    && let Ok(rd) = std::fs::read_dir(path)
  {
    for e in rd.flatten() {
      out.add(du(&e.path()));
    }
  }
  out
}

fn older(d: &Du, now: SystemTime, age: Duration) -> bool {
  d.newest.and_then(|m| now.duration_since(m).ok()).is_some_and(|a| a >= age)
}

fn iso(t: Option<SystemTime>) -> String {
  let ms = t.and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
  acpira_shared::time::iso_of_ms(ms)
}

fn kind_tag(k: StorageResidueKind) -> &'static str {
  match k {
    StorageResidueKind::AgentLog => "agentLog",
    StorageResidueKind::EngineLog => "engineLog",
    StorageResidueKind::Blob => "blob",
    StorageResidueKind::Unreadable => "unreadable",
    StorageResidueKind::Backup => "backup",
    StorageResidueKind::Legacy => "legacy",
  }
}

fn leftover(kind: StorageResidueKind, shown: &Path, d: Du, targets: Vec<PathBuf>, session: Option<String>) -> Residue {
  let path = shown.to_string_lossy().into_owned();
  let preselected = matches!(kind, StorageResidueKind::AgentLog | StorageResidueKind::EngineLog);
  Residue {
    item: StorageResidue { id: format!("{}:{path}", kind_tag(kind)), kind, path, bytes: d.bytes, modified_at: iso(d.newest), preselected },
    targets,
    session,
  }
}

fn names(dir: &Path) -> Vec<(String, PathBuf)> {
  let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
  let mut out: Vec<(String, PathBuf)> = rd.flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), e.path())).collect();
  out.sort();
  out
}

fn is_dir(p: &Path) -> bool {
  std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// The built-in agent's session id a record file drives, read without building the transcript
fn builtin_session_of(path: &Path) -> Option<String> {
  #[derive(Deserialize)]
  #[serde(rename_all = "camelCase")]
  struct Owner {
    #[serde(default)]
    agent: String,
    #[serde(default)]
    acp_session_id: Option<String>,
  }
  let owner: Owner = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
  owner.acp_session_id.filter(|_| owner.agent == SELF_AGENT_ID)
}

#[derive(Default)]
struct Tally {
  segments: HashMap<StorageSegmentKind, (u64, u64)>,
  other: HashMap<String, (PathBuf, u64)>,
}

impl Tally {
  fn seg(&mut self, k: StorageSegmentKind, bytes: u64, count: u64) {
    let e = self.segments.entry(k).or_default();
    e.0 += bytes;
    e.1 += count;
  }
  fn other(&mut self, name: &str, path: &Path, bytes: u64) {
    self.other.entry(name.to_owned()).or_insert_with(|| (path.to_path_buf(), 0)).1 += bytes;
  }
}

pub fn scan(input: &ScanInput) -> Scan {
  let root = &input.root;
  let now = input.now;
  let mut t = Tally::default();
  let mut usage: HashMap<String, SessionUsage> = HashMap::new();
  let mut residue = vec![];
  let listed: HashSet<&str> = input.sessions.iter().filter(|s| s.external.is_none()).map(|s| s.id.as_str()).collect();

  // Sessions: records, their blob folders, unreadable records, unreferenced attachments
  let sessions_dir = &input.sessions_dir;
  let mut has_record: HashSet<String> = HashSet::new();
  let mut unreadable: HashSet<String> = HashSet::new();
  let entries = names(sessions_dir);
  for (name, path) in &entries {
    let Some(id) = name.strip_suffix(".json").filter(|id| is_session_id(id)) else { continue };
    if is_dir(path) {
      continue;
    }
    let size = du(path);
    // A record the list does not know is either one another window just wrote or one that does not parse
    let readable = listed.contains(id)
      || std::fs::read(path).ok().is_none_or(|raw| serde_json::from_slice::<SessionRecord>(&raw).is_ok_and(|r| r.id == id));
    if readable {
      has_record.insert(id.to_owned());
      usage.entry(id.to_owned()).or_default().record = size.bytes;
      t.seg(StorageSegmentKind::Sessions, size.bytes, 1);
    } else {
      unreadable.insert(id.to_owned());
      let mut d = size;
      let mut targets = vec![path.clone()];
      let blobs = sessions_dir.join(id);
      if is_dir(&blobs) {
        d.add(du(&blobs));
        targets.push(blobs);
      }
      residue.push(leftover(StorageResidueKind::Unreadable, path, d, targets, Some(id.to_owned())));
    }
  }
  for (name, path) in &entries {
    if name.ends_with(".json") && !is_dir(path) && name.strip_suffix(".json").is_some_and(is_session_id) {
      continue;
    }
    if unreadable.contains(name) {
      // Part of that record's leftover already
      continue;
    }
    if !is_dir(path) || !is_session_id(name) {
      // index.json, prefs.json, categories.json, trash/, temp and lock files
      t.other("sessions", sessions_dir, du(path).bytes);
      continue;
    }
    if has_record.contains(name) {
      let raw = std::fs::read(sessions_dir.join(format!("{name}.json"))).unwrap_or_default();
      let text = String::from_utf8_lossy(&raw);
      let mut kept = 0;
      let (mut orphan, mut targets) = (Du::default(), vec![]);
      for (blob, file) in names(path) {
        let d = du(&file);
        // Any mention keeps it: a blob is named by its content hash, so its name in the record is the reference
        if is_dir(&file) || !older(&d, now, FRESH) || text.contains(blob.as_str()) {
          kept += d.bytes;
        } else {
          orphan.add(d);
          targets.push(file);
        }
      }
      usage.entry(name.clone()).or_default().blobs = kept;
      t.seg(StorageSegmentKind::Attachments, kept, 0);
      if !targets.is_empty() {
        residue.push(leftover(StorageResidueKind::Blob, path, orphan, targets, None));
      }
    } else {
      let d = du(path);
      if older(&d, now, FRESH) {
        residue.push(leftover(StorageResidueKind::Blob, path, d, vec![path.clone()], None));
      } else {
        t.seg(StorageSegmentKind::Attachments, d.bytes, 0);
      }
    }
  }

  // The built-in agent's logs: owned by the record that drives the agent session
  let mut drives: HashMap<String, String> = HashMap::new();
  for s in &input.sessions {
    if s.agent == SELF_AGENT_ID
      && let Some(acp) = &s.acp_session_id
    {
      drives.entry(acp.clone()).or_insert_with(|| s.id.clone());
    }
  }
  let trash_drives: HashSet<String> =
    names(&sessions_dir.join("trash")).into_iter().filter(|(n, _)| n.ends_with(".json")).filter_map(|(_, p)| builtin_session_of(&p)).collect();
  let agent_root = acpira_agent::acp::sessions_root(root);
  let mut logs: HashMap<String, (Du, Vec<PathBuf>)> = HashMap::new();
  for (name, path) in names(&agent_root) {
    let id = if is_dir(&path) { Some(name.as_str()) } else { name.strip_suffix(".jsonl") };
    match id.filter(|id| is_session_id(id)) {
      Some(id) => {
        let e = logs.entry(id.to_owned()).or_default();
        e.0.add(du(&path));
        e.1.push(path);
      }
      None => t.other("agent", &root.join("agent"), du(&path).bytes),
    }
  }
  let mut logs: Vec<_> = logs.into_iter().collect();
  logs.sort_by(|a, b| a.0.cmp(&b.0));
  for (acp, (d, targets)) in logs {
    if let Some(owner) = drives.get(&acp) {
      usage.entry(owner.clone()).or_default().log += d.bytes;
      t.seg(StorageSegmentKind::AgentLogs, d.bytes, 0);
    } else if trash_drives.contains(&acp) || !older(&d, now, FRESH) {
      t.seg(StorageSegmentKind::AgentLogs, d.bytes, 0);
    } else {
      let shown = targets.iter().find(|p| !is_dir(p)).cloned().unwrap_or_else(|| targets[0].clone());
      residue.push(leftover(StorageResidueKind::AgentLog, &shown, d, targets, None));
    }
  }

  // Engine logs: one per workspace and engine build, kept by rotation only
  let run = root.join("run");
  for (name, path) in names(&run) {
    let Some(key) = name.strip_prefix("engine-").and_then(|n| n.strip_suffix(".log")) else {
      // The rotated `.log.1` goes with its log; locks, sockets and leases stay
      if !(name.starts_with("engine-") && name.ends_with(".log.1") && path.with_extension("").exists()) {
        t.other("run", &run, du(&path).bytes);
      }
      continue;
    };
    let mut d = du(&path);
    let mut targets = vec![path.clone()];
    let rotated = run.join(format!("{name}.1"));
    if rotated.exists() {
      d.add(du(&rotated));
      targets.push(rotated);
    }
    if !engine_running(&run.join(format!("engine-{key}.lock"))) && older(&d, now, FRESH) {
      residue.push(leftover(StorageResidueKind::EngineLog, &path, d, targets, None));
    } else {
      t.other("run", &run, d.bytes);
    }
  }

  // Shared tab backups: past the age and with nothing in the ledger pointing into them
  let backups = root.join("backups");
  let shared_backups = backups.join("shared");
  let restorable = ledger_backups(root);
  for (_, path) in names(&backups) {
    if path != shared_backups {
      t.other("backups", &backups, du(&path).bytes);
    }
  }
  for (name, path) in names(&shared_backups) {
    let mut d = du(&path);
    // Moved-aside files keep their own mtime; the folder is named after the moment of the backup
    d.newest = backup_time(&name).or_else(|| std::fs::symlink_metadata(&path).ok().and_then(|m| m.modified().ok()));
    let pinned = restorable.iter().any(|b| b.starts_with(&path));
    if is_dir(&path) && !pinned && older(&d, now, BACKUP_AGE) {
      residue.push(leftover(StorageResidueKind::Backup, &path, d, vec![path.clone()], None));
    } else {
      t.other("backups", &backups, d.bytes);
    }
  }

  // Everything else at the top level; the agent folder beyond its sessions (the built-in agent's own settings)
  for (name, path) in names(root) {
    if path == *sessions_dir {
      continue;
    }
    match name.as_str() {
      "run" | "backups" => {}
      "agent" => {
        for (_, p) in names(&path).into_iter().filter(|(_, p)| *p != agent_root) {
          t.other("agent", &path, du(&p).bytes);
        }
      }
      _ => t.other(&name, &path, du(&path).bytes),
    }
  }

  // The legacy globalStorage copies a migration merged and left in place: outside the root, listed by `.migrated`
  for dir in legacy_sources(root) {
    let d = du(&dir);
    residue.push(leftover(StorageResidueKind::Legacy, &dir, d, vec![dir.clone()], None));
  }

  for r in &residue {
    t.seg(StorageSegmentKind::Reclaimable, r.item.bytes, 1);
  }
  let mut other: Vec<StorageEntry> =
    t.other.iter().map(|(name, (path, bytes))| StorageEntry { name: name.clone(), path: path.to_string_lossy().into_owned(), bytes: *bytes }).collect();
  other.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
  t.seg(StorageSegmentKind::Other, other.iter().map(|e| e.bytes).sum(), other.len() as u64);
  // Sessions count their records; attachments and agent logs count the sessions that have some
  let with = |f: fn(&SessionUsage) -> u64| usage.values().filter(|u| f(u) > 0).count() as u64;
  let (blobs_n, logs_n) = (with(|u| u.blobs), with(|u| u.log));
  let segments: Vec<StorageSegment> = [
    StorageSegmentKind::Sessions,
    StorageSegmentKind::Attachments,
    StorageSegmentKind::AgentLogs,
    StorageSegmentKind::Reclaimable,
    StorageSegmentKind::Other,
  ]
  .into_iter()
  .map(|kind| {
    let (bytes, count) = t.segments.get(&kind).copied().unwrap_or_default();
    let count = match kind {
      StorageSegmentKind::Attachments => blobs_n,
      StorageSegmentKind::AgentLogs => logs_n,
      _ => count,
    };
    StorageSegment { kind, bytes, count }
  })
  .collect();
  residue.sort_by(|a, b| b.item.bytes.cmp(&a.item.bytes).then_with(|| a.item.id.cmp(&b.item.id)));
  let view = StorageView {
    root: crate::platform::paths::for_cli(root.to_path_buf()).to_string_lossy().into_owned(),
    total: segments.iter().map(|s| s.bytes).sum(),
    segments,
    other,
    scanned_at: iso(Some(now)),
    sessions: None,
    residue: None,
  };
  Scan { view, usage, residue }
}

/// An engine serving a socket holds `engine-<key>.lock` for its whole life (`acpira serve`); a lock nobody holds, or
/// none at all, is an engine that has stopped. The probe takes the lock for an instant only
fn engine_running(lock: &Path) -> bool {
  let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(lock) else { return false };
  match f.try_lock() {
    Ok(()) => {
      let _ = f.unlock();
      false
    }
    Err(std::fs::TryLockError::WouldBlock) => true,
    // Unknown: leave its log alone
    Err(std::fs::TryLockError::Error(_)) => true,
  }
}

/// `backups/shared/<stamp>`: the ISO time with every separator turned into `-` (`2026-09-30T22-38-51-717Z`)
fn backup_time(name: &str) -> Option<SystemTime> {
  let b = name.as_bytes();
  if b.len() < 24 || !name.is_ascii() || b[10] != b'T' {
    return None;
  }
  let iso = format!("{}T{}:{}:{}.{}Z", &name[..10], &name[11..13], &name[14..16], &name[17..19], &name[20..23]);
  let ms = acpira_shared::time::ms_of_iso(&iso)?;
  Some(SystemTime::UNIX_EPOCH + Duration::from_millis(u64::try_from(ms).ok()?))
}

/// Every path the Shared tab's ledger can still put back (`shared-links.json` entries' `backup`)
fn ledger_backups(root: &Path) -> Vec<PathBuf> {
  fn walk(v: &Value, out: &mut Vec<PathBuf>) {
    match v {
      Value::Object(m) => {
        for (k, v) in m {
          match (k.as_str(), v) {
            ("backup", Value::String(p)) => out.push(PathBuf::from(p)),
            _ => walk(v, out),
          }
        }
      }
      Value::Array(a) => a.iter().for_each(|v| walk(v, out)),
      _ => {}
    }
  }
  let mut out = vec![];
  if let Ok(raw) = std::fs::read(root.join("shared-links.json"))
    && let Ok(v) = serde_json::from_slice::<Value>(&raw)
  {
    walk(&v, &mut out);
  }
  let canon: Vec<PathBuf> = out.iter().filter_map(|p| std::fs::canonicalize(p).ok()).collect();
  out.extend(canon);
  out
}

/// The `.migrated` sources still on disk that are the extension's own globalStorage folder
fn legacy_sources(root: &Path) -> Vec<PathBuf> {
  let Ok(raw) = std::fs::read(root.join(".migrated")) else { return vec![] };
  let Ok(v) = serde_json::from_slice::<Value>(&raw) else { return vec![] };
  let Some(list) = v.get("sources").and_then(Value::as_array) else { return vec![] };
  let mut out: Vec<PathBuf> = list.iter().filter_map(Value::as_str).map(PathBuf::from).filter(|p| is_legacy_store(p, root)).collect();
  out.sort();
  out.dedup();
  out
}

fn is_legacy_store(p: &Path, root: &Path) -> bool {
  let named = p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(LEGACY_DIR));
  let parent = p.parent().and_then(Path::file_name).is_some_and(|n| n == "globalStorage");
  // A real folder (not a link), and never the data root or anything holding it
  named && parent && p.is_absolute() && is_dir(p) && !root.starts_with(p)
}

/// Remove the given leftovers as the scan found them. Each target must still sit under the data root (after following
/// links), except a legacy folder, which `scan` only lists once it passed `is_legacy_store`. Returns the bytes freed and
/// the error per id that failed
pub fn clean(root: &Path, residue: &[&Residue]) -> (u64, Vec<(String, String)>) {
  let base = std::fs::canonicalize(root).ok();
  let mut freed = 0;
  let mut failed = vec![];
  for r in residue {
    let mut error = None;
    for target in &r.targets {
      let inside = match r.item.kind {
        StorageResidueKind::Legacy => is_legacy_store(target, root),
        _ => confined(base.as_deref(), target),
      };
      if !inside {
        error = Some(format!("{} is outside {}", target.display(), root.display()));
        break;
      }
      let res = match std::fs::symlink_metadata(target) {
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(target),
        Ok(_) => std::fs::remove_file(target),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
      };
      if let Err(e) = res {
        error = Some(format!("{}: {e}", target.display()));
        break;
      }
    }
    match error {
      None => freed += r.item.bytes,
      Some(e) => failed.push((r.item.id.clone(), e)),
    }
  }
  (freed, failed)
}

/// `target` resolves (its parent, for the link itself) under the canonical root
fn confined(base: Option<&Path>, target: &Path) -> bool {
  let Some(base) = base else { return false };
  let (Some(parent), Some(name)) = (target.parent(), target.file_name()) else { return false };
  let Ok(parent) = std::fs::canonicalize(parent) else { return false };
  let resolved = parent.join(name);
  resolved != base && resolved.starts_with(base)
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::storage::StorageSegmentKind as K;
  use std::fs;

  const OLD: Duration = Duration::from_secs(40 * 24 * 3600);

  fn put(p: &Path, bytes: usize, old: bool) {
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, vec![b'x'; bytes]).unwrap();
    if old {
      fs::File::options().write(true).open(p).unwrap().set_modified(SystemTime::now() - OLD).unwrap();
    }
  }

  fn record(root: &Path, id: &str, agent: &str, acp: Option<&str>, body: &str) -> SessionSummary {
    let mut v = serde_json::json!({ "id": id, "agent": agent, "cwd": "/w", "title": id, "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z", "turns": [], "controls": { "modes": [], "options": [] }, "commands": [], "note": body });
    if let Some(a) = acp {
      v["acpSessionId"] = a.into();
    }
    let p = root.join("sessions").join(format!("{id}.json"));
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, serde_json::to_vec(&v).unwrap()).unwrap();
    serde_json::from_value::<SessionRecord>(v).unwrap().summary()
  }

  fn seg(s: &Scan, k: K) -> (u64, u64) {
    let x = s.view.segments.iter().find(|x| x.kind == k).unwrap();
    (x.bytes, x.count)
  }

  #[test]
  fn files_are_placed_by_what_the_records_reference() {
    let t = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let root = t.path();
    let s = root.join("sessions");
    let a = record(root, "a", "grok", None, "see 1111111111111111.png");
    let b = record(root, "b", SELF_AGENT_ID, Some("acp-b"), "");
    let (ra, rb) = (fs::metadata(s.join("a.json")).unwrap().len(), fs::metadata(s.join("b.json")).unwrap().len());
    put(&s.join("a/1111111111111111.png"), 100, true);
    put(&s.join("a/2222222222222222.png"), 30, true);
    // Too fresh to call unreferenced: its record may still be on the way
    put(&s.join("a/3333333333333333.png"), 7, false);
    put(&s.join("bad.json"), 50, true);
    put(&s.join("bad/4444444444444444.png"), 5, true);
    put(&s.join("ghost/5555555555555555.png"), 11, true);
    put(&s.join("index.json"), 3, false);
    put(&s.join("a.json.99999999.tmp"), 4, true);
    let agent = acpira_agent::acp::sessions_root(root);
    put(&agent.join("acp-b.jsonl"), 40, true);
    put(&agent.join("acp-b/plan.md"), 2, true);
    put(&agent.join("acp-old.jsonl"), 60, true);
    put(&agent.join("acp-old/outputs/1.txt"), 6, true);
    put(&agent.join("acp-new.jsonl"), 9, false);
    put(&root.join("agent/settings.json"), 8, false);
    put(&root.join("run/engine-k1.log"), 70, true);
    put(&root.join("run/engine-k1.log.1"), 10, true);
    put(&root.join("run/engine-k1.lock"), 0, true);
    put(&root.join("run/engine-k2.log"), 20, true);
    put(&root.join("run/engine-k2.lock"), 0, true);
    let held = fs::File::options().read(true).write(true).open(root.join("run/engine-k2.lock")).unwrap();
    held.lock().unwrap();
    let kept = root.join("backups/shared/2026-01-01T00-00-00-000Z");
    put(&kept.join("x"), 12, false);
    put(&root.join("backups/shared/2026-01-02T00-00-00-000Z/y"), 13, false);
    put(&root.join("backups/pre.tar.gz"), 14, true);
    fs::write(root.join("shared-links.json"), serde_json::to_vec(&serde_json::json!({ "entries": [{ "path": "/p", "target": "/t", "kind": "skill", "backup": kept.join("x") }] })).unwrap()).unwrap();
    put(&root.join("exports/e.md"), 15, false);
    put(&elsewhere.path().join("big"), 10_000, true);
    #[cfg(unix)]
    std::os::unix::fs::symlink(elsewhere.path(), root.join("link")).unwrap();
    let legacy = elsewhere.path().join("Code/User/globalStorage/hotic.acpira");
    put(&legacy.join("sessions/old.json"), 16, true);
    let not_ours = elsewhere.path().join("Code/User/globalStorage/other.ext");
    put(&not_ours.join("f"), 1, true);
    fs::write(root.join(".migrated"), serde_json::to_vec(&serde_json::json!({ "sources": [legacy, not_ours, root] })).unwrap()).unwrap();

    let scan = scan(&ScanInput { root: root.to_path_buf(), sessions_dir: s.clone(), sessions: vec![a, b], now: SystemTime::now() });
    assert_eq!(seg(&scan, K::Sessions), (ra + rb, 2));
    assert_eq!(seg(&scan, K::Attachments), (107, 1));
    // b's log, plus the fresh one no record names yet
    assert_eq!(seg(&scan, K::AgentLogs), (42 + 9, 1));
    assert_eq!(scan.usage["a"], SessionUsage { record: ra, blobs: 107, log: 0 });
    assert_eq!(scan.usage["b"].log, 42);
    let kinds: Vec<(StorageResidueKind, u64, bool)> = scan.residue.iter().map(|r| (r.item.kind, r.item.bytes, r.item.preselected)).collect();
    let has = |k: StorageResidueKind, bytes: u64, pre: bool| assert!(kinds.contains(&(k, bytes, pre)), "{k:?} {bytes} in {kinds:?}");
    has(StorageResidueKind::EngineLog, 80, true);
    has(StorageResidueKind::AgentLog, 66, true);
    has(StorageResidueKind::Unreadable, 55, false);
    has(StorageResidueKind::Blob, 30, false);
    has(StorageResidueKind::Blob, 11, false);
    has(StorageResidueKind::Backup, 13, false);
    has(StorageResidueKind::Legacy, 16, false);
    assert_eq!(kinds.len(), 7, "{kinds:?}");
    assert_eq!(seg(&scan, K::Reclaimable), (80 + 66 + 55 + 30 + 11 + 13 + 16, 7));
    let other = |name: &str| scan.view.other.iter().find(|e| e.name == name).map(|e| e.bytes);
    assert_eq!(other("sessions"), Some(7));
    assert_eq!(other("run"), Some(20));
    assert_eq!(other("backups"), Some(12 + 14));
    assert_eq!(other("agent"), Some(8));
    assert_eq!(other("exports"), Some(15));
    #[cfg(unix)]
    assert!(other("link").unwrap() < 1000, "a link is not followed");
    let parts: u64 = scan.view.segments.iter().map(|s| s.bytes).sum();
    assert_eq!(scan.view.total, parts);

    // Cleaning removes exactly the picked items, the referenced attachment and the live engine's log stay
    let picked: Vec<&Residue> = scan.residue.iter().filter(|r| r.item.kind != StorageResidueKind::Unreadable).collect();
    let (freed, failed) = clean(root, &picked);
    assert!(failed.is_empty(), "{failed:?}");
    assert_eq!(freed, 80 + 66 + 30 + 11 + 13 + 16);
    assert!(!root.join("run/engine-k1.log").exists() && !root.join("run/engine-k1.log.1").exists());
    assert!(root.join("run/engine-k1.lock").exists() && root.join("run/engine-k2.log").exists());
    assert!(!agent.join("acp-old.jsonl").exists() && !agent.join("acp-old").exists() && agent.join("acp-b.jsonl").exists());
    assert!(s.join("a/1111111111111111.png").exists() && s.join("a/3333333333333333.png").exists());
    assert!(!s.join("a/2222222222222222.png").exists() && !s.join("ghost").exists());
    assert!(kept.exists() && !legacy.exists() && not_ours.exists());
    assert!(elsewhere.path().join("big").exists());
    drop(held);
  }

  #[test]
  fn a_target_outside_the_root_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    put(&out.path().join("keep"), 1, true);
    let r = leftover(StorageResidueKind::Blob, &out.path().join("keep"), Du { bytes: 1, newest: None }, vec![out.path().join("keep")], None);
    let (freed, failed) = clean(t.path(), &[&r]);
    assert_eq!((freed, failed.len()), (0, 1));
    assert!(out.path().join("keep").exists());
    #[cfg(unix)]
    {
      // Through a link inside the root: the link itself is what resolves inside, its target is never touched
      std::os::unix::fs::symlink(out.path(), t.path().join("l")).unwrap();
      let via = leftover(StorageResidueKind::Blob, &t.path().join("l/keep"), Du::default(), vec![t.path().join("l/keep")], None);
      let (_, failed) = clean(t.path(), &[&via]);
      assert_eq!(failed.len(), 1);
      assert!(out.path().join("keep").exists());
      let link = leftover(StorageResidueKind::Blob, &t.path().join("l"), Du::default(), vec![t.path().join("l")], None);
      let (_, failed) = clean(t.path(), &[&link]);
      assert!(failed.is_empty());
      assert!(!t.path().join("l").exists() && out.path().join("keep").exists());
    }
  }

  #[test]
  fn backup_stamps_read_as_times() {
    let t = backup_time("2026-09-30T22-38-51-717Z").unwrap();
    assert_eq!(iso(Some(t)), "2026-09-30T22:38:51.717Z");
    assert!(backup_time("pre-shared-trial").is_none());
  }
}
