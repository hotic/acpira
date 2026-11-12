//! Session bodies as rows, split at the record's own boundaries: the record head, each turn's head, each agent block, and
//! the same again per subagent. A save hashes the pieces it is handed against the hashes it last wrote and touches only
//! the rows that differ, so a streaming reply costs its tail and a finished subagent costs nothing.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, anyhow};
use rusqlite::{Connection, params};
use serde_json::Value;

use super::super::record::{RecordParts, SessionRecord, TurnPart};

const ROOT: i64 = -1;
const HEADER: i64 = -1;
/// Smaller rows stay plain: compressing them saves little and every read pays for it
const COMPRESS_AT: usize = 4 * 1024;
const ZSTD_LEVEL: i32 = 3;

pub type RowHash = [u8; 16];

fn hash_of(raw: &[u8]) -> RowHash {
  let mut out = [0; 16];
  out.copy_from_slice(&blake3::hash(raw).as_bytes()[..16]);
  out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ItemKey {
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

/// What one store remembers of a session's rows between saves, so a save reads nothing back. Dropped whenever the rows
/// may have changed behind it (a load, a copy, a revert): the next save then reads the hashes once
#[derive(Default)]
pub struct SaveCache {
  loaded: bool,
  /// `PRAGMA data_version` at the last save: it moves when another connection (another engine) commits, and what this
  /// cache says about the rows is then no longer known to be true
  version: Option<i64>,
  rows: HashMap<ItemKey, RowHash>,
  summary: String,
  /// Subagent id → (its position, the stamp of the transcript stored for it)
  subagents: HashMap<String, (i64, u64)>,
}

impl SaveCache {
  /// The record head as last written, by its hash
  pub fn head(&self) -> Option<RowHash> {
    self.rows.get(&ItemKey { subagent: ROOT, turn: HEADER, block: HEADER }).copied()
  }

  /// The subagent transcripts a source may leave out of its next `record_parts`
  pub fn known(&self) -> HashMap<String, u64> {
    if !self.loaded {
      return HashMap::new();
    }
    self.subagents.iter().filter(|(_, (_, stamp))| *stamp != 0).map(|(id, (_, stamp))| (id.clone(), *stamp)).collect()
  }
}

pub enum Saved {
  Done(SaveStats),
  /// A subagent came without its turns but is not where the stored rows have it: ask the source for everything
  NeedsFull,
}

/// Save one body and its catalogue summary in one transaction. Hashes cover the uncompressed JSON, so compression and
/// page layout never make an unchanged row look dirty
pub(super) fn save(connection: &mut Connection, id: &str, parts: &RecordParts, cache: &mut SaveCache) -> Result<Saved> {
  let version: i64 = connection.query_row("PRAGMA data_version", [], |row| row.get(0))?;
  if cache.version != Some(version) {
    cache.loaded = false;
    cache.version = Some(version);
  }
  if !cache.loaded {
    let mut statement = connection.prepare("SELECT subagent, turn, block, hash FROM item WHERE session_id = ?1")?;
    cache.rows = statement
      .query_map(params![id], |row| Ok((ItemKey { subagent: row.get(0)?, turn: row.get(1)?, block: row.get(2)? }, row.get::<_, Vec<u8>>(3)?)))?
      // A hash of another length (an older build's) matches nothing: the row is known, and rewritten by this save
      .map(|row| row.map(|(key, hash)| (key, RowHash::try_from(hash.as_slice()).unwrap_or_default())))
      .collect::<rusqlite::Result<_>>()?;
    cache.summary.clear();
    cache.subagents.clear();
    cache.loaded = true;
  }
  for (index, subagent) in parts.subagents.iter().enumerate() {
    if subagent.turns.is_none() && cache.subagents.get(&subagent.id).map(|(at, _)| *at) != Some(index as i64) {
      return Ok(Saved::NeedsFull);
    }
  }
  let summary = serde_json::to_string(&parts.summary)?;
  let tx = connection.transaction()?;
  // The catalogue row comes first: the item rows refer to it. A session that is saved is listed: a mark left by a
  // reconcile that missed its file goes, or the search would skip the body until the next reconcile
  if summary != cache.summary {
    tx.execute(
      "INSERT INTO session (id, summary, body) VALUES (?1, ?2, 'db')
       ON CONFLICT(id) DO UPDATE SET summary = excluded.summary, body = 'db', trashed_at = NULL",
      params![id, summary],
    )?;
  }
  let mut stats = SaveStats::default();
  let mut seen: HashSet<ItemKey> = HashSet::new();
  let mut written: Vec<(ItemKey, RowHash)> = vec![];
  {
    let mut upsert = tx.prepare(
      "INSERT INTO item (session_id, subagent, turn, block, codec, payload, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
       ON CONFLICT(session_id, subagent, turn, block) DO UPDATE SET codec = excluded.codec, payload = excluded.payload, hash = excluded.hash",
    )?;
    let mut put = |key: ItemKey, raw: &[u8]| -> Result<()> {
      if raw.is_empty() {
        return Err(anyhow!("session {id}: a record piece did not serialize ({key:?})"));
      }
      stats.rows += 1;
      seen.insert(key);
      let hash = hash_of(raw);
      if cache.rows.get(&key) == Some(&hash) {
        return Ok(());
      }
      let compressed = (raw.len() >= COMPRESS_AT).then(|| zstd::bulk::compress(raw, ZSTD_LEVEL)).transpose()?;
      let (codec, payload) = match &compressed {
        Some(compressed) if compressed.len() < raw.len() => (1, compressed.as_slice()),
        _ => (0, raw),
      };
      stats.changed += 1;
      stats.payload_bytes += payload.len() as u64;
      upsert.execute(params![id, key.subagent, key.turn, key.block, codec, payload, hash.as_slice()])?;
      written.push((key, hash));
      Ok(())
    };
    let put_turns = |subagent: i64, turns: &[TurnPart], put: &mut dyn FnMut(ItemKey, &[u8]) -> Result<()>| -> Result<()> {
      for (turn, part) in turns.iter().enumerate() {
        let turn = turn as i64;
        put(ItemKey { subagent, turn, block: HEADER }, &part.head)?;
        for (block, raw) in part.blocks.iter().enumerate() {
          put(ItemKey { subagent, turn, block: block as i64 }, raw)?;
        }
      }
      Ok(())
    };
    put(ItemKey { subagent: ROOT, turn: HEADER, block: HEADER }, &parts.head)?;
    put_turns(ROOT, &parts.turns, &mut put)?;
    for (index, subagent) in parts.subagents.iter().enumerate() {
      put(ItemKey { subagent: index as i64, turn: HEADER, block: HEADER }, &subagent.head)?;
      if let Some(turns) = &subagent.turns {
        put_turns(index as i64, turns, &mut put)?;
      }
    }
  }
  // Rows of every scope this save covered that it did not write again are gone from the record (an edit or a retry cut
  // turns; a subagent left). A subagent that came without turns keeps the rows it has
  let covered = |key: &ItemKey| match usize::try_from(key.subagent) {
    Err(_) => true,
    Ok(index) => parts.subagents.get(index).is_none_or(|subagent| subagent.turns.is_some()),
  };
  let gone: Vec<ItemKey> = cache.rows.keys().filter(|key| covered(key) && !seen.contains(key)).copied().collect();
  {
    let mut delete = tx.prepare("DELETE FROM item WHERE session_id = ?1 AND subagent = ?2 AND turn = ?3 AND block = ?4")?;
    for key in &gone {
      delete.execute(params![id, key.subagent, key.turn, key.block])?;
      stats.deleted += 1;
    }
  }
  if stats.changed + stats.deleted > 0 || summary != cache.summary {
    tx.execute("UPDATE session SET revision = revision + 1 WHERE id = ?1", params![id])?;
  }
  tx.commit()?;
  // The cache follows the database only once the transaction is in
  for key in &gone {
    cache.rows.remove(key);
  }
  cache.rows.extend(written);
  cache.summary = summary;
  cache.subagents = parts.subagents.iter().enumerate().map(|(index, subagent)| (subagent.id.clone(), (index as i64, subagent.stamp))).collect();
  Ok(Saved::Done(stats))
}

fn decoded(id: &str, codec: i64, payload: Vec<u8>, expected: &[u8], at: ItemKey) -> Result<Vec<u8>> {
  let raw = match codec {
    0 => payload,
    // The frame names its content size (`bulk::compress` writes it): one allocation of the right size
    1 => match zstd::zstd_safe::get_frame_content_size(&payload) {
      Ok(Some(size)) => zstd::bulk::decompress(&payload, usize::try_from(size)?)?,
      _ => zstd::stream::decode_all(payload.as_slice())?,
    },
    other => return Err(anyhow!("session {id}: unknown body codec {other}")),
  };
  if expected.len() == 16 && hash_of(&raw).as_slice() != expected {
    return Err(anyhow!("session {id}: body row hash mismatch at {}/{}/{}", at.subagent, at.turn, at.block));
  }
  Ok(raw)
}

/// (subagent, turn, block, the row's JSON)
type StoredRow = (i64, i64, i64, Vec<u8>);
/// A turn's head and its blocks
type StoredTurn = (Vec<u8>, Vec<Vec<u8>>);

/// Every stored row of a session in record order, decoded; `main_only` leaves the subagents out
pub(super) fn rows(connection: &Connection, id: &str, main_only: bool) -> Result<Vec<StoredRow>> {
  let sql = if main_only {
    "SELECT subagent, turn, block, codec, payload, hash FROM item WHERE session_id = ?1 AND subagent = -1 ORDER BY turn, block"
  } else {
    "SELECT subagent, turn, block, codec, payload, hash FROM item WHERE session_id = ?1 ORDER BY subagent, turn, block"
  };
  let mut statement = connection.prepare(sql)?;
  let stored = statement
    .query_map(params![id], |row| {
      Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, Vec<u8>>(4)?, row.get::<_, Vec<u8>>(5)?))
    })?
    .collect::<rusqlite::Result<Vec<_>>>()?;
  stored
    .into_iter()
    .map(|(subagent, turn, block, codec, payload, expected)| Ok((subagent, turn, block, decoded(id, codec, payload, &expected, ItemKey { subagent, turn, block })?)))
    .collect()
}

