//! Upkeep of the SQLite store: SQLite's routine checks, quarantine of a damaged database, bodies whose file is gone, and the way back to a whole record file

use std::time::SystemTime;

use anyhow::{Result, anyhow};
use tokio::fs;

use super::{LOST_BODY_KEPT, TRASH_DIR, TranscriptStore, is_session_id, timestamp_ms};
use crate::db::{BodyLocation, Health, SessionDb};

impl TranscriptStore {
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
      // Only while it is still hidden: an undo or a save since the listing above lists it again
      if let Err(error) = self.db_call(move |db| db.delete_trashed(&gone, cutoff)).await {
        (self.log)(&format!("session {id}: SQLite row not removed ({error:#})"));
      }
    }
  }

  pub(super) async fn quarantine_database(&self) -> Result<()> {
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
      let source = self.dir.join(format!("{}{suffix}", crate::db::DB_FILE));
      if fs::metadata(&source).await.is_err() {
        continue;
      }
      let target = self.dir.join(format!("{}{suffix}.corrupt-{stamp}", crate::db::DB_FILE));
      fs::rename(&source, &target).await?;
      kept.push(target.display().to_string());
    }
    (self.log)(&format!("sessions: SQLite failed, using record files; kept {}", kept.join(", ")));
    Ok(())
  }
}
