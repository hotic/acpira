//! SQLite lifecycle and catalogue for session storage. Connections are synchronous: callers move database work to
//! blocking tasks, while construction can always fall back to the JSON record backend.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, ErrorCode, OptionalExtension, TransactionBehavior, params};

use acpira_shared::transcript::SessionSummary;

use super::record::{RecordParts, SessionRecord};

mod body;
pub mod migration;
pub use body::{SaveCache, SaveStats, Saved};

pub const DB_FILE: &str = "sessions.db";
const SCHEMA_VERSION: u32 = 5;
/// Set in `user_version` beside the schema on a store created with no record files: its sessions live in the database
/// alone, with no compatibility file. An engine that predates the flag reads a newer schema and keeps away, so it never
/// takes a body without a file for a deleted session
const STUBLESS: u32 = 1 << 16;
/// Free pages handed back per upkeep run, so the write lock is held for a moment only
const VACUUM_PAGES: u32 = 4096;

/// Where the complete record lives. The other location still carries a compatibility stub after body migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyLocation {
  File,
  Database,
}

impl BodyLocation {
  fn of(value: &str) -> Self {
    if value == "db" { Self::Database } else { Self::File }
  }
}

/// The catalogue's persistent summary and the file stamp it was reconciled against. A missing stamp is the initial
/// `index.json` import: trust that summary once, then let later file changes trigger a record read.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogEntry {
  pub summary: SessionSummary,
  pub body: BodyLocation,
  pub file_size: Option<u64>,
  pub file_modified_ns: Option<i64>,
  pub trashed_at: Option<i64>,
}

/// What the routine check found
#[derive(Debug, PartialEq, Eq)]
pub enum Health {
  Sound,
  /// The file itself is damaged (not a lock held elsewhere or a disk that is busy)
  Corrupt(String),
}

/// One database per session store, over two connections: saves go through the writer, everything read (lists, loads,
/// search) through the reader, so a long read never holds a save up (WAL readers do not block the writer).
#[derive(Debug)]
pub struct SessionDb {
  path: PathBuf,
  writer: parking_lot::Mutex<Connection>,
  reader: parking_lot::Mutex<Connection>,
  /// No compatibility files: the catalogue alone says which sessions exist and which wait in the trash
  stubless: bool,
}

fn connect(path: &Path) -> Result<Connection> {
  let connection = Connection::open(path)?;
  connection.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
  Ok(connection)
}

fn is_corruption(error: &rusqlite::Error) -> bool {
  matches!(error, rusqlite::Error::SqliteFailure(failure, _) if matches!(failure.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase))
}

type CatalogRow = (String, String, String, Option<i64>, Option<i64>, Option<i64>);

fn catalog_entry((id, raw, body, file_size, file_modified_ns, trashed_at): CatalogRow) -> CatalogEntry {
  // A summary that does not read back keeps its row: the empty folder makes the next reconcile read the record again
  let summary = serde_json::from_str::<SessionSummary>(&raw).ok().filter(|summary| summary.id == id).unwrap_or_else(|| bare_summary(&id));
  CatalogEntry { summary, body: BodyLocation::of(&body), file_size: file_size.and_then(|v| u64::try_from(v).ok()), file_modified_ns, trashed_at }
}

#[expect(clippy::expect_used, reason = "every other field of a summary has a default")]
fn bare_summary(id: &str) -> SessionSummary {
  serde_json::from_value(serde_json::json!({ "id": id })).expect("a summary needs only its id")
}

const CATALOG_COLUMNS: &str = "id, summary, body, file_size, file_modified_ns, trashed_at";

fn catalog_row(row: &rusqlite::Row) -> rusqlite::Result<CatalogRow> {
  Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
}

impl SessionDb {
  /// Open and migrate the store database. WAL is required: a filesystem that cannot provide it returns an error and the
  /// caller keeps using record files. A newer schema is likewise left untouched for the build that understands it.
  pub fn open(dir: &Path) -> Result<Self> {
    Self::open_as(dir, false)
  }