/// The record head alone (controls, commands, title): what most readers of a record want, at the cost of one row
pub(super) fn head(connection: &Connection, id: &str) -> Result<Option<Vec<u8>>> {
  use rusqlite::OptionalExtension;
  let row = connection
    .prepare("SELECT codec, payload, hash FROM item WHERE session_id = ?1 AND subagent = -1 AND turn = -1 AND block = -1")?
    .query_row(params![id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?, row.get::<_, Vec<u8>>(2)?)))
    .optional()?;
  row.map(|(codec, payload, expected)| decoded(id, codec, payload, &expected, ItemKey { subagent: ROOT, turn: HEADER, block: HEADER })).transpose()
}

/// The whole record as JSON text, spliced from the stored pieces without building a value tree: what a typed load, the
/// search and the Storage tab parse or scan. `main_only` leaves the subagents' rows out (their list stays empty).
/// None when a head does not carry exactly one empty array to splice into; `load` then joins the rows as values
pub(super) fn raw(connection: &Connection, id: &str, main_only: bool) -> Result<Option<Vec<u8>>> {
  let stored = rows(connection, id, main_only)?;
  let mut root: Option<Vec<u8>> = None;
  // (subagent, its head, its turns as (head, blocks)); the root's turns sit under ROOT
  let mut groups: Vec<(i64, Option<Vec<u8>>, Vec<StoredTurn>)> = vec![];
  for (subagent, turn, block, raw) in stored {
    if subagent == ROOT && turn == HEADER {
      root = Some(raw);
      continue;
    }
    if groups.last().is_none_or(|group| group.0 != subagent) {
      groups.push((subagent, None, vec![]));
    }
    let Some(group) = groups.last_mut() else { continue };
    if turn == HEADER {
      group.1 = Some(raw);
    } else if block == HEADER {
      if turn != group.2.len() as i64 {
        return Err(anyhow!("session {id}: turn {turn} is out of order"));
      }
      group.2.push((raw, vec![]));
    } else {
      let at = group.2.len() as i64 - 1;
      match group.2.last_mut() {
        Some((_, blocks)) if turn == at => blocks.push(raw),
        _ => return Err(anyhow!("session {id}: turn {turn} header is missing")),
      }
    }
  }
  let root = root.ok_or_else(|| anyhow!("session {id}: record header is missing"))?;
  let turns_of = |turns: Vec<StoredTurn>| -> Option<Vec<Vec<u8>>> {
    turns.into_iter().map(|(head, blocks)| if blocks.is_empty() { Some(head) } else { splice(&head, &[("blocks", &blocks)]) }).collect()
  };
  let mut main = vec![];
  let mut subagents = vec![];
  for (subagent, head, turns) in groups {
    let Some(turns) = turns_of(turns) else { return Ok(None) };
    if subagent == ROOT {
      main = turns;
      continue;
    }
    let head = head.ok_or_else(|| anyhow!("session {id}: subagent header is missing"))?;
    let Some(whole) = splice(&head, &[("turns", &turns)]) else { return Ok(None) };
    subagents.push(whole);
  }
  let spliced = if subagents.is_empty() { splice(&root, &[("turns", &main)]) } else { splice(&root, &[("turns", &main), ("subagents", &subagents)]) };
  Ok(spliced)
}

