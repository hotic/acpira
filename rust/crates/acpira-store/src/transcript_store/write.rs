//! The save path: debounced writes per session, serialized in each record's chain, to a whole file or to database rows plus a compatibility stub

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Result, anyhow};
use tokio::fs;

use acpira_shared::transcript::{collect_text_blobs, SessionSummary};

use super::{
  ARCHIVE_LEVEL, FileStamp, Pending, RecordChange, STUB_REFRESH, StubState, TranscriptStore, confined,
  file_modified_ns, file_stamp, is_session_id, rm_confined,
};
use crate::blob_pool;
use crate::db::{BodyLocation, Saved, SessionDb, migration};
use crate::diff_texts;
use crate::pack;
use crate::file_lock::write_atomic;
use crate::record::{RecordParts, RecordSource, SessionRecord};

impl TranscriptStore {
  /// A complete record file in place of whatever the session had, the catalogue following: the rows a database body
  /// had go once the file is there
  pub(super) async fn write_whole(&self, id: &str, bytes: &[u8], summary: SessionSummary) -> Result<()> {
    let path = confined(&self.dir, &format!("{id}.json")).await.ok_or_else(|| anyhow!("session {id}: outside the store"))?;
    write_atomic(&path, bytes, None).await?;
    // The archive of an earlier migration is older than this file: nothing may fall back to it from here on
    rm_confined(&self.dir.join(id), migration::MIGRATED_FILE, false).await;
    self.forget_saved(id);
    self.state.lock().plain.insert(id.to_owned());
    let stamp = file_stamp(&path).await.map(|(size, modified)| (size, file_modified_ns(modified)));
    let _ = self.db_call(move |db| db.mark_file(&summary, stamp)).await?;
    Ok(())
  }

  /// Debounced write; a continuous stream still lands within the max wait
  pub fn save(&self, source: Arc<dyn RecordSource>) {
    self.save_after(source, self.debounce)
  }

  pub fn save_after(&self, source: Arc<dyn RecordSource>, delay: Duration) {
    let id = source.record_id();
    if !is_session_id(&id) {
      (self.log)(&format!("session {id}: illegal id, not saved"));
      return;
    }
    let now = Instant::now();
    let mut st = self.state.lock();
    if let Some(p) = st.pending.get_mut(&id) {
      p.source = source;
      p.due = (now + delay).min(p.deadline.max(now));
      return;
    }
    let deadline = now + self.max_wait;
    st.pending.insert(id.clone(), Pending { source, due: now + delay.min(self.max_wait), deadline });
    drop(st);
    let Some(me) = self.me.upgrade() else { return };
    tokio::spawn(async move { me.debounce_loop(id).await });
  }

  // One timer task per pending id: sleeps until the (movable) due time, then writes
  async fn debounce_loop(self: Arc<Self>, id: String) {
    loop {
      let due = match self.state.lock().pending.get(&id) {
        Some(p) => p.due,
        None => return,
      };
      if due > Instant::now() {
        tokio::time::sleep_until(due.into()).await;
        continue;
      }
      // The write chain is held before the record leaves `pending`, so flush_pending always sees it in one place or the other
      let chain = self.chain(&id);
      let _g = chain.lock().await;
      let Some(p) = self.state.lock().pending.remove(&id) else { return };
      if let Err(e) = self.write_locked(p.source, false).await {
        self.report(&id, &e);
      }
      return;
    }
  }

  fn report(&self, id: &str, e: &anyhow::Error) {
    (self.log)(&format!("session {id}: save failed ({e})"));
    if let Some(h) = &self.on_save_error {
      h(id, &e.to_string());
    }
  }

  pub async fn flush(&self, source: Arc<dyn RecordSource>) -> Result<()> {
    let id = source.record_id();
    if !is_session_id(&id) {
      return Ok(());
    }
    self.state.lock().pending.remove(&id);
    // An explicit flush hands the row store every piece again: whatever an incremental save may have passed over
    // (`RecordSource::record_parts`) is settled here
    self.write(source, true).await
  }

  /// Whether this store has had the record on disk
  pub fn knew(&self, id: &str) -> bool {
    self.state.lock().known.contains(id)
  }

  /// `pack::pack` inside the record's write chain: a write of the same record waits for it, and it for the write, so
  /// the stub never lands on top of a record written meanwhile. A record with a write still pending is not cold
  pub async fn pack(&self, id: &str, now: SystemTime) -> Result<Result<u64, pack::Kept>> {
    if !is_session_id(id) || self.state.lock().pending.contains_key(id) {
      return Ok(Err(pack::Kept::NotCold));
    }
    let chain = self.chain(id);
    let _g = chain.lock().await;
    let (dir, id) = (self.dir.clone(), id.to_owned());
    Ok(tokio::task::spawn_blocking(move || pack::pack(&dir, &id, now)).await??)
  }

