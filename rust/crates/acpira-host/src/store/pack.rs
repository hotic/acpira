//! Cold records packed. A record nobody has written for `PACK_AFTER` is compressed whole into its session's blob folder
//! (`<sessions>/<id>/record.json.zst`) and `<id>.json` shrinks to a stub: the same record with its subagents left out
//! and its turns replaced by one placeholder turn. Loading a stub (`splice`) puts the archived turns back in place of
//! the placeholder; the first write after that is the whole record again, and the next upkeep packs it anew.
//!
//! The stub keeps `~/.acpira` readable by older engines, which know nothing of the archive:
//! - every reader that wants a record's metadata (the index, the controls, the agent log's owner) reads the stub as it
//!   would any record, so the session stays listed with its title, folder and model;
//! - opening it shows the placeholder, whose text asks for a newer Acpira;
//! - their Storage tab keeps a blob a record mentions, so the placeholder's block id names the archive and every blob
//!   the archived turns use (an older engine rewriting the stub keeps the id, unlike an unknown field);
//! - turns an older engine adds after the placeholder are kept after the archived ones, since the placeholder stays
//!   where it was.
//!
//! A record whose placeholder has no archive next to it (an older engine's fork of a stub) loses the placeholder only.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use acpira_shared::transcript::{AgentBlock, AgentTurn, TextBlock, Turn};

use super::file_lock::temp_writer_pid;
use super::record::SessionRecord;
use crate::i18n::t;

/// The archive's name inside the session's blob folder. Not a blob name (`is_blob_name` takes one dot only), so blob
/// listings and host copies pass it over
pub const PACKED_FILE: &str = "record.json.zst";
/// The placeholder's text block id starts so; the archive and blob names follow, comma separated
const MARK: &str = "acpira-packed:";
/// A record untouched this long is packed
pub const PACK_AFTER: Duration = Duration::from_secs(30 * 24 * 3600);
/// Smaller records stay as they are: packing frees little, and the stub has a size of its own
pub const PACK_MIN: u64 = 64 * 1024;
/// Measured on real records: level 12 compresses a 36 MB record about 24x in a tenth of a second; 19 gains a third more
/// for twenty times the time
const LEVEL: i32 = 12;

/// Whether `turn` is a stub's placeholder
pub fn is_placeholder(turn: &Turn) -> bool {
  turn.as_agent().and_then(|a| a.blocks.first()).is_some_and(|b| matches!(b, AgentBlock::Text(t) if t.id.as_deref().is_some_and(|id| id.starts_with(MARK))))
}

fn placeholder(names: &[String]) -> Turn {
  let mut id = format!("{MARK}{PACKED_FILE}");
  for n in names {
    id.push(',');
    id.push_str(n);
  }
  Turn::Agent(AgentTurn {
    blocks: vec![AgentBlock::Text(TextBlock { id: Some(id), phase: None, markdown: t("host.packedNotice"), streaming: None, images: None })],
    ..Default::default()
  })
}

/// Whether `raw` (a record file's bytes) may be a stub, without parsing it
pub fn maybe_stub(raw: &[u8]) -> bool {
  raw.windows(MARK.len()).any(|w| w == MARK.as_bytes())
}

/// Put a stub's archived turns and subagents back. `archive` is the archive's bytes, None when there is none. Err when
/// the archive cannot be read: the record must then not be used, or its next write would lose the archived turns
pub fn splice(mut stub: SessionRecord, archive: Option<&[u8]>) -> Result<SessionRecord, String> {
  let Some(at) = stub.turns.iter().position(is_placeholder) else { return Ok(stub) };
  let Some(bytes) = archive else {
    stub.turns.remove(at);
    return Ok(stub);
  };
  let raw = zstd::stream::decode_all(bytes).map_err(|e| format!("{PACKED_FILE}: {e}"))?;
  let archived: SessionRecord = serde_json::from_slice(&raw).map_err(|e| format!("{PACKED_FILE}: {e}"))?;
  if archived.id != stub.id {
    return Err(format!("{PACKED_FILE} holds session {}", archived.id));
  }
  let later = stub.turns.split_off(at + 1);
  stub.turns.truncate(at);
  stub.turns.extend(archived.turns);
  stub.turns.extend(later);
  // Subagents an older engine added to the stub are newer than the archive's
  let mut subagents = archived.subagents.unwrap_or_default();
  for s in stub.subagents.take().unwrap_or_default() {
    subagents.retain(|o| o.core.id != s.core.id);
    subagents.push(s);
  }
  stub.subagents = (!subagents.is_empty()).then_some(subagents);
  Ok(stub)
}

