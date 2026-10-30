//! Session persistence: `<dir>/index.json` caches the summary list,
//! `<dir>/prefs.json` the per-agent memory, `<dir>/categories.json` the session list's user categories, `<dir>/<id>.json` the full record, `<dir>/<id>/` its blobs, `<dir>/trash/`
//! the soft-deleted ones during their undo window. The directory is shared by every host, so the index is reconciled
//! with the record files before every write, and every file is written tmp + rename. Record writes are debounced per
//! session and serialized from the live session at write time

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::fs;

use acpira_shared::model_shapes::{ModelShapes, session_model};
use acpira_shared::transcript::{AgentId, CategoryOp, SessionCategories, SessionControls, SessionModel, SessionSummary, TurnSettings};

use super::file_lock::{with_file_lock, write_atomic};
use super::record::{RecordSource, SessionRecord};
use crate::acp::agents::registry::SELF_AGENT_ID;
use crate::i18n::tp;

mod categories;

pub use self::categories::apply_category_op;

const META_FILES: [&str; 3] = ["index.json", "prefs.json", "categories.json"];
const TRASH_DIR: &str = "trash";
const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);
const SAVE_MAX_WAIT: Duration = Duration::from_millis(2000);

/// Session ids name files and directories under the store root: plain tokens only (the rule session links parse by)
pub use acpira_shared::session_links::is_session_id;

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
  /// Sessions with a reply nobody has looked at yet: a user fact, the same in every engine and window on the machine and
  /// kept across restarts (edited only through `update_unread`; engines up to 1.10.8 keep it as an unknown key)
  #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
  pub unread: BTreeSet<String>,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

/// One change to the shared unread marks
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnreadOp {
  Add(String),
  Remove(String),
  Clear,
}

pub type SaveErrorHook = Arc<dyn Fn(&str, &str) + Send + Sync>;
pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

/// Why a session record could not be loaded. Keep this distinction all the way to the UI: a missing file can be
/// removed from a stale index, while a read or parse failure should stay listed so a later retry can recover it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordLoadError {
  InvalidId,
  Missing,
  StoreUnavailable,
  Unreadable { reason: String },
  Corrupt { reason: String },
  IdMismatch { actual: String },
}

impl RecordLoadError {
  pub fn code(&self) -> &'static str {
    match self {
      Self::InvalidId => "SESSION_RECORD_INVALID_ID",
      Self::Missing => "SESSION_RECORD_MISSING",
      Self::StoreUnavailable => "SESSION_STORE_UNAVAILABLE",
      Self::Unreadable { .. } => "SESSION_RECORD_UNREADABLE",
      Self::Corrupt { .. } => "SESSION_RECORD_CORRUPT",
      Self::IdMismatch { .. } => "SESSION_RECORD_MISMATCH",
    }
  }

  pub fn detail(&self) -> String {
    match self {
      Self::InvalidId => "the session id is invalid".into(),
      Self::Missing => "the record file does not exist".into(),
      Self::StoreUnavailable => "the session store is unavailable".into(),
      Self::Unreadable { reason } => reason.clone(),
      Self::Corrupt { reason } => reason.clone(),
      Self::IdMismatch { actual } => format!("the file contains id {actual}"),
    }
  }

  pub fn is_missing(&self) -> bool {
    matches!(self, Self::Missing)
  }
}

struct Pending {
  source: Arc<dyn RecordSource>,
  due: Instant,
  deadline: Instant,
}

struct State {
  pending: HashMap<String, Pending>,
  known: HashSet<String>,
  /// Records whose content did not parse, by the file stamp they had then: the index sync (every webview ready / window
  /// focus) passes over them until the file changes, instead of reading and failing on them again
  unreadable: HashMap<String, FileStamp>,
  /// Models read from records whose index entry has none (written before the field existed, or by an older engine that
  /// drops it), by the file stamp they were read at: the index sync fills them in while the record is unchanged
  models: HashMap<String, (FileStamp, Option<SessionModel>)>,
}

type FileStamp = (u64, Option<SystemTime>);

async fn file_stamp(path: &Path) -> Option<FileStamp> {
  fs::metadata(path).await.ok().map(|m| (m.len(), m.modified().ok()))
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
  /// Asked before every record write: false skips it (another engine drives the session, `store::session_lease`)
  write_guard: parking_lot::RwLock<Option<WriteGuard>>,
  /// The data root the built-in agent keeps its session logs under; set, a record removed for good takes its log along
  agent_home: parking_lot::RwLock<Option<PathBuf>>,
  /// Told of every record written, trashed, restored or removed (`remote::host_store` copies host folders' records)
  on_change: parking_lot::RwLock<Option<ChangeHook>>,
  me: Weak<TranscriptStore>,
}