  pub(super) fn chain(&self, id: &str) -> Arc<tokio::sync::Mutex<()>> {
    self.inflight.lock().entry(id.to_owned()).or_default().clone()
  }

  pub(super) async fn settle_inflight(&self, id: &str) {
    let c = self.chain(id);
    drop(c.lock().await);
  }

  /// Creates the file or replaces it while it is still there; a record this store once had on disk that is gone now was
  /// deleted by another window, and writing it back would undo that
  async fn write(&self, source: Arc<dyn RecordSource>, full: bool) -> Result<()> {
    let chain = self.chain(&source.record_id());
    let _g = chain.lock().await;
    self.write_locked(source, full).await
  }

  /// `write` with the record's chain already held. `full` asks the source for every piece of the record; without it a
  /// database save may skip what the store is known to hold
  async fn write_locked(&self, source: Arc<dyn RecordSource>, full: bool) -> Result<()> {
    let id = source.record_id();
    if !is_session_id(&id) {
      return Ok(());
    }
    self.ensure().await?;
    let Some(path) = confined(&self.dir, &format!("{id}.json")).await else { return Ok(()) };
    // A copy this engine keeps of a session another engine is driving (two workspaces with it open) must not overwrite
    // what that engine writes; the guard is cloned out so no lock is held across the call
    let guard = self.write_guard.read().clone();
    if guard.is_some_and(|may_write| !may_write(&id)) {
      (self.log)(&format!("session {id}: another engine holds it, not written"));
      return Ok(());
    }
    let on_disk = file_stamp(&path).await;
    if self.knew(&id) && on_disk.is_none() && !self.listed_body(&id).await {
      (self.log)(&format!("session {id}: deleted by another window, not written back"));
      return Ok(());
    }
    match self.database().filter(|_| !Self::files_mode()) {
      Some(db) => self.write_rows(db, &id, &path, on_disk, &source, full).await?,
      None => self.write_file(&id, &path, on_disk.is_some(), &source).await?,
    }
    self.state.lock().known.insert(id.clone());
    if self.on_change.read().is_some() {
      self.changed(&id, RecordChange::Written { cwd: &source.record_cwd() });
    }
    Ok(())
  }

  /// Big diff texts go to the session's blob folder before the record that names them is written, so it never names a
  /// blob that is not there
  async fn store_texts(&self, id: &str, known: &Arc<parking_lot::Mutex<HashSet<String>>>, texts: Vec<(String, String)>) -> Result<()> {
    if texts.is_empty() {
      return Ok(());
    }
    fs::create_dir_all(self.dir.join(id)).await?;
    let dir = confined(&self.dir, id).await.ok_or_else(|| anyhow!("session {id}: blob folder outside the store"))?;
    let pool = self.dir.join(blob_pool::POOL_DIR);
    let texts = tokio::task::spawn_blocking(move || diff_texts::store(&pool, &dir, &texts).map(|_| texts)).await??;
    known.lock().extend(texts.into_iter().map(|(name, _)| name));
    self.stub_stale.lock().insert(id.to_owned());
    Ok(())
  }

  /// The whole record as one file: the backend of a store without a database, and of `ACPIRA_STORE=files`
  async fn write_file(&self, id: &str, path: &Path, exists: bool, source: &Arc<dyn RecordSource>) -> Result<()> {
    if self.state.lock().stale.contains(id) {
      return Err(anyhow!("session {id}: its transcript is kept in {}, which could not be opened; not written, so the newer transcript there is not replaced", crate::db::DB_FILE));
    }
    // A stub this store never loaded (a record handed in from elsewhere) has its body in the database too
    if exists && self.database().is_none() && !self.db.lock().quarantined && !self.state.lock().plain.contains(id) {
      if fs::read(path).await.is_ok_and(|raw| migration::is_stub(&raw)) {
        return Err(anyhow!("session {id}: its transcript is kept in {}, which could not be opened; not written", crate::db::DB_FILE));
      }
      self.state.lock().plain.insert(id.to_owned());
    }
    let known = self.texts.lock().entry(id.to_owned()).or_default().clone();
    let (bytes, texts) = collect_text_blobs(diff_texts::sink(known.clone(), self.dir.join(id)), || source.record_json());
    self.store_texts(id, &known, texts).await?;
    // `ACPIRA_STORE=files` with the database open: a body it holds moves back to its file with this write
    let key = id.to_owned();
    if self.db_call(move |db| db.body_location(&key)).await?.flatten() == Some(BodyLocation::Database) {
      let summary = serde_json::from_slice::<SessionRecord>(&bytes)?.summary();
      return self.write_whole(id, &bytes, summary).await;
    }
    write_atomic(path, &bytes, None).await
  }

