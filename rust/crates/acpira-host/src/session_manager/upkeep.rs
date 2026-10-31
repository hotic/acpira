//! Storage upkeep in the background, so `~/.acpira` stops growing with what it already holds:
//! - blobs written before the shared index (`store::blob_pool`) are linked to one file per content, and index entries
//!   no session names any more are dropped;
//! - records nobody wrote for a month are packed (`store::pack`);
//! - cold built-in agent logs get the runaway command output older builds wrote cut to the replay cap
//!   (`acpira_agent::store::REPLAY_OUTPUT_MAX`).
//!
//! Every step is lossless and safe against older engines sharing the folder (see each module). One engine on the
//! machine runs it at a time and at most once per `UPKEEP_EVERY`, through a stamp file under the file lock; a session
//! open here, running or held by another engine is left for a later run. `ACPIRA_NO_UPKEEP=1` turns it off.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::SessionManager;
use crate::acp::agents::registry::SELF_AGENT_ID;
use crate::store::blob_pool;
use crate::store::file_lock::{temp_writer_pid, with_file_lock, write_atomic};
use crate::store::pack::{self, PACK_AFTER};
use crate::store::transcript_store::{is_blob_name, is_session_id};

/// How long after the engine starts the first run waits, so a starting window never competes with it for the disk
pub const UPKEEP_DELAY: Duration = Duration::from_secs(10 * 60);
/// At most one run per this, across every engine on the machine
pub const UPKEEP_EVERY: Duration = Duration::from_secs(20 * 3600);
/// A pause between sessions, so a run stays in the background on a slow disk
const PAUSE: Duration = Duration::from_millis(20);

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stamp {
  /// Milliseconds since the epoch of the last run's start
  #[serde(default)]
  last: u64,
}

/// What a run did, for the engine log
#[derive(Debug, Default, PartialEq, Eq)]
pub struct UpkeepReport {
  pub shared: u64,
  pub packed: u64,
  pub packed_sessions: usize,
  pub logs: u64,
}

