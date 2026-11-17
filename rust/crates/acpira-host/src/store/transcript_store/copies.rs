//! Copies of sessions another machine drives, and the stamps that tell whether a copy is current

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Result, anyhow};
use tokio::fs;

use super::{CopyStamp, Stamped, TranscriptStore, confined, is_session_id, sort_index};
use crate::store::file_lock::write_atomic;
use crate::store::record::SessionRecord;

impl TranscriptStore {
  /// Write a copy of a record another machine drives (a session it runs in a folder of this host, docs/dev/remote.md),
  /// unless the copy here was updated later or the record waits in the trash here (deleted on this host). False when
  /// it was not written. `agent_sessions` (`<home>/agent/sessions`) parks the built-in agent's previous log at
  /// `<sid>/.sync.jsonl` under the same lease before the new copy lands: a promotion before `storeAgentLog` never
  /// finds the stale context at `<sid>.jsonl`, while `storeAgentLog` can append only the new bytes onto the parked copy
  pub async fn put_copy(&self, record: &SessionRecord, agent_sessions: Option<&Path>) -> Result<bool> {
    let id = &record.id;
    if !is_session_id(id) {
      return Err(anyhow!("{id} is not a session id"));
    }
    self.ensure().await?;
    let Some(path) = confined(&self.dir, &format!("{id}.json")).await else { return Err(anyhow!("{id}: outside the store")) };
    if self.in_trash(id).await {
      return Ok(false);
    }
    // The helper shares the host's lease: a promotion cannot race the ownership check and write below.
    let leases = crate::store::session_lease::SessionLeases::new(self.dir.parent().unwrap_or(&self.dir), self.log.clone());
    if !matches!(leases.claim(id), crate::store::session_lease::Claim::Held(_)) {
      return Ok(false);
    }
    let prev = self.stamp_of(id).await;
    if prev.as_ref().is_some_and(|here| here.copy_of.is_none() || here.copy_of.as_ref().map(|c| &c.machine) != record.copy_of.as_ref().map(|c| &c.machine) || here.updated_at > record.updated_at) {
      return Ok(false);
    }
    if let Some(logs) = agent_sessions {
      let prev_sid = prev.as_ref().filter(|h| h.agent == "acpira").and_then(|h| h.acp_session_id.as_deref()).filter(|s| is_session_id(s));
      let next_sid = (record.agent == "acpira").then_some(record.acp_session_id.as_deref()).flatten().filter(|s| is_session_id(s));
      if let Some(old) = prev_sid.filter(|old| Some(*old) != next_sid) {
        let _ = fs::remove_file(logs.join(format!("{old}.jsonl"))).await;
        let _ = fs::remove_file(logs.join(old).join(".sync.jsonl")).await;
        let _ = fs::remove_dir(logs.join(old)).await;
      }
      if let Some(sid) = next_sid {
        let live = logs.join(format!("{sid}.jsonl"));
        let staged = logs.join(sid).join(".sync.jsonl");
        match fs::metadata(&live).await {
          Ok(_) => {
            let dir = logs.join(sid);
            fs::create_dir_all(&dir).await.map_err(|e| anyhow!("{}: {e}", dir.display()))?;
            fs::rename(&live, &staged).await.map_err(|e| anyhow!("{}: {e}", live.display()))?;
          }
          Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
          Err(e) => return Err(anyhow!("{}: {e}", live.display())),
        }
      }
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
  async fn stamp_of(&self, id: &str) -> Option<Stamped> {
    let raw = match self.db_head(id).await {
      Some(head) => head,
      None => fs::read(self.dir.join(format!("{id}.json"))).await.ok()?,
    };
    serde_json::from_slice::<Stamped>(&raw).ok()
  }

  /// For each of `ids` with a record here (not in the trash): its `updatedAt` and the blobs it has
  pub async fn copy_stamps(&self, ids: &[String]) -> HashMap<String, CopyStamp> {
    let mut out = HashMap::new();
    for id in ids.iter().filter(|id| is_session_id(id)) {
      if self.in_trash(id).await {
        continue;
      }
      let Some(stamp) = self.stamp_of(id).await else { continue };
      out.insert(id.clone(), CopyStamp { updated_at: stamp.updated_at, blobs: self.blob_names(id).await, taken_over: stamp.copy_of.is_none(), copy_of: stamp.copy_of });
    }
    out
  }
}
