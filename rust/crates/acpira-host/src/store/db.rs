//! SQLite lifecycle for session storage. The connection is deliberately synchronous: callers move database work to a
//! blocking task when they start using it, while construction can always fall back to the JSON record backend.

use std::path::Path;

use anyhow::{Result, anyhow};
use rusqlite::Connection;

pub const DB_FILE: &str = "sessions.db";
const SCHEMA_VERSION: u32 = 1;

/// One database per session store. `parking_lot::Mutex` makes the connection shareable by the store; database operations
/// hold it only inside one blocking call.
#[derive(Debug)]
pub struct SessionDb {
  pub(crate) _connection: parking_lot::Mutex<Connection>,
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
    Ok(Self { _connection: parking_lot::Mutex::new(connection) })
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
  if version == 0 {
    // Version 1 is the lifecycle-only schema. Catalogue and body tables arrive as their own additive migrations.
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
  }
  tx.commit()?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn opens_wal_and_stamps_the_schema() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let connection = db._connection.lock();
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
      db._connection.lock().pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
    }
    let error = SessionDb::open(root.path()).unwrap_err();
    assert!(error.to_string().contains("newer"), "{error:#}");
    let connection = Connection::open(root.path().join(DB_FILE)).unwrap();
    let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap();
    assert_eq!(version, SCHEMA_VERSION + 1);
  }
}
