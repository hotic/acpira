//! SQLite lifecycle and catalogue for session storage. The connection is deliberately synchronous: callers move database
//! work to blocking tasks, while construction can always fall back to the JSON record backend.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};

use acpira_shared::transcript::SessionSummary;

use super::record::SessionRecord;

mod body;
pub use body::SaveStats;

pub const DB_FILE: &str = "sessions.db";
const SCHEMA_VERSION: u32 = 3;

/// Where the complete record lives. The other location still carries a compatibility stub after body migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyLocation {
  File,
  Database,
}

impl BodyLocation {
  fn as_str(self) -> &'static str {
    match self {
      Self::File => "file",
      Self::Database => "db",
    }
  }

  fn of(value: &str) -> Self {
    if value == "db" { Self::Database } else { Self::File }
  }
}

/// The catalogue's persistent summary and the file stamp it was reconciled against. A missing stamp is the initial
/// `index.json` import: trust that summary once, then let later file changes trigger a record read.
#[derive(Debug, Clone)]
pub struct CatalogEntry {
  pub summary: SessionSummary,
  pub body: BodyLocation,
  pub file_size: Option<u64>,
  pub file_modified_ns: Option<i64>,
}

/// One database per session store. `parking_lot::Mutex` makes the connection shareable by the store; database operations
/// hold it only inside one blocking call.
#[derive(Debug)]
pub struct SessionDb {
  connection: parking_lot::Mutex<Connection>,
}

impl SessionDb {
  /// Open and migrate the store database. WAL is required: a filesystem that cannot provide it returns an error and the
  /// caller keeps using record files. A newer schema is likewise left untouched for the build that understands it.
  pub fn open(dir: &Path) -> Result<Self> {
    std::fs::create_dir_all(dir)?;
    let mut connection = Connection::open(dir.join(DB_FILE))?;
    connection.execute_batch("PRAGMA busy_timeout = 5000; PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
    let journal_mode: String = connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
      return Err(anyhow!("{}: WAL unavailable (journal mode {journal_mode})", dir.join(DB_FILE).display()));
    }
    migrate(&mut connection)?;
    Ok(Self { connection: parking_lot::Mutex::new(connection) })
  }

  /// Import or incrementally save a complete record and mark its body as database-backed.
  pub fn save_record(&self, record: &SessionRecord) -> Result<SaveStats> {
    let mut connection = self.connection.lock();
    body::save(&mut connection, record)
  }

  /// Read a body only when the catalogue names SQLite as its source. File-backed sessions return `Ok(None)`.
  pub fn load_record(&self, id: &str) -> Result<Option<SessionRecord>> {
    let connection = self.connection.lock();
    let body = connection.query_row("SELECT body FROM session WHERE id = ?1", params![id], |row| row.get::<_, String>(0)).optional()?;
    if body.as_deref() == Some("db") { body::load(&connection, id).map(Some) } else { Ok(None) }
  }

  pub fn catalog(&self) -> Result<Vec<CatalogEntry>> {
    let connection = self.connection.lock();
    let mut statement = connection.prepare("SELECT id, summary, body, file_size, file_modified_ns FROM session")?;
    let rows = statement
      .query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?))
      })?
      .collect::<rusqlite::Result<Vec<_>>>()?;
    rows
      .into_iter()
      .map(|(id, raw, body, file_size, file_modified_ns)| {
        let summary: SessionSummary = serde_json::from_str(&raw)?;
        if summary.id != id {
          return Err(anyhow!("catalogue row {id} holds session {}", summary.id));
        }
        Ok(CatalogEntry { summary, body: BodyLocation::of(&body), file_size: file_size.and_then(|v| u64::try_from(v).ok()), file_modified_ns })
      })
      .collect()
  }

  /// Reconcile the whole small catalogue without rewriting unchanged rows. Body rows are deliberately not touched here.
  pub fn apply_catalog(&self, entries: &[CatalogEntry]) -> Result<()> {
    let mut connection = self.connection.lock();
    let tx = connection.transaction()?;
    let existing = {
      let mut statement = tx.prepare("SELECT id, summary, body, file_size, file_modified_ns FROM session")?;
      statement
        .query_map([], |row| {
          Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<i64>>(3)?, row.get::<_, Option<i64>>(4)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(id, summary, body, file_size, file_modified_ns)| (id, (summary, body, file_size, file_modified_ns)))
        .collect::<HashMap<_, _>>()
    };
    for entry in entries {
      let summary = serde_json::to_string(&entry.summary)?;
      let body = entry.body.as_str();
      let file_size = entry.file_size.map(|v| i64::try_from(v).unwrap_or(i64::MAX));
      let unchanged = existing.get(&entry.summary.id).is_some_and(|(old_summary, old_body, old_size, old_modified)| {
        *old_summary == summary && *old_body == body && *old_size == file_size && *old_modified == entry.file_modified_ns
      });
      if unchanged {
        continue;
      }
      tx.execute(
        "INSERT INTO session (id, summary, body, file_size, file_modified_ns) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET summary = excluded.summary, body = excluded.body, file_size = excluded.file_size, file_modified_ns = excluded.file_modified_ns",
        params![entry.summary.id, summary, body, file_size, entry.file_modified_ns],
      )?;
    }
    let wanted: HashSet<&str> = entries.iter().map(|entry| entry.summary.id.as_str()).collect();
    for id in existing.keys().filter(|id| !wanted.contains(id.as_str())) {
      tx.execute("DELETE FROM session WHERE id = ?1", params![id])?;
    }
    tx.commit()?;
    Ok(())
  }
}