  /// A database save: the pieces that changed go to their rows in one transaction, then the compatibility stub follows
  /// when it is due. The body is committed first, so a failure afterwards leaves a complete file or a stale stub, both
  /// of which the next save repairs
  async fn write_rows(&self, db: Arc<SessionDb>, id: &str, path: &Path, on_disk: Option<FileStamp>, source: &Arc<dyn RecordSource>, full: bool) -> Result<()> {
    let known = self.texts.lock().entry(id.to_owned()).or_default().clone();
    // Out of its slot while the save runs: a failure leaves the slot empty, and the next save reads the stored hashes again
    let mut cache = self.saves.lock().insert(id.to_owned(), None).flatten().unwrap_or_default();
    let mut full = full;
    let parts = loop {
      let skip = if full { HashMap::new() } else { cache.known() };
      let (parts, texts) = collect_text_blobs(diff_texts::sink(known.clone(), self.dir.join(id)), || source.record_parts(&skip, full));
      self.store_texts(id, &known, texts).await?;
      let (saving, key) = (db.clone(), id.to_owned());
      let (saved, parts, back) = tokio::task::spawn_blocking(move || {
        let saved = saving.save_parts(&key, &parts, &mut cache);
        (saved, parts, cache)
      })
      .await?;
      cache = back;
      match saved? {
        Saved::Done(_) => break parts,
        Saved::NeedsFull if !full => full = true,
        Saved::NeedsFull => return Err(anyhow!("session {id}: the row store asked for more than the whole record")),
      }
    };
    let head = cache.head();
    if let Some(slot) = self.saves.lock().get_mut(id) {
      *slot = Some(cache);
    }
    self.state.lock().stale.remove(id);
    let grown = self.stub_stale.lock().remove(id);
    if db.stubless() {
      // No compatibility file in this store: a whole record that was there (an older engine's, a revert, a copy) is in
      // the rows just committed
      if on_disk.is_some() {
        rm_confined(&self.dir, &format!("{id}.json"), false).await;
      }
      return Ok(());
    }
    let due = match self.stubs.lock().get(id) {
      None => true,
      Some(stub) => stub.stamp != on_disk || grown || (stub.head != head && (full || stub.at.elapsed() >= STUB_REFRESH)),
    };
    if due {
      self.write_stub(&db, id, path, on_disk.is_some(), &parts, head).await.inspect_err(|_| {
        // Whatever the stub had to tell is still owed
        self.stubs.lock().remove(id);
      })?;
    }
    Ok(())
  }

  /// The small record an older engine reads in place of a database body (`migration::stub`). One written over a whole
  /// record archives that record beside the blobs first: it is what a store without the database falls back to. An
  /// archive already there is from an earlier migration (the session went back to its file since) and is replaced
  async fn write_stub(&self, db: &Arc<SessionDb>, id: &str, path: &Path, exists: bool, parts: &RecordParts, head: Option<[u8; 16]>) -> Result<()> {
    let folder = self.dir.join(id);
    if exists && !self.stubs.lock().contains_key(id) {
      let raw = fs::read(path).await?;
      if !migration::is_stub(&raw) {
        fs::create_dir_all(&folder).await?;
        let archive = confined(&folder, migration::MIGRATED_FILE).await.ok_or_else(|| anyhow!("session {id}: migration archive outside the store"))?;
        let dir = self.dir.clone();
        // A cold-packed record is archived whole, like any other
        let packed = tokio::task::spawn_blocking(move || zstd::bulk::compress(&pack::expand_raw(&dir, raw), ARCHIVE_LEVEL)).await??;
        write_atomic(&archive, &packed, None).await?;
      }
    }
    let names = self.blob_names(id).await;
    write_atomic(path, &migration::stub(&parts.head, &names)?, None).await?;
    let stamp = file_stamp(path).await;
    self.stubs.lock().insert(id.to_owned(), StubState { head, at: Instant::now(), stamp });
    if let Some((size, modified)) = stamp {
      // The catalogue learns the stub's stamp, so the next reconcile does not read the record to see what changed
      let (db, key) = (db.clone(), id.to_owned());
      tokio::task::spawn_blocking(move || db.set_file_stamp(&key, size, file_modified_ns(modified))).await??;
    }
    Ok(())
  }

  /// Writes whatever is still debounced
  pub async fn dispose(&self) {
    self.flush_queued(true).await;
  }

  /// The debounced saves brought forward, as they would have been written
  pub(super) async fn flush_pending(&self) {
    self.flush_queued(false).await;
  }

  async fn flush_queued(&self, full: bool) {
    let queued: Vec<(String, Arc<dyn RecordSource>)> = self.state.lock().pending.drain().map(|(k, p)| (k, p.source)).collect();
    for (id, src) in queued {
      if let Err(e) = self.write(src, full).await {
        self.report(&id, &e);
      }
    }
    let chains: Vec<_> = self.inflight.lock().values().cloned().collect();
    for c in chains {
      drop(c.lock().await);
    }
  }
}
