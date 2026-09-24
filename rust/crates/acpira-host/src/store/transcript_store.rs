//! Session persistence: `<dir>/index.json` caches the summary list,
//! `<dir>/prefs.json` the per-agent memory, `<dir>/<id>.json` the full record, `<dir>/<id>/` its blobs, `<dir>/trash/`
//! the soft-deleted ones during their undo window. The directory is shared by every host, so the index is reconciled
//! with the record files before every write, and every file is written tmp + rename. Record writes are debounced per
//! session and serialized from the live session at write time

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::fs;

use acpira_shared::model_shapes::ModelShapes;
use acpira_shared::transcript::{AgentId, SessionSummary, TurnSettings};

use super::file_lock::{with_file_lock, write_atomic};
use super::record::{RecordSource, SessionRecord};
use crate::i18n::tp;

const META_FILES: [&str; 2] = ["index.json", "prefs.json"];
const TRASH_DIR: &str = "trash";
const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);
const SAVE_MAX_WAIT: Duration = Duration::from_millis(2000);

/// Session ids name files and directories under the store root: plain tokens only
pub fn is_session_id(id: &str) -> bool {
  !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && !matches!(id, "index" | "prefs" | "trash")
}

fn is_blob_name(name: &str) -> bool {
  let Some((stem, ext)) = name.rsplit_once('.') else { return false };
  !stem.is_empty()
    && !ext.is_empty()
    && stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    && ext.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Content-derived blob name: identical payloads share one file, and a transcript can reference a blob before its write lands
pub fn blob_name(ext: &str, bytes: &[u8]) -> String {
  let digest = Sha256::digest(bytes);
  let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
  format!("{hex}{ext}")
}

/// Cross-session memory that is not a setting; unknown keys another build wrote are kept
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPrefs {
  #[serde(default)]
  pub last_settings: BTreeMap<AgentId, TurnSettings>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model_shapes: Option<BTreeMap<AgentId, ModelShapes>>,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

pub type SaveErrorHook = Arc<dyn Fn(&str, &str) + Send + Sync>;
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

struct Pending {
  source: Arc<dyn RecordSource>,
  due: Instant,
  deadline: Instant,
}

struct State {
  pending: HashMap<String, Pending>,
  known: HashSet<String>,
}

pub struct TranscriptStore {
  dir: PathBuf,
  log: LogFn,
  on_save_error: Option<SaveErrorHook>,
  debounce: Duration,
  max_wait: Duration,
  state: parking_lot::Mutex<State>,
  // One write chain per id: concurrent writes of a record would share its temp file
  inflight: parking_lot::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
  me: Weak<TranscriptStore>,
}

impl TranscriptStore {
  pub fn new(dir: PathBuf, log: LogFn, on_save_error: Option<SaveErrorHook>) -> Arc<Self> {
    Self::with_timing(dir, log, on_save_error, SAVE_DEBOUNCE, SAVE_MAX_WAIT)
  }

  pub fn with_timing(dir: PathBuf, log: LogFn, on_save_error: Option<SaveErrorHook>, debounce: Duration, max_wait: Duration) -> Arc<Self> {
    Arc::new_cyclic(|me| TranscriptStore {
      dir,
      log,
      on_save_error,
      debounce,
      max_wait,
      state: parking_lot::Mutex::new(State { pending: HashMap::new(), known: HashSet::new() }),
      inflight: Default::default(),
      me: me.clone(),
    })
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }

  async fn ensure(&self) -> Result<()> {
    fs::create_dir_all(&self.dir).await?;
    Ok(())
  }

  /// The list as the disk knows it
  pub async fn load_index(&self) -> Result<Vec<SessionSummary>> {
    self.sync_index(&[], &HashSet::new()).await
  }

  pub async fn load_prefs(&self) -> SessionPrefs {
    match fs::read(self.dir.join("prefs.json")).await {
      Ok(raw) => serde_json::from_slice(&raw).unwrap_or_default(),
      Err(_) => SessionPrefs::default(),
    }
  }

  /// prefs.json is shared by every host: re-read under its lock and replace only the given agents' entries
  pub async fn save_prefs(&self, prefs: &SessionPrefs, agents: &[AgentId]) -> Result<SessionPrefs> {
    self.ensure().await?;
    let file = self.dir.join("prefs.json");
    with_file_lock(&file, || async {
      let mut disk = self.load_prefs().await;
      for agent in agents {
        if let Some(v) = prefs.last_settings.get(agent) {
          disk.last_settings.insert(agent.clone(), v.clone());
        }
        if let Some(shapes) = prefs.model_shapes.as_ref().and_then(|m| m.get(agent)) {
          let all = disk.model_shapes.get_or_insert_with(BTreeMap::new);
          let mine = all.entry(agent.clone()).or_default();
          for (k, v) in shapes {
            mine.insert(k.clone(), v.clone());
          }
        }
      }
      write_atomic(&file, serde_json::to_string_pretty(&disk)?.as_bytes(), None).await?;
      Ok(disk)
    })
    .await
  }

  /// Merge this host's view of the list with what is on disk, write the result and return it. The record files are the
  /// truth; where both have an entry, `mine` wins only for ids in `own`. Debounced records are written first
  pub async fn sync_index(&self, mine: &[SessionSummary], own: &HashSet<String>) -> Result<Vec<SessionSummary>> {
    self.ensure().await?;
    self.flush_pending().await;
    let disk: HashMap<String, SessionSummary> = self.read_index().await.into_iter().map(|s| (s.id.clone(), s)).collect();
    let local: HashMap<&str, &SessionSummary> = mine.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut out = vec![];
    for id in self.record_ids().await? {
      let pick = if own.contains(&id) {
        local.get(id.as_str()).copied().or(disk.get(&id))
      } else {
        disk.get(&id).or(local.get(id.as_str()).copied())
      };
      let s = match pick {
        Some(s) if !s.cwd.is_empty() => s.clone(),
        other => {
          let Some(r) = self.load(&id).await else { continue };
          let mut base = other.cloned().unwrap_or_else(|| r.summary());
          let fresh = r.summary();
          base.id = fresh.id;
          base.title = fresh.title;
          base.agent = fresh.agent;
          base.account_id = fresh.account_id;
          base.acp_session_id = fresh.acp_session_id;
          base.cwd = fresh.cwd;
          base.updated_at = fresh.updated_at;
          base.pinned = fresh.pinned;
          base
        }
      };
      out.push(s);
    }
    sort_index(&mut out);
    write_atomic(&self.dir.join("index.json"), serde_json::to_string_pretty(&out)?.as_bytes(), None).await?;
    Ok(out)
  }

  async fn read_index(&self) -> Vec<SessionSummary> {
    let Ok(raw) = fs::read(self.dir.join("index.json")).await else { return vec![] };
    let Ok(Value::Array(items)) = serde_json::from_slice::<Value>(&raw) else { return vec![] };
    items.into_iter().filter(|v| v.get("id").is_some_and(Value::is_string)).filter_map(|v| serde_json::from_value(v).ok()).collect()
  }

  /// Ids with a record file in the live directory (not the trash)
  async fn record_ids(&self) -> Result<Vec<String>> {
    let mut rd = fs::read_dir(&self.dir).await?;
    let mut ids = vec![];
    while let Some(e) = rd.next_entry().await? {
      let name = e.file_name().to_string_lossy().into_owned();
      if META_FILES.contains(&name.as_str()) {
        continue;
      }
      if let Some(id) = name.strip_suffix(".json")
        && is_session_id(id)
      {
        ids.push(id.to_owned());
      }
    }
    Ok(ids)
  }

  /// A record that fails to parse, or lacks the fields every reader relies on, counts as missing
  pub async fn load(&self, id: &str) -> Option<SessionRecord> {
    if !is_session_id(id) {
      return None;
    }
    let pending = self.state.lock().pending.get(id).map(|p| p.source.clone());
    if let Some(src) = pending {
      return Some(src.record());
    }
    let path = confined(&self.dir, &format!("{id}.json")).await?;
    let raw = fs::read(&path).await.ok()?;
    match serde_json::from_slice::<SessionRecord>(&raw) {
      Ok(r) if r.id == id && is_session_id(&r.id) => {
        self.state.lock().known.insert(id.to_owned());
        Some(r)
      }
      Ok(_) => {
        (self.log)(&format!("session {id}: record unreadable (not a session record)"));
        None
      }
      Err(e) => {
        (self.log)(&format!("session {id}: record unreadable ({e})"));
        None
      }
    }
  }

  /// Debounced write; a continuous stream still lands within the max wait
  pub fn save(&self, source: Arc<dyn RecordSource>) {
    self.save_after(source, self.debounce)
  }

  pub fn save_after(&self, source: Arc<dyn RecordSource>, delay: Duration) {
    let id = source.record_id();
    if !is_session_id(&id) {
      (self.log)(&format!("session {id}: illegal id, not saved"));
      return;
    }
    let now = Instant::now();
    let mut st = self.state.lock();
    if let Some(p) = st.pending.get_mut(&id) {
      p.source = source;
      p.due = (now + delay).min(p.deadline.max(now));
      return;
    }
    let deadline = now + self.max_wait;
    st.pending.insert(id.clone(), Pending { source, due: now + delay.min(self.max_wait), deadline });
    drop(st);
    let Some(me) = self.me.upgrade() else { return };
    tokio::spawn(async move { me.debounce_loop(id).await });
  }

  // One timer task per pending id: sleeps until the (movable) due time, then writes
  async fn debounce_loop(self: Arc<Self>, id: String) {
    loop {
      let due = match self.state.lock().pending.get(&id) {
        Some(p) => p.due,
        None => return,
      };
      if due > Instant::now() {
        tokio::time::sleep_until(due.into()).await;
        continue;
      }
      let Some(p) = self.state.lock().pending.remove(&id) else { return };
      if let Err(e) = self.write(p.source).await {
        self.report(&id, &e);
      }
      return;
    }
  }

  fn report(&self, id: &str, e: &anyhow::Error) {
    (self.log)(&format!("session {id}: save failed ({e})"));
    if let Some(h) = &self.on_save_error {
      h(id, &e.to_string());
    }
  }

  pub async fn flush(&self, source: Arc<dyn RecordSource>) -> Result<()> {
    let id = source.record_id();
    if !is_session_id(&id) {
      return Ok(());
    }
    self.state.lock().pending.remove(&id);
    self.write(source).await
  }

  /// Whether this store has had the record on disk
  pub fn knew(&self, id: &str) -> bool {
    self.state.lock().known.contains(id)
  }

  fn chain(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
    self.inflight.lock().entry(id.to_owned()).or_default().clone()
  }

  async fn settle_inflight(&self, id: &str) {
    let c = self.chain(id);
    drop(c.lock().await);
  }

  /// Creates the file or replaces it while it is still there; a record this store once had on disk that is gone now was
  /// deleted by another window, and writing it back would undo that
  async fn write(&self, source: Arc<dyn RecordSource>) -> Result<()> {
    let id = source.record_id();
    let chain = self.chain(&id);
    let _g = chain.lock().await;
    if !is_session_id(&id) {
      return Ok(());
    }
    self.ensure().await?;
    let Some(path) = confined(&self.dir, &format!("{id}.json")).await else { return Ok(()) };
    if self.knew(&id) && fs::metadata(&path).await.is_err() {
      (self.log)(&format!("session {id}: deleted by another window, not written back"));
      return Ok(());
    }
    let bytes = source.record_json();
    write_atomic(&path, &bytes, None).await?;
    self.state.lock().known.insert(id);
    Ok(())
  }

  /// Removes the record and its blob directory for good, wherever they are (live or trash)
  pub async fn remove(&self, id: &str) {
    if !is_session_id(id) {
      return;
    }
    self.state.lock().pending.remove(id);
    self.settle_inflight(id).await;
    self.state.lock().known.remove(id);
    for dir in [self.dir.clone(), self.dir.join(TRASH_DIR)] {
      rm_confined(&dir, &format!("{id}.json"), false).await;
      rm_confined(&dir, id, true).await;
    }
  }

  /// Soft deletion into trash/, surviving a crash (sweep_trash cleans up on the next start)
  pub async fn trash(&self, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    self.state.lock().pending.remove(id);
    self.settle_inflight(id).await;
    let trash = self.dir.join(TRASH_DIR);
    fs::create_dir_all(&trash).await?;
    move_record(&self.dir, &trash, id).await;
    // rename keeps the mtime; stamp the moment it was trashed
    if let Some(stamped) = confined(&trash, &format!("{id}.json")).await
      && let Ok(f) = std::fs::File::options().write(true).open(&stamped)
    {
      let _ = f.set_modified(SystemTime::now());
    }
    Ok(())
  }

  pub async fn restore(&self, id: &str) {
    if is_session_id(id) {
      move_record(&self.dir.join(TRASH_DIR), &self.dir, id).await;
    }
  }

  /// Remove what was trashed more than `older_than` ago
  pub async fn sweep_trash(&self, older_than: Duration) {
    let trash = self.dir.join(TRASH_DIR);
    let Ok(mut rd) = fs::read_dir(&trash).await else { return };
    let cutoff = SystemTime::now() - older_than;
    while let Ok(Some(e)) = rd.next_entry().await {
      let f = e.file_name().to_string_lossy().into_owned();
      let Some(id) = f.strip_suffix(".json") else { continue };
      if !is_session_id(id) {
        continue;
      }
      let mtime = e.metadata().await.ok().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
      if mtime > cutoff {
        continue;
      }
      rm_confined(&trash, &f, false).await;
      rm_confined(&trash, id, true).await;
    }
  }

  /// Writes whatever is still debounced
  pub async fn dispose(&self) {
    self.flush_pending().await;
  }

  async fn flush_pending(&self) {
    let queued: Vec<(String, Arc<dyn RecordSource>)> = self.state.lock().pending.drain().map(|(k, p)| (k, p.source)).collect();
    for (id, src) in queued {
      if let Err(e) = self.write(src).await {
        self.report(&id, &e);
      }
    }
    let chains: Vec<_> = self.inflight.lock().values().cloned().collect();
    for c in chains {
      drop(c.lock().await);
    }
  }

  pub async fn save_blob(&self, session_id: &str, ext: &str, bytes: &[u8]) -> Result<(String, PathBuf)> {
    let legal_ext = ext.len() > 1 && ext.starts_with('.') && ext[1..].chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{session_id}/*{ext}"))]));
    if !is_session_id(session_id) || !legal_ext {
      return Err(illegal());
    }
    let name = blob_name(ext, bytes);
    fs::create_dir_all(self.dir.join(session_id)).await?;
    let dir = confined(&self.dir, session_id).await.ok_or_else(illegal)?;
    let path = dir.join(&name);
    fs::write(&path, bytes).await?;
    Ok((name, path))
  }

  pub fn blob_path(&self, session_id: &str, name: &str) -> Option<PathBuf> {
    if !is_session_id(session_id) || !is_blob_name(name) {
      return None;
    }
    Some(self.dir.join(session_id).join(name))
  }

  pub async fn read_blob(&self, session_id: &str, name: &str) -> Result<Vec<u8>> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{session_id}/{name}"))]));
    if !is_session_id(session_id) || !is_blob_name(name) {
      return Err(illegal());
    }
    let dir = confined(&self.dir, session_id).await.ok_or_else(illegal)?;
    let file = confined(&dir, name).await.ok_or_else(illegal)?;
    Ok(fs::read(file).await?)
  }

  /// A finished export lands next to the sessions dir (~/.acpira/exports)
  pub async fn write_export(&self, name: &str, content: &str) -> Result<PathBuf> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", name)]));
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
      return Err(illegal());
    }
    let dir = self.dir.parent().unwrap_or(&self.dir).join("exports");
    fs::create_dir_all(&dir).await?;
    let path = confined(&dir, name).await.ok_or_else(illegal)?;
    write_atomic(&path, content.as_bytes(), None).await?;
    Ok(path)
  }
}

/// Pinned first, then newest first
pub fn sort_index(list: &mut [SessionSummary]) {
  list.sort_by(|a, b| b.pinned.unwrap_or(false).cmp(&a.pinned.unwrap_or(false)).then_with(|| b.updated_at.cmp(&a.updated_at)));
}

/// Resolves `root/name` and refuses anything that is not still under `root` after following symlinks; a missing target
/// is allowed when its parent stays inside the root
async fn confined(root: &Path, name: &str) -> Option<PathBuf> {
  let base = fs::canonicalize(root).await.ok()?;
  let target = root.join(name);
  let resolved = match fs::canonicalize(&target).await {
    Ok(p) => p,
    Err(_) => {
      let parent = fs::canonicalize(target.parent()?).await.ok()?;
      parent.join(target.file_name()?)
    }
  };
  (resolved == base || resolved.starts_with(&base)).then_some(resolved)
}

async fn rm_confined(root: &Path, name: &str, recursive: bool) {
  if let Some(p) = confined(root, name).await {
    let _ = if recursive { fs::remove_dir_all(&p).await } else { fs::remove_file(&p).await };
  }
}

async fn move_record(from: &Path, to: &Path, id: &str) {
  if let (Some(a), Some(b)) = (confined(from, &format!("{id}.json")).await, confined(to, &format!("{id}.json")).await) {
    let _ = fs::rename(a, b).await;
  }
  if let (Some(a), Some(b)) = (confined(from, id).await, confined(to, id).await) {
    let _ = fs::rename(a, b).await;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn record(id: &str, updated: &str) -> SessionRecord {
    serde_json::from_value(serde_json::json!({ "id": id, "agent": "grok", "cwd": "/w", "title": "t", "createdAt": updated, "updatedAt": updated, "turns": [], "controls": { "modes": [], "options": [] }, "commands": [] })).unwrap()
  }

  fn store(dir: &Path) -> Arc<TranscriptStore> {
    TranscriptStore::with_timing(dir.to_path_buf(), Arc::new(|_| {}), None, Duration::from_millis(20), Duration::from_millis(60))
  }

  #[tokio::test]
  async fn save_debounces_and_index_reconciles() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    s.save(Arc::new(record("a", "2026-01-01T00:00:00.000Z")));
    s.save(Arc::new(record("b", "2026-01-02T00:00:00.000Z")));
    assert!(s.load("a").await.is_some(), "pending record is served from memory");
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(dir.path().join("a.json").exists());
    let idx = s.load_index().await.unwrap();
    assert_eq!(idx.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
    s.trash("a").await.unwrap();
    assert_eq!(s.load_index().await.unwrap().len(), 1);
    s.restore("a").await;
    assert_eq!(s.load_index().await.unwrap().len(), 2);
    // Deleted by another window: never written back
    std::fs::remove_file(dir.path().join("b.json")).unwrap();
    s.flush(Arc::new(record("b", "2026-01-03T00:00:00.000Z"))).await.unwrap();
    assert!(!dir.path().join("b.json").exists());
  }

  #[tokio::test]
  async fn blobs_are_content_named_and_confined() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let (name, path) = s.save_blob("s1", ".png", b"abc").await.unwrap();
    assert_eq!(name, format!("{}.png", &"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"[..16]));
    assert!(path.exists());
    assert_eq!(s.read_blob("s1", &name).await.unwrap(), b"abc");
    assert!(s.save_blob("../x", ".png", b"a").await.is_err());
    assert!(s.blob_path("s1", "../../etc").is_none());
  }
}
