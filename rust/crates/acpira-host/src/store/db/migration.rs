//! Compatibility files for database-backed records. Older engines see a valid record with one upgrade notice and keep the
//! pre-migration archive and every referenced blob alive through the placeholder block id.

use acpira_shared::transcript::{AgentBlock, AgentTurn, TextBlock, Turn};
use anyhow::{Result, anyhow};
use super::super::record::SessionRecord;
use crate::i18n::t;

pub const MIGRATED_FILE: &str = "record.pre-db.json.zst";
const MARK: &str = "acpira-sqlite:";

pub fn is_placeholder(turn: &Turn) -> bool {
  turn.as_agent().and_then(|turn| turn.blocks.first()).is_some_and(|block| matches!(block, AgentBlock::Text(text) if text.id.as_deref().is_some_and(|id| id.starts_with(MARK))))
}

pub fn maybe_stub(raw: &[u8]) -> bool {
  raw.windows(MARK.len()).any(|window| window == MARK.as_bytes())
}

/// Merge a compatibility stub with the current database body. Turns and subagents an older engine added after the
/// placeholder win over their archived counterparts, matching cold-pack splicing.
pub fn splice(mut stub: SessionRecord, archived: SessionRecord) -> Result<SessionRecord> {
  let Some(at) = stub.turns.iter().position(is_placeholder) else { return Ok(stub) };
  if archived.id != stub.id {
    return Err(anyhow!("database body holds session {}", archived.id));
  }
  let later = stub.turns.split_off(at + 1);
  stub.turns.truncate(at);
  stub.turns.extend(archived.turns);
  stub.turns.extend(later);
  let mut subagents = archived.subagents.unwrap_or_default();
  for subagent in stub.subagents.take().unwrap_or_default() {
    subagents.retain(|other| other.core.id != subagent.core.id);
    subagents.push(subagent);
  }
  stub.subagents = (!subagents.is_empty()).then_some(subagents);
  Ok(stub)
}

/// Expand a migration stub while SQLite is disabled or quarantined. A missing archive is an error, never an empty history.
pub fn splice_archive(stub: SessionRecord, archive: &[u8]) -> Result<SessionRecord> {
  let raw = zstd::stream::decode_all(archive)?;
  let archived: SessionRecord = serde_json::from_slice(&raw)?;
  splice(stub, archived)
}

/// Build the small legacy record without cloning the full turns and subagents. The current summary fields stay readable
/// to older engines, while its one block names the archive and all blobs that the Storage tab must retain.
pub fn stub(record: &SessionRecord, names: &[String]) -> Result<Vec<u8>> {
  let mut id = format!("{MARK}{MIGRATED_FILE}");
  for name in names {
    id.push(',');
    id.push_str(name);
  }
  let placeholder = Turn::Agent(AgentTurn {
    blocks: vec![AgentBlock::Text(TextBlock { id: Some(id), phase: None, markdown: t("host.packedNotice"), streaming: None, images: None })],
    ..Default::default()
  });
  let mut value = serde_json::to_value(record)?;
  let object = value.as_object_mut().ok_or_else(|| anyhow!("session record is not an object"))?;
  object.insert("turns".into(), serde_json::to_value([placeholder])?);
  // None and an empty list both deserialize to no visible subagents; omitting keeps the old file smaller.
  object.shift_remove("subagents");
  Ok(serde_json::to_vec(&value)?)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn the_stub_keeps_summary_and_references_but_not_the_transcript() {
    let record: SessionRecord = serde_json::from_value(json!({
      "id": "s", "agent": "claude", "cwd": "/w", "title": "current", "updatedAt": "1",
      "turns": [{ "role": "user", "text": "secret transcript" }],
      "subagents": [{ "id": "child", "turnIndex": 0, "visibility": "session", "state": "completed", "stateSource": "agent", "announcedAt": 1, "turns": [] }]
    }))
    .unwrap();
    let raw = stub(&record, &["blob.png".into()]).unwrap();
    let stub: SessionRecord = serde_json::from_slice(&raw).unwrap();
    assert_eq!(stub.title, "current");
    assert_eq!(stub.turns.len(), 1);
    assert!(is_placeholder(&stub.turns[0]));
    assert!(stub.subagents.is_none());
    let text = String::from_utf8(raw).unwrap();
    assert!(text.contains(MIGRATED_FILE) && text.contains("blob.png"));
    assert!(!text.contains("secret transcript") && !text.contains("child"));
  }
}