  /// `open` for a store set up as an upgraded machine's: compatibility files whatever the directory holds
  #[cfg(test)]
  pub(crate) fn open_stubbed(dir: &Path) -> Result<Self> {
    Self::open_as(dir, true)
  }

  fn open_as(dir: &Path, stubs: bool) -> Result<Self> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(DB_FILE);
    let mut writer = connect(&path)?;
    // An import writes a whole session through the log: without a limit the log file keeps that size for good
    writer.execute_batch("PRAGMA auto_vacuum = INCREMENTAL; PRAGMA journal_size_limit = 8388608;")?;
    let journal_mode: String = writer.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
      return Err(anyhow!("{}: WAL unavailable (journal mode {journal_mode})", path.display()));
    }
    let stubless = migrate(&mut writer, dir, stubs)?;
    let reader = connect(&path)?;
    reader.execute_batch("PRAGMA query_only = ON;")?;
    Ok(Self { path, writer: parking_lot::Mutex::new(writer), reader: parking_lot::Mutex::new(reader), stubless })
  }

  /// Whether this store keeps no compatibility files (`STUBLESS`)
  pub fn stubless(&self) -> bool {
    self.stubless
  }

  /// Save a body from its pieces and mark it database-backed. `cache` is the caller's memory of this session's rows
  pub fn save_parts(&self, id: &str, parts: &RecordParts, cache: &mut SaveCache) -> Result<Saved> {
    body::save(&mut self.writer.lock(), id, parts, cache)
  }

  /// Import or save a complete record, comparing against the stored row hashes
  pub fn save_record(&self, record: &SessionRecord) -> Result<SaveStats> {
    match self.save_parts(&record.id, &RecordParts::of(record.clone()), &mut SaveCache::default())? {
      Saved::Done(stats) => Ok(stats),
      Saved::NeedsFull => Err(anyhow!("session {}: a whole record cannot need more", record.id)),
    }
  }

  /// Hand a body back to its record file: the rows go, the catalogue entry stays. The complete file is written first, so
  /// a crash in between leaves the database body authoritative
  pub fn mark_file(&self, summary: &SessionSummary, stamp: Option<(u64, Option<i64>)>) -> Result<()> {
    let mut connection = self.writer.lock();
    let tx = connection.transaction()?;
    tx.execute("DELETE FROM item WHERE session_id = ?1", params![summary.id])?;
    let (size, modified) = stamp.map_or((None, None), |(size, modified)| (i64::try_from(size).ok(), modified));
    tx.execute(
      "INSERT INTO session (id, summary, body, file_size, file_modified_ns) VALUES (?1, ?2, 'file', ?3, ?4)
       ON CONFLICT(id) DO UPDATE SET summary = excluded.summary, body = 'file', file_size = excluded.file_size, file_modified_ns = excluded.file_modified_ns, trashed_at = NULL",
      params![summary.id, serde_json::to_string(summary)?, size, modified],
    )?;
    tx.commit()?;
    Ok(())
  }

  fn in_database(connection: &Connection, id: &str) -> Result<bool> {
    let body = connection.prepare("SELECT body FROM session WHERE id = ?1")?.query_row(params![id], |row| row.get::<_, String>(0)).optional()?;
    Ok(body.as_deref() == Some("db"))
  }

  /// Read a body only when the catalogue names SQLite as its source. File-backed sessions return `Ok(None)`.
  pub fn load_record(&self, id: &str) -> Result<Option<SessionRecord>> {
    let connection = self.reader.lock();
    if Self::in_database(&connection, id)? { body::load(&connection, id).map(Some) } else { Ok(None) }
  }

  /// A database body as record JSON, without parsing it (`body::raw`); `main_only` leaves the subagents out
  pub fn load_raw(&self, id: &str, main_only: bool) -> Result<Option<Vec<u8>>> {
    let connection = self.reader.lock();
    if !Self::in_database(&connection, id)? {
      return Ok(None);
    }
    match body::raw(&connection, id, main_only)? {
      Some(raw) => Ok(Some(raw)),
      None => Ok(Some(serde_json::to_vec(&body::load(&connection, id)?)?)),
    }
  }

  /// A database body's record head: the record with no turns, one small row
  pub fn load_head(&self, id: &str) -> Result<Option<Vec<u8>>> {
    let connection = self.reader.lock();
    if Self::in_database(&connection, id)? { body::head(&connection, id) } else { Ok(None) }
  }

  /// How many subagents a database body holds
  pub fn subagent_count(&self, id: &str) -> Result<u32> {
    let connection = self.reader.lock();
    Ok(connection.prepare("SELECT count(*) FROM item WHERE session_id = ?1 AND subagent >= 0 AND turn = -1")?.query_row(params![id], |row| row.get(0))?)
  }

  pub fn body_location(&self, id: &str) -> Result<Option<BodyLocation>> {
    let connection = self.reader.lock();
    let body = connection.prepare("SELECT body FROM session WHERE id = ?1")?.query_row(params![id], |row| row.get::<_, String>(0)).optional()?;
    Ok(body.map(|body| BodyLocation::of(&body)))
  }

  /// The routine integrity check, on a connection of its own (it reads every page: neither saves nor reads wait for
  /// it), then a bounded hand-back of free pages. Only a damaged file is `Corrupt`; a lock or an I/O hiccup is an error
  /// the caller logs and tries again another day
  pub fn maintain(&self) -> Result<Health> {
    let checked = connect(&self.path).and_then(|connection| Ok(connection.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))?));
    match checked {
      Ok(result) if result.eq_ignore_ascii_case("ok") => {}
      Ok(result) => return Ok(Health::Corrupt(result)),
      Err(error) if error.downcast_ref::<rusqlite::Error>().is_some_and(is_corruption) => return Ok(Health::Corrupt(format!("{error:#}"))),
      Err(error) => return Err(error),
    }
    self.writer.lock().execute_batch(&format!("PRAGMA incremental_vacuum({VACUUM_PAGES});"))?;
    Ok(Health::Sound)
  }

  pub fn set_trashed(&self, id: &str, trashed_at: Option<i64>) -> Result<()> {
    self.writer.lock().execute("UPDATE session SET trashed_at = ?2 WHERE id = ?1", params![id, trashed_at])?;
    Ok(())
  }

  /// The compatibility file's stamp after this store wrote it, so the next reconcile does not take it for another
  /// engine's write
  pub fn set_file_stamp(&self, id: &str, size: u64, modified_ns: Option<i64>) -> Result<()> {
    self.writer.lock().execute("UPDATE session SET file_size = ?2, file_modified_ns = ?3 WHERE id = ?1", params![id, i64::try_from(size).ok(), modified_ns])?;
    Ok(())
  }

  /// Remove a session for good: its catalogue row and, through the foreign key, its body
  pub fn delete_session(&self, id: &str) -> Result<()> {
    self.writer.lock().execute("DELETE FROM session WHERE id = ?1", params![id])?;
    Ok(())
  }

  /// Stored bytes per database body (compressed payloads; the page overhead is not attributed)
  pub fn body_sizes(&self) -> Result<Vec<(String, u64)>> {
    let connection = self.reader.lock();
    let mut statement = connection.prepare("SELECT session_id, sum(length(payload)) FROM item GROUP BY session_id")?;
    Ok(statement.query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)?.max(0) as u64)))?.collect::<rusqlite::Result<Vec<_>>>()?)
  }

  /// (id, revision, updatedAt) of every listed database body; the revision moves with every save that changed something
  pub fn body_revisions(&self) -> Result<Vec<(String, u64, String)>> {
    let connection = self.reader.lock();
    let mut statement =
      connection.prepare("SELECT id, revision, coalesce(json_extract(summary, '$.updatedAt'), '') FROM session WHERE body = 'db' AND trashed_at IS NULL")?;
    Ok(statement.query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)?.max(0) as u64, row.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?)
  }

  /// A database body's revision; None for a session whose body is not here
  pub fn body_revision(&self, id: &str) -> Result<Option<u64>> {
    let connection = self.reader.lock();
    let revision = connection.prepare("SELECT revision FROM session WHERE id = ?1 AND body = 'db'")?.query_row(params![id], |row| row.get::<_, i64>(0)).optional()?;
    Ok(revision.map(|revision| revision.max(0) as u64))
  }

  /// Take a session out of the trash. False when it is not there (any more): a sweep removed it, or another engine
  /// restored it first
  pub fn untrash(&self, id: &str) -> Result<bool> {
    Ok(self.writer.lock().execute("UPDATE session SET trashed_at = NULL WHERE id = ?1 AND trashed_at IS NOT NULL", params![id])? > 0)
  }

  /// Listed database bodies' summaries, from a reconcile. A row trashed or removed since that reconcile's snapshot is
  /// left as it is: the snapshot never undoes a delete
  pub fn set_listed_summaries(&self, summaries: &[SessionSummary]) -> Result<()> {
    if summaries.is_empty() {
      return Ok(());
    }
    let mut connection = self.writer.lock();
    let tx = connection.transaction()?;
    {
      let mut update = tx.prepare("UPDATE session SET summary = ?2 WHERE id = ?1 AND body = 'db' AND trashed_at IS NULL")?;
      for summary in summaries {
        update.execute(params![summary.id, serde_json::to_string(summary)?])?;
      }
    }
    tx.commit()?;
    Ok(())
  }

  /// Remove a session that has waited in the trash since `cutoff` or earlier. False when it is not (any more): an undo
  /// in another engine between the listing and this call keeps it
  pub fn delete_trashed(&self, id: &str, cutoff: i64) -> Result<bool> {
    Ok(self.writer.lock().execute("DELETE FROM session WHERE id = ?1 AND trashed_at IS NOT NULL AND trashed_at <= ?2", params![id, cutoff])? > 0)
  }

  pub fn entry(&self, id: &str) -> Result<Option<CatalogEntry>> {
    let connection = self.reader.lock();
    let row = connection.prepare(&format!("SELECT {CATALOG_COLUMNS} FROM session WHERE id = ?1"))?.query_row(params![id], catalog_row).optional()?;
    Ok(row.map(catalog_entry))
  }

  pub fn catalog(&self) -> Result<Vec<CatalogEntry>> {
    let connection = self.reader.lock();
    let mut statement = connection.prepare(&format!("SELECT {CATALOG_COLUMNS} FROM session"))?;
    let rows = statement.query_map([], catalog_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows.into_iter().map(catalog_entry).collect())
  }

  /// Write the catalogue entries a reconcile found changed, and drop the rows of file-backed sessions whose file is
  /// gone. An entry's body location is never changed here and a database-backed row is never deleted: only a save, a
  /// revert or an explicit removal decides those, so no reconcile can take a body along
  pub fn apply_catalog(&self, changed: &[CatalogEntry], gone: &[String]) -> Result<()> {
    if changed.is_empty() && gone.is_empty() {
      return Ok(());
    }
    let mut connection = self.writer.lock();
    let tx = connection.transaction()?;
    {
      let mut upsert = tx.prepare(
        "INSERT INTO session (id, summary, body, file_size, file_modified_ns, trashed_at) VALUES (?1, ?2, 'file', ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET summary = excluded.summary, file_size = excluded.file_size, file_modified_ns = excluded.file_modified_ns, trashed_at = excluded.trashed_at",
      )?;
      for entry in changed {
        let file_size = entry.file_size.map(|v| i64::try_from(v).unwrap_or(i64::MAX));
        upsert.execute(params![entry.summary.id, serde_json::to_string(&entry.summary)?, file_size, entry.file_modified_ns, entry.trashed_at])?;
      }
      let mut delete = tx.prepare("DELETE FROM session WHERE id = ?1 AND body = 'file'")?;
      for id in gone {
        delete.execute(params![id])?;
      }
    }
    tx.commit()?;
    Ok(())
  }
}