/// `head` with each `"<key>":[]` filled with `items`. Inside a JSON string every quote is escaped, so the marker can only
/// be a key; None unless the head has it exactly once (a nested object with the same empty key makes it ambiguous)
fn splice(head: &[u8], fills: &[(&str, &[Vec<u8>])]) -> Option<Vec<u8>> {
  let mut places: Vec<(usize, usize, &[Vec<u8>])> = vec![];
  for (key, items) in fills {
    let mark = format!("\"{key}\":[]");
    let mut found = head.windows(mark.len()).enumerate().filter(|(_, window)| *window == mark.as_bytes()).map(|(at, _)| at);
    let at = found.next()?;
    if found.next().is_some() {
      return None;
    }
    // Just past the opening bracket
    places.push((at + mark.len() - 1, mark.len(), items));
  }
  places.sort_by_key(|place| place.0);
  let mut out = Vec::with_capacity(head.len() + fills.iter().flat_map(|(_, items)| items.iter()).map(|item| item.len() + 1).sum::<usize>());
  let mut from = 0;
  for (at, _, items) in places {
    out.extend_from_slice(&head[from..at]);
    for (index, item) in items.iter().enumerate() {
      if index > 0 {
        out.push(b',');
      }
      out.extend_from_slice(item);
    }
    from = at;
  }
  out.extend_from_slice(&head[from..]);
  Some(out)
}