/// What happened to a record, for `set_change_hook`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordChange<'a> {
  /// Written, with the session's folder
  Written { cwd: &'a str },
  /// Moved into the trash or removed for good, with the folder it was in (empty when unknown)
  Gone { cwd: &'a str },
  /// Back from the trash
  Restored { cwd: &'a str },
}

pub type ChangeHook = Arc<dyn Fn(&str, RecordChange) + Send + Sync>;

/// A record's `updatedAt` and folder alone, the transcript skipped over by the parser
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stamped {
  #[serde(default)]
  updated_at: String,
  #[serde(default)]
  cwd: String,
}

/// The folder a record file names, empty when it cannot be read
async fn cwd_of(path: &Path) -> String {
  let Ok(raw) = fs::read(path).await else { return String::new() };
  serde_json::from_slice::<Stamped>(&raw).map(|s| s.cwd).unwrap_or_default()
}

/// A record copy as `copy_stamps` reports it: when it was last updated and the blobs it has
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyStamp {
  pub updated_at: String,
  pub blobs: Vec<String>,
}

/// The two fields of a record the log cascade needs; the transcript is skipped over by the parser instead of built
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LogOwner {
  #[serde(default)]
  agent: String,
  #[serde(default)]
  acp_session_id: Option<String>,
}

/// The built-in agent's session id a record file was driving, if it was one of its sessions
async fn builtin_session_of(path: &Path) -> Option<String> {
  let raw = fs::read(path).await.ok()?;
  let owner: LogOwner = serde_json::from_slice(&raw).ok()?;
  owner.acp_session_id.filter(|acp| owner.agent == SELF_AGENT_ID && is_session_id(acp))
}

pub type WriteGuard = Arc<dyn Fn(&str) -> bool + Send + Sync>;

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
      state: parking_lot::Mutex::new(State { pending: HashMap::new(), known: HashSet::new(), unreadable: HashMap::new(), models: HashMap::new() }),
      inflight: Default::default(),
      write_guard: Default::default(),
      agent_home: Default::default(),
      on_change: Default::default(),
      me: me.clone(),
    })
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }

  /// Install the check every record write passes first
  pub fn set_write_guard(&self, guard: WriteGuard) {
    *self.write_guard.write() = Some(guard);
  }

  /// Where the built-in agent's logs live (the data root it is launched with); from then on a session removed for good
  /// takes its log along
  pub fn set_agent_home(&self, home: PathBuf) {
    *self.agent_home.write() = Some(home);
  }

  /// Install the hook told of every record change
  pub fn set_change_hook(&self, hook: ChangeHook) {
    *self.on_change.write() = Some(hook);
  }

  fn changed(&self, id: &str, change: RecordChange) {
    let hook = self.on_change.read().clone();
    if let Some(f) = hook {
      f(id, change);
    }
  }

  async fn ensure(&self) -> Result<()> {
    fs::create_dir_all(&self.dir).await?;
    Ok(())
  }

  /// Write a copy of a record another machine drives (a session it runs in a folder of this host, docs/dev/remote.md),
  /// unless the copy here was updated later or the record waits in the trash here (deleted on this host). False when
  /// it was not written
  pub async fn put_copy(&self, record: &SessionRecord) -> Result<bool> {
    let id = &record.id;
    if !is_session_id(id) {
      return Err(anyhow!("{id} is not a session id"));
    }
    self.ensure().await?;
    let Some(path) = confined(&self.dir, &format!("{id}.json")).await else { return Err(anyhow!("{id}: outside the store")) };
    if fs::metadata(self.dir.join(TRASH_DIR).join(format!("{id}.json"))).await.is_ok() {
      return Ok(false);
    }
    if let Ok(raw) = fs::read(&path).await
      && let Ok(here) = serde_json::from_slice::<Stamped>(&raw)
      && here.updated_at > record.updated_at
    {
      return Ok(false);
    }
    write_atomic(&path, &serde_json::to_vec(record)?, None).await?;
    // The engines here prefer an index entry to the record for a session they do not drive: the entry goes too
    let mut index = self.read_index().await;
    let summary = record.summary();
    match index.iter_mut().find(|s| s.id == *id) {
      Some(entry) => *entry = summary,
      None => index.push(summary),
    }
    sort_index(&mut index);
    write_atomic(&self.dir.join("index.json"), serde_json::to_string_pretty(&index)?.as_bytes(), None).await?;
    Ok(true)
  }

  /// For each of `ids` with a record here (not in the trash): its `updatedAt` and the blobs it has
  pub async fn copy_stamps(&self, ids: &[String]) -> HashMap<String, CopyStamp> {
    let mut out = HashMap::new();
    for id in ids.iter().filter(|id| is_session_id(id)) {
      let Ok(raw) = fs::read(self.dir.join(format!("{id}.json"))).await else { continue };
      let Ok(s) = serde_json::from_slice::<Stamped>(&raw) else { continue };
      out.insert(id.clone(), CopyStamp { updated_at: s.updated_at, blobs: self.blob_names(id).await });
    }
    out
  }

  /// The blobs a session has here, by name
  pub async fn blob_names(&self, id: &str) -> Vec<String> {
    let mut names = vec![];
    if !is_session_id(id) {
      return names;
    }
    if let Ok(mut rd) = fs::read_dir(self.dir.join(id)).await {
      while let Ok(Some(e)) = rd.next_entry().await {
        let name = e.file_name().to_string_lossy().into_owned();
        if is_blob_name(&name) {
          names.push(name);
        }
      }
    }
    names.sort();
    names
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

  /// The unread marks are shared by every engine: the ops are applied under prefs.json's lock to what is on disk, in order,
  /// so two engines' marks compose. Returns the marks as written
  pub async fn update_unread(&self, ops: &[UnreadOp]) -> Result<BTreeSet<String>> {
    self.ensure().await?;
    let file = self.dir.join("prefs.json");
    with_file_lock(&file, || async {
      let mut disk = self.load_prefs().await;
      let before = disk.unread.clone();
      for op in ops {
        match op {
          UnreadOp::Add(id) if is_session_id(id) => {
            disk.unread.insert(id.clone());
          }
          UnreadOp::Remove(id) => {
            disk.unread.remove(id);
          }
          UnreadOp::Clear => disk.unread.clear(),
          UnreadOp::Add(_) => {}
        }
      }
      if disk.unread != before {
        write_atomic(&file, serde_json::to_string_pretty(&disk)?.as_bytes(), None).await?;
      }
      Ok(disk.unread)
    })
    .await
  }

  pub async fn load_categories(&self) -> SessionCategories {
    match fs::read(self.dir.join("categories.json")).await {
      Ok(raw) => serde_json::from_slice(&raw).unwrap_or_default(),
      Err(_) => SessionCategories::default(),
    }
  }

  /// categories.json is shared by every host: the op is applied under its lock to what is on disk, so two windows'
  /// edits compose instead of the later write replacing the earlier one. Returns the list as written
  pub async fn update_categories(&self, op: &CategoryOp) -> Result<SessionCategories> {
    self.ensure().await?;
    let file = self.dir.join("categories.json");
    with_file_lock(&file, || async {
      let mut disk = self.load_categories().await;
      apply_category_op(&mut disk, op);
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
          let stamp = file_stamp(&self.dir.join(format!("{id}.json"))).await;
          if stamp.is_some() && self.state.lock().unreadable.get(&id) == stamp.as_ref() {
            continue;
          }
          let r = match self.load_detailed(&id).await {
            Ok(r) => r,
            Err(error) => {
              if !error.is_missing() {
                (self.log)(&format!("session {id}: record unreadable ({})", error.detail()));
              }
              // Only a file whose content is wrong is passed over: an I/O failure (permissions) may clear without the file changing
              if let (Some(stamp), RecordLoadError::Corrupt { .. } | RecordLoadError::IdMismatch { .. }) = (stamp, &error) {
                self.state.lock().unreadable.insert(id.clone(), stamp);
              }
              continue;
            }
          };
          let mut base = other.cloned().unwrap_or_else(|| r.summary());
          let fresh = r.summary();
          base.id = fresh.id;
          base.title = fresh.title;
          base.agent = fresh.agent;
          base.account_id = fresh.account_id;
          base.acp_session_id = fresh.acp_session_id;
          base.cwd = fresh.cwd;
          base.updated_at = fresh.updated_at;
          base.completed_at = fresh.completed_at;
          base.pinned = fresh.pinned;
          base.archived = fresh.archived;
          base.category = fresh.category;
          base.model = fresh.model;
          base
        }
      };
      let mut s = s;
      if s.model.is_none() {
        s.model = self.cached_model(&id).await;
      }
      out.push(s);
    }
    sort_index(&mut out);
    write_atomic(&self.dir.join("index.json"), serde_json::to_string_pretty(&out)?.as_bytes(), None).await?;
    Ok(out)
  }

  /// A model `read_models` found for this record, while the file is still the one it read
  async fn cached_model(&self, id: &str) -> Option<SessionModel> {
    // A record with no model control costs no stat
    let (stamp, model) = self.state.lock().models.get(id).and_then(|(stamp, m)| Some((*stamp, m.clone()?)))?;
    (file_stamp(&self.dir.join(format!("{id}.json"))).await == Some(stamp)).then_some(model)
  }

  /// Read the current model of each record not read yet (its controls only, the transcript skipped by the parser), for
  /// index entries that have none. True when one was found, so a sync then writes them into the index
  pub async fn read_models(&self, ids: &[String]) -> bool {
    let mut found = false;
    for id in ids {
      if self.state.lock().models.contains_key(id) {
        continue;
      }
      let Some(stamp) = file_stamp(&self.dir.join(format!("{id}.json"))).await else { continue };
      let model = self.load_controls(id).await.and_then(|c| session_model(&c));
      found |= model.is_some();
      self.state.lock().models.insert(id.clone(), (stamp, model));
    }
    found
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

  /// Load a record with a reason that callers can present or log. The filename is still confined to the session root.
  pub async fn load_detailed(&self, id: &str) -> Result<SessionRecord, RecordLoadError> {
    if !is_session_id(id) {
      return Err(RecordLoadError::InvalidId);
    }
    let pending = self.state.lock().pending.get(id).map(|p| p.source.clone());
    if let Some(src) = pending {
      return Ok(src.record());
    }
    let path = confined(&self.dir, &format!("{id}.json")).await.ok_or(RecordLoadError::StoreUnavailable)?;
    let raw = match fs::read(&path).await {
      Ok(raw) => raw,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(RecordLoadError::Missing),
      Err(e) => return Err(RecordLoadError::Unreadable { reason: e.to_string() }),
    };
    match serde_json::from_slice::<SessionRecord>(&raw) {
      Ok(mut r) if r.id == id && is_session_id(&r.id) => {
        self.state.lock().known.insert(id.to_owned());
        r.fold_turn_usage();
        Ok(r)
      }
      Ok(r) => Err(RecordLoadError::IdMismatch { actual: r.id }),
      Err(e) => Err(RecordLoadError::Corrupt { reason: e.to_string() }),
    }
  }

  /// Only a record's controls, for the chips a new session paints before its own arrive: the transcript is skipped over
  /// by the parser instead of built (a long record runs to tens of MB)
  pub async fn load_controls(&self, id: &str) -> Option<SessionControls> {
    #[derive(Deserialize)]
    struct ControlsOnly {
      id: String,
      #[serde(default)]
      controls: SessionControls,
    }
    if !is_session_id(id) {
      return None;
    }
    let pending = self.state.lock().pending.get(id).map(|p| p.source.clone());
    if let Some(src) = pending {
      return Some(src.record().controls);
    }
    let path = confined(&self.dir, &format!("{id}.json")).await?;
    let raw = fs::read(&path).await.ok()?;
    serde_json::from_slice::<ControlsOnly>(&raw).ok().filter(|r| r.id == id).map(|r| r.controls)
  }

  /// Compatibility wrapper for callers that only need the record. Detailed callers should use `load_detailed`.
  pub async fn load(&self, id: &str) -> Option<SessionRecord> {
    match self.load_detailed(id).await {
      Ok(record) => Some(record),
      Err(error) => {
        if !error.is_missing() {
          (self.log)(&format!("session {id}: record unreadable ({})", error.detail()));
        }
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
      // The write chain is held before the record leaves `pending`, so flush_pending always sees it in one place or the other
      let chain = self.chain(&id);
      let _g = chain.lock().await;
      let Some(p) = self.state.lock().pending.remove(&id) else { return };
      if let Err(e) = self.write_locked(p.source).await {
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
    let chain = self.chain(&source.record_id());
    let _g = chain.lock().await;
    self.write_locked(source).await
  }

  /// `write` with the record's chain already held
  async fn write_locked(&self, source: Arc<dyn RecordSource>) -> Result<()> {
    let id = source.record_id();
    if !is_session_id(&id) {
      return Ok(());
    }
    self.ensure().await?;
    let Some(path) = confined(&self.dir, &format!("{id}.json")).await else { return Ok(()) };
    // A copy this engine keeps of a session another engine is driving (two workspaces with it open) must not overwrite
    // what that engine writes; the guard is cloned out so no lock is held across the call
    let guard = self.write_guard.read().clone();
    if guard.is_some_and(|may_write| !may_write(&id)) {
      (self.log)(&format!("session {id}: another engine holds it, not written"));
      return Ok(());
    }
    if self.knew(&id) && fs::metadata(&path).await.is_err() {
      (self.log)(&format!("session {id}: deleted by another window, not written back"));
      return Ok(());
    }
    let bytes = source.record_json();
    write_atomic(&path, &bytes, None).await?;
    self.state.lock().known.insert(id.clone());
    if self.on_change.read().is_some() {
      self.changed(&id, RecordChange::Written { cwd: &source.record_cwd() });
    }
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
    let mut log = None;
    let mut cwd = String::new();
    let hooked = self.on_change.read().is_some();
    for dir in [self.dir.clone(), self.dir.join(TRASH_DIR)] {
      let file = dir.join(format!("{id}.json"));
      if log.is_none() {
        log = builtin_session_of(&file).await;
      }
      if hooked && cwd.is_empty() {
        cwd = cwd_of(&file).await;
      }
      rm_confined(&dir, &format!("{id}.json"), false).await;
      rm_confined(&dir, id, true).await;
    }
    if let Some(acp) = log {
      self.remove_agent_log(id, &acp).await;
    }
    if hooked {
      self.changed(id, RecordChange::Gone { cwd: &cwd });
    }
  }

  /// The built-in agent's log and folder for a session that is gone for good. Kept while another record (listed, or
  /// still in its own undo window) drives the same agent session, e.g. a session imported twice
  async fn remove_agent_log(&self, removed: &str, acp: &str) {
    let Some(home) = self.agent_home.read().clone() else { return };
    let listed = self.read_index().await.into_iter().any(|s| s.id != removed && s.agent == SELF_AGENT_ID && s.acp_session_id.as_deref() == Some(acp));
    if listed {
      return;
    }
    let trash = self.dir.join(TRASH_DIR);
    if let Ok(mut rd) = fs::read_dir(&trash).await {
      while let Ok(Some(e)) = rd.next_entry().await {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.strip_suffix(".json").is_some_and(|other| other != removed && is_session_id(other))
          && builtin_session_of(&e.path()).await.as_deref() == Some(acp)
        {
          return;
        }
      }
    }
    let root = acpira_agent::acp::sessions_root(&home);
    for path in acpira_agent::acp::session_paths(&home, acp) {
      let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
      let Some(target) = confined(&root, &name).await else { continue };
      let result = if target.is_dir() { fs::remove_dir_all(&target).await } else { fs::remove_file(&target).await };
      // A file another process still has open on Windows stays; the Storage page lists it as an orphaned log
      if let Err(e) = result.or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }) {
        (self.log)(&format!("session {removed}: built-in agent log {} not removed ({e})", target.display()));
      }
    }
  }

  /// Soft deletion into trash/, surviving a crash (sweep_trash cleans up on the next start). An error means the record
  /// was not moved: whatever this call had moved is back in place, unless the error says otherwise. Nothing here
  /// (an id already trashed or never stored) and an illegal id are no-ops; a trashed copy is never replaced
  pub async fn trash(&self, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    self.state.lock().pending.remove(id);
    self.settle_inflight(id).await;
    let trash = self.dir.join(TRASH_DIR);
    fs::create_dir_all(&trash).await?;
    let file = format!("{id}.json");
    // The record leaves first: an entry left behind by a failed undo is then a stray blob folder, not a listed session
    let moved = async {
      real_dir(&trash).await?;
      move_entries(&self.dir, &trash, [file.as_str(), id]).await
    }
    .await
    .map_err(|e| anyhow!("session {id}: not moved to the trash ({e:#})"))?;
    // rename keeps the mtime; stamp the moment it was trashed (a regular file only, so a symlink's target is not touched)
    let stamped = trash.join(&file);
    if moved.contains(&file.as_str())
      && fs::symlink_metadata(&stamped).await.is_ok_and(|m| m.file_type().is_file())
      && let Ok(f) = std::fs::File::options().write(true).open(&stamped)
    {
      let _ = f.set_modified(SystemTime::now());
    }
    if self.on_change.read().is_some() {
      let cwd = cwd_of(&stamped).await;
      self.changed(id, RecordChange::Gone { cwd: &cwd });
    }
    Ok(())
  }

  /// Undo `trash`. An error means the record is still in the trash: whatever this call had moved is back there, unless
  /// the error says otherwise. Nothing in the trash and an illegal id are no-ops; a record in the live directory (written
  /// again since) is never replaced
  pub async fn restore(&self, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let trash = self.dir.join(TRASH_DIR);
    let file = format!("{id}.json");
    // The record arrives last: an entry left behind by a failed undo is then a stray blob folder, not a listed session
    async {
      if real_dir(&trash).await? {
        move_entries(&trash, &self.dir, [id, file.as_str()]).await?;
      }
      anyhow::Ok(())
    }
    .await
    .map_err(|e| anyhow!("session {id}: not restored from the trash ({e:#})"))?;
    if self.on_change.read().is_some() {
      let cwd = cwd_of(&self.dir.join(&file)).await;
      self.changed(id, RecordChange::Restored { cwd: &cwd });
    }
    Ok(())
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
      let log = builtin_session_of(&e.path()).await;
      rm_confined(&trash, &f, false).await;
      rm_confined(&trash, id, true).await;
      if let Some(acp) = log {
        self.remove_agent_log(id, &acp).await;
      }
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
    let legal_ext = ext.strip_prefix('.').is_some_and(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
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
    // canonicalize() hands back `\\?\C:\…` on Windows; the path is shown to the user and opened by the IDE, so use the
    // ordinary drive / UNC spelling
    Ok(crate::platform::paths::for_cli(path))
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

/// Whether an entry is at `path` itself; a symlink there counts, whatever it points at
async fn entry_exists(path: &Path) -> Result<bool> {
  match fs::symlink_metadata(path).await {
    Ok(_) => Ok(true),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
    Err(e) => Err(anyhow!("{}: {e}", path.display())),
  }
}

/// Whether `dir` is there as a real directory. Anything else in its place is an error, a symlink above all: the trash
/// behind one could carry records out of the store or pull outside entries in
async fn real_dir(dir: &Path) -> Result<bool> {
  match fs::symlink_metadata(dir).await {
    Ok(m) if m.file_type().is_dir() => Ok(true),
    Ok(_) => Err(anyhow!("{} is not a directory", dir.display())),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
    Err(e) => Err(anyhow!("{}: {e}", dir.display())),
  }
}

/// Moves a session's entries (`<id>.json`, `<id>/`) between two store directories in the given order and returns the
/// ones it moved. Names are session ids, so each entry is a direct child, and nothing is followed: a symlink moves as the
/// link, its target untouched. An absent entry is skipped (blobs are optional, a repeated call finds nothing); one
/// already at the destination is never replaced. On a failure the entries this call moved go back before the error is
/// returned; any that cannot are named in it
async fn move_entries<'a>(from: &Path, to: &Path, names: [&'a str; 2]) -> Result<Vec<&'a str>> {
  let mut moved: Vec<&str> = vec![];
  for name in names {
    let (src, dst) = (from.join(name), to.join(name));
    let step = async {
      if !entry_exists(&src).await? {
        return Ok(false);
      }
      if entry_exists(&dst).await? {
        return Err(anyhow!("{} already exists", dst.display()));
      }
      fs::rename(&src, &dst).await.map_err(|e| anyhow!("moving {} to {}: {e}", src.display(), dst.display()))?;
      anyhow::Ok(true)
    };
    match step.await {
      Ok(true) => moved.push(name),
      Ok(false) => {}
      Err(e) => return Err(undo_moves(from, to, &moved, e).await),
    }
  }
  Ok(moved)
}

/// Puts back what a failed `move_entries` moved, newest first, under the same rule: a place taken again meanwhile is
/// not replaced. Whatever stays out of place is added to the error
async fn undo_moves(from: &Path, to: &Path, moved: &[&str], error: anyhow::Error) -> anyhow::Error {
  let mut stuck = vec![];
  for name in moved.iter().rev() {
    let (src, dst) = (from.join(name), to.join(name));
    let back = match entry_exists(&src).await {
      Ok(false) => fs::rename(&dst, &src).await.map_err(|e| anyhow!("{e}")),
      Ok(true) => Err(anyhow!("{} was taken meanwhile", src.display())),
      Err(e) => Err(e),
    };
    if let Err(e) = back {
      stuck.push(format!("{} left at {} ({e:#})", src.display(), dst.display()));
    }
  }
  if stuck.is_empty() { error } else { anyhow!("{error:#}; not put back: {}", stuck.join("; ")) }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn category_reorder_stays_inside_the_project_and_ops_on_missing_ids_do_nothing() {
    let mut list = SessionCategories::default();
    let create = |id: &str, cwd: &str| CategoryOp::Create { id: id.into(), name: id.into(), cwd: cwd.into() };
    for op in [create("a1", "/a"), create("b1", "/b"), create("a2", "/a"), create("a3", "/a"), create("a1", "/b")] {
      apply_category_op(&mut list, &op);
    }
    let ids = |l: &SessionCategories| l.categories.iter().map(|c| c.id.clone()).collect::<Vec<_>>();
    // A repeated create keeps the first one
    assert_eq!(ids(&list), ["a1", "b1", "a2", "a3"]);
    assert_eq!(list.categories[0].cwd, "/a");
    // No anchor: after the project's last category
    apply_category_op(&mut list, &CategoryOp::Reorder { id: "a1".into(), before: None });
    assert_eq!(ids(&list), ["b1", "a2", "a3", "a1"]);
    // An anchor of another project is not followed
    apply_category_op(&mut list, &CategoryOp::Reorder { id: "a1".into(), before: Some("b1".into()) });
    assert_eq!(ids(&list), ["b1", "a2", "a3", "a1"]);
    apply_category_op(&mut list, &CategoryOp::Reorder { id: "a1".into(), before: Some("a2".into()) });
    assert_eq!(ids(&list), ["b1", "a1", "a2", "a3"]);
    let before = list.clone();
    apply_category_op(&mut list, &CategoryOp::Rename { id: "gone".into(), name: "x".into() });
    apply_category_op(&mut list, &CategoryOp::Reorder { id: "gone".into(), before: None });
    assert_eq!(list, before);
  }

  #[test]
  fn projects_keep_their_order_and_seeding_happens_once() {
    let mut list = SessionCategories::default();
    let seed = |cwds: &[&str]| CategoryOp::SeedProjects { cwds: cwds.iter().map(|c| c.to_string()).collect() };
    apply_category_op(&mut list, &seed(&["/b", "/a", "/b", ""]));
    assert_eq!(list.projects.as_deref(), Some(&["/b".to_string(), "/a".to_string()][..]));
    // A second seed (another window, or a later connect) leaves the order alone
    apply_category_op(&mut list, &seed(&["/z"]));
    let add = |c: &str| CategoryOp::AddProject { cwd: c.into() };
    for op in [add("/c"), add("/a"), add("")] {
      apply_category_op(&mut list, &op);
    }
    let order = |l: &SessionCategories| l.projects.clone().unwrap_or_default();
    assert_eq!(order(&list), ["/b", "/a", "/c"]);
    apply_category_op(&mut list, &CategoryOp::ReorderProject { cwd: "/c".into(), before: Some("/b".into()) });
    assert_eq!(order(&list), ["/c", "/b", "/a"]);
    apply_category_op(&mut list, &CategoryOp::ReorderProject { cwd: "/c".into(), before: None });
    assert_eq!(order(&list), ["/b", "/a", "/c"]);
    apply_category_op(&mut list, &CategoryOp::ReorderProject { cwd: "/gone".into(), before: None });
    apply_category_op(&mut list, &CategoryOp::RemoveProject { cwd: "/a".into() });
    assert_eq!(order(&list), ["/b", "/c"]);
    assert_eq!(list.removed_projects, ["/a"]);
    // Removing every folder keeps an empty list, so nothing seeds it again
    for c in ["/b", "/c", "/b"] {
      apply_category_op(&mut list, &CategoryOp::RemoveProject { cwd: c.into() });
    }
    apply_category_op(&mut list, &seed(&["/a"]));
    assert_eq!(list.projects, Some(vec![]));
    assert_eq!(list.removed_projects, ["/a", "/b", "/c"]);
    // Adding a removed folder again forgets the removal
    apply_category_op(&mut list, &add("/b"));
    assert_eq!(order(&list), ["/b"]);
    assert_eq!(list.removed_projects, ["/a", "/c"]);
  }

  #[test]
  fn sections_hold_each_project_once_and_give_them_back_when_deleted() {
    let mut list = SessionCategories::default();
    let op = |o: CategoryOp, l: &mut SessionCategories| apply_category_op(l, &o);
    let file = |cwd: &str, section: Option<&str>| CategoryOp::SetProjectSection { cwd: cwd.into(), section: section.map(Into::into), before: None };
    let create = |id: &str, name: &str, project: Option<&str>| CategoryOp::CreateSection { id: id.into(), name: name.into(), project: project.map(Into::into) };
    op(create("w", " Work ", None), &mut list);
    // Made from a project's menu: the project goes in with it
    op(create("p", "", Some("/z")), &mut list);
    // A second create with a taken id (two windows) keeps the first
    op(create("w", "Other", None), &mut list);
    assert_eq!(list.sections.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Work", crate::i18n::t("session.section.untitled").as_str()]);
    for o in [file("/a", Some("w")), file("/b", Some("w")), file("/a", Some("p")), file("/c", Some("gone")), file("", Some("w"))] {
      op(o, &mut list);
    }
    let held = |l: &SessionCategories, id: &str| l.sections.iter().find(|s| s.id == id).map(|s| s.projects.clone()).unwrap_or_default();
    // One section per project: filing /a under p took it out of w; a section that is gone files nothing
    assert_eq!(held(&list, "w"), ["/b"]);
    assert_eq!(held(&list, "p"), ["/z", "/a"]);
    // A drag places it: before a project of the section, re-placed within its own one, appended past a gone anchor
    let place = |cwd: &str, section: &str, before: &str| CategoryOp::SetProjectSection { cwd: cwd.into(), section: Some(section.into()), before: Some(before.into()) };
    op(place("/b", "p", "/a"), &mut list);
    assert_eq!(held(&list, "p"), ["/z", "/b", "/a"]);
    op(place("/a", "p", "/z"), &mut list);
    assert_eq!(held(&list, "p"), ["/a", "/z", "/b"]);
    op(place("/b", "w", "/gone"), &mut list);
    assert_eq!(held(&list, "w"), ["/b"]);
    assert_eq!(held(&list, "p"), ["/a", "/z"]);
    op(CategoryOp::ReorderSection { id: "p".into(), before: Some("w".into()) }, &mut list);
    op(CategoryOp::CollapseSection { id: "w".into(), collapsed: true }, &mut list);
    op(CategoryOp::RenameSection { id: "p".into(), name: "Personal".into() }, &mut list);
    assert_eq!(list.sections.iter().map(|s| (s.name.as_str(), s.collapsed)).collect::<Vec<_>>(), [("Personal", None), ("Work", Some(true))]);
    op(file("/b", None), &mut list);
    assert!(held(&list, "w").is_empty());
    // Off the list is out of its section; deleting a section leaves its projects under Projects
    op(CategoryOp::RemoveProject { cwd: "/a".into() }, &mut list);
    assert_eq!(held(&list, "p"), ["/z"]);
    op(CategoryOp::DeleteSection { id: "w".into() }, &mut list);
    assert_eq!(list.sections.len(), 1);
    // The file keeps no empty sections key for lists that never had one
    let fresh = serde_json::to_value(SessionCategories::default()).unwrap();
    assert!(fresh.get("sections").is_none(), "{fresh}");
  }

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
    s.restore("a").await.unwrap();
    assert_eq!(s.load_index().await.unwrap().len(), 2);
    // Deleted by another window: never written back
    std::fs::remove_file(dir.path().join("b.json")).unwrap();
    s.flush(Arc::new(record("b", "2026-01-03T00:00:00.000Z"))).await.unwrap();
    assert!(!dir.path().join("b.json").exists());
  }

  #[tokio::test]
  async fn index_entries_without_a_model_learn_it_from_their_record() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut r = serde_json::to_value(record("a", "2026-01-01T00:00:00.000Z")).unwrap();
    r["controls"]["options"] = serde_json::json!([{ "id": "model", "name": "Model", "value": "opus", "options": [{ "id": "opus", "name": "Opus 5.5" }] }]);
    std::fs::write(dir.path().join("a.json"), r.to_string()).unwrap();
    // An index an older engine wrote: the entry is there, its model is not
    let mut entry = serde_json::to_value(serde_json::from_value::<SessionRecord>(r).unwrap().summary()).unwrap();
    entry.as_object_mut().unwrap().remove("model");
    std::fs::write(dir.path().join("index.json"), serde_json::json!([entry]).to_string()).unwrap();
    assert_eq!(s.load_index().await.unwrap()[0].model, None, "the entry is taken as it is");
    assert!(s.read_models(&["a".into()]).await);
    assert_eq!(s.load_index().await.unwrap()[0].model.as_ref().map(|m| m.name.as_str()), Some("Opus 5.5"));
  }

  fn builtin(id: &str, acp: &str) -> SessionRecord {
    let mut r = record(id, "2026-01-01T00:00:00.000Z");
    r.agent = SELF_AGENT_ID.into();
    r.acp_session_id = Some(acp.into());
    r
  }

  /// A data root with `sessions/` for the store and the built-in agent's logs under `agent/sessions/`
  fn rooted() -> (tempfile::TempDir, Arc<TranscriptStore>, impl Fn(&str) -> bool) {
    let root = tempfile::tempdir().unwrap();
    let s = store(&root.path().join("sessions"));
    s.set_agent_home(root.path().to_path_buf());
    let home = root.path().to_path_buf();
    let log_exists = move |acp: &str| {
      let [file, dir] = acpira_agent::acp::session_paths(&home, acp);
      assert_eq!(file.exists(), dir.exists(), "log and folder go together");
      file.exists()
    };
    (root, s, log_exists)
  }

  fn write_log(root: &Path, acp: &str) {
    let [file, dir] = acpira_agent::acp::session_paths(root, acp);
    std::fs::create_dir_all(dir.join("outputs")).unwrap();
    std::fs::write(dir.join("plan.md"), b"plan").unwrap();
    std::fs::write(file, b"{}\n").unwrap();
  }

  #[tokio::test]
  async fn removing_a_builtin_session_for_good_takes_its_log_along() {
    let (root, s, log_exists) = rooted();
    write_log(root.path(), "acp-1");
    s.flush(Arc::new(builtin("a", "acp-1"))).await.unwrap();
    s.load_index().await.unwrap();
    // The undo window keeps it; a restore finds the log untouched
    s.trash("a").await.unwrap();
    s.restore("a").await.unwrap();
    assert!(log_exists("acp-1"));
    s.trash("a").await.unwrap();
    s.load_index().await.unwrap();
    s.remove("a").await;
    assert!(!log_exists("acp-1"));
    assert!(root.path().join("agent/sessions").exists(), "only the session's own files go");
  }

  #[tokio::test]
  async fn a_log_another_record_still_drives_is_kept() {
    let (root, s, log_exists) = rooted();
    write_log(root.path(), "shared");
    s.flush(Arc::new(builtin("a", "shared"))).await.unwrap();
    s.flush(Arc::new(builtin("b", "shared"))).await.unwrap();
    // Another CLI's session whose acp id happens to match is not the built-in agent's log
    write_log(root.path(), "grok-1");
    let mut other = record("g", "2026-01-01T00:00:00.000Z");
    other.acp_session_id = Some("grok-1".into());
    s.flush(Arc::new(other)).await.unwrap();
    s.load_index().await.unwrap();
    s.trash("a").await.unwrap();
    s.load_index().await.unwrap();
    s.remove("a").await;
    assert!(log_exists("shared"), "b is still listed");
    // b in its own undo window still counts
    s.trash("b").await.unwrap();
    s.flush(Arc::new(builtin("c", "shared"))).await.unwrap();
    s.trash("c").await.unwrap();
    s.load_index().await.unwrap();
    s.remove("c").await;
    assert!(log_exists("shared"), "b may still be undone");
    s.remove("b").await;
    assert!(!log_exists("shared"));
    s.remove("g").await;
    assert!(log_exists("grok-1"));
  }

  #[tokio::test]
  async fn a_crashed_hosts_trash_sweep_takes_the_log_along() {
    let (root, s, log_exists) = rooted();
    write_log(root.path(), "acp-2");
    s.flush(Arc::new(builtin("a", "acp-2"))).await.unwrap();
    s.trash("a").await.unwrap();
    // Another host after the crash: no summary in memory, only the trashed record
    let next = store(&root.path().join("sessions"));
    next.set_agent_home(root.path().to_path_buf());
    next.sweep_trash(Duration::from_secs(3600)).await;
    assert!(log_exists("acp-2"), "a fresh trash entry is still in its window");
    next.sweep_trash(Duration::ZERO).await;
    assert!(!log_exists("acp-2"));
    assert!(!root.path().join("sessions/trash/a.json").exists());
  }

  #[tokio::test]
  async fn blobs_are_content_named_and_confined() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let (name, path) = s.save_blob("s1", ".png", b"abc").await.unwrap();
    assert_eq!(name, format!("{}.png", "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".get(..16).unwrap()));
    assert!(path.exists());
    assert_eq!(s.read_blob("s1", &name).await.unwrap(), b"abc");
    assert!(s.save_blob("../x", ".png", b"a").await.is_err());
    assert!(s.blob_path("s1", "../../etc").is_none());
  }
}