fn migrate(connection: &mut Connection) -> Result<()> {
  let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
  if version > SCHEMA_VERSION {
    return Err(anyhow!("database schema {version} is newer than supported schema {SCHEMA_VERSION}"));
  }
  if version == SCHEMA_VERSION {
    return Ok(());
  }
  let tx = connection.transaction()?;
  if version < 2 {
    // Concurrent first runs may both have read the old version; IF NOT EXISTS makes the additive migration idempotent.
    tx.execute_batch(
      "CREATE TABLE IF NOT EXISTS session (
        id TEXT PRIMARY KEY NOT NULL,
        summary TEXT NOT NULL,
        body TEXT NOT NULL DEFAULT 'file' CHECK(body IN ('file', 'db')),
        file_size INTEGER,
        file_modified_ns INTEGER
      );",
    )?;
  }
  if version < 3 {
    tx.execute_batch(
      "CREATE TABLE IF NOT EXISTS item (
        session_id TEXT NOT NULL REFERENCES session(id) ON DELETE CASCADE,
        subagent INTEGER NOT NULL,
        turn INTEGER NOT NULL,
        block INTEGER NOT NULL,
        codec INTEGER NOT NULL,
        payload BLOB NOT NULL,
        hash BLOB NOT NULL,
        PRIMARY KEY(session_id, subagent, turn, block)
      ) WITHOUT ROWID;",
    )?;
  }
  tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
  tx.commit()?;
  Ok(())
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
    let connection = db.connection.lock();
    let mode: String = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0)).unwrap();
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    assert_eq!(version, SCHEMA_VERSION);
  }

  #[test]
  fn a_newer_schema_is_refused_without_being_rewritten() {
    let root = tempfile::tempdir().unwrap();
    {
      let db = SessionDb::open(root.path()).unwrap();
      db.connection.lock().pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
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
    let connection = db.connection.lock();
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
  fn catalogue_rows_round_trip_and_stale_ids_are_removed() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let summary: SessionSummary = serde_json::from_value(json!({ "id": "a", "cwd": "/w", "title": "one", "updatedAt": "1" })).unwrap();
    db.apply_catalog(&[CatalogEntry { summary, body: BodyLocation::File, file_size: Some(12), file_modified_ns: Some(34) }]).unwrap();
    let mut entry = db.catalog().unwrap().remove(0);
    assert_eq!((entry.summary.title.as_str(), entry.body, entry.file_size, entry.file_modified_ns), ("one", BodyLocation::File, Some(12), Some(34)));
    let changes = db.connection.lock().total_changes();
    db.apply_catalog(std::slice::from_ref(&entry)).unwrap();
    assert_eq!(db.connection.lock().total_changes(), changes, "an identical catalogue writes no rows");
    entry.summary.title = "two".into();
    entry.body = BodyLocation::Database;
    db.apply_catalog(std::slice::from_ref(&entry)).unwrap();
    assert_eq!(db.catalog().unwrap()[0].summary.title, "two");
    db.apply_catalog(&[]).unwrap();
    assert!(db.catalog().unwrap().is_empty());
  }
}