pub(super) fn load(connection: &Connection, id: &str) -> Result<SessionRecord> {
  let record: SessionRecord = match raw(connection, id, false)? {
    Some(raw) => serde_json::from_slice(&raw)?,
    None => {
      let items = rows(connection, id, false)?
        .into_iter()
        .map(|(subagent, turn, block, raw)| Ok(BodyRow { key: ItemKey { subagent, turn, block }, value: serde_json::from_slice(&raw)? }))
        .collect::<Result<Vec<_>>>()?;
      join(id, items)?
    }
  };
  if record.id != id {
    return Err(anyhow!("session {id}: body holds session {}", record.id));
  }
  Ok(record)
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
  use crate::store::record::RecordParts;
  use serde_json::json;

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
  fn a_small_change_to_a_21_mb_record_writes_less_than_1_mb() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let mut record = record("large");
    if let acpira_shared::transcript::Turn::Agent(turn) = &mut record.turns[1]
      && let acpira_shared::transcript::AgentBlock::Text(text) = &mut turn.blocks[0]
    {
      text.markdown = "x".repeat(21 * 1024 * 1024);
    }
    db.save_record(&record).unwrap();
    record.title = "one changed header".into();
    let saved = db.save_record(&record).unwrap();
    assert_eq!(saved.changed, 1);
    assert!(saved.payload_bytes < 1024 * 1024, "{saved:?}");
  }

  #[test]
  fn bodies_round_trip_compress_and_save_only_changed_rows() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let mut record = record("rows");
    let imported = db.save_record(&record).unwrap();
    assert_eq!(imported.changed, 7);
    assert_eq!(db.load_record("rows").unwrap().unwrap(), record);
    let compressed: i64 = db.writer.lock().query_row("SELECT count(*) FROM item WHERE codec = 1", [], |row| row.get(0)).unwrap();
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
  #[test]
  fn a_subagent_handed_over_without_its_turns_keeps_its_rows() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let mut record = record("skip");
    let mut cache = SaveCache::default();
    let mut parts = RecordParts::of(record.clone());
    parts.subagents[0].stamp = 7;
    assert!(matches!(db.save_parts("skip", &parts, &mut cache).unwrap(), Saved::Done(_)));
    assert_eq!(cache.known().get("child"), Some(&7));

    // The parent moves on, the child is known: its turns stay out of the save and in the database
    record.title = "later".into();
    let mut parts = RecordParts::of(record.clone());
    parts.subagents[0].stamp = 7;
    parts.subagents[0].turns = None;
    let Saved::Done(saved) = db.save_parts("skip", &parts, &mut cache).unwrap() else { panic!("the child is where the rows have it") };
    assert_eq!((saved.changed, saved.deleted), (1, 0), "only the record head changed");
    assert_eq!(db.load_record("skip").unwrap().unwrap(), record);

    // Another engine wrote meanwhile: nothing this cache says is trusted, so a save without the child's turns is refused
    Connection::open(root.path().join(crate::store::db::DB_FILE)).unwrap().execute("UPDATE session SET trashed_at = 1", []).unwrap();
    assert!(matches!(db.save_parts("skip", &parts, &mut cache).unwrap(), Saved::NeedsFull));
    assert!(cache.known().is_empty());
    assert!(matches!(db.save_parts("skip", &RecordParts::of(record.clone()), &mut cache).unwrap(), Saved::Done(_)));
    assert_eq!(db.load_record("skip").unwrap().unwrap(), record);
  }

  #[test]
  fn a_head_with_an_ambiguous_empty_list_still_loads() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    let mut record = record("nested");
    // A plugin pick that happens to carry an empty `turns` key: the text splice steps aside for the value join
    record.plugin_menus.insert("cmd".into(), json!({ "turns": [] }).as_object().unwrap().clone());
    db.save_record(&record).unwrap();
    assert_eq!(db.load_record("nested").unwrap().unwrap(), record);
    let raw = db.load_raw("nested", false).unwrap().unwrap();
    assert_eq!(serde_json::from_slice::<SessionRecord>(&raw).unwrap(), record);
  }

  #[test]
  fn a_missing_turn_head_is_an_error_not_a_shorter_record() {
    let root = tempfile::tempdir().unwrap();
    let db = SessionDb::open(root.path()).unwrap();
    db.save_record(&record("holes")).unwrap();
    db.writer.lock().execute("DELETE FROM item WHERE session_id = 'holes' AND subagent = -1 AND turn = 0", []).unwrap();
    assert!(db.load_record("holes").is_err());
  }
}
