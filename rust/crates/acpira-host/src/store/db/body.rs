//! Session bodies split at their existing structural boundaries. JSON values keep this independent of every block variant:
//! a turn header owns its metadata, each agent block owns its potentially large content, and subagents use the same shape.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::record::SessionRecord;

const ROOT: i64 = -1;
const HEADER: i64 = -1;
const COMPRESS_AT: usize = 64 * 1024;
const ZSTD_LEVEL: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ItemKey {
  subagent: i64,
  turn: i64,
  block: i64,
}

struct BodyRow {
  key: ItemKey,
  value: Value,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SaveStats {
  pub rows: u64,
  pub changed: u64,
  pub deleted: u64,
  pub payload_bytes: u64,
}

/// Save one body and its catalogue summary in the same transaction. Hashes cover uncompressed JSON, so compression and
/// SQLite page layout cannot make an unchanged row look dirty.
pub(super) fn save(connection: &mut Connection, record: &SessionRecord) -> Result<SaveStats> {
  let rows = split(record)?;
  let summary = serde_json::to_string(&record.summary())?;
  let tx = connection.transaction()?;
  let old_summary = tx
    .query_row("SELECT summary, body FROM session WHERE id = ?1", params![record.id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
    .optional()?;
  if old_summary.as_ref().is_none_or(|(old, body)| old != &summary || body != "db") {
    tx.execute(
      "INSERT INTO session (id, summary, body) VALUES (?1, ?2, 'db')
       ON CONFLICT(id) DO UPDATE SET summary = excluded.summary, body = 'db'",
      params![record.id, summary],
    )?;
  }
  let existing = {
    let mut statement = tx.prepare("SELECT subagent, turn, block, hash FROM item WHERE session_id = ?1")?;
    statement
      .query_map(params![record.id], |row| {
        Ok((ItemKey { subagent: row.get(0)?, turn: row.get(1)?, block: row.get(2)? }, row.get::<_, Vec<u8>>(3)?))
      })?
      .collect::<rusqlite::Result<HashMap<_, _>>>()?
  };
  let mut stats = SaveStats::default();
  let mut seen = HashSet::new();
  for row in rows {
    stats.rows += 1;
    let key = row.key;
    if !seen.insert(key) {
      return Err(anyhow!("duplicate body key {key:?} for session {}", record.id));
    }
    let raw = serde_json::to_vec(&row.value)?;
    let hash = Sha256::digest(&raw).to_vec();
    if existing.get(&key) == Some(&hash) {
      continue;
    }
    let compressed = (raw.len() >= COMPRESS_AT).then(|| zstd::bulk::compress(&raw, ZSTD_LEVEL)).transpose()?;
    let (codec, payload) = match compressed {
      Some(compressed) if compressed.len() < raw.len() => (1, compressed),
      _ => (0, raw),
    };
    stats.changed += 1;
    stats.payload_bytes += payload.len() as u64;
    tx.execute(
      "INSERT INTO item (session_id, subagent, turn, block, codec, payload, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
       ON CONFLICT(session_id, subagent, turn, block) DO UPDATE SET codec = excluded.codec, payload = excluded.payload, hash = excluded.hash",
      params![record.id, key.subagent, key.turn, key.block, codec, payload, hash],
    )?;
  }
  for key in existing.keys().filter(|key| !seen.contains(key)) {
    tx.execute(
      "DELETE FROM item WHERE session_id = ?1 AND subagent = ?2 AND turn = ?3 AND block = ?4",
      params![record.id, key.subagent, key.turn, key.block],
    )?;
    stats.deleted += 1;
  }
  tx.commit()?;
  Ok(stats)
}

pub(super) fn load(connection: &Connection, id: &str) -> Result<SessionRecord> {
  let mut statement = connection.prepare("SELECT subagent, turn, block, codec, payload, hash FROM item WHERE session_id = ?1 ORDER BY subagent, turn, block")?;
  let rows = statement
    .query_map(params![id], |row| {
      Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, Vec<u8>>(4)?, row.get::<_, Vec<u8>>(5)?))
    })?
    .collect::<rusqlite::Result<Vec<_>>>()?;
  let mut items = vec![];
  for (subagent, turn, block, codec, payload, expected) in rows {
    let raw = match codec {
      0 => payload,
      1 => zstd::stream::decode_all(payload.as_slice())?,
      other => return Err(anyhow!("session {id}: unknown body codec {other}")),
    };
    if Sha256::digest(&raw).to_vec() != expected {
      return Err(anyhow!("session {id}: body row hash mismatch at {subagent}/{turn}/{block}"));
    }
    items.push(BodyRow { key: ItemKey { subagent, turn, block }, value: serde_json::from_slice(&raw)? });
  }
  join(id, items)
}

fn split(record: &SessionRecord) -> Result<Vec<BodyRow>> {
  let mut root = serde_json::to_value(record)?;
  let root_object = root.as_object_mut().ok_or_else(|| anyhow!("session record is not an object"))?;
  let turns = root_object.shift_remove("turns").unwrap_or_else(|| json!([]));
  root_object.insert("turns".into(), json!([]));
  let subagents = root_object.shift_remove("subagents");
  if subagents.is_some() {
    root_object.insert("subagents".into(), json!([]));
  }
  let mut rows = vec![BodyRow { key: ItemKey { subagent: ROOT, turn: HEADER, block: HEADER }, value: root }];
  split_turns(&mut rows, ROOT, turns)?;
  if let Some(subagents) = subagents {
    for (index, mut subagent) in subagents.as_array().cloned().unwrap_or_default().into_iter().enumerate() {
      let index = i64::try_from(index).unwrap_or(i64::MAX);
      let object = subagent.as_object_mut().ok_or_else(|| anyhow!("subagent is not an object"))?;
      let turns = object.shift_remove("turns").unwrap_or_else(|| json!([]));
      object.insert("turns".into(), json!([]));
      rows.push(BodyRow { key: ItemKey { subagent: index, turn: HEADER, block: HEADER }, value: subagent });
      split_turns(&mut rows, index, turns)?;
    }
  }
  Ok(rows)
}

fn split_turns(rows: &mut Vec<BodyRow>, subagent: i64, turns: Value) -> Result<()> {
  for (index, mut turn) in turns.as_array().cloned().unwrap_or_default().into_iter().enumerate() {
    let turn_index = i64::try_from(index).unwrap_or(i64::MAX);
    let is_agent = turn.get("role").and_then(Value::as_str) == Some("agent");
    if !is_agent {
      rows.push(BodyRow { key: ItemKey { subagent, turn: turn_index, block: HEADER }, value: turn });
      continue;
    }
    let object = turn.as_object_mut().ok_or_else(|| anyhow!("agent turn is not an object"))?;
    let blocks = object.shift_remove("blocks").unwrap_or_else(|| json!([]));
    object.insert("blocks".into(), json!([]));
    rows.push(BodyRow { key: ItemKey { subagent, turn: turn_index, block: HEADER }, value: turn });
    for (block, value) in blocks.as_array().cloned().unwrap_or_default().into_iter().enumerate() {
      rows.push(BodyRow { key: ItemKey { subagent, turn: turn_index, block: i64::try_from(block).unwrap_or(i64::MAX) }, value });
    }
  }
  Ok(())
}

#[derive(Default)]
struct TurnParts {
  head: Option<Value>,
  blocks: BTreeMap<i64, Value>,
}

#[derive(Default)]
struct SubagentParts {
  head: Option<Value>,
  turns: BTreeMap<i64, TurnParts>,
}

fn join(id: &str, rows: Vec<BodyRow>) -> Result<SessionRecord> {
  let mut root = None;
  let mut turns: BTreeMap<i64, TurnParts> = BTreeMap::new();
  let mut subagents: BTreeMap<i64, SubagentParts> = BTreeMap::new();
  for row in rows {
    if row.key.turn == HEADER {
      if row.key.subagent == ROOT {
        root = Some(row.value);
      } else {
        subagents.entry(row.key.subagent).or_default().head = Some(row.value);
      }
      continue;
    }
    if row.key.subagent == ROOT {
      let turn = turns.entry(row.key.turn).or_default();
      if row.key.block == HEADER { turn.head = Some(row.value); } else { turn.blocks.insert(row.key.block, row.value); }
    } else {
      let turn = subagents.entry(row.key.subagent).or_default().turns.entry(row.key.turn).or_default();
      if row.key.block == HEADER { turn.head = Some(row.value); } else { turn.blocks.insert(row.key.block, row.value); }
    }
  }
  let mut root = root.ok_or_else(|| anyhow!("session {id}: record header is missing"))?;
  root.as_object_mut().ok_or_else(|| anyhow!("session {id}: record header is not an object"))?.insert("turns".into(), Value::Array(finish_turns(id, turns)?));
  if !subagents.is_empty() {
    let mut values = vec![];
    for (_, parts) in subagents {
      let mut head = parts.head.ok_or_else(|| anyhow!("session {id}: subagent header is missing"))?;
      head.as_object_mut().ok_or_else(|| anyhow!("session {id}: subagent header is not an object"))?.insert("turns".into(), Value::Array(finish_turns(id, parts.turns)?));
      values.push(head);
    }
    root.as_object_mut().ok_or_else(|| anyhow!("session {id}: record header is not an object"))?.insert("subagents".into(), Value::Array(values));
  }
  let record: SessionRecord = serde_json::from_value(root)?;
  if record.id != id {
    return Err(anyhow!("session {id}: body holds session {}", record.id));
  }
  Ok(record)
}

fn finish_turns(id: &str, turns: BTreeMap<i64, TurnParts>) -> Result<Vec<Value>> {
  turns
    .into_iter()
    .map(|(index, parts)| {
      let mut head = parts.head.ok_or_else(|| anyhow!("session {id}: turn {index} header is missing"))?;
      if head.get("role").and_then(Value::as_str) == Some("agent") {
        head.as_object_mut().ok_or_else(|| anyhow!("session {id}: turn {index} is not an object"))?.insert("blocks".into(), Value::Array(parts.blocks.into_values().collect()));
      }
      Ok(head)
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::store::db::SessionDb;

  pub(crate) fn record(id: &str) -> SessionRecord {
    serde_json::from_value(json!({
      "id": id,
      "agent": "claude",
      "cwd": "/w",
      "title": "body rows",
      "createdAt": "2026-01-01T00:00:00.000Z",
      "updatedAt": "2026-01-01T00:00:00.000Z",
      "controls": { "modes": [], "options": [] },
      "commands": [],
      "turns": [
        { "role": "user", "text": "read this" },
        { "role": "agent", "startedAt": 1, "blocks": [{ "type": "text", "markdown": "x".repeat(200_000) }] }
      ],
      "subagents": [{ "id": "child", "turnIndex": 0, "visibility": "session", "state": "completed", "stateSource": "agent", "announcedAt": 1,
        "turns": [{ "role": "agent", "blocks": [{ "type": "text", "markdown": "cold child reply" }] }] }]
    }))
    .unwrap()
  }

  #[test]
  fn bodies_round_trip_compress_and_save_only_changed_rows() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let mut record = record("rows");
    let imported = db.save_record(&record).unwrap();
    assert_eq!(imported.changed, 7);
    assert_eq!(db.load_record("rows").unwrap().unwrap(), record);
    let compressed: i64 = db.connection.lock().query_row("SELECT count(*) FROM item WHERE codec = 1", [], |row| row.get(0)).unwrap();
    assert_eq!(compressed, 1);

    record.title = "changed".into();
    if let acpira_shared::transcript::Turn::Agent(turn) = &mut record.turns[1] {
      turn.blocks.clear();
    }
    let saved = db.save_record(&record).unwrap();
    assert!(saved.changed <= 3, "only the record and turn headers change: {saved:?}");
    assert_eq!(saved.deleted, 1, "the removed block row is deleted");
    assert_eq!(db.load_record("rows").unwrap().unwrap(), record);
    let unchanged = db.save_record(&record).unwrap();
    assert_eq!((unchanged.changed, unchanged.deleted, unchanged.payload_bytes), (0, 0, 0));
  }
}
