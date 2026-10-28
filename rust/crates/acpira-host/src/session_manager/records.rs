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
    self.patch_record(id, |r| r.title = t).await
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
    if !is_session_id(id) {
      return Ok(());
    }
    let _hold = self.hold_record(id).await?;
    self.plugin_event("session.deleted", || serde_json::json!({ "session": id }));
    if let Some(c) = self.deps.chatgpt.clone().filter(|c| c.owns(id)) {
      c.delete(id).await?;
      self.rehome(id).await;
      return Ok(());
    }
    if let Some(live) = self.live(id) {
      self.deps.store.flush(live as Arc<dyn RecordSource>).await.ok();
    }
    self.forget(id);
    let sum = {
      let mut st = self.state.lock();
      let sum = st.index.iter().find(|s| s.id == id).cloned();
      st.index.retain(|s| s.id != id);
      sum
    };
    if let Some(summary) = sum {
      let weak = self.me.clone();
      let tid = id.to_owned();
      let timer = tokio::spawn(async move {
        tokio::time::sleep(TRASH_TTL).await;
        let Some(m) = weak.upgrade() else { return };
        let gone = m.state.lock().trash.remove(&tid);
        m.deps.store.remove(&tid).await;
        if let Some(t) = gone {
          m.remove_chat_folder(&t.summary.cwd, false).await;
        }
      });
      self.state.lock().trash.insert(id.to_owned(), Trash { summary, timer: timer.abort_handle() });
    }
    self.deps.store.trash(id).await?;
    // Read by nobody any more; the other engines drop it from their lists now rather than on their next refresh
    self.mark_read(id);
    self.bus.touch("index");
    self.save_index();
    self.rehome(id).await;
    self.emit_sessions();
    Ok(())
  }

  /// Close a live session and drop every trace of it in memory; out of the live map first, so its dispose callback
  /// cannot write the record back
  pub(super) fn forget(&self, id: &str) {
    // The unread mark stays: it is the user's, shared by every engine, and outlives this engine's copy (a session handed to
    // another engine is just as unread there)
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
      st.live.remove(id)
    };
    if let Some(s) = live {
      if let Some(pin) = s.drop_lease() {
        self.leases.release(id, pin);
      }
      s.dispose();
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
        match me.most_recent() {
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
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.restore(id).await;
    }
    let Some(entry) = self.state.lock().trash.remove(id) else { return Ok(()) };
    entry.timer.abort();
    self.deps.store.restore(id).await;
    self.bus.touch("index");
    {
      let mut st = self.state.lock();
      st.index.push(entry.summary);
      sort_index(&mut st.index);
      st.touched.insert(id.to_owned());
    }
    self.save_index();
    self.emit_sessions();
    Ok(())
  }
}
