//! The session list: `index.json` and the SQLite catalogue reconciled with the record files on disk

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use anyhow::Result;
use serde_json::Value;
use tokio::fs;

use acpira_shared::transcript::SessionSummary;

use super::{
  FileStamp, META_FILES, RecordLoadError, TRASH_DIR, TranscriptStore, file_modified_ns, file_stamp, is_session_id,
  sort_index, timestamp_ms,
};
use crate::store::db::{BodyLocation, CatalogEntry, SessionDb};
use crate::store::file_lock::write_atomic;

impl TranscriptStore {
  /// The list as the disk knows it
  pub async fn load_index(&self) -> Result<Vec<SessionSummary>> {
    self.sync_index(&[], &HashSet::new()).await
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
    let (mut changed, mut gone, mut listed) = (vec![], vec![], vec![]);
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
      let stubless = self.stubless();
      for entry in stored.values().filter(|entry| !seen.contains(&entry.summary.id)) {
        // A stubless store's body has no file to be seen by: listed while the catalogue does not have it in the trash,
        // this host's view winning for the sessions it drives, as for a file
        if stubless && entry.body == BodyLocation::Database {
          if entry.trashed_at.is_none() {
            let id = entry.summary.id.as_str();
            let summary = match local.get(id) {
              Some(mine) if own.contains(id) && !mine.cwd.is_empty() => (*mine).clone(),
              _ => entry.summary.clone(),
            };
            let stored = stored_form(&summary);
            if stored != entry.summary {
              listed.push(stored);
            }
            out.push(summary);
          }
          continue;
        }
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
    if !listed.is_empty() {
      let _ = self.db_call(move |db| db.set_listed_summaries(&listed)).await?;
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

  pub(super) async fn read_index(&self) -> Vec<SessionSummary> {
    let Ok(raw) = fs::read(self.dir.join("index.json")).await else { return vec![] };
    let Ok(Value::Array(items)) = serde_json::from_slice::<Value>(&raw) else { return vec![] };
    items.into_iter().filter(|v| v.get("id").is_some_and(Value::is_string)).filter_map(|v| serde_json::from_value(v).ok()).collect()
  }

  /// Every record file in the live directory with its stamp, read in one blocking pass (a stat per file through the
  /// async runtime would cost a thread hop each)
  pub(super) async fn record_files(&self) -> Result<Vec<(String, FileStamp)>> {
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
}

/// A summary as the catalogue keeps it: what a session is, without the marks of what it is doing right now (those
/// belong to the engine driving it, and would rewrite the row on every state change)
fn stored_form(summary: &SessionSummary) -> SessionSummary {
  SessionSummary { state: None, unread: None, question: None, wake_at: None, ..summary.clone() }
}
