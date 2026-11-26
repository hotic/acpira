//! Soft deletion into `trash/`, its undo, the sweep of expired entries and removal for good

use std::path::Path;
use std::time::{Duration, SystemTime};

use anyhow::{Result, anyhow};
use tokio::fs;

use super::{
  RecordChange, Stamped, TRASH_DIR, TranscriptStore, builtin_session_in, builtin_session_of, cwd_of,
  is_session_id, rm_confined, timestamp_ms,
};
use crate::db::{BodyLocation, SessionDb};
use acpira_shared::host_tools::SELF_AGENT_ID;

impl TranscriptStore {
  /// Removes the record and its blob directory for good, wherever they are (live or trash)
  pub async fn remove(&self, id: &str) {
    if !is_session_id(id) {
      return;
    }
    self.state.lock().pending.remove(id);
    self.settle_inflight(id).await;
    {
      let mut state = self.state.lock();
      state.known.remove(id);
      state.stale.remove(id);
      state.plain.remove(id);
    }
    self.texts.lock().remove(id);
    self.forget_saved(id);
    let mut log = None;
    let mut cwd = String::new();
    let hooked = self.on_change.read().is_some();
    for dir in [self.dir.clone(), self.dir.join(TRASH_DIR)] {
      let file = dir.join(format!("{id}.json"));
      if log.is_none() {
        log = builtin_session_of(&file).await;
      }
      if hooked && cwd.is_empty() {
        cwd = cwd_of(&file).await;
      }
      rm_confined(&dir, &format!("{id}.json"), false).await;
      rm_confined(&dir, id, true).await;
    }
    // A stubless body has no file to tell them: its head does
    if (log.is_none() || (hooked && cwd.is_empty()))
      && let Some(raw) = self.db_head(id).await
    {
      log = log.or_else(|| builtin_session_in(&raw));
      if hooked && cwd.is_empty() {
        cwd = serde_json::from_slice::<Stamped>(&raw).map(|s| s.cwd).unwrap_or_default();
      }
    }
    let db_id = id.to_owned();
    if let Err(error) = self.db_call(move |db| db.delete_session(&db_id)).await {
      (self.log)(&format!("session {id}: SQLite row not removed ({error:#})"));
    }
    if let Some(acp) = log {
      self.remove_agent_log(id, &acp).await;
    }
    if hooked {
      self.changed(id, RecordChange::Gone { cwd: &cwd });
    }
  }

  /// The built-in agent's log and folder for a session that is gone for good. Kept while another record (listed, or
  /// still in its own undo window) drives the same agent session, e.g. a session imported twice
  async fn remove_agent_log(&self, removed: &str, acp: &str) {
    let Some(hook) = self.on_purge.read().clone() else { return };
    let listed = match self.load_index().await {
      Ok(index) => index.into_iter().any(|s| s.id != removed && s.agent == SELF_AGENT_ID && s.acp_session_id.as_deref() == Some(acp)),
      Err(error) => {
        (self.log)(&format!("session {removed}: catalogue unreadable, built-in agent log kept ({error})"));
        return;
      }
    };
    if listed {
      return;
    }
    // A stubless body in its undo window has no file in the trash: the catalogue knows whose session it drives
    if self.stubless()
      && let Ok(Some(entries)) = self.db_call(SessionDb::catalog).await
      && entries.iter().any(|e| e.summary.id != removed && e.trashed_at.is_some() && e.summary.agent == SELF_AGENT_ID && e.summary.acp_session_id.as_deref() == Some(acp))
    {
      return;
    }
    let trash = self.dir.join(TRASH_DIR);
    if let Ok(mut rd) = fs::read_dir(&trash).await {
      while let Ok(Some(e)) = rd.next_entry().await {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.strip_suffix(".json").is_some_and(|other| other != removed && is_session_id(other))
          && builtin_session_of(&e.path()).await.as_deref() == Some(acp)
        {
          return;
        }
      }
    }
    hook(removed.to_owned(), acp.to_owned()).await;
  }

