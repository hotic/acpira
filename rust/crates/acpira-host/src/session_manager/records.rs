//! List edits and record changes: rename, pin, category, move, soft deletion with its undo, and the blobs and folder a
//! session's record names

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::protocol::{HostMsg, RawJson};

use super::leases::LeaseHold;
use super::{SessionManager, TRASH_TTL, Trash};
use crate::acp::session::AcpSession;
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::{t, tp};
use crate::limits::RENAME_MAX;
use crate::store::record::{RecordSource, SessionRecord};
use crate::store::transcript_store::{is_session_id, sort_index};
use crate::util::clip;

impl SessionManager {
  /// Rename / pin: a live session mutates itself; one not loaded is patched on disk
  /// An event for the plugins' processes that no live session carries (the session may not be open here)
  fn plugin_event(&self, event: &str, data: impl FnOnce() -> Value) {
    if let Some(rt) = &self.deps.plugin_runtime {
      rt.emit(event, data);
    }
  }

  pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.rename(id, title).await;
    }
    let t = clip(title.trim(), RENAME_MAX);
    if t.is_empty() {
      return Ok(());
    }
    self.plugin_event("session.renamed", || serde_json::json!({ "session": id, "title": t }));
    if let Some(live) = self.live(id) {
      live.rename(&t);
      return self.save_now(live, hold).await;
    }
    self.patch_record(id, |r| {
      r.title = t;
      r.renamed = true;
    })
    .await
  }

  /// The transcript a session's owner pushed for one of its subagents (a session shown here read-only)
  pub(super) fn pushed_node(&self, session: &str, node: &str) -> Option<(RawJson, i64, bool)> {
    self.state.lock().remote_nodes.get(&(session.to_owned(), node.to_owned())).map(|(rev, running, turns)| (turns.clone(), *rev, *running))
  }

  pub async fn pin_session(&self, id: &str, pinned: bool) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.pin(id, pinned).await;
    }
    if let Some(live) = self.live(id) {
      live.set_pinned(pinned);
      return self.save_now(live, hold).await;
    }
    self.patch_record(id, |r| {
      r.pinned = pinned.then_some(true);
      if pinned {
        r.category = None;
      }
    })
    .await
  }

  /// Archive `id` (left out of the list, kept on disk) or bring it back; pin and category stay for its return. ChatGPT
  /// mirrors have no archive here
  pub async fn archive_session(&self, id: &str, archived: bool) -> Result<()> {
    if !is_session_id(id) || self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(id)) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if let Some(live) = self.live(id) {
      live.set_archived(archived);
      return self.save_now(live, hold).await;
    }
    self.patch_record(id, |r| r.archived = archived.then_some(true)).await
  }

  /// File `id` under `category` (or take it out). A category of another project, or one gone meanwhile, is ignored.
  /// ChatGPT mirrors have no categories
  pub async fn set_session_category(&self, id: &str, category: Option<String>) -> Result<()> {
    if !is_session_id(id) || self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(id)) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if let Some(live) = self.live(id) {
      let Some(category) = self.category_for(category, &live.cwd) else { return Ok(()) };
      live.set_category(category);
      return self.save_now(live, hold).await;
    }
    let categories = self.categories();
    self
      .patch_record(id, move |r| {
        let fits = category.as_ref().is_none_or(|c| categories.categories.iter().any(|x| &x.id == c && x.cwd == r.cwd));
        if fits {
          if category.is_some() {
            r.pinned = None;
          }
          r.category = category;
        }
      })
      .await
  }

  /// A live session's edit written through now, while the edit still holds the lease (not on the debounce). A failed
  /// write is reported, and the lease stays with the retrying save: the edit is in memory and must not land after
  /// another engine took the session
  async fn save_now(&self, live: Arc<AcpSession>, hold: LeaseHold<'_>) -> Result<()> {
    match self.deps.store.flush(live.clone() as Arc<dyn RecordSource>).await {
      Ok(()) => Ok(()),
      Err(e) => {
        self.log(&format!("session {}: save failed ({e})", live.id));
        self.release_after_save(live, hold.keep());
        Err(anyhow!(tp("host.saveFailed", &[("error", &e.to_string())])))
      }
    }
  }

  async fn patch_record(&self, id: &str, patch: impl FnOnce(&mut SessionRecord)) -> Result<()> {
    let Some(mut r) = self.deps.store.load(id).await else { return Ok(()) };
    patch(&mut r);
    if let Err(e) = self.deps.store.flush(Arc::new(r.clone())).await {
      self.log(&format!("session {id}: save failed ({e})"));
      return Err(anyhow!(tp("host.saveFailed", &[("error", &e.to_string())])));
    }
    self.replace_summary(&r);
    // Open nowhere, so no engine announces it: the others read the list again
    self.bus.touch("index");
    Ok(())
  }

  /// Soft deletion with a 30-second undo window; viewers showing it move on
  pub async fn delete_session(self: &Arc<Self>, id: &str) -> Result<()> {
    self.delete_record(id).await?;
    self.rehome(id).await;
    Ok(())
  }

  /// Commit the deletion before navigation can start another agent. True means this engine holds the undo; a record
  /// already deleted elsewhere has no undo here, even though its stale list entry also goes away.
  pub(super) async fn delete_record(self: &Arc<Self>, id: &str) -> Result<bool> {
    if !is_session_id(id) {
      return Ok(false);
    }
    // A repeated request waits for the same operation; it must neither confirm early nor delete an immediate undo again.
    let repeated = self.state.lock().deleting.contains_key(id);
    let op = self.session_op(id).await;
    let undo = self.state.lock().trash.contains_key(id);
    if repeated || undo {
      return Ok(undo);
    }
    if let Some(c) = self.deps.chatgpt.clone().filter(|c| c.owns(id)) {
      let _hold = self.hold_record(id).await?;
      c.delete(id).await?;
      self.plugin_event("session.deleted", || serde_json::json!({ "session": id }));
      return Ok(true);
    }
    let snapshot = self.sessions().into_iter().find(|s| s.id == id)
      .or_else(|| self.live(id).or_else(|| self.mirror_of(id)).map(|s| s.summary()));
    if let Some(summary) = snapshot {
      self.state.lock().deleting.insert(id.to_owned(), summary);
    }
    // Not every foreign copy has been opened here: check the record as well as the mirror before asking any owner to yield.
    if self.deps.store.load(id).await.is_some_and(|r| r.copy_of.is_some_and(|c| c.machine != crate::remote::host_store::this_machine().0)) {
      return Err(anyhow!(t("host.sessionBusyElsewhere")));
    }
    let hold = self.hold_deletion(id).await?;
    if let Some(live) = self.live(id) {
      live.cancel_for_handover().await;
      let until = tokio::time::Instant::now() + super::YIELD_SETTLE;
      while live.is_running() && tokio::time::Instant::now() < until {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
      }
      if live.is_running() {
        return Err(anyhow!(t("host.takeOverTimeout")));
      }
      if let Err(e) = self.deps.store.flush(live.clone() as Arc<dyn RecordSource>).await {
        self.release_after_save(live, hold.keep());
        return Err(anyhow!(tp("host.saveFailed", &[("error", &e.to_string())])));
      }
    }
    self.forget(id);
    self.sync_leases();
    let closing = self.state.lock().closing.get(id).cloned();
    if let Some(mut rx) = closing {
      let _ = rx.wait_for(|done| *done).await;
    }
    // Read only after taking the lease: this is the former owner's last record. A second deleting engine must not
    // arm a timer from its stale list entry when the first one has already moved the record to the trash.
    let sum = match self.deps.store.load_detailed(id).await {
      Ok(record) => {
        if record.copy_of.as_ref().is_some_and(|c| c.machine != crate::remote::host_store::this_machine().0) {
          return Err(anyhow!(t("host.sessionBusyElsewhere")));
        }
        Some(record.summary())
      }
      Err(error) if error.is_missing() => None,
      Err(error) => return Err(anyhow!(error.detail())),
    };
    self.deps.store.trash(id).await?;
    let undo = sum.is_some();
    if let Some(summary) = sum {
      let weak = self.me.clone();
      let tid = id.to_owned();
      let timer = tokio::spawn(async move {
        tokio::time::sleep(TRASH_TTL).await;
        let Some(m) = weak.upgrade() else { return };
        let _op = m.session_op(&tid).await;
        let gone = m.state.lock().trash.remove(&tid);
        if let Some(t) = gone {
          m.deps.store.remove(&tid).await;
          m.remove_chat_folder(&t.summary.cwd, false).await;
        }
      });
      self.state.lock().trash.insert(id.to_owned(), Trash { summary, timer: timer.abort_handle() });
    }
    {
      let mut st = self.state.lock();
      st.index.retain(|s| s.id != id);
      st.deleting.remove(id);
      for list in st.elsewhere.values_mut() {
        list.retain(|s| s.id != id);
      }
    }
    self.plugin_event("session.deleted", || serde_json::json!({ "session": id }));
    // Read by nobody any more; the other engines drop it from their lists now rather than on their next refresh
    self.mark_read(id);
    self.bus.touch("index");
    self.save_index();
    drop(hold);
    self.sync_leases();
    self.emit_sessions();
    // Rehoming can load a different record: release this operation before the caller confirms and navigates away.
    drop(op);
    Ok(undo)
  }

  /// Close a live session and drop every trace of it in memory; out of the live map first, so its dispose callback
  /// cannot write the record back
  pub(super) fn forget(&self, id: &str) {
    // The unread mark stays: it is the user's, shared by every engine, and outlives this engine's copy (a session handed to
    // another engine is just as unread there)
    // Keep the native-session lock until session/close and process exit, just like idle release. Undo can immediately
    // reopen the record in this or another engine, so releasing the lease at forget() would be too early.
    let close_pin = self.live(id).and_then(|_| match self.leases.claim(id) {
      crate::store::session_lease::Claim::Held(pin) => Some(pin),
      _ => None,
    });
    let (closed_tx, closed_rx) = tokio::sync::watch::channel(false);
    let live = {
      let mut st = self.state.lock();
      st.was_running.remove(id);
      st.node_watch.retain(|(sid, _), _| sid != id);
      st.was_busy.remove(id);
      st.was_waiting.remove(id);
      st.settle_seq.remove(id);
      st.health_seen.remove(id);
      st.dirty.remove(id);
      st.mirrors.remove(id);
      st.idle_since.remove(id);
      let live = st.live.remove(id);
      if live.is_some() {
        st.closing.insert(id.to_owned(), closed_rx);
      }
      live
    };
    if let Some(s) = live {
      if let Some(pin) = s.drop_lease() {
        self.leases.release(id, pin);
      }
      let (_, closing) = s.shutdown();
      let weak = self.me.clone();
      let id = id.to_owned();
      tokio::spawn(async move {
        if let Some(f) = closing {
          f.await;
        }
        if let Some(m) = weak.upgrade() {
          if let Some(pin) = close_pin {
            m.leases.release(&id, pin);
          }
          m.state.lock().closing.remove(&id);
        }
        let _ = closed_tx.send(true);
      });
    } else if let Some(pin) = close_pin {
      self.leases.release(id, pin);
    }
  }

  /// Viewers left on a session that is gone move to the newest one in scope, or a new session
  pub(super) fn rehome<'a>(self: &'a Arc<Self>, id: &'a str) -> BoxFuture<()> {
    let me = self.clone();
    let id = id.to_owned();
    Box::pin(async move {
      if me.state.lock().disposed {
        return;
      }
      for v in me.viewers_on(&id, None) {
        me.set_active(&v, None);
        match me.most_recent_except(Some(&id)) {
          Some(next) => me.select_session_for(&v, &next).await,
          // A project shell whose last session went and that has no project left shows its empty state
          None if me.folder_for(Some(&v), None).is_none() => v.emit(HostMsg::NoSession),
          None => {
            if let Err(e) = me.new_session_for(&v, None, None, None, None).await {
              me.log(&format!("new session failed: {e}"));
            }
          }
        }
      }
    })
  }

  /// Re-home a session into another project folder: `to` (a project the list shows), or this window's workspace folder
  /// without one. `category` files it under one of the destination project's categories in the same step; any other id
  /// leaves it unfiled there
  pub async fn move_session(self: &Arc<Self>, id: &str, to: Option<String>, category: Option<String>) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(id)) {
      return Err(anyhow!(t("chatgpt.projectBound")));
    }
    let cwd = match to {
      // The agent is spawned in the folder from the next open on, so a project whose folder is gone cannot take it
      Some(dir) => self.check_folder(&dir, "host.moveFolderMissing").await?,
      None => self.folder_for(None, None).ok_or_else(|| anyhow!(t("host.noProject")))?,
    };
    // A category belongs to the project the session leaves; only one of the destination's survives the move
    let category = self.category_for(category, &cwd).flatten();
    if let Some(live) = self.live(id) {
      if live.cwd == cwd {
        // Already there: a category given is still a filing
        if category.is_none() {
          return Ok(());
        }
        live.set_category(category);
        return self.save_now(live, hold).await;
      }
      if live.is_running() {
        return Err(anyhow!(t("host.moveWhileRunning")));
      }
      let mut record = live.to_record();
      self.forget(id);
      record.cwd = cwd;
      // Filing unpins, as everywhere else
      if category.is_some() {
        record.pinned = None;
      }
      record.category = category;
      self.deps.store.flush(Arc::new(record.clone())).await.ok();
      self.replace_summary(&record);
      self.bus.touch("index");
      for v in self.viewers_on(id, None) {
        self.set_active(&v, None);
        self.select_session_for(&v, id).await;
      }
      return Ok(());
    }
    self
      .patch_record(id, |r| {
        if r.cwd == cwd && category.is_none() {
          return;
        }
        r.cwd = cwd;
        if category.is_some() {
          r.pinned = None;
        }
        r.category = category;
      })
      .await
  }

  /// The working directory of a session a viewer shows: live, a mirror of another engine's session, a ChatGPT mirror,
  /// else whatever the index recorded (a session not opened here yet)
  pub fn session_cwd(&self, id: &str) -> Option<String> {
    {
      let st = self.state.lock();
      if let Some(s) = st.live.get(id).or_else(|| st.mirrors.get(id)) {
        return Some(s.cwd.clone());
      }
    }
    if let Some(v) = self.deps.chatgpt.as_ref().and_then(|c| c.view(id)) {
      return Some(v.cwd);
    }
    let st = self.state.lock();
    st.index.iter().find(|s| s.id == id).map(|s| s.cwd.clone()).filter(|c| !c.is_empty())
  }

  pub fn blob_path(&self, session_id: &str, name: &str) -> Option<std::path::PathBuf> {
    self.deps.store.blob_path(session_id, name)
  }

  pub async fn save_blob(&self, session_id: &str, ext: &str, bytes: &[u8]) -> Option<String> {
    self.deps.store.save_blob(session_id, ext, bytes).await.ok().map(|(name, _)| name)
  }

  /// A record changed on disk without a live session: this host's copy wins for it
  fn replace_summary(&self, record: &SessionRecord) {
    {
      let mut st = self.state.lock();
      let sum = record.summary();
      match st.index.iter().position(|s| s.id == record.id) {
        Some(i) => st.index[i] = sum,
        None => st.index.push(sum),
      }
      sort_index(&mut st.index);
      st.touched.insert(record.id.clone());
    }
    self.save_index();
    self.emit_sessions();
  }

  /// Undo deletion: move it back out of the trash
  pub async fn restore_session(&self, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let _op = self.session_op(id).await;
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.restore(id).await;
    }
    if !self.state.lock().trash.contains_key(id) {
      return Ok(());
    }
    let _hold = self.hold_record(id).await?;
    // The expiry task takes the same session_op, so it cannot remove the undo while this move is in flight. On failure
    // the entry and timer remain intact for retry; a missing trash record must not resurrect only its list summary.
    self.deps.store.restore(id).await?;
    let restored = self.deps.store.load_detailed(id).await.map_err(|e| anyhow!(e.detail()))?;
    let Some(entry) = self.state.lock().trash.remove(id) else { return Ok(()) };
    entry.timer.abort();
    self.bus.touch("index");
    {
      let mut st = self.state.lock();
      st.index.push(restored.summary());
      sort_index(&mut st.index);
      st.touched.insert(id.to_owned());
    }
    self.save_index();
    self.emit_sessions();
    Ok(())
  }
}
