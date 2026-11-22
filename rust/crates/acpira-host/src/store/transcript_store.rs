//! Session persistence: `<dir>/index.json` caches the summary list,
//! `<dir>/prefs.json` the per-agent memory, `<dir>/categories.json` the session list's user categories, `<dir>/<id>.json` the full record, `<dir>/<id>/` its blobs, `<dir>/trash/`
//! the soft-deleted ones during their undo window. The directory is shared by every host, so the index is reconciled
//! with the record files before every write, and every file is written tmp + rename. Record writes are debounced per
//! session and serialized from the live session at write time. A store whose database was created with no record files
//! (`SessionDb::stubless`) keeps no `<id>.json` for a database body: the catalogue alone lists it and marks it trashed

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use tokio::fs;

use acpira_shared::model_shapes::{ModelMemory, ModelShapes};
use acpira_shared::transcript::{AgentId, SessionModel, SessionSummary, TurnSettings};

use super::db::{BodyLocation, CatalogEntry, SaveCache, SessionDb};
use super::record::{CopyOf, RecordSource, SessionRecord};
use crate::acp::agents::registry::SELF_AGENT_ID;

mod blobs;
mod categories;
mod copies;
mod database;
mod index;
mod prefs;
mod read;
mod trash;
mod write;

pub use self::categories::apply_category_op;

const META_FILES: [&str; 3] = ["index.json", "prefs.json", "categories.json"];
const TRASH_DIR: &str = "trash";
const SAVE_DEBOUNCE: Duration = Duration::from_millis(400);
const SAVE_MAX_WAIT: Duration = Duration::from_millis(2000);
/// How stale the compatibility stub of a database-backed session may run while only its head changes (a title, the
/// update time): an older engine's list reads it, nothing else does. A flush, new blobs or a foreign write refresh it at once
const STUB_REFRESH: Duration = Duration::from_secs(30);
/// How long a database body whose compatibility file vanished (an older engine deleted the session) is kept
const LOST_BODY_KEPT: Duration = Duration::from_secs(30 * 24 * 3600);
/// The pre-migration archive is written once per session and read only when SQLite is unavailable
const ARCHIVE_LEVEL: i32 = 3;

/// Session ids name files and directories under the store root: plain tokens only (the rule session links parse by)
pub use acpira_shared::session_links::is_session_id;

