//! `prefs.json`: the per-agent memory and the unread marks every engine shares

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use tokio::fs;

use acpira_shared::transcript::AgentId;

use super::{SessionPrefs, TranscriptStore, UnreadOp, is_session_id};
use crate::store::file_lock::{with_file_lock, write_atomic};

impl TranscriptStore {
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
        // This host's picks win; models only another host ran stay behind them
        if let Some(memory) = prefs.model_memory.get(agent) {
          let mut next = memory.clone();
          if let Some(theirs) = disk.model_memory.get(agent) {
            next.absorb(theirs);
          }
          disk.model_memory.insert(agent.clone(), next);
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
}