fn ms(t: SystemTime) -> u64 {
  t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn upkeep_disabled() -> bool {
  std::env::var("ACPIRA_NO_UPKEEP").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// Link every blob of the session folders (live and trashed) to the index; returns the bytes freed. Blocking
fn share_blobs(sessions: &Path) -> u64 {
  let pool = sessions.join(blob_pool::POOL_DIR);
  let mut live = HashSet::new();
  let mut freed = 0;
  for parent in [sessions.to_path_buf(), sessions.join("trash")] {
    let Ok(rd) = std::fs::read_dir(&parent) else { continue };
    for e in rd.flatten() {
      let name = e.file_name().to_string_lossy().into_owned();
      if !is_session_id(&name) || !e.file_type().is_ok_and(|t| t.is_dir()) {
        continue;
      }
      for f in std::fs::read_dir(e.path()).into_iter().flatten().flatten() {
        let blob = f.file_name().to_string_lossy().into_owned();
        if is_blob_name(&blob) && temp_writer_pid(&blob).is_none() && f.file_type().is_ok_and(|t| t.is_file()) {
          freed += blob_pool::share(&pool, &f.path(), &blob);
          live.insert(blob);
        }
      }
      std::thread::sleep(PAUSE);
    }
  }
  blob_pool::prune(&pool, &live);
  freed
}

/// Cut the oversized output lines of one built-in agent log through a temp file; returns the bytes freed. Blocking
fn clip_log(path: &Path) -> std::io::Result<u64> {
  let before = std::fs::metadata(path)?.len();
  let reader = BufReader::new(std::fs::File::open(path)?);
  let tmp = path.with_extension(format!("jsonl.{}.tmp", std::process::id()));
  let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
  let mut changed = false;
  let written = (|| {
    for line in reader.split(b'\n') {
      let line = line?;
      let text = String::from_utf8_lossy(&line);
      match acpira_agent::store::clip_logged_output(&text) {
        Some(cut) => {
          changed = true;
          out.write_all(cut.as_bytes())?;
        }
        None => out.write_all(&line)?,
      }
      out.write_all(b"\n")?;
    }
    out.flush()
  })();
  drop(out);
  if written.is_err() || !changed {
    let _ = std::fs::remove_file(&tmp);
    return written.map(|_| 0);
  }
  // A write that landed meanwhile (the session was reopened) wins: the copy is dropped
  if std::fs::metadata(path)?.len() != before {
    let _ = std::fs::remove_file(&tmp);
    return Ok(0);
  }
  std::fs::rename(&tmp, path)?;
  Ok(before.saturating_sub(std::fs::metadata(path)?.len()))
}

/// The upkeep's schedule: a first run `UPKEEP_DELAY` after the engine starts, then a look every few hours (the stamp
/// keeps it to one run per `UPKEEP_EVERY`); ends with the manager
pub(super) async fn upkeep_loop(weak: std::sync::Weak<SessionManager>) {
  if upkeep_disabled() {
    return;
  }
  let mut wait = UPKEEP_DELAY;
  loop {
    tokio::time::sleep(wait).await;
    let Some(m) = weak.upgrade() else { return };
    if m.state.lock().disposed {
      return;
    }
    m.storage_upkeep().await;
    wait = UPKEEP_EVERY / 4;
  }
}

impl SessionManager {
  /// Run the upkeep unless another engine ran it lately (or is running it). None when skipped
  pub async fn storage_upkeep(self: &Arc<Self>) -> Option<UpkeepReport> {
    if upkeep_disabled() {
      return None;
    }
    let stamp = self.data_root().join("run").join("storage-upkeep.json");
    let now = SystemTime::now();
    let due = with_file_lock(&stamp, || async {
      let last: Stamp = tokio::fs::read(&stamp).await.ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default();
      if ms(now).saturating_sub(last.last) < UPKEEP_EVERY.as_millis() as u64 {
        return Ok(false);
      }
      if let Some(dir) = stamp.parent() {
        tokio::fs::create_dir_all(dir).await?;
      }
      write_atomic(&stamp, &serde_json::to_vec(&Stamp { last: ms(now) })?, None).await?;
      Ok(true)
    })
    .await;
    match due {
      Ok(true) => {}
      Ok(false) => return None,
      Err(e) => {
        self.log(&format!("storage upkeep: skipped ({e:#})"));
        return None;
      }
    }
    let report = self.run_upkeep(now).await;
    self.log(&format!(
      "storage upkeep: {} B shared, {} sessions packed ({} B), {} B of agent logs",
      report.shared, report.packed_sessions, report.packed, report.logs
    ));
    Some(report)
  }

  /// One run, without the once-a-day gate (tests call it directly)
  pub async fn run_upkeep(self: &Arc<Self>, now: SystemTime) -> UpkeepReport {
    let mut report = UpkeepReport::default();
    let sessions = self.deps.store.dir().to_path_buf();
    let dir = sessions.clone();
    report.shared = tokio::task::spawn_blocking(move || share_blobs(&dir)).await.unwrap_or(0);

    let list = self.deps.store.load_index().await.unwrap_or_default();
    for s in &list {
      if let Ok(freed) = self.pack_one(&s.id, now).await {
        report.packed += freed;
        report.packed_sessions += usize::from(freed > 0);
      }
      tokio::time::sleep(PAUSE).await;
    }

    // Built-in agent logs, by the session that drives each
    let root = acpira_agent::acp::sessions_root(&self.data_root());
    for s in list.iter().filter(|s| s.agent == SELF_AGENT_ID) {
      let Some(acp) = s.acp_session_id.as_deref().filter(|a| is_session_id(a)) else { continue };
      if let Ok(freed) = self.clip_one(&s.id, &root.join(format!("{acp}.jsonl")), now).await {
        report.logs += freed;
      }
    }
    report
  }

  /// Pack one record when no engine is using it; the lease is held while the file changes
  async fn pack_one(self: &Arc<Self>, id: &str, now: SystemTime) -> Result<u64> {
    if self.live(id).is_some() || self.storage_block(id, false).is_some() {
      return Ok(0);
    }
    let _hold = self.hold_record(id).await?;
    let (dir, owned) = (self.deps.store.dir().to_path_buf(), id.to_owned());
    match tokio::task::spawn_blocking(move || pack::pack(&dir, &owned, now)).await?? {
      Ok(freed) => Ok(freed),
      Err(_) => Ok(0),
    }
  }

  /// Clip one cold built-in agent log, its session neither open here nor held elsewhere
  async fn clip_one(self: &Arc<Self>, id: &str, log: &Path, now: SystemTime) -> Result<u64> {
    let Ok(meta) = tokio::fs::metadata(log).await else { return Ok(0) };
    let cold = meta.modified().ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|a| a >= PACK_AFTER);
    // A log no longer than the cap has no line to cut
    if !cold || meta.len() <= acpira_agent::store::REPLAY_OUTPUT_MAX as u64 || self.live(id).is_some() || self.storage_block(id, false).is_some()
    {
      return Ok(0);
    }
    let _hold = self.hold_record(id).await?;
    let path = log.to_path_buf();
    Ok(tokio::task::spawn_blocking(move || clip_log(&path)).await??)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_logs_runaway_output_is_cut_and_the_rest_kept_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.jsonl");
    let big = json!({ "type": "update", "update": { "_meta": { "terminal_output_delta": { "data": "y".repeat(300_000) } } } }).to_string();
    let other = r#"{"type":"item","item":{"role":"user","parts":[]},"t":1}"#;
    std::fs::write(&path, format!("{other}\n{big}\n{other}\n")).unwrap();
    let freed = clip_log(&path).unwrap();
    assert!(freed > 200_000);
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!((lines.len(), lines[0], lines[2]), (3, other, other));
    // Nothing left to cut: the file stays as it is
    assert_eq!(clip_log(&path).unwrap(), 0);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1, "no temp file is left");
  }

  #[test]
  fn sharing_walks_live_and_trashed_sessions() {
    let root = tempfile::tempdir().unwrap();
    let s = root.path();
    for (dir, name) in [("a", "1111.png"), ("b", "1111.png"), ("trash/c", "1111.png"), ("b", "2222.png")] {
      std::fs::create_dir_all(s.join(dir)).unwrap();
      std::fs::write(s.join(dir).join(name), b"same bytes").unwrap();
    }
    std::fs::create_dir_all(s.join(blob_pool::POOL_DIR)).unwrap();
    std::fs::write(s.join(blob_pool::POOL_DIR).join("dead.png"), b"x").unwrap();
    assert_eq!(share_blobs(s), 20);
    assert!(same_file::is_same_file(s.join("a/1111.png"), s.join("trash/c/1111.png")).unwrap());
    assert!(!same_file::is_same_file(s.join("a/1111.png"), s.join("b/2222.png")).unwrap(), "another name is another blob");
    assert!(!s.join(blob_pool::POOL_DIR).join("dead.png").exists());
  }
}