/// A record file's bytes as the whole record, for readers that parse the file themselves (session search). Blocking
pub fn expand_raw(dir: &Path, raw: Vec<u8>) -> Vec<u8> {
  if !maybe_stub(&raw) {
    return raw;
  }
  let Ok(stub) = serde_json::from_slice::<SessionRecord>(&raw) else { return raw };
  // An archive that cannot be read now leaves the stub as it is rather than dropping its history
  let archive = match fs::read(dir.join(&stub.id).join(PACKED_FILE)) {
    Ok(bytes) => Some(bytes),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
    Err(_) => return raw,
  };
  match splice(stub, archive.as_deref()) {
    Ok(r) => serde_json::to_vec(&r).unwrap_or(raw),
    Err(_) => raw,
  }
}

/// Why `pack` left a record alone
#[derive(Debug, PartialEq, Eq)]
pub enum Kept {
  /// Missing, a stub already, too small, written lately, a copy of another machine's session or one still filling in
  NotCold,
  /// Written while it was being packed: the next upkeep tries again
  Changed,
}

/// Pack record `id` in the sessions folder `dir` if it is cold by `now`. The caller holds the session's lease and
/// knows it is not open. Returns the bytes freed. Blocking
pub fn pack(dir: &Path, id: &str, now: SystemTime) -> std::io::Result<Result<u64, Kept>> {
  let path = dir.join(format!("{id}.json"));
  let Ok(meta) = fs::metadata(&path) else { return Ok(Err(Kept::NotCold)) };
  let stamp = (meta.len(), meta.modified().ok());
  let cold = stamp.1.and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age >= PACK_AFTER);
  if !meta.is_file() || meta.len() < PACK_MIN || !cold {
    return Ok(Err(Kept::NotCold));
  }
  let raw = fs::read(&path)?;
  let Ok(record) = serde_json::from_slice::<SessionRecord>(&raw) else { return Ok(Err(Kept::NotCold)) };
  if record.id != id
    || record.turns.iter().any(is_placeholder)
    || record.copy_of.is_some()
    || record.history_pending
    || record.import_pending
    || record.turns.is_empty()
  {
    return Ok(Err(Kept::NotCold));
  }
  // The archive first: a stub never names an archive that is not there
  let folder = dir.join(id);
  fs::create_dir_all(&folder)?;
  let packed = zstd::bulk::compress(&raw, LEVEL)?;
  let archive = folder.join(PACKED_FILE);
  let tmp = folder.join(format!("{PACKED_FILE}.{}.tmp", std::process::id()));
  fs::write(&tmp, &packed).and_then(|_| fs::rename(&tmp, &archive)).inspect_err(|_| {
    let _ = fs::remove_file(&tmp);
  })?;
  // The blobs the archived turns use, named in the placeholder so an older Storage tab keeps them
  let text = String::from_utf8_lossy(&raw);
  let mut names: Vec<String> = fs::read_dir(&folder)
    .into_iter()
    .flatten()
    .flatten()
    .map(|e| e.file_name().to_string_lossy().into_owned())
    .filter(|n| n != PACKED_FILE && temp_writer_pid(n).is_none() && text.contains(n.as_str()))
    .collect::<HashSet<_>>()
    .into_iter()
    .collect();
  names.sort();
  let stub = SessionRecord { turns: vec![placeholder(&names)], subagents: None, ..record };
  let bytes = serde_json::to_vec(&stub)?;
  // A write that landed since the read wins; the archive stays until the next pack replaces it
  let now_meta = fs::metadata(&path)?;
  if (now_meta.len(), now_meta.modified().ok()) != stamp {
    return Ok(Err(Kept::Changed));
  }
  // Not `write_atomic`'s temp name: a writer that bypassed the store's chain must still not share one file with it
  let tmp = dir.join(format!("{id}.json.pack.{}.tmp", std::process::id()));
  fs::write(&tmp, &bytes).and_then(|_| fs::rename(&tmp, &path)).inspect_err(|_| {
    let _ = fs::remove_file(&tmp);
  })?;
  Ok(Ok((raw.len() as u64).saturating_sub(bytes.len() as u64 + packed.len() as u64)))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const OLD: Duration = Duration::from_secs(40 * 24 * 3600);

  fn sub(id: &str) -> serde_json::Value {
    json!({ "id": id, "turnIndex": 0, "visibility": "session", "state": "completed", "stateSource": "agent", "announcedAt": 1, "turns": [] })
  }

  fn record(id: &str, turns: usize) -> SessionRecord {
    let turn = |i: usize| {
      if i.is_multiple_of(2) {
        json!({ "role": "user", "text": format!("prompt {i} {}", "x".repeat(30_000)), "attachments": [{ "kind": "image", "blob": "abcd.png", "mimeType": "image/png" }] })
      } else {
        json!({ "role": "agent", "blocks": [{ "type": "text", "markdown": format!("reply {i}") }] })
      }
    };
    serde_json::from_value(json!({
      "id": id, "agent": "claude", "cwd": "/w", "title": "t", "updatedAt": "2026-01-01T00:00:00.000Z",
      "turns": (0..turns).map(turn).collect::<Vec<_>>(),
      "subagents": [sub("c1")],
    }))
    .unwrap()
  }

  fn write(dir: &Path, r: &SessionRecord) {
    fs::write(dir.join(format!("{}.json", r.id)), serde_json::to_vec(r).unwrap()).unwrap();
    fs::create_dir_all(dir.join(&r.id)).unwrap();
    fs::write(dir.join(&r.id).join("abcd.png"), b"png").unwrap();
    fs::write(dir.join(&r.id).join("ffff.png"), b"unused").unwrap();
  }

  fn load(dir: &Path, id: &str) -> SessionRecord {
    let stub: SessionRecord = serde_json::from_slice(&fs::read(dir.join(format!("{id}.json"))).unwrap()).unwrap();
    splice(stub, fs::read(dir.join(id).join(PACKED_FILE)).ok().as_deref()).unwrap()
  }

  #[test]
  fn a_cold_record_packs_into_a_stub_and_loads_back_whole() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let r = record("s1", 6);
    write(dir, &r);
    // Not cold yet
    assert_eq!(pack(dir, "s1", SystemTime::now()).unwrap(), Err(Kept::NotCold));
    let freed = pack(dir, "s1", SystemTime::now() + OLD).unwrap().unwrap();
    assert!(freed > 50_000, "freed {freed}");
    let raw = fs::read(dir.join("s1.json")).unwrap();
    let stub: SessionRecord = serde_json::from_slice(&raw).unwrap();
    assert_eq!((stub.title.as_str(), stub.turns.len(), stub.subagents.is_none()), ("t", 1, true));
    // The placeholder names the archive and the blobs in use, so an older Storage tab keeps them
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains(PACKED_FILE) && text.contains("abcd.png") && !text.contains("ffff.png"));
    assert_eq!(load(dir, "s1"), r);
    assert_eq!(serde_json::from_slice::<SessionRecord>(&expand_raw(dir, raw)).unwrap(), r);
    // A stub is not packed twice
    assert_eq!(pack(dir, "s1", SystemTime::now() + OLD).unwrap(), Err(Kept::NotCold));
  }

  #[test]
  fn turns_an_older_engine_added_to_a_stub_follow_the_archived_ones() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let r = record("s2", 6);
    write(dir, &r);
    pack(dir, "s2", SystemTime::now() + OLD).unwrap().unwrap();
    // An older engine opened the stub, ran one more turn and rewrote the record (dropping nothing it knows)
    let mut stub: SessionRecord = serde_json::from_slice(&fs::read(dir.join("s2.json")).unwrap()).unwrap();
    stub.turns.push(serde_json::from_value(json!({ "role": "user", "text": "one more" })).unwrap());
    stub.subagents = Some(vec![serde_json::from_value(sub("c2")).unwrap()]);
    fs::write(dir.join("s2.json"), serde_json::to_vec(&stub).unwrap()).unwrap();
    let back = load(dir, "s2");
    assert_eq!(back.turns.len(), 7);
    assert_eq!(back.turns[..6], r.turns[..]);
    assert_eq!(back.turns[6].as_user().map(|u| u.text.as_str()), Some("one more"));
    assert_eq!(back.subagents.unwrap().iter().map(|s| s.core.id.as_str()).collect::<Vec<_>>(), vec!["c1", "c2"]);
    // A stub copied without its archive (an older engine's fork) just loses the placeholder
    let orphan = splice(stub, None).unwrap();
    assert_eq!(orphan.turns.len(), 1);
    // A damaged archive is an error, never an empty transcript
    let stub: SessionRecord = serde_json::from_slice(&fs::read(dir.join("s2.json")).unwrap()).unwrap();
    assert!(splice(stub, Some(b"not zstd")).is_err());
  }

  #[test]
  fn small_copied_or_changing_records_stay() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    fs::write(dir.join("s3.json"), br#"{"id":"s3","agent":"claude","cwd":"/w","updatedAt":"x","turns":[{"role":"user","text":"hi"}]}"#).unwrap();
    assert_eq!(pack(dir, "s3", SystemTime::now() + OLD).unwrap(), Err(Kept::NotCold));
    let mut copy = record("s4", 6);
    copy.copy_of = serde_json::from_value(json!({ "machine": "m" })).unwrap();
    write(dir, &copy);
    assert_eq!(pack(dir, "s4", SystemTime::now() + OLD).unwrap(), Err(Kept::NotCold));
    assert!(!dir.join("s4").join(PACKED_FILE).exists());
  }
}