pub(crate) fn is_blob_name(name: &str) -> bool {
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

/// `blob_name` of a file's contents, read in pieces (a blob can be large). Blocking
pub fn blob_name_of_file(ext: &str, path: &Path) -> std::io::Result<String> {
  use std::io::Read;
  let mut f = std::fs::File::open(path)?;
  let mut h = Sha256::new();
  let mut buf = vec![0u8; 256 * 1024];
  loop {
    let n = f.read(&mut buf)?;
    if n == 0 {
      break;
    }
    h.update(&buf[..n]);
  }
  let hex: String = h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect();
  Ok(format!("{hex}{ext}"))
}

/// Cross-session memory that is not a setting; unknown keys another build wrote are kept
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPrefs {
  #[serde(default)]
  pub last_settings: BTreeMap<AgentId, TurnSettings>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model_shapes: Option<BTreeMap<AgentId, ModelShapes>>,
  /// What the user last ran each model with, per agent: the model's own parameters only, from the user's own picks
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub model_memory: BTreeMap<AgentId, ModelMemory>,
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
  /// Sessions loaded from their pre-migration archive because the database holding their body could not be opened:
  /// what this store has of them is old, and writing it would bury the newer body
  stale: HashSet<String>,
  /// Record files the file backend has checked to be whole records, not compatibility stubs
  plain: HashSet<String>,
}

#[derive(Default)]
struct DatabaseState {
  attempted: bool,
  db: Option<Arc<SessionDb>>,
  /// The database was found damaged and set aside: its bodies are gone for this store, the files are all there is
  quarantined: bool,
}

/// The compatibility stub of a database-backed session as this store last wrote it
struct StubState {
  /// The record head it shows, by its row hash
  head: Option<[u8; 16]>,
  at: Instant,
  /// The file afterwards: another stamp means another engine wrote it
  stamp: Option<FileStamp>,
}

/// What the search needs to know of a record to tell whether its cached text is current
pub struct SearchSource {
  pub id: String,
  /// The record file's time and size; a database body's revision in place of the size, beside its stub's time (the
  /// hits are ordered by the time, whichever backend holds the session)
  pub stamp: (Option<SystemTime>, u64),
  pub in_database: bool,
}

type FileStamp = (u64, Option<SystemTime>);

async fn file_stamp(path: &Path) -> Option<FileStamp> {
  fs::metadata(path).await.ok().map(|m| (m.len(), m.modified().ok()))
}

fn file_modified_ns(time: Option<SystemTime>) -> Option<i64> {
  let nanos = time?.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_nanos();
  i64::try_from(nanos).ok()
}

fn timestamp_ms(time: SystemTime) -> Option<i64> {
  i64::try_from(time.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_millis()).ok()
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
  /// Per session, the diff text blobs (`diff_texts`) its folder is known to hold, so a record write hands back only new ones
  texts: parking_lot::Mutex<HashMap<String, Arc<parking_lot::Mutex<HashSet<String>>>>>,
  // Opened on first use: constructing a store must not create its sessions directory. Failure keeps the JSON backend.
  db: parking_lot::Mutex<DatabaseState>,
  /// Per session, what the row store was last handed (`db::SaveCache`). A save takes it out (leaving None) and puts it
  /// back only if the entry is still there: `forget_saved` in between means the rows changed behind that save
  saves: parking_lot::Mutex<HashMap<String, Option<SaveCache>>>,
  stubs: parking_lot::Mutex<HashMap<String, StubState>>,
  /// Sessions whose folder gained a blob since their stub was written: the next save names it there
  stub_stale: parking_lot::Mutex<HashSet<String>>,
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

/// A record's `updatedAt`, folder and native-log owner alone, the transcript skipped over by the parser
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stamped {
  #[serde(default)]
  updated_at: String,
  #[serde(default)]
  cwd: String,
  #[serde(default)]
  copy_of: Option<CopyOf>,
  #[serde(default)]
  agent: String,
  #[serde(default)]
  acp_session_id: Option<String>,
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
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub copy_of: Option<CopyOf>,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub taken_over: bool,
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
  builtin_session_in(&fs::read(path).await.ok()?)
}

/// `builtin_session_of` for a record (or record head) already read
fn builtin_session_in(raw: &[u8]) -> Option<String> {
  let owner: LogOwner = serde_json::from_slice(raw).ok()?;
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
      state: parking_lot::Mutex::new(State { pending: HashMap::new(), known: HashSet::new(), unreadable: HashMap::new(), models: HashMap::new(), stale: HashSet::new(), plain: HashSet::new() }),
      inflight: Default::default(),
      write_guard: Default::default(),
      agent_home: Default::default(),
      on_change: Default::default(),
      texts: Default::default(),
      db: Default::default(),
      saves: Default::default(),
      stubs: Default::default(),
      stub_stale: Default::default(),
      me: me.clone(),
    })
  }

  pub fn dir(&self) -> &Path {
    &self.dir
  }

  /// `ACPIRA_STORE=files`: bodies are written as whole record files again. The database is still read, so a migrated
  /// session opens with everything it has and moves back to its file on its next save
  fn files_mode() -> bool {
    std::env::var("ACPIRA_STORE").is_ok_and(|value| value.eq_ignore_ascii_case("files"))
  }

  fn database(&self) -> Option<Arc<SessionDb>> {
    let mut state = self.db.lock();
    if !state.attempted {
      // A read of a missing store stays side-effect free. `ensure` creates the directory before writes and list syncs.
      if !self.dir.is_dir() {
        return None;
      }
      state.attempted = true;
      state.db = SessionDb::open(&self.dir).inspect_err(|e| (self.log)(&format!("sessions: SQLite unavailable, using record files ({e:#})"))).ok().map(Arc::new);
    }
    state.db.clone()
  }

  /// Forget what this store remembers of a session's rows and stub: they changed behind the save path
  fn forget_saved(&self, id: &str) {
    self.saves.lock().remove(id);
    self.stubs.lock().remove(id);
  }

  /// Whether this store keeps database bodies without a compatibility file (`SessionDb::stubless`)
  fn stubless(&self) -> bool {
    self.database().is_some_and(|db| db.stubless())
  }

  /// A stubless store's catalogue entry for a database body: what stands in for the record file a session is known by
  async fn stubless_entry(&self, id: &str) -> Option<CatalogEntry> {
    if !self.stubless() {
      return None;
    }
    let key = id.to_owned();
    self.db_call(move |db| db.entry(&key)).await.ok().flatten().flatten().filter(|entry| entry.body == BodyLocation::Database)
  }

  /// A stubless store's database body that is listed (not in the trash)
  async fn listed_body(&self, id: &str) -> bool {
    self.stubless_entry(id).await.is_some_and(|entry| entry.trashed_at.is_none())
  }

  /// Whether the session waits in the trash here: its record file there, or a stubless body the catalogue has trashed
  async fn in_trash(&self, id: &str) -> bool {
    fs::metadata(self.dir.join(TRASH_DIR).join(format!("{id}.json"))).await.is_ok() || self.stubless_entry(id).await.is_some_and(|entry| entry.trashed_at.is_some())
  }

  /// Whether a session is stored here and listed: its record file in the live directory, or a listed stubless body
  pub async fn has_record(&self, id: &str) -> bool {
    is_session_id(id) && (fs::try_exists(self.dir.join(format!("{id}.json"))).await.unwrap_or(false) || self.listed_body(id).await)
  }

  /// Sessions a stubless store's catalogue has in the trash: they have no record file there to say whose built-in agent
  /// log they keep for their undo
  pub async fn trashed_summaries(&self) -> Vec<SessionSummary> {
    if !self.stubless() {
      return vec![];
    }
    let Ok(Some(entries)) = self.db_call(SessionDb::catalog).await else { return vec![] };
    entries.into_iter().filter(|entry| entry.body == BodyLocation::Database && entry.trashed_at.is_some()).map(|entry| entry.summary).collect()
  }

  /// What moves whenever the stored session changes, for a reader polling it (the mirror of a session another engine
  /// drives): the record file's time, or a stubless body's revision. None when the session is not stored here
  pub async fn record_stamp(&self, id: &str) -> Option<(Option<SystemTime>, u64)> {
    if !is_session_id(id) {
      return None;
    }
    if let Some((_, modified)) = file_stamp(&self.dir.join(format!("{id}.json"))).await {
      return Some((modified, 0));
    }
    if !self.listed_body(id).await {
      return None;
    }
    let key = id.to_owned();
    self.db_call(move |db| db.body_revision(&key)).await.ok().flatten().flatten().map(|revision| (None, revision))
  }

  /// A database body's revision; None for a session whose body is not in the database. Blocking
  pub fn body_revision(&self, id: &str) -> Option<u64> {
    self.database()?.body_revision(id).ok().flatten()
  }

  /// The folder a record names: its file, else a database body's head
  async fn cwd_of_session(&self, id: &str, file: &Path) -> String {
    let cwd = cwd_of(file).await;
    if !cwd.is_empty() {
      return cwd;
    }
    self.db_head(id).await.and_then(|raw| serde_json::from_slice::<Stamped>(&raw).ok()).map(|s| s.cwd).unwrap_or_default()
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

  /// Notify change hooks that a record moved away from `old_cwd` so `HostMirror` drops any copy left on that host
  pub fn notify_left_cwd(&self, id: &str, old_cwd: &str) {
    if self.on_change.read().is_some() {
      self.changed(id, RecordChange::Gone { cwd: old_cwd });
    }
  }

  async fn ensure(&self) -> Result<()> {
    fs::create_dir_all(&self.dir).await?;
    Ok(())
  }

  /// The record head of a database body (`SessionDb::load_head`); None for a file-backed session
  async fn db_head(&self, id: &str) -> Option<Vec<u8>> {
    let id = id.to_owned();
    self.db_call(move |db| db.load_head(&id)).await.ok().flatten().flatten()
  }

  async fn set_trashed(&self, id: &str, trashed_at: Option<i64>) -> Result<()> {
    let id = id.to_owned();
    let _ = self.db_call(move |db| db.set_trashed(&id, trashed_at)).await?;
    Ok(())
  }

  async fn read_db_record(&self, id: &str) -> Result<Option<SessionRecord>, RecordLoadError> {
    let id = id.to_owned();
    self
      .db_call(move |db| db.load_record(&id))
      .await
      .map(|record| record.flatten())
      .map_err(|error| RecordLoadError::Corrupt { reason: format!("SQLite: {error:#}") })
  }

  async fn db_call<T>(&self, f: impl FnOnce(&SessionDb) -> Result<T> + Send + 'static) -> Result<Option<T>>
  where
    T: Send + 'static,
  {
    let Some(db) = self.database() else { return Ok(None) };
    Ok(Some(tokio::task::spawn_blocking(move || f(&db)).await??))
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

#[cfg(test)]
mod tests;
