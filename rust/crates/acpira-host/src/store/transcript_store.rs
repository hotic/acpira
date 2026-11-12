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
use acpira_shared::transcript::{AgentId, CategoryOp, DiffSource, collect_text_blobs, SessionCategories, SessionControls, SessionModel, SessionSummary, SlashCommand, TurnSettings};

use super::blob_pool;
use super::db::{BodyLocation, CatalogEntry, Health, SaveCache, Saved, SessionDb, migration};
use super::diff_texts;
use super::pack;
use super::file_lock::{with_file_lock, write_atomic};
use super::record::{RecordParts, RecordSource, SessionRecord};
use crate::acp::agents::registry::SELF_AGENT_ID;
use crate::i18n::tp;

mod categories;

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

  /// Write the current database body back to a complete legacy record, then remove its rows. The file comes first so a
  /// crash can only leave the old database body authoritative, never a stub with no database behind it.
  pub async fn revert_to_file(&self, id: &str) -> Result<bool> {
    if !is_session_id(id) {
      return Err(anyhow!("{id} is not a session id"));
    }
    self.ensure().await?;
    let chain = self.chain(id);
    let _g = chain.lock().await;
    let location = {
      let id = id.to_owned();
      self.db_call(move |db| db.body_location(&id)).await?.flatten()
    };
    if location != Some(BodyLocation::Database) {
      return Ok(false);
    }
    let record = self.load_detailed(id).await.map_err(|error| anyhow!(error.detail()))?;
    self.write_whole(id, &serde_json::to_vec(&record)?, record.summary()).await?;
    Ok(true)
  }

  /// A complete record file in place of whatever the session had, the catalogue following: the rows a database body
  /// had go once the file is there
  async fn write_whole(&self, id: &str, bytes: &[u8], summary: SessionSummary) -> Result<()> {
    let path = confined(&self.dir, &format!("{id}.json")).await.ok_or_else(|| anyhow!("session {id}: outside the store"))?;
    write_atomic(&path, bytes, None).await?;
    // The archive of an earlier migration is older than this file: nothing may fall back to it from here on
    rm_confined(&self.dir.join(id), migration::MIGRATED_FILE, false).await;
    self.forget_saved(id);
    self.state.lock().plain.insert(id.to_owned());
    let stamp = file_stamp(&path).await.map(|(size, modified)| (size, file_modified_ns(modified)));
    let _ = self.db_call(move |db| db.mark_file(&summary, stamp)).await?;
    Ok(())
  }

  /// Run SQLite's routine checks. Only a database found damaged is set aside under a quarantine name, this store
  /// carrying on with record files; a lock another engine holds or a disk hiccup is an error to log, and the database
  /// stays where it is
  pub async fn maintain_database(&self) -> Result<()> {
    match self.db_call(SessionDb::maintain).await? {
      Some(Health::Corrupt(why)) => {
        if let Err(quarantine) = self.quarantine_database().await {
          (self.log)(&format!("sessions: SQLite quarantine failed ({quarantine:#})"));
        }
        Err(anyhow!("SQLite check failed: {why}"))
      }
      Some(Health::Sound) => {
        self.purge_lost_bodies().await;
        Ok(())
      }
      None => Ok(()),
    }
  }

  /// Database bodies whose compatibility file has been gone from both the list and the trash for `LOST_BODY_KEPT`: an
  /// older engine deleted those sessions. Until then a reconcile only hides them, so a file that is missing for any
  /// other reason costs nothing
  async fn purge_lost_bodies(&self) {
    let Ok(Some(entries)) = self.db_call(SessionDb::catalog).await else { return };
    let cutoff = SystemTime::now().checked_sub(LOST_BODY_KEPT).and_then(timestamp_ms).unwrap_or(0);
    for entry in entries {
      let id = entry.summary.id;
      if entry.body != BodyLocation::Database || entry.trashed_at.is_none_or(|at| at > cutoff) {
        continue;
      }
      let file = format!("{id}.json");
      if fs::metadata(self.dir.join(&file)).await.is_ok() || fs::metadata(self.dir.join(TRASH_DIR).join(&file)).await.is_ok() {
        continue;
      }
      self.forget_saved(&id);
      let gone = id.clone();
      if let Err(error) = self.db_call(move |db| db.delete_session(&gone)).await {
        (self.log)(&format!("session {id}: SQLite row not removed ({error:#})"));
      }
    }
  }

  async fn quarantine_database(&self) -> Result<()> {
    {
      let mut state = self.db.lock();
      state.db = None;
      state.quarantined = true;
    }
    self.saves.lock().clear();
    self.stubs.lock().clear();
    let stamp = timestamp_ms(SystemTime::now()).unwrap_or_default();
    let mut kept = vec![];
    for suffix in ["", "-wal", "-shm"] {
      let source = self.dir.join(format!("{}{suffix}", super::db::DB_FILE));
      if fs::metadata(&source).await.is_err() {
        continue;
      }
      let target = self.dir.join(format!("{}{suffix}.corrupt-{stamp}", super::db::DB_FILE));
      fs::rename(&source, &target).await?;
      kept.push(target.display().to_string());
    }
    (self.log)(&format!("sessions: SQLite failed, using record files; kept {}", kept.join(", ")));
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
    if self.updated_at_of(id).await.is_some_and(|here| here > record.updated_at) {
      return Ok(false);
    }
    let summary = record.summary();
    if self.database().is_some() {
      // Remote copies stay whole JSON on disk; a migrated local copy gives its rows up once the file is complete
      self.write_whole(id, &serde_json::to_vec(record)?, summary).await?;
    } else {
      write_atomic(&path, &serde_json::to_vec(record)?, None).await?;
      // The file backend's index entry goes with the record for engines that do not drive this copy.
      let mut index = self.read_index().await;
      match index.iter_mut().find(|s| s.id == *id) {
        Some(entry) => *entry = summary,
        None => index.push(summary),
      }
      sort_index(&mut index);
      write_atomic(&self.dir.join("index.json"), serde_json::to_string_pretty(&index)?.as_bytes(), None).await?;
    }
    Ok(true)
  }

  /// A stored record's `updatedAt` without building it: the head row of a database body, else the file with its
  /// transcript skipped over
  async fn updated_at_of(&self, id: &str) -> Option<String> {
    let raw = match self.db_head(id).await {
      Some(head) => head,
      None => fs::read(self.dir.join(format!("{id}.json"))).await.ok()?,
    };
    serde_json::from_slice::<Stamped>(&raw).ok().map(|stamped| stamped.updated_at)
  }

  /// The record head of a database body (`SessionDb::load_head`); None for a file-backed session
  async fn db_head(&self, id: &str) -> Option<Vec<u8>> {
    let id = id.to_owned();
    self.db_call(move |db| db.load_head(&id)).await.ok().flatten().flatten()
  }

  /// For each of `ids` with a record here (not in the trash): its `updatedAt` and the blobs it has
  pub async fn copy_stamps(&self, ids: &[String]) -> HashMap<String, CopyStamp> {
    let mut out = HashMap::new();
    for id in ids.iter().filter(|id| is_session_id(id)) {
      if fs::metadata(self.dir.join(TRASH_DIR).join(format!("{id}.json"))).await.is_ok() {
        continue;
      }
      let Some(updated_at) = self.updated_at_of(id).await else { continue };
      out.insert(id.clone(), CopyStamp { updated_at, blobs: self.blob_names(id).await });
    }
    out
  }

  /// What the search checks its cache against: every listed record with the stamp its text goes by
  pub async fn search_sources(&self) -> Vec<SearchSource> {
    let files = self.record_files().await.unwrap_or_default();
    let bodies: HashMap<String, u64> = self.db_call(SessionDb::body_revisions).await.ok().flatten().unwrap_or_default().into_iter().collect();
    files
      .into_iter()
      .map(|(id, (len, modified))| match bodies.get(&id) {
        Some(revision) => SearchSource { id, stamp: (modified, *revision), in_database: true },
        None => SearchSource { id, stamp: (modified, len), in_database: false },
      })
      .collect()
  }

  /// A database body as record JSON, not parsed; None for a file-backed session. `main_only` leaves the subagents'
  /// transcripts out. Blocking
  pub fn body_text(&self, id: &str, main_only: bool) -> Option<Vec<u8>> {
    self.database()?.load_raw(id, main_only).inspect_err(|error| (self.log)(&format!("session {id}: body unreadable ({error:#})"))).ok().flatten()
  }

  /// Stored bytes per database body, for the Storage tab
  pub async fn body_sizes(&self) -> HashMap<String, u64> {
    self.db_call(SessionDb::body_sizes).await.ok().flatten().unwrap_or_default().into_iter().collect()
  }

  /// How many subagents a database body holds; None for a file-backed session. Blocking
  pub fn body_subagents(&self, id: &str) -> Option<u32> {
    let db = self.database()?;
    (db.body_location(id).ok().flatten() == Some(BodyLocation::Database)).then(|| db.subagent_count(id).unwrap_or(0))
  }

  /// A record file as the whole record's JSON: a packed one expanded, a compatibility stub whose database is out of
  /// reach as its pre-migration archive. Blocking
  pub fn file_text(&self, id: &str) -> Option<Vec<u8>> {
    let raw = std::fs::read(self.dir.join(format!("{id}.json"))).ok()?;
    if migration::is_stub(&raw) {
      let archive = std::fs::read(self.dir.join(id).join(migration::MIGRATED_FILE)).ok();
      return Some(archive.and_then(|bytes| zstd::stream::decode_all(bytes.as_slice()).ok()).unwrap_or(raw));
    }
    Some(pack::expand_raw(&self.dir, raw))
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
  /// truth for unmigrated bodies; where both have an entry, `mine` wins only for ids in `own`. Debounced records flush first.
  pub async fn sync_index(&self, mine: &[SessionSummary], own: &HashSet<String>) -> Result<Vec<SessionSummary>> {
    self.ensure().await?;
    self.flush_pending().await;
    if self.database().is_some() { self.sync_catalog(mine, own).await } else { self.sync_index_files(mine, own).await }
  }

  async fn sync_index_files(&self, mine: &[SessionSummary], own: &HashSet<String>) -> Result<Vec<SessionSummary>> {
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
          let Some(summary) = self.load_summary(&id, other.cloned(), stamp).await else { continue };
          summary
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

  /// Reconcile the SQLite catalogue with record files at startup and on the bus's index event, writing only the entries
  /// that differ. The initial index import has no stamp and is trusted once; a later stamp change means another engine
  /// rewrote the file and forces a fresh read. A database body is never dropped here: a session whose compatibility
  /// file is missing is hidden like a trashed one (`purge_lost_bodies` removes it much later), and one that cannot be
  /// read right now keeps the entry it has
  async fn sync_catalog(&self, mine: &[SessionSummary], own: &HashSet<String>) -> Result<Vec<SessionSummary>> {
    let mut entries = self.db_call(SessionDb::catalog).await?.unwrap_or_default();
    // An empty catalogue starts from the legacy index: nothing of it is stored until its record file is seen below
    let seeded = entries.is_empty();
    if seeded {
      entries = self
        .read_index()
        .await
        .into_iter()
        .map(|summary| CatalogEntry { summary, body: BodyLocation::File, file_size: None, file_modified_ns: None, trashed_at: None })
        .collect();
    }
    let stored: HashMap<String, CatalogEntry> = entries.into_iter().map(|entry| (entry.summary.id.clone(), entry)).collect();
    let local: HashMap<&str, &SessionSummary> = mine.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut seen = HashSet::new();
    let (mut changed, mut gone) = (vec![], vec![]);
    let mut out = vec![];
    for (id, stamp) in self.record_files().await? {
      seen.insert(id.clone());
      let (file_size, modified_ns) = (Some(stamp.0), file_modified_ns(stamp.1));
      let entry = stored.get(&id);
      let stored_summary = entry.map(|entry| &entry.summary);
      let other = stored_summary.or_else(|| local.get(id.as_str()).copied());
      let unchanged = entry.is_some_and(|entry| entry.file_size == file_size && entry.file_modified_ns == modified_ns);
      let initial_import = entry.is_some_and(|entry| entry.file_size.is_none());
      let pick = if own.contains(&id) {
        local.get(id.as_str()).copied().or(stored_summary)
      } else if unchanged || initial_import {
        stored_summary.or_else(|| local.get(id.as_str()).copied())
      } else {
        None
      };
      let mut summary = match pick {
        Some(summary) if !summary.cwd.is_empty() => summary.clone(),
        _ => match self.load_summary(&id, other.cloned(), Some(stamp)).await {
          Some(summary) => summary,
          // A database body that does not read now (a lock, a damaged row) stays listed as the catalogue has it: opening
          // it reports why, and nothing is lost meanwhile. An unreadable file is left out, its entry untouched
          None => match entry.filter(|entry| entry.body == BodyLocation::Database && !entry.summary.cwd.is_empty()) {
            Some(entry) => {
              out.push(entry.summary.clone());
              continue;
            }
            None => continue,
          },
        },
      };
      if summary.model.is_none() {
        summary.model = self.cached_model(&id).await;
      }
      let desired = CatalogEntry { summary: stored_form(&summary), body: entry.map_or(BodyLocation::File, |entry| entry.body), file_size, file_modified_ns: modified_ns, trashed_at: None };
      if seeded || entry != Some(&desired) {
        changed.push(desired);
      }
      out.push(summary);
    }
    // No live record. A file in the trash keeps the entry for undo. Without one, a file-backed entry has nothing left and
    // goes; a database body stays, hidden, in case the file is only missing (a crash before the first stub, a restore
    // from a backup): only an explicit removal or `purge_lost_bodies` takes a body
    if !seeded {
      let now = timestamp_ms(SystemTime::now());
      for entry in stored.values().filter(|entry| !seen.contains(&entry.summary.id)) {
        let trashed_file = self.dir.join(TRASH_DIR).join(format!("{}.json", entry.summary.id));
        let in_trash = fs::metadata(trashed_file).await.ok().and_then(|meta| meta.modified().ok()).and_then(timestamp_ms);
        let trashed_at = match (in_trash, entry.body) {
          (Some(at), _) => Some(at),
          (None, BodyLocation::Database) => entry.trashed_at.or(now),
          (None, BodyLocation::File) => {
            gone.push(entry.summary.id.clone());
            continue;
          }
        };
        if entry.trashed_at != trashed_at {
          changed.push(CatalogEntry { trashed_at, ..entry.clone() });
        }
      }
    }
    sort_index(&mut out);
    if !changed.is_empty() || !gone.is_empty() {
      let _ = self.db_call(move |db| db.apply_catalog(&changed, &gone)).await?;
    }
    Ok(out)
  }

  async fn load_summary(&self, id: &str, other: Option<SessionSummary>, stamp: Option<FileStamp>) -> Option<SessionSummary> {
    if stamp.is_some() && self.state.lock().unreadable.get(id) == stamp.as_ref() {
      return None;
    }
    let record = match self.load_detailed(id).await {
      Ok(record) => record,
      Err(error) => {
        if !error.is_missing() {
          (self.log)(&format!("session {id}: record unreadable ({})", error.detail()));
        }
        // Only wrong content is cached: an I/O failure may clear without the file changing.
        if let (Some(stamp), RecordLoadError::Corrupt { .. } | RecordLoadError::IdMismatch { .. }) = (stamp, &error) {
          self.state.lock().unreadable.insert(id.to_owned(), stamp);
        }
        return None;
      }
    };
    let mut base = other.unwrap_or_else(|| record.summary());
    let fresh = record.summary();
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
    Some(base)
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

  /// A model `read_models` found for this record, while the file is still the one it read
  async fn cached_model(&self, id: &str) -> Option<SessionModel> {
    // A record with no model control costs no stat
    let (stamp, model) = self.state.lock().models.get(id).and_then(|(stamp, m)| Some((*stamp, m.clone()?)))?;
    (file_stamp(&self.dir.join(format!("{id}.json"))).await == Some(stamp)).then_some(model)
  }

  /// Read the current model of each record not read yet (its controls only, the transcript skipped by the parser), for
  /// index entries that have none. True when one was found, so a sync then writes them into the catalogue
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

  /// Every record file in the live directory with its stamp, read in one blocking pass (a stat per file through the
  /// async runtime would cost a thread hop each)
  async fn record_files(&self) -> Result<Vec<(String, FileStamp)>> {
    let dir = self.dir.clone();
    tokio::task::spawn_blocking(move || {
      let mut files = vec![];
      for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if META_FILES.contains(&name.as_str()) {
          continue;
        }
        let Some(id) = name.strip_suffix(".json").filter(|id| is_session_id(id)) else { continue };
        // The directory entry's own metadata costs nothing on Windows; only a link is followed to the record itself
        let Ok(meta) = entry.metadata().and_then(|meta| if meta.file_type().is_symlink() { std::fs::metadata(entry.path()) } else { Ok(meta) }) else { continue };
        files.push((id.to_owned(), (meta.len(), meta.modified().ok())));
      }
      Ok(files)
    })
    .await?
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
    // A session is here while its file is in the live directory, wherever its body is kept: one moved to the trash or
    // deleted by another window is gone for every reader, as it was when the file was the record
    let path = confined(&self.dir, &format!("{id}.json")).await;
    let stamp = match &path {
      Some(path) => file_stamp(path).await,
      None => None,
    };
    if path.is_some() && stamp.is_none() {
      return Err(RecordLoadError::Missing);
    }
    let mut file_wins = false;
    if let Some(mut record) = self.read_db_record(id).await? {
      // The compatibility file is read only when it is not the stub this store wrote: an older engine may have added
      // to it, or a file backend may have written the whole record over it while the database could not be opened
      let ours = stamp.is_some() && self.stubs.lock().get(id).is_some_and(|stub| stub.stamp == stamp);
      if !ours
        && let Some(path) = &path
        && let Ok(raw) = fs::read(path).await
      {
        if migration::is_stub(&raw) {
          if let Ok(stub) = serde_json::from_slice::<SessionRecord>(&raw) {
            record = migration::merge(record, stub, migration::rewritten(&raw)).map_err(|error| RecordLoadError::Corrupt { reason: error.to_string() })?;
          }
        } else {
          file_wins = serde_json::from_slice::<Stamped>(&raw).is_ok_and(|file| file.updated_at > record.updated_at);
        }
      }
      if !file_wins {
        self.state.lock().known.insert(id.to_owned());
        record.fold_turn_usage();
        return Ok(record);
      }
      // A whole record newer than the database body: the file is the session now, and the rows give way to it
      (self.log)(&format!("session {id}: its record file is newer than its database body, the file is kept"));
      self.forget_saved(id);
    }
    let path = path.ok_or(RecordLoadError::StoreUnavailable)?;
    let raw = match fs::read(&path).await {
      Ok(raw) => raw,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(RecordLoadError::Missing),
      Err(e) => return Err(RecordLoadError::Unreadable { reason: e.to_string() }),
    };
    match serde_json::from_slice::<SessionRecord>(&raw) {
      Ok(r) if r.id == id && is_session_id(&r.id) => {
        // Database and cold-pack stubs both keep their archived body beside the compatibility file.
        let mut r = if r.turns.iter().any(migration::is_placeholder) {
          // The body is in a database this store cannot read: the archive has the session as it was when it moved there
          let archive = match fs::read(self.dir.join(id).join(migration::MIGRATED_FILE)).await {
            Ok(bytes) => bytes,
            Err(error) => return Err(RecordLoadError::Unreadable { reason: format!("the session is kept in {}, which is unavailable, and {} cannot be read ({error})", super::db::DB_FILE, migration::MIGRATED_FILE) }),
          };
          if !self.db.lock().quarantined {
            self.state.lock().stale.insert(id.to_owned());
          }
          migration::splice_archive(r, &archive).map_err(|error| RecordLoadError::Corrupt { reason: error.to_string() })?
        } else if r.turns.iter().any(pack::is_placeholder) {
          // Only a missing archive means there is none; one that cannot be read now must not load as a record without
          // its history, or the next write would make that loss permanent
          let archive = match fs::read(self.dir.join(id).join(pack::PACKED_FILE)).await {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(RecordLoadError::Unreadable { reason: format!("{}: {e}", pack::PACKED_FILE) }),
          };
          pack::splice(r, archive.as_deref()).map_err(|reason| RecordLoadError::Corrupt { reason })?
        } else {
          r
        };
        if file_wins {
          let (summary, stamp) = (r.summary(), file_stamp(&path).await.map(|(size, modified)| (size, file_modified_ns(modified))));
          if let Err(error) = self.db_call(move |db| db.mark_file(&summary, stamp)).await {
            (self.log)(&format!("session {id}: catalogue not updated ({error:#})"));
          }
        }
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
    let raw = match self.db_head(id).await {
      Some(head) => head,
      None => fs::read(confined(&self.dir, &format!("{id}.json")).await?).await.ok()?,
    };
    serde_json::from_slice::<ControlsOnly>(&raw).ok().filter(|r| r.id == id).map(|r| r.controls)
  }

  /// Only a record's slash commands, read like `load_controls` without building the transcript
  pub async fn load_commands(&self, id: &str) -> Option<Vec<SlashCommand>> {
    #[derive(Deserialize)]
    struct CommandsOnly {
      id: String,
      #[serde(default)]
      commands: Vec<SlashCommand>,
    }
    if !is_session_id(id) {
      return None;
    }
    let pending = self.state.lock().pending.get(id).map(|p| p.source.clone());
    if let Some(src) = pending {
      return Some(src.record().commands);
    }
    let raw = match self.db_head(id).await {
      Some(head) => head,
      None => fs::read(confined(&self.dir, &format!("{id}.json")).await?).await.ok()?,
    };
    serde_json::from_slice::<CommandsOnly>(&raw).ok().filter(|r| r.id == id).map(|r| r.commands)
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
      if let Err(e) = self.write_locked(p.source, false).await {
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
    // An explicit flush hands the row store every piece again: whatever an incremental save may have passed over
    // (`RecordSource::record_parts`) is settled here
    self.write(source, true).await
  }

  /// Whether this store has had the record on disk
  pub fn knew(&self, id: &str) -> bool {
    self.state.lock().known.contains(id)
  }

  /// `pack::pack` inside the record's write chain: a write of the same record waits for it, and it for the write, so
  /// the stub never lands on top of a record written meanwhile. A record with a write still pending is not cold
  pub async fn pack(&self, id: &str, now: SystemTime) -> Result<Result<u64, pack::Kept>> {
    if !is_session_id(id) || self.state.lock().pending.contains_key(id) {
      return Ok(Err(pack::Kept::NotCold));
    }
    let chain = self.chain(id);
    let _g = chain.lock().await;
    let (dir, id) = (self.dir.clone(), id.to_owned());
    Ok(tokio::task::spawn_blocking(move || pack::pack(&dir, &id, now)).await??)
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
  async fn write(&self, source: Arc<dyn RecordSource>, full: bool) -> Result<()> {
    let chain = self.chain(&source.record_id());
    let _g = chain.lock().await;
    self.write_locked(source, full).await
  }

  /// `write` with the record's chain already held. `full` asks the source for every piece of the record; without it a
  /// database save may skip what the store is known to hold
  async fn write_locked(&self, source: Arc<dyn RecordSource>, full: bool) -> Result<()> {
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
    let on_disk = file_stamp(&path).await;
    if self.knew(&id) && on_disk.is_none() {
      (self.log)(&format!("session {id}: deleted by another window, not written back"));
      return Ok(());
    }
    match self.database().filter(|_| !Self::files_mode()) {
      Some(db) => self.write_rows(db, &id, &path, on_disk, &source, full).await?,
      None => self.write_file(&id, &path, on_disk.is_some(), &source).await?,
    }
    self.state.lock().known.insert(id.clone());
    if self.on_change.read().is_some() {
      self.changed(&id, RecordChange::Written { cwd: &source.record_cwd() });
    }
    Ok(())
  }

  /// Big diff texts go to the session's blob folder before the record that names them is written, so it never names a
  /// blob that is not there
  async fn store_texts(&self, id: &str, known: &Arc<parking_lot::Mutex<HashSet<String>>>, texts: Vec<(String, String)>) -> Result<()> {
    if texts.is_empty() {
      return Ok(());
    }
    fs::create_dir_all(self.dir.join(id)).await?;
    let dir = confined(&self.dir, id).await.ok_or_else(|| anyhow!("session {id}: blob folder outside the store"))?;
    let pool = self.dir.join(blob_pool::POOL_DIR);
    let texts = tokio::task::spawn_blocking(move || diff_texts::store(&pool, &dir, &texts).map(|_| texts)).await??;
    known.lock().extend(texts.into_iter().map(|(name, _)| name));
    self.stub_stale.lock().insert(id.to_owned());
    Ok(())
  }

  /// The whole record as one file: the backend of a store without a database, and of `ACPIRA_STORE=files`
  async fn write_file(&self, id: &str, path: &Path, exists: bool, source: &Arc<dyn RecordSource>) -> Result<()> {
    if self.state.lock().stale.contains(id) {
      return Err(anyhow!("session {id}: its transcript is kept in {}, which could not be opened; not written, so the newer transcript there is not replaced", super::db::DB_FILE));
    }
    // A stub this store never loaded (a record handed in from elsewhere) has its body in the database too
    if exists && self.database().is_none() && !self.db.lock().quarantined && !self.state.lock().plain.contains(id) {
      if fs::read(path).await.is_ok_and(|raw| migration::is_stub(&raw)) {
        return Err(anyhow!("session {id}: its transcript is kept in {}, which could not be opened; not written", super::db::DB_FILE));
      }
      self.state.lock().plain.insert(id.to_owned());
    }
    let known = self.texts.lock().entry(id.to_owned()).or_default().clone();
    let (bytes, texts) = collect_text_blobs(diff_texts::sink(known.clone(), self.dir.join(id)), || source.record_json());
    self.store_texts(id, &known, texts).await?;
    // `ACPIRA_STORE=files` with the database open: a body it holds moves back to its file with this write
    let key = id.to_owned();
    if self.db_call(move |db| db.body_location(&key)).await?.flatten() == Some(BodyLocation::Database) {
      let summary = serde_json::from_slice::<SessionRecord>(&bytes)?.summary();
      return self.write_whole(id, &bytes, summary).await;
    }
    write_atomic(path, &bytes, None).await
  }

  /// A database save: the pieces that changed go to their rows in one transaction, then the compatibility stub follows
  /// when it is due. The body is committed first, so a failure afterwards leaves a complete file or a stale stub, both
  /// of which the next save repairs
  async fn write_rows(&self, db: Arc<SessionDb>, id: &str, path: &Path, on_disk: Option<FileStamp>, source: &Arc<dyn RecordSource>, full: bool) -> Result<()> {
    let known = self.texts.lock().entry(id.to_owned()).or_default().clone();
    // Out of its slot while the save runs: a failure leaves the slot empty, and the next save reads the stored hashes again
    let mut cache = self.saves.lock().insert(id.to_owned(), None).flatten().unwrap_or_default();
    let mut full = full;
    let parts = loop {
      let skip = if full { HashMap::new() } else { cache.known() };
      let (parts, texts) = collect_text_blobs(diff_texts::sink(known.clone(), self.dir.join(id)), || source.record_parts(&skip, full));
      self.store_texts(id, &known, texts).await?;
      let (saving, key) = (db.clone(), id.to_owned());
      let (saved, parts, back) = tokio::task::spawn_blocking(move || {
        let saved = saving.save_parts(&key, &parts, &mut cache);
        (saved, parts, cache)
      })
      .await?;
      cache = back;
      match saved? {
        Saved::Done(_) => break parts,
        Saved::NeedsFull if !full => full = true,
        Saved::NeedsFull => return Err(anyhow!("session {id}: the row store asked for more than the whole record")),
      }
    };
    let head = cache.head();
    if let Some(slot) = self.saves.lock().get_mut(id) {
      *slot = Some(cache);
    }
    self.state.lock().stale.remove(id);
    let grown = self.stub_stale.lock().remove(id);
    let due = match self.stubs.lock().get(id) {
      None => true,
      Some(stub) => stub.stamp != on_disk || grown || (stub.head != head && (full || stub.at.elapsed() >= STUB_REFRESH)),
    };
    if due {
      self.write_stub(&db, id, path, on_disk.is_some(), &parts, head).await.inspect_err(|_| {
        // Whatever the stub had to tell is still owed
        self.stubs.lock().remove(id);
      })?;
    }
    Ok(())
  }

  /// The small record an older engine reads in place of a database body (`migration::stub`). One written over a whole
  /// record archives that record beside the blobs first: it is what a store without the database falls back to. An
  /// archive already there is from an earlier migration (the session went back to its file since) and is replaced
  async fn write_stub(&self, db: &Arc<SessionDb>, id: &str, path: &Path, exists: bool, parts: &RecordParts, head: Option<[u8; 16]>) -> Result<()> {
    let folder = self.dir.join(id);
    if exists && !self.stubs.lock().contains_key(id) {
      let raw = fs::read(path).await?;
      if !migration::is_stub(&raw) {
        fs::create_dir_all(&folder).await?;
        let archive = confined(&folder, migration::MIGRATED_FILE).await.ok_or_else(|| anyhow!("session {id}: migration archive outside the store"))?;
        let dir = self.dir.clone();
        // A cold-packed record is archived whole, like any other
        let packed = tokio::task::spawn_blocking(move || zstd::bulk::compress(&pack::expand_raw(&dir, raw), ARCHIVE_LEVEL)).await??;
        write_atomic(&archive, &packed, None).await?;
      }
    }
    let names = self.blob_names(id).await;
    write_atomic(path, &migration::stub(&parts.head, &names)?, None).await?;
    let stamp = file_stamp(path).await;
    self.stubs.lock().insert(id.to_owned(), StubState { head, at: Instant::now(), stamp });
    if let Some((size, modified)) = stamp {
      // The catalogue learns the stub's stamp, so the next reconcile does not read the record to see what changed
      let (db, key) = (db.clone(), id.to_owned());
      tokio::task::spawn_blocking(move || db.set_file_stamp(&key, size, file_modified_ns(modified))).await??;
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
    {
      let mut state = self.state.lock();
      state.known.remove(id);
      state.stale.remove(id);
      state.plain.remove(id);
    }
    self.texts.lock().remove(id);
    self.forget_saved(id);
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
    let db_id = id.to_owned();
    if let Err(error) = self.db_call(move |db| db.delete_session(&db_id)).await {
      (self.log)(&format!("session {id}: SQLite row not removed ({error:#})"));
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
    let listed = match self.load_index().await {
      Ok(index) => index.into_iter().any(|s| s.id != removed && s.agent == SELF_AGENT_ID && s.acp_session_id.as_deref() == Some(acp)),
      Err(error) => {
        (self.log)(&format!("session {removed}: catalogue unreadable, built-in agent log kept ({error})"));
        return;
      }
    };
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
    // The session's scratch folder in its project, found through the log's header, so before the log goes. One left
    // behind (a file Windows keeps open) is swept by the next session there (`acpira_agent::scratch::sweep`)
    if let Some(log) = confined(&root, &format!("{acp}.jsonl")).await {
      let data = home.clone();
      if let Ok(Err(e)) = tokio::task::spawn_blocking(move || acpira_agent::scratch::remove_for_log(&log, &data)).await {
        (self.log)(&format!("session {removed}: scratch folder not removed ({e})"));
      }
    }
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
    self.forget_saved(id);
    let trash = self.dir.join(TRASH_DIR);
    fs::create_dir_all(&trash).await?;
    let file = format!("{id}.json");
    let now = SystemTime::now();
    self.set_trashed(id, timestamp_ms(now)).await?;
    // The record leaves first: an entry left behind by a failed undo is then a stray blob folder, not a listed session.
    let moved = async {
      real_dir(&trash).await?;
      move_entries(&self.dir, &trash, [file.as_str(), id]).await
    }
    .await;
    let moved = match moved {
      Ok(moved) => moved,
      Err(error) => {
        let _ = self.set_trashed(id, None).await;
        return Err(anyhow!("session {id}: not moved to the trash ({error:#})"));
      }
    };
    // rename keeps the mtime; stamp the moment it was trashed (a regular file only, so a symlink's target is not touched)
    let stamped = trash.join(&file);
    if moved.contains(&file.as_str())
      && fs::symlink_metadata(&stamped).await.is_ok_and(|m| m.file_type().is_file())
      && let Ok(f) = std::fs::File::options().write(true).open(&stamped)
    {
      let _ = f.set_modified(now);
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
    self.set_trashed(id, None).await?;
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
      // Still in the trash (not removable now), or back in the list (another engine's undo since the listing above): the
      // body stays with its record
      if entry_exists(&e.path()).await.unwrap_or(true) || entry_exists(&self.dir.join(&f)).await.unwrap_or(true) {
        continue;
      }
      let db_id = id.to_owned();
      if let Err(error) = self.db_call(move |db| db.delete_session(&db_id)).await {
        (self.log)(&format!("session {id}: SQLite row not removed ({error:#})"));
      }
      if let Some(acp) = log {
        self.remove_agent_log(id, &acp).await;
      }
    }
  }

  /// Writes whatever is still debounced
  pub async fn dispose(&self) {
    self.flush_queued(true).await;
  }

  /// The debounced saves brought forward, as they would have been written
  async fn flush_pending(&self) {
    self.flush_queued(false).await;
  }

  async fn flush_queued(&self, full: bool) {
    let queued: Vec<(String, Arc<dyn RecordSource>)> = self.state.lock().pending.drain().map(|(k, p)| (k, p.source)).collect();
    for (id, src) in queued {
      if let Err(e) = self.write(src, full).await {
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
    // Shared with every session holding the same content (`blob_pool`); a file already there is never rewritten
    let (pool, owned, file) = (self.dir.join(blob_pool::POOL_DIR), bytes.to_vec(), name.clone());
    let path = tokio::task::spawn_blocking(move || blob_pool::place(&pool, &dir, &file, &owned)).await??;
    self.stub_stale.lock().insert(session_id.to_owned());
    Ok((name, path))
  }

  /// Copy blob `name` of session `from` into session `to` under the same name (one file on disk, `blob_pool`)
  pub async fn copy_blob(&self, from: &str, to: &str, name: &str) -> Result<()> {
    let bytes = self.read_blob(from, name).await?;
    self.put_blob(to, name, bytes).await
  }

  /// Store `bytes` as blob `name` of session `id`; the caller vouches that the name fits the content (`save_blob` names
  /// by the bytes, a diff text blob by its text, `diff_texts::name_of`)
  pub async fn put_blob(&self, id: &str, name: &str, bytes: Vec<u8>) -> Result<()> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{id}/{name}"))]));
    if !is_session_id(id) || !is_blob_name(name) {
      return Err(illegal());
    }
    fs::create_dir_all(self.dir.join(id)).await?;
    let dir = confined(&self.dir, id).await.ok_or_else(illegal)?;
    let (pool, file) = (self.dir.join(blob_pool::POOL_DIR), name.to_owned());
    tokio::task::spawn_blocking(move || blob_pool::place(&pool, &dir, &file, &bytes)).await??;
    self.stub_stale.lock().insert(id.to_owned());
    Ok(())
  }

  /// `src` with its texts read back from the session's blobs when the record keeps them there (`diff_texts`)
  pub async fn resolve_diff(&self, session_id: &str, src: DiffSource) -> Option<DiffSource> {
    diff_texts::resolve(src, |name| async move { self.read_blob(session_id, &name).await }).await
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

  /// Blob `name` of session `id` opened for reading, with its length
  pub async fn open_blob(&self, session_id: &str, name: &str) -> Result<(fs::File, u64)> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{session_id}/{name}"))]));
    if !is_session_id(session_id) || !is_blob_name(name) {
      return Err(illegal());
    }
    let dir = confined(&self.dir, session_id).await.ok_or_else(illegal)?;
    let file = confined(&dir, name).await.ok_or_else(illegal)?;
    let f = fs::File::open(file).await?;
    let len = f.metadata().await?.len();
    Ok((f, len))
  }

  /// Where a blob still coming over (`session_manager/hosts.rs`) is written: inside the sessions folder, so it moves in
  /// with a rename, and under a dot name no reader takes for a session
  pub fn partial_dir(&self) -> PathBuf {
    self.dir.join(".partial")
  }

  /// Move the finished file `src` in as blob `name` of session `id` (`blob_pool::adopt`); the caller checked that the
  /// name fits the content
  pub async fn adopt_blob(&self, id: &str, name: &str, src: PathBuf) -> Result<PathBuf> {
    let illegal = || anyhow!(tp("host.blobIllegal", &[("path", &format!("{id}/{name}"))]));
    if !is_session_id(id) || !is_blob_name(name) {
      return Err(illegal());
    }
    fs::create_dir_all(self.dir.join(id)).await?;
    let dir = confined(&self.dir, id).await.ok_or_else(illegal)?;
    let (pool, file) = (self.dir.join(blob_pool::POOL_DIR), name.to_owned());
    let path = tokio::task::spawn_blocking(move || blob_pool::adopt(&pool, &dir, &file, &src)).await??;
    self.stub_stale.lock().insert(id.to_owned());
    Ok(path)
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

/// A summary as the catalogue keeps it: what a session is, without the marks of what it is doing right now (those
/// belong to the engine driving it, and would rewrite the row on every state change)
fn stored_form(summary: &SessionSummary) -> SessionSummary {
  SessionSummary { state: None, unread: None, question: None, wake_at: None, ..summary.clone() }
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
  async fn an_unavailable_database_keeps_the_file_backend_usable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(crate::store::db::DB_FILE)).unwrap();
    let messages = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let logged = messages.clone();
    let s = TranscriptStore::with_timing(
      dir.path().to_path_buf(),
      Arc::new(move |message| logged.lock().push(message.to_owned())),
      None,
      Duration::from_millis(20),
      Duration::from_millis(60),
    );
    assert!(s.database().is_none());
    s.flush(Arc::new(record("files", "2026-01-01T00:00:00.000Z"))).await.unwrap();
    assert_eq!(s.load("files").await.unwrap().id, "files");
    s.load_index().await.unwrap();
    assert!(dir.path().join("index.json").exists(), "the fallback still maintains the legacy index");
    assert!(messages.lock().iter().any(|message| message.contains("using record files")));
  }

  #[tokio::test]
  async fn save_debounces_and_index_reconciles() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    s.save(Arc::new(record("a", "2026-01-01T00:00:00.000Z")));
    s.save(Arc::new(record("b", "2026-01-02T00:00:00.000Z")));
    assert!(s.load("a").await.is_some(), "pending record is served from memory");
    // The debounce is 20 ms; the first save also opens the database, which a loaded machine takes its time over
    for _ in 0..200 {
      if dir.path().join("a.json").exists() && dir.path().join("b.json").exists() {
        break;
      }
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(dir.path().join("a.json").exists());
    let idx = s.load_index().await.unwrap();
    assert_eq!(idx.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
    assert!(!dir.path().join("index.json").exists(), "the SQLite catalogue replaces index.json");
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
  async fn turns_an_older_engine_added_to_a_migration_stub_are_recovered() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("legacy", "2026-01-01T00:00:00.000Z");
    stored.turns = serde_json::from_value(serde_json::json!([{ "role": "user", "text": "before migration" }])).unwrap();
    s.flush(Arc::new(stored.clone())).await.unwrap();
    let path = dir.path().join("legacy.json");
    let mut stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stub.turns.push(serde_json::from_value(serde_json::json!({ "role": "user", "text": "added by old engine" })).unwrap());
    stub.subagents = Some(vec![serde_json::from_value(serde_json::json!({ "id": "old-child", "turnIndex": 1, "visibility": "session", "state": "completed", "stateSource": "agent", "announcedAt": 1, "turns": [] })).unwrap()]);
    std::fs::write(&path, serde_json::to_vec(&stub).unwrap()).unwrap();

    let recovered = s.load("legacy").await.unwrap();
    assert_eq!(recovered.turns.len(), 2);
    assert_eq!(recovered.turns[1].as_user().unwrap().text, "added by old engine");
    assert_eq!(recovered.subagents.as_ref().unwrap()[0].core.id, "old-child");
    s.flush(Arc::new(recovered.clone())).await.unwrap();
    assert_eq!(s.database().unwrap().load_record("legacy").unwrap().unwrap().turns, recovered.turns);
    let stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stub.turns.len(), 1, "the next save puts the compatibility placeholder back");
  }

  #[tokio::test]
  async fn reverting_writes_a_complete_file_before_dropping_database_rows() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("revert", "2026-01-01T00:00:00.000Z");
    stored.turns = serde_json::from_value(serde_json::json!([{ "role": "user", "text": "complete on disk" }])).unwrap();
    s.flush(Arc::new(stored.clone())).await.unwrap();
    assert!(s.revert_to_file("revert").await.unwrap());
    let file: SessionRecord = serde_json::from_slice(&std::fs::read(dir.path().join("revert.json")).unwrap()).unwrap();
    assert_eq!(file, stored);
    assert_eq!(s.database().unwrap().body_location("revert").unwrap(), Some(BodyLocation::File));
    assert_eq!(s.load("revert").await.unwrap(), stored);
    assert!(!s.revert_to_file("revert").await.unwrap());
  }

  #[tokio::test]
  async fn a_quarantined_database_falls_back_to_the_migration_archive() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("quarantine", "2026-01-01T00:00:00.000Z");
    stored.turns = serde_json::from_value(serde_json::json!([{ "role": "user", "text": "archived" }])).unwrap();
    // A record from before the database: its first save here archives it
    std::fs::write(dir.path().join("quarantine.json"), serde_json::to_vec(&stored).unwrap()).unwrap();
    s.flush(Arc::new(stored.clone())).await.unwrap();
    s.maintain_database().await.unwrap();
    s.quarantine_database().await.unwrap();
    assert!(s.database().is_none());
    assert!(std::fs::read_dir(dir.path()).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with("sessions.db.corrupt-")));
    assert_eq!(s.load("quarantine").await.unwrap(), stored);
  }

  #[tokio::test]
  async fn saving_migrates_then_only_refreshes_the_small_compatibility_stub() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("migrate", "2026-01-01T00:00:00.000Z");
    stored.turns = serde_json::from_value(serde_json::json!([
      { "role": "user", "text": "hello" },
      { "role": "agent", "blocks": [{ "type": "text", "markdown": "kept in SQLite" }] }
    ]))
    .unwrap();
    let path = dir.path().join("migrate.json");
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    s.flush(Arc::new(stored.clone())).await.unwrap();

    let stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(stub.turns.iter().any(migration::is_placeholder));
    let archive = dir.path().join("migrate").join(migration::MIGRATED_FILE);
    let archived = zstd::stream::decode_all(std::fs::File::open(&archive).unwrap()).unwrap();
    assert_eq!(serde_json::from_slice::<SessionRecord>(&archived).unwrap(), stored);
    assert_eq!(s.load("migrate").await.unwrap(), stored);
    assert!(s.database().unwrap().catalog().unwrap().iter().any(|entry| entry.summary.id == "migrate" && entry.body == BodyLocation::Database));

    let archive_before = std::fs::read(&archive).unwrap();
    stored.title = "new title".into();
    stored.updated_at = "2026-01-02T00:00:00.000Z".into();
    s.flush(Arc::new(stored.clone())).await.unwrap();
    assert_eq!(std::fs::read(&archive).unwrap(), archive_before, "later saves never rewrite the pre-migration archive");
    let stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(stub.title, "new title");
    assert!(stub.turns.iter().any(migration::is_placeholder));
  }

  #[tokio::test]
  async fn database_bodies_share_the_normal_record_read_path() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("database", "2026-01-01T00:00:00.000Z");
    stored.turns = serde_json::from_value(serde_json::json!([
      { "role": "user", "text": "hello" },
      { "role": "agent", "blocks": [{ "type": "text", "markdown": "from SQLite" }] }
    ]))
    .unwrap();
    s.database().unwrap().save_record(&stored).unwrap();
    std::fs::write(dir.path().join("database.json"), migration::stub(&RecordParts::of(stored.clone()).head, &[]).unwrap()).unwrap();
    assert_eq!(s.load("database").await.unwrap(), stored);
    assert_eq!(s.load_controls("database").await.unwrap(), stored.controls);
    assert_eq!(s.load_commands("database").await.unwrap(), stored.commands);
    assert_eq!(s.load_index().await.unwrap().iter().map(|summary| summary.id.as_str()).collect::<Vec<_>>(), ["database"]);
  }

  #[tokio::test]
  async fn migrated_sessions_keep_their_undo_window_in_the_catalogue() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    s.flush(Arc::new(record("undo", "2026-01-01T00:00:00.000Z"))).await.unwrap();
    s.trash("undo").await.unwrap();
    assert!(s.load_index().await.unwrap().is_empty());
    assert!(s.database().unwrap().catalog().unwrap()[0].trashed_at.is_some());
    s.restore("undo").await.unwrap();
    assert_eq!(s.load_index().await.unwrap()[0].id, "undo");
    assert_eq!(s.database().unwrap().catalog().unwrap()[0].trashed_at, None);
    s.trash("undo").await.unwrap();
    s.sweep_trash(Duration::ZERO).await;
    assert!(s.database().unwrap().catalog().unwrap().is_empty());
  }

  #[tokio::test]
  async fn the_catalogue_reconciles_records_written_by_an_older_engine() {
    let dir = tempfile::tempdir().unwrap();
    let original = record("a", "2026-01-01T00:00:00.000Z");
    std::fs::write(dir.path().join("a.json"), serde_json::to_vec(&original).unwrap()).unwrap();
    std::fs::write(dir.path().join("index.json"), serde_json::to_string_pretty(&[original.summary()]).unwrap()).unwrap();
    let s = store(dir.path());
    assert_eq!(s.load_index().await.unwrap()[0].title, "t");
    let legacy_index = std::fs::read(dir.path().join("index.json")).unwrap();

    let mut changed = record("a", "2026-01-03T00:00:00.000Z");
    changed.title = "written by an old engine".into();
    std::fs::write(dir.path().join("a.json"), serde_json::to_vec(&changed).unwrap()).unwrap();
    assert_eq!(s.load_index().await.unwrap()[0].title, "written by an old engine");
    assert_eq!(std::fs::read(dir.path().join("index.json")).unwrap(), legacy_index, "the old cache is not rewritten");

    std::fs::write(dir.path().join("b.json"), serde_json::to_vec(&record("b", "2026-01-02T00:00:00.000Z")).unwrap()).unwrap();
    let listed = s.load_index().await.unwrap();
    assert_eq!(listed.iter().map(|summary| summary.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);

    // A second engine can use the stamps and summaries already in SQLite without trusting the stale legacy index.
    let next = store(dir.path());
    let listed = next.load_index().await.unwrap();
    assert_eq!(listed.iter().find(|summary| summary.id == "a").unwrap().title, "written by an old engine");
    assert!(listed.iter().any(|summary| summary.id == "b"));
    let catalogue = next.database().unwrap().catalog().unwrap();
    assert!(catalogue.iter().all(|entry| entry.body == BodyLocation::File && entry.file_size.is_some()));
    assert_eq!(std::fs::read(dir.path().join("index.json")).unwrap(), legacy_index);
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
  async fn the_scratch_folder_goes_with_the_last_record_driving_the_session() {
    let (root, s, log_exists) = rooted();
    let project = tempfile::tempdir().unwrap();
    let acp = "abcdef12-0000-0000-0000-000000000001";
    write_log(root.path(), acp);
    let [file, _] = acpira_agent::acp::session_paths(root.path(), acp);
    std::fs::write(&file, format!("{}\n", serde_json::json!({ "type": "session", "id": acp, "cwd": project.path() }))).unwrap();
    let folder = project.path().join(".acpira/tmp/abcdef12");
    acpira_agent::scratch::Scratch::new(Some(folder.clone()), acp, root.path()).ensure().unwrap();
    s.flush(Arc::new(builtin("a", acp))).await.unwrap();
    s.flush(Arc::new(builtin("b", acp))).await.unwrap();
    s.load_index().await.unwrap();
    s.trash("a").await.unwrap();
    s.load_index().await.unwrap();
    s.remove("a").await;
    assert!(folder.exists(), "b still drives the session");
    s.trash("b").await.unwrap();
    s.load_index().await.unwrap();
    s.remove("b").await;
    assert!(!log_exists(acp));
    assert!(!project.path().join(".acpira").exists(), "no empty shell is left in the project");
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
  fn turns(texts: &[&str]) -> Vec<acpira_shared::transcript::Turn> {
    serde_json::from_value(texts.iter().map(|text| serde_json::json!({ "role": "user", "text": text })).collect()).unwrap()
  }

  fn item_rows(s: &TranscriptStore, id: &str) -> i64 {
    let db = rusqlite::Connection::open(s.dir().join(crate::store::db::DB_FILE)).unwrap();
    db.query_row("SELECT count(*) FROM item WHERE session_id = ?1", [id], |row| row.get(0)).unwrap()
  }

  #[tokio::test]
  async fn a_reconcile_never_takes_a_database_body() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("kept", "2026-01-01T00:00:00.000Z");
    stored.turns = turns(&["one", "two"]);
    s.flush(Arc::new(stored.clone())).await.unwrap();
    let rows = item_rows(&s, "kept");
    assert!(rows >= 3);

    // The compatibility file is gone (an older engine deleted it, or it never made it to disk): the session leaves the
    // list, its body stays
    let path = dir.path().join("kept.json");
    let stub = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let next = store(dir.path());
    assert!(next.load_index().await.unwrap().is_empty());
    assert_eq!(item_rows(&next, "kept"), rows);
    assert!(next.database().unwrap().entry("kept").unwrap().unwrap().trashed_at.is_some());
    // Routine upkeep keeps it too while it is not long gone
    next.maintain_database().await.unwrap();
    assert_eq!(item_rows(&next, "kept"), rows);
    // Back again: listed and whole
    std::fs::write(&path, &stub).unwrap();
    assert_eq!(next.load_index().await.unwrap()[0].id, "kept");
    assert_eq!(next.load("kept").await.unwrap(), stored);

    // A row that does not read back (a hash that no longer fits) with the stub touched: still listed, nothing dropped
    rusqlite::Connection::open(dir.path().join(crate::store::db::DB_FILE)).unwrap().execute("UPDATE item SET hash = zeroblob(16) WHERE session_id = 'kept' AND turn = 0", []).unwrap();
    std::fs::write(&path, [stub.as_slice(), b" "].concat()).unwrap();
    let third = store(dir.path());
    assert!(third.load_detailed("kept").await.is_err());
    assert_eq!(third.load_index().await.unwrap()[0].id, "kept");
    assert_eq!(item_rows(&third, "kept"), rows);
  }

  #[tokio::test]
  async fn a_store_without_the_database_never_buries_a_database_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("forked.json");
    let mut stored = record("forked", "2026-01-01T00:00:00.000Z");
    stored.turns = turns(&["before the database"]);
    std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
    {
      let s = store(dir.path());
      let mut live = s.load("forked").await.unwrap();
      live.turns.extend(turns(&["after the database"]));
      live.updated_at = "2026-01-02T00:00:00.000Z".into();
      s.flush(Arc::new(live)).await.unwrap();
    }
    // The database cannot be opened this time
    let db = dir.path().join(crate::store::db::DB_FILE);
    let aside = dir.path().join("aside");
    std::fs::create_dir(&aside).unwrap();
    for suffix in ["", "-wal", "-shm"] {
      let _ = std::fs::rename(dir.path().join(format!("sessions.db{suffix}")), aside.join(format!("sessions.db{suffix}")));
    }
    std::fs::create_dir(&db).unwrap();
    let stub = std::fs::read(&path).unwrap();
    {
      let s = store(dir.path());
      assert!(s.database().is_none());
      // What it can show is the session as it was archived; writing that back is refused, the stub stays
      let mut old = s.load("forked").await.unwrap();
      assert_eq!(old.turns.len(), 1);
      old.turns.extend(turns(&["typed without the database"]));
      old.updated_at = "2026-01-03T00:00:00.000Z".into();
      assert!(s.flush(Arc::new(old)).await.is_err());
      assert_eq!(std::fs::read(&path).unwrap(), stub);
    }
    std::fs::remove_dir(&db).unwrap();
    for suffix in ["", "-wal", "-shm"] {
      let _ = std::fs::rename(aside.join(format!("sessions.db{suffix}")), dir.path().join(format!("sessions.db{suffix}")));
    }
    let s = store(dir.path());
    let back = s.load("forked").await.unwrap();
    assert_eq!(back.turns.len(), 2);
    assert_eq!(back.turns[1].as_user().unwrap().text, "after the database");
  }

  #[tokio::test]
  async fn a_whole_record_file_newer_than_the_database_body_wins() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("newer", "2026-01-01T00:00:00.000Z");
    stored.turns = turns(&["in the database"]);
    s.flush(Arc::new(stored.clone())).await.unwrap();
    // A file backend wrote the whole session over the stub later on
    let mut whole = stored.clone();
    whole.turns.extend(turns(&["written as a file"]));
    whole.updated_at = "2026-01-02T00:00:00.000Z".into();
    std::fs::write(dir.path().join("newer.json"), serde_json::to_vec(&whole).unwrap()).unwrap();
    let next = store(dir.path());
    assert_eq!(next.load("newer").await.unwrap(), whole);
    assert_eq!(next.database().unwrap().body_location("newer").unwrap(), Some(BodyLocation::File));
    assert_eq!(item_rows(&next, "newer"), 0);
  }

  #[tokio::test]
  async fn the_stub_follows_lazily_and_the_body_never_reads_stale() {
    let dir = tempfile::tempdir().unwrap();
    let s = TranscriptStore::with_timing(dir.path().to_path_buf(), Arc::new(|_| {}), None, Duration::from_millis(5), Duration::from_millis(20));
    let path = dir.path().join("lazy.json");
    let mut stored = record("lazy", "2026-01-01T00:00:00.000Z");
    s.flush(Arc::new(stored.clone())).await.unwrap();
    let first = std::fs::read(&path).unwrap();
    // A debounced save right after: the body has the new title, the stub is left as it is for now
    stored.title = "renamed".into();
    stored.updated_at = "2026-01-02T00:00:00.000Z".into();
    s.save(Arc::new(stored.clone()));
    for _ in 0..200 {
      if s.database().unwrap().entry("lazy").unwrap().is_some_and(|entry| entry.summary.title == "renamed") {
        break;
      }
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
    s.flush_pending().await;
    assert_eq!(std::fs::read(&path).unwrap(), first);
    assert_eq!(s.load("lazy").await.unwrap(), stored);
    assert_eq!(store(dir.path()).load("lazy").await.unwrap(), stored, "another store reads the body, not the old stub");
    assert_eq!(s.load_index().await.unwrap()[0].title, "renamed");
    // A blob joining the folder is named by the very next save: an older engine's cleanup goes by the stub
    let (blob, _) = s.save_blob("lazy", ".png", b"image").await.unwrap();
    s.save(Arc::new(stored.clone()));
    s.flush_pending().await;
    let text = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
    assert!(text.contains(&blob) && text.contains("renamed"));
  }

  #[tokio::test]
  async fn a_session_that_went_back_to_its_file_archives_anew_when_it_migrates_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("again", "2026-01-01T00:00:00.000Z");
    stored.turns = turns(&["first"]);
    std::fs::write(dir.path().join("again.json"), serde_json::to_vec(&stored).unwrap()).unwrap();
    s.flush(Arc::new(stored.clone())).await.unwrap();
    let archive = dir.path().join("again").join(migration::MIGRATED_FILE);
    let first = std::fs::read(&archive).unwrap();
    stored.turns = turns(&["first", "second"]);
    stored.updated_at = "2026-01-02T00:00:00.000Z".into();
    s.flush(Arc::new(stored.clone())).await.unwrap();
    assert!(s.revert_to_file("again").await.unwrap());
    assert!(!archive.exists(), "a whole record has nothing older to fall back to");
    // One left behind all the same (a crash right after the file was written) is not taken for this migration's
    std::fs::write(&archive, &first).unwrap();
    stored.turns = turns(&["first", "second", "third"]);
    stored.updated_at = "2026-01-03T00:00:00.000Z".into();
    s.flush(Arc::new(stored.clone())).await.unwrap();
    let archived: SessionRecord = serde_json::from_slice(&zstd::stream::decode_all(std::fs::File::open(&archive).unwrap()).unwrap()).unwrap();
    assert_eq!(archived.turns, turns(&["first", "second"]), "the fallback is the record as it went in this time");
  }

  #[tokio::test]
  async fn a_trash_sweep_leaves_the_body_of_a_session_that_is_listed_again() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("back", "2026-01-01T00:00:00.000Z");
    stored.turns = turns(&["kept"]);
    s.flush(Arc::new(stored.clone())).await.unwrap();
    let rows = item_rows(&s, "back");
    s.trash("back").await.unwrap();
    // Another engine's undo between the sweep's look at the trash and its removal: the record is listed again
    std::fs::copy(dir.path().join(TRASH_DIR).join("back.json"), dir.path().join("back.json")).unwrap();
    s.sweep_trash(Duration::ZERO).await;
    assert!(!dir.path().join(TRASH_DIR).join("back.json").exists());
    assert_eq!(item_rows(&s, "back"), rows);
    assert_eq!(store(dir.path()).load("back").await.unwrap(), stored);
  }

  #[tokio::test]
  async fn a_save_lists_a_body_a_reconcile_had_hidden() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("hidden", "2026-01-01T00:00:00.000Z");
    s.flush(Arc::new(stored.clone())).await.unwrap();
    s.database().unwrap().set_trashed("hidden", Some(1)).unwrap();
    assert!(s.search_sources().await.iter().all(|source| !source.in_database));
    stored.updated_at = "2026-01-02T00:00:00.000Z".into();
    s.flush(Arc::new(stored)).await.unwrap();
    assert_eq!(s.database().unwrap().entry("hidden").unwrap().unwrap().trashed_at, None);
    assert!(s.search_sources().await.iter().any(|source| source.id == "hidden" && source.in_database));
  }

  #[tokio::test]
  async fn a_list_edit_an_older_engine_made_on_the_stub_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(dir.path());
    let mut stored = record("renamed", "2026-01-01T00:00:00.000Z");
    stored.turns = turns(&["kept"]);
    s.flush(Arc::new(stored.clone())).await.unwrap();
    let path = dir.path().join("renamed.json");
    // The older engine reads the stub as a record, edits it and writes it back: `updatedAt` stays as it was
    let mut stub: SessionRecord = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    stub.title = "renamed by an old engine".into();
    stub.renamed = true;
    stub.pinned = Some(true);
    std::fs::write(&path, serde_json::to_vec(&stub).unwrap()).unwrap();
    let loaded = s.load("renamed").await.unwrap();
    assert_eq!((loaded.title.as_str(), loaded.renamed, loaded.pinned), ("renamed by an old engine", true, Some(true)));
    assert_eq!(loaded.turns, stored.turns);
    assert_eq!(s.load_index().await.unwrap()[0].title, "renamed by an old engine");
    // Saved from there, the body has the edit and the stub is this store's again
    s.flush(Arc::new(loaded)).await.unwrap();
    assert_eq!(s.database().unwrap().load_record("renamed").unwrap().unwrap().title, "renamed by an old engine");
    assert!(!migration::rewritten(&std::fs::read(&path).unwrap()));
  }
}