  /// Soft deletion into trash/, surviving a crash (sweep_trash cleans up on the next start). An error means the record
  /// was not moved: whatever this call had moved is back in place, unless the error says otherwise. Nothing here
  /// (an id already trashed or never stored) and an illegal id are no-ops; a trashed copy is never replaced
  pub async fn trash(&self, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    self.state.lock().pending.remove(id);
    self.settle_inflight(id).await;
    self.forget_saved(id);
    let trash = self.dir.join(TRASH_DIR);
    fs::create_dir_all(&trash).await?;
    let file = format!("{id}.json");
    let now = SystemTime::now();
    self.set_trashed(id, timestamp_ms(now)).await?;
    // The record leaves first: an entry left behind by a failed undo is then a stray blob folder, not a listed session.
    let moved = async {
      real_dir(&trash).await?;
      move_entries(&self.dir, &trash, [file.as_str(), id]).await
    }
    .await;
    let moved = match moved {
      Ok(moved) => moved,
      Err(error) => {
        let _ = self.set_trashed(id, None).await;
        return Err(anyhow!("session {id}: not moved to the trash ({error:#})"));
      }
    };
    // rename keeps the mtime; stamp the moment it was trashed (a regular file only, so a symlink's target is not touched)
    let stamped = trash.join(&file);
    if moved.contains(&file.as_str())
      && fs::symlink_metadata(&stamped).await.is_ok_and(|m| m.file_type().is_file())
      && let Ok(f) = std::fs::File::options().write(true).open(&stamped)
    {
      let _ = f.set_modified(now);
    }
    if self.on_change.read().is_some() {
      let cwd = self.cwd_of_session(id, &stamped).await;
      self.changed(id, RecordChange::Gone { cwd: &cwd });
    }
    Ok(())
  }

  /// Undo `trash`. An error means the record is still in the trash: whatever this call had moved is back there, unless
  /// the error says otherwise. Nothing in the trash and an illegal id are no-ops; a record in the live directory (written
  /// again since) is never replaced
  pub async fn restore(&self, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let trash = self.dir.join(TRASH_DIR);
    let file = format!("{id}.json");
    // A stubless body leaves the trash in the catalogue first, so a sweep in another engine meanwhile finds it listed and
    // keeps it; one already gone (swept) has nothing to restore
    let mut untrashed = None;
    if let Some(entry) = self.stubless_entry(id).await
      && let Some(at) = entry.trashed_at
      && !entry_exists(&trash.join(&file)).await.unwrap_or(true)
    {
      let key = id.to_owned();
      if self.db_call(move |db| db.untrash(&key)).await?.unwrap_or(false) {
        untrashed = Some(at);
      } else if !self.listed_body(id).await {
        return Ok(());
      }
    }
    // The record arrives last: an entry left behind by a failed undo is then a stray blob folder, not a listed session
    let moved = async {
      if real_dir(&trash).await? {
        move_entries(&trash, &self.dir, [id, file.as_str()]).await?;
      }
      anyhow::Ok(())
    }
    .await;
    if let Err(e) = moved {
      if untrashed.is_some() {
        let _ = self.set_trashed(id, untrashed).await;
      }
      return Err(anyhow!("session {id}: not restored from the trash ({e:#})"));
    }
    self.set_trashed(id, None).await?;
    if self.on_change.read().is_some() {
      let cwd = self.cwd_of_session(id, &self.dir.join(&file)).await;
      self.changed(id, RecordChange::Restored { cwd: &cwd });
    }
    Ok(())
  }

  /// Remove what was trashed more than `older_than` ago
  pub async fn sweep_trash(&self, older_than: Duration) {
    let trash = self.dir.join(TRASH_DIR);
    let cutoff = SystemTime::now() - older_than;
    if self.stubless() {
      self.sweep_trashed_bodies(cutoff).await;
    }
    let Ok(mut rd) = fs::read_dir(&trash).await else { return };
    while let Ok(Some(e)) = rd.next_entry().await {
      let f = e.file_name().to_string_lossy().into_owned();
      let Some(id) = f.strip_suffix(".json") else { continue };
      if !is_session_id(id) {
        continue;
      }
      let mtime = e.metadata().await.ok().and_then(|m| m.modified().ok()).unwrap_or(SystemTime::UNIX_EPOCH);
      if mtime > cutoff {
        continue;
      }
      let log = builtin_session_of(&e.path()).await;
      rm_confined(&trash, &f, false).await;
      rm_confined(&trash, id, true).await;
      // Still in the trash (not removable now), or back in the list (another engine's undo since the listing above): the
      // body stays with its record
      if entry_exists(&e.path()).await.unwrap_or(true) || entry_exists(&self.dir.join(&f)).await.unwrap_or(true) {
        continue;
      }
      let db_id = id.to_owned();
      // A stubless store has no file to show an undo and first save since the listing: the row goes only while trashed
      let removed = if self.stubless() {
        self.db_call(move |db| db.delete_trashed(&db_id, i64::MAX).map(|_| ())).await
      } else {
        self.db_call(move |db| db.delete_session(&db_id)).await
      };
      if let Err(error) = removed {
        (self.log)(&format!("session {id}: SQLite row not removed ({error:#})"));
      }
      if let Some(acp) = log {
        self.remove_agent_log(id, &acp).await;
      }
    }
  }

