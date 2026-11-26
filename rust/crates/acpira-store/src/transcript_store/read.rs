//! Reading stored records: whole, or only their controls, commands or model, plus the raw texts search and export go by

use std::collections::HashMap;
use std::time::SystemTime;

use anyhow::Result;
use serde::Deserialize;
use tokio::fs;

use acpira_shared::model_shapes::session_model;
use acpira_shared::transcript::{SessionControls, SessionModel, SlashCommand};

use super::{RecordLoadError, SearchSource, Stamped, TranscriptStore, confined, file_modified_ns, file_stamp, is_blob_name, is_session_id};
use crate::db::{BodyLocation, SessionDb, migration};
use crate::pack;
use crate::record::SessionRecord;

impl TranscriptStore {
  /// What the search checks its cache against: every listed record with the stamp its text goes by
  pub async fn search_sources(&self) -> Vec<SearchSource> {
    let files = self.record_files().await.unwrap_or_default();
    let mut bodies: HashMap<String, (u64, String)> =
      self.db_call(SessionDb::body_revisions).await.ok().flatten().unwrap_or_default().into_iter().map(|(id, revision, updated)| (id, (revision, updated))).collect();
    let mut out: Vec<SearchSource> = files
      .into_iter()
      .map(|(id, (len, modified))| match bodies.remove(&id) {
        Some((revision, _)) => SearchSource { id, stamp: (modified, revision), in_database: true },
        None => SearchSource { id, stamp: (modified, len), in_database: false },
      })
      .collect();
    // A stubless body has no file to take the time from: its `updatedAt` orders the hits instead
    if self.stubless() {
      out.extend(bodies.into_iter().map(|(id, (revision, updated))| {
        let time = chrono::DateTime::parse_from_rfc3339(&updated).ok().map(SystemTime::from);
        SearchSource { id, stamp: (time, revision), in_database: true }
      }));
    }
    out
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

  /// A model `read_models` found for this record, while the file is still the one it read
  pub(super) async fn cached_model(&self, id: &str) -> Option<SessionModel> {
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
    if path.is_some() && stamp.is_none() && !self.listed_body(id).await {
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
            Err(error) => return Err(RecordLoadError::Unreadable { reason: format!("the session is kept in {}, which is unavailable, and {} cannot be read ({error})", crate::db::DB_FILE, migration::MIGRATED_FILE) }),
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
}