/// Whether `dir` (or its trash) holds a record file: sessions written before this database, which keep their stubs
fn has_records(dir: &Path) -> bool {
  [dir.to_path_buf(), dir.join("trash")].iter().any(|dir| {
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|entry| {
      let name = entry.file_name();
      name.to_str().and_then(|name| name.strip_suffix(".json")).is_some_and(acpira_shared::session_links::is_session_id)
    })
  })
}

/// Bring the schema up to date; returns whether the store is `STUBLESS`. Decided once, under the migration lock when the
/// schema is first written (two engines opening a new store at once agree): a directory that already holds records keeps
/// their stubs for good. `stubs` keeps them regardless
fn migrate(connection: &mut Connection, dir: &Path, stubs: bool) -> Result<bool> {
  let raw: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
  let version = raw & !STUBLESS;
  if version > SCHEMA_VERSION {
    return Err(anyhow!("database schema {version} is newer than supported schema {SCHEMA_VERSION}"));
  }
  if version == SCHEMA_VERSION {
    return Ok(raw & STUBLESS != 0);
  }
  // Serialize migrations, then re-read the version: another engine may have just applied the same additive steps.
  let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
  let raw: u32 = tx.query_row("PRAGMA user_version", [], |row| row.get(0))?;
  let version = raw & !STUBLESS;
  if version > SCHEMA_VERSION {
    return Err(anyhow!("database schema {version} became newer than supported schema {SCHEMA_VERSION}"));
  }
  if version == SCHEMA_VERSION {
    tx.commit()?;
    return Ok(raw & STUBLESS != 0);
  }
  let stubless = raw & STUBLESS != 0 || (version == 0 && !stubs && !has_records(dir));
  if version < 2 {
    // Concurrent first runs may both have read the old version; IF NOT EXISTS makes the additive migration idempotent.
    tx.execute_batch(
      "CREATE TABLE IF NOT EXISTS session (
        id TEXT PRIMARY KEY NOT NULL,
        summary TEXT NOT NULL,
        body TEXT NOT NULL DEFAULT 'file' CHECK(body IN ('file', 'db')),
        file_size INTEGER,
        file_modified_ns INTEGER,
        trashed_at INTEGER
      );",
    )?;
  }
  if version < 3 {
    // The hash sits ahead of the payload: reading a session's hashes then never walks a large row's overflow pages
    tx.execute_batch(
      "CREATE TABLE IF NOT EXISTS item (
        session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
        subagent INTEGER NOT NULL,
        turn INTEGER NOT NULL,
        block INTEGER NOT NULL,
        codec INTEGER NOT NULL,
        hash BLOB NOT NULL,
        payload BLOB NOT NULL,
        PRIMARY KEY(session_id, subagent, turn, block)
      ) WITHOUT ROWID;",
    )?;
  }
  if (2..4).contains(&version) {
    tx.execute_batch("ALTER TABLE session ADD COLUMN trashed_at INTEGER;")?;
  }
  if version < 5 {
    // Counts the saves that changed a body: what a reader keeping its own copy (the search) compares against
    tx.execute_batch("ALTER TABLE session ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;")?;
  }
  tx.pragma_update(None, "user_version", SCHEMA_VERSION | if stubless { STUBLESS } else { 0 })?;
  tx.commit()?;
  Ok(stubless)
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  use serde_json::{Value, json};

  #[test]
  fn opens_wal_and_stamps_the_schema() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let connection = db.writer.lock();
    let mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    assert_eq!(version, SCHEMA_VERSION | STUBLESS);
    assert!(db.stubless(), "an empty directory starts without compatibility files");
  }

  #[test]
  fn a_directory_with_records_keeps_its_stubs_and_an_older_engine_refuses_a_stubless_one() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("trash")).unwrap();
    std::fs::write(root.path().join("trash").join("old.json"), b"{}").unwrap();
    assert!(!SessionDb::open(root.path()).unwrap().stubless(), "a trashed record counts");
    std::fs::remove_file(root.path().join("trash").join("old.json")).unwrap();
    assert!(!SessionDb::open(root.path()).unwrap().stubless(), "decided when the database is created, for good");

    let fresh = tempfile::tempdir().unwrap();
    drop(SessionDb::open(fresh.path()).unwrap());
    assert!(SessionDb::open(fresh.path()).unwrap().stubless(), "the flag survives a reopen");
    let raw: u32 = Connection::open(fresh.path().join(DB_FILE)).unwrap().query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert!(raw > SCHEMA_VERSION, "an engine before the flag reads a newer schema and keeps away");

    // Another engine created the file a moment ago and has not written the schema yet: still a new store
    let racing = tempfile::tempdir().unwrap();
    drop(Connection::open(racing.path().join(DB_FILE)).unwrap());
    assert!(SessionDb::open(racing.path()).unwrap().stubless());
  }

  #[test]
  fn a_trashed_row_outlives_a_stale_reconcile_and_a_restore_beats_the_sweep() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let record: SessionRecord = serde_json::from_value(json!({ "id": "s", "agent": "claude", "cwd": "/w", "title": "old", "updatedAt": "1", "turns": [] })).unwrap();
    db.save_record(&record).unwrap();
    let mut summary = record.summary();
    db.set_trashed("s", Some(10)).unwrap();
    summary.title = "from a snapshot".into();
    db.set_listed_summaries(&[summary]).unwrap();
    let entry = db.entry("s").unwrap().unwrap();
    assert_eq!((entry.summary.title.as_str(), entry.trashed_at), ("old", Some(10)), "a snapshot never undoes a delete");
    assert!(db.untrash("s").unwrap());
    assert!(!db.untrash("s").unwrap(), "already out");
    assert!(!db.delete_trashed("s", i64::MAX).unwrap(), "a restored row is not swept");
    db.set_trashed("s", Some(10)).unwrap();
    assert!(!db.delete_trashed("s", 9).unwrap(), "still in its window");
    assert!(db.delete_trashed("s", 10).unwrap());
    assert!(db.entry("s").unwrap().is_none());
  }

  #[test]
  fn a_newer_schema_is_refused_without_being_rewritten() {
    let root = tempfile::tempdir().unwrap();
    {
      let db = SessionDb::open(root.path()).unwrap();
      db.writer.lock().pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
    }
    let error = SessionDb::open(root.path()).unwrap_err();
    assert!(error.to_string().contains("newer"), "{error:#}");
    let connection = Connection::open(root.path().join(DB_FILE)).unwrap();
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION + 1);
  }

  #[test]
  fn the_version_one_schema_gains_the_catalogue_additively() {
    let root = tempfile::tempdir().unwrap();
    let connection = Connection::open(root.path().join(DB_FILE)).unwrap();
    connection.execute_batch("CREATE TABLE legacy_marker (id INTEGER); PRAGMA user_version = 1;").unwrap();
    drop(connection);
    let db = SessionDb::open(root.path()).unwrap();
    let connection = db.writer.lock();
    let tables: i64 = connection.query_row("SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN ('legacy_marker', 'session')", [], |row| row.get(0)).unwrap();
    assert_eq!(tables, 2);
  }

  #[test]
  fn engine_fixture_records_round_trip_through_body_rows() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let fixtures: Value = serde_json::from_str(include_str!("../../../../../test/fixtures/engine-patches.json")).unwrap();
    let mut checked = 0;
    for case in fixtures.as_array().unwrap() {
      for key in ["first", "second"] {
        let Ok(record) = serde_json::from_value::<SessionRecord>(case[key].clone()) else { continue };
        db.save_record(&record).unwrap();
        assert_eq!(db.load_record(&record.id).unwrap().unwrap(), record, "{} / {key}", case["name"]);
        checked += 1;
      }
    }
    assert!(checked >= 10, "the fixtures exercised {checked} records");
  }

  /// Opt-in acceptance against a real session directory; the directory is only read and every import lands in a temp DB.
  #[test]
  #[ignore]
  fn local_session_records_round_trip_through_body_rows() {
    let dir = std::env::var_os("ACPIRA_SESSIONS_FIXTURE_DIR").map(PathBuf::from).unwrap_or_else(|| crate::store::data_dir::acpira_home().join("sessions"));
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let mut checked = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
      let entry = entry.unwrap();
      let name = entry.file_name().to_string_lossy().into_owned();
      let Some(id) = name.strip_suffix(".json") else { continue };
      if !crate::store::transcript_store::is_session_id(id) {
        continue;
      }
      let raw = crate::store::pack::expand_raw(&dir, std::fs::read(entry.path()).unwrap());
      let Ok(record) = serde_json::from_slice::<SessionRecord>(&raw) else { continue };
      db.save_record(&record).unwrap();
      assert_eq!(db.load_record(id).unwrap().unwrap(), record, "{}", entry.path().display());
      checked += 1;
    }
    assert!(checked > 0, "no records found under {}", dir.display());
    eprintln!("round-tripped {checked} records from {}", dir.display());
  }

  #[test]
  fn catalogue_rows_round_trip_and_only_file_backed_ids_are_dropped() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let summary: SessionSummary = serde_json::from_value(json!({ "id": "a", "cwd": "/w", "title": "one", "updatedAt": "1" })).unwrap();
    db.apply_catalog(&[CatalogEntry { summary, body: BodyLocation::File, file_size: Some(12), file_modified_ns: Some(34), trashed_at: None }], &[]).unwrap();
    let mut entry = db.catalog().unwrap().remove(0);
    assert_eq!((entry.summary.title.as_str(), entry.body, entry.file_size, entry.file_modified_ns), ("one", BodyLocation::File, Some(12), Some(34)));
    entry.summary.title = "two".into();
    db.apply_catalog(std::slice::from_ref(&entry), &[]).unwrap();
    assert_eq!(db.entry("a").unwrap().unwrap().summary.title, "two");

    // A database body survives whatever a reconcile hands in: its location is not rewritten and its row is not dropped
    let record: SessionRecord = serde_json::from_value(json!({ "id": "b", "agent": "claude", "cwd": "/w", "updatedAt": "1", "turns": [{ "role": "user", "text": "kept" }] })).unwrap();
    db.save_record(&record).unwrap();
    let mut held = db.entry("b").unwrap().unwrap();
    held.body = BodyLocation::File;
    held.trashed_at = Some(5);
    db.apply_catalog(&[held], &["a".into(), "b".into()]).unwrap();
    assert!(db.entry("a").unwrap().is_none());
    let kept = db.entry("b").unwrap().unwrap();
    assert_eq!((kept.body, kept.trashed_at), (BodyLocation::Database, Some(5)));
    assert_eq!(db.load_record("b").unwrap().unwrap(), record);
  }

  #[test]
  fn a_summary_that_does_not_read_back_keeps_its_row() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    db.writer.lock().execute("INSERT INTO session (id, summary) VALUES ('odd', 'not json')", []).unwrap();
    let entry = db.catalog().unwrap().remove(0);
    assert_eq!((entry.summary.id.as_str(), entry.summary.cwd.as_str()), ("odd", ""));
  }

  #[test]
  fn a_lock_held_elsewhere_is_not_damage() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    assert_eq!(db.maintain().unwrap(), Health::Sound);
    // Another engine in the middle of a write: the check itself reads fine, the hand-back of pages cannot get its lock
    db.writer.lock().execute_batch("PRAGMA busy_timeout = 0;").unwrap();
    let other = Connection::open(root.path().join(DB_FILE)).unwrap();
    other.execute_batch("BEGIN IMMEDIATE; CREATE TABLE held (id INTEGER);").unwrap();
    assert!(db.maintain().is_err(), "busy is an error to retry, never `Corrupt`");
    other.execute_batch("ROLLBACK;").unwrap();
    assert_eq!(db.maintain().unwrap(), Health::Sound);
  }
}