  /// The trash of a stubless store: bodies the catalogue has had in the trash since `cutoff`, and their blob folders. The
  /// row goes first and only while it is still trashed, so an undo in another engine mid-sweep keeps the session whole
  async fn sweep_trashed_bodies(&self, cutoff: SystemTime) {
    let Some(cutoff) = timestamp_ms(cutoff) else { return };
    let Ok(Some(entries)) = self.db_call(SessionDb::catalog).await else { return };
    let trash = self.dir.join(TRASH_DIR);
    for entry in entries {
      let id = entry.summary.id;
      if entry.body != BodyLocation::Database || entry.trashed_at.is_none_or(|at| at > cutoff) || !is_session_id(&id) {
        continue;
      }
      // A record file beside it (an older engine's, a revert) goes with the file sweep, which takes the row along
      let file = format!("{id}.json");
      if entry_exists(&trash.join(&file)).await.unwrap_or(true) || entry_exists(&self.dir.join(&file)).await.unwrap_or(true) {
        continue;
      }
      let log = self.db_head(&id).await.and_then(|raw| builtin_session_in(&raw));
      self.forget_saved(&id);
      let key = id.clone();
      match self.db_call(move |db| db.delete_trashed(&key, cutoff)).await {
        Ok(Some(true)) => {}
        Ok(_) => continue,
        Err(error) => {
          (self.log)(&format!("session {id}: SQLite row not removed ({error:#})"));
          continue;
        }
      }
      rm_confined(&trash, &id, true).await;
      if let Some(acp) = log {
        self.remove_agent_log(&id, &acp).await;
      }
    }
  }
}

/// Whether an entry is at `path` itself; a symlink there counts, whatever it points at
async fn entry_exists(path: &Path) -> Result<bool> {
  match fs::symlink_metadata(path).await {
    Ok(_) => Ok(true),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
    Err(e) => Err(anyhow!("{}: {e}", path.display())),
  }
}

/// Whether `dir` is there as a real directory. Anything else in its place is an error, a symlink above all: the trash
/// behind one could carry records out of the store or pull outside entries in
async fn real_dir(dir: &Path) -> Result<bool> {
  match fs::symlink_metadata(dir).await {
    Ok(m) if m.file_type().is_dir() => Ok(true),
    Ok(_) => Err(anyhow!("{} is not a directory", dir.display())),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
    Err(e) => Err(anyhow!("{}: {e}", dir.display())),
  }
}

/// Moves a session's entries (`<id>.json`, `<id>/`) between two store directories in the given order and returns the
/// ones it moved. Names are session ids, so each entry is a direct child, and nothing is followed: a symlink moves as the
/// link, its target untouched. An absent entry is skipped (blobs are optional, a repeated call finds nothing); one
/// already at the destination is never replaced. On a failure the entries this call moved go back before the error is
/// returned; any that cannot are named in it
async fn move_entries<'a>(from: &Path, to: &Path, names: [&'a str; 2]) -> Result<Vec<&'a str>> {
  let mut moved: Vec<&str> = vec![];
  for name in names {
    let (src, dst) = (from.join(name), to.join(name));
    let step = async {
      if !entry_exists(&src).await? {
        return Ok(false);
      }
      if entry_exists(&dst).await? {
        return Err(anyhow!("{} already exists", dst.display()));
      }
      fs::rename(&src, &dst).await.map_err(|e| anyhow!("moving {} to {}: {e}", src.display(), dst.display()))?;
      anyhow::Ok(true)
    };
    match step.await {
      Ok(true) => moved.push(name),
      Ok(false) => {}
      Err(e) => return Err(undo_moves(from, to, &moved, e).await),
    }
  }
  Ok(moved)
}

/// Puts back what a failed `move_entries` moved, newest first, under the same rule: a place taken again meanwhile is
/// not replaced. Whatever stays out of place is added to the error
async fn undo_moves(from: &Path, to: &Path, moved: &[&str], error: anyhow::Error) -> anyhow::Error {
  let mut stuck = vec![];
  for name in moved.iter().rev() {
    let (src, dst) = (from.join(name), to.join(name));
    let back = match entry_exists(&src).await {
      Ok(false) => fs::rename(&dst, &src).await.map_err(|e| anyhow!("{e}")),
      Ok(true) => Err(anyhow!("{} was taken meanwhile", src.display())),
      Err(e) => Err(e),
    };
    if let Err(e) = back {
      stuck.push(format!("{} left at {} ({e:#})", src.display(), dst.display()));
    }
  }
  if stuck.is_empty() { error } else { anyhow!("{error:#}; not put back: {}", stuck.join("; ")) }
}
