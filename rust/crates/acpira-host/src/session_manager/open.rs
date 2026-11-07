//! Opening sessions: a new one for a viewer (or the empty one it already has), selecting a listed session and
//! loading its record into a live agent

use std::sync::Arc;

use anyhow::{Result, anyhow};

use acpira_shared::transcript::*;
use acpira_shared::turn_settings::capture_turn_settings;

use super::leases::LeaseHold;
use super::{SessionManager, Viewer};
use crate::acp::session::AcpSession;
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::{t, tp};
use crate::store::session_lease::Claim;
use crate::store::transcript_store::is_session_id;

fn keep_empty(cur: &AcpSession, agent: &str, account: Option<&str>, cwd: &str) -> bool {
  if cur.agent != agent || cur.account_id().as_deref() != account || cur.cwd != cwd {
    return false;
  }
  if cur.turn_count() > 0 || cur.is_running() {
    return false;
  }
  matches!(cur.status(), SessionStatus::Starting | SessionStatus::Ready)
}

/// One record's lifecycle operation. Dropping it also wakes waiters after cancellation or an early error.
pub(super) struct SessionOp<'a> {
  manager: &'a SessionManager,
  id: &'a str,
  done: tokio::sync::watch::Sender<bool>,
}

impl Drop for SessionOp<'_> {
  fn drop(&mut self) {
    let failed_delete = {
      let mut st = self.manager.state.lock();
      st.loading.remove(self.id);
      // A failed handover or disk move must not make a bus-only entry look successfully deleted.
      let snapshot = st.deleting.remove(self.id);
      if let Some(summary) = &snapshot
        && !st.index.iter().any(|s| s.id == self.id)
      {
        st.index.push(summary.clone());
      }
      snapshot.is_some()
    };
    let _ = self.done.send(true);
    if failed_delete {
      self.manager.emit_sessions();
    }
  }
}

impl SessionManager {
  pub(super) async fn session_op<'a>(&'a self, id: &'a str) -> SessionOp<'a> {
    loop {
      let pending = {
        let mut st = self.state.lock();
        match st.loading.entry(id.to_owned()) {
          std::collections::hash_map::Entry::Occupied(entry) => entry.get().clone(),
          std::collections::hash_map::Entry::Vacant(entry) => {
            let (done, rx) = tokio::sync::watch::channel(false);
            entry.insert(rx);
            return SessionOp { manager: self, id, done };
          }
        }
      };
      let mut pending = pending;
      let _ = pending.wait_for(|done| *done).await;
    }
  }

  /// With no session (or a gone one) start a new one; otherwise bring its session live
  pub async fn ensure_active_for(self: &Arc<Self>, v: &Arc<Viewer>) {
    let active = v.active_id();
    if let Some(id) = &active
      && self.has_view(id)
    {
      return;
    }
    match active {
      Some(id) => self.select_session_for(v, &id).await,
      None if v.state.lock().empty => {}
      // A project shell with no project yet shows its empty state instead (`folder_for`)
      None if self.folder_for(Some(v), None).is_none() => {}
      None => {
        if let Err(e) = self.new_session_for(v, None, None, None, None).await {
          self.log(&format!("new session failed: {e}"));
        }
      }
    }
  }

  /// The folder a new session asked with `cwd` / `category` names, if any: a project shell's "new in this category"
  /// starts in the category's project
  pub(super) fn category_folder(&self, v: Option<&Viewer>, cwd: Option<String>, category: Option<&String>) -> Option<String> {
    cwd.or_else(|| {
      let id = category.filter(|_| self.projects_of(v))?;
      self.state.lock().categories.categories.iter().find(|c| &c.id == id).map(|c| c.cwd.clone())
    })
  }

  /// Where `new_session_for` would start a session asked with `cwd` / `category`, before checking the folder
  pub(super) fn new_session_folder(&self, v: &Viewer, cwd: Option<String>, category: Option<&String>) -> Option<String> {
    self.folder_for(Some(v), self.category_folder(Some(v), cwd, category))
  }

  pub fn new_session_for<'a>(
    self: &'a Arc<Self>,
    v: &'a Arc<Viewer>,
    agent: Option<String>,
    account: Option<String>,
    category: Option<String>,
    cwd: Option<String>,
  ) -> BoxFuture<Result<()>> {
    let me = self.clone();
    let v = v.clone();
    Box::pin(async move {
      let explicit = me.category_folder(Some(&v), cwd, category.as_ref());
      let explicit = match explicit {
        Some(dir) => Some(me.check_folder(&dir, "host.folderMissing").await?),
        None => None,
      };
      let Some(cwd) = me.folder_for(Some(&v), explicit) else {
        return Err(anyhow!(t("host.noProject")));
      };
      let id = agent.unwrap_or_else(|| me.default_agent());
      let acc = if me.deps.accounts.as_ref().is_some_and(|a| a.supports(&id)) { account.or_else(|| me.default_account(&id)) } else { None };
      // "New session in this category": filed on the session itself before anything awaits, so a switch made while it
      // spawns can never move the filing onto whichever session the viewer shows by then
      let category = me.category_for(category, &cwd).flatten();
      if let Some(cur) = me.current(&v)
        && keep_empty(&cur, &id, acc.as_deref(), &cwd)
      {
        if category.is_some() {
          cur.set_category(category);
        }
        return Ok(());
      }
      me.drop_empty_current(&v).await;
      // Starting over in the same chat (another agent) just took its folder along with the empty session it left
      if me.is_chat(&cwd) {
        tokio::fs::create_dir_all(&cwd).await?;
      }
      me.register_project(Some(&v), &cwd).await;
      // Inheritable settings are snapped before the session starts spawning
      let last = me.last_settings(&id);
      let s = AcpSession::fresh(&id, &cwd, me.session_deps(), acc);
      if category.is_some() {
        s.set_category(category);
      }
      s.preview_controls(&me.known_start_controls(&id).await, last.as_ref());
      // Held even with nothing remembered: a pick made while the session starts then waits for the replay instead of
      // being dropped
      let held = last.unwrap_or_default();
      s.hold_settings(&held);
      me.state.lock().live.insert(s.id.clone(), s.clone());
      me.set_active(&v, Some(s.id.clone()));
      me.process_change(&s.id);
      s.emit("session.created", || serde_json::json!({ "how": "new" }));
      s.start().await;
      // A real session just read the current configOptions; the probe snapshot retires
      me.state.lock().probed.remove(&id);
      let picked = s.adopt_controls(held).await;
      me.remember_picked(&s, &picked);
      Ok(())
    })
  }

  /// A viewer's session that hasn't said a word yet is replaced directly, never left as an empty entry
  pub(super) async fn drop_empty_current(&self, v: &Viewer) {
    let Some(cur) = self.current(v) else { return };
    if cur.turn_count() > 0 || cur.is_running() || !self.viewers_on(&cur.id, Some(v)).is_empty() {
      return;
    }
    self.forget(&cur.id);
    self.state.lock().index.retain(|x| x.id != cur.id);
    self.deps.store.remove(&cur.id).await;
    self.save_index();
    self.remove_chat_folder(&cur.cwd, true).await;
  }

  pub fn select_session_for<'a>(self: &'a Arc<Self>, v: &'a Arc<Viewer>, id: &'a str) -> BoxFuture<()> {
    let me = self.clone();
    let v = v.clone();
    let id = id.to_owned();
    Box::pin(async move {
      if !is_session_id(&id) {
        return;
      }
      // A host's own session (`hosts.rs`): its view comes from the host's engine, a page's pick or a restored one alike
      let previous = v.active_id();
      me.set_active(&v, Some(id.clone()));
      if me.open_on_host(&v, &id).await {
        return;
      }
      let _op = me.session_op(&id).await;
      // A delete may have rehomed this viewer while this pick waited. Never reopen its trashed record afterwards.
      if v.active_id().as_deref() != Some(id.as_str()) {
        return;
      }
      if me.state.lock().trash.contains_key(&id) {
        me.set_active(&v, previous.filter(|old| old != &id));
        return;
      }
      if let Some(live) = me.live(&id) {
        v.emit(live.view_msg());
        me.emit_sessions();
        return;
      }
      // Already mirrored for another viewer (or this one a moment ago): its poll keeps this one current too. The owner's
      // pushed view when there is one, as `emit_session` sends it
      if let Some(mirror) = me.mirror_of(&id) {
        // Still waiting for the owner's first push: that push, or the grace's end, reaches this viewer too
        if !me.awaiting_first_push(&id) {
          v.emit(me.remote_msg(&id).unwrap_or_else(|| mirror.view_msg()));
        }
        me.emit_sessions();
        return;
      }
      me.load_session(&v, &id, previous).await;
    })
  }

  async fn load_session(self: &Arc<Self>, v: &Arc<Viewer>, id: &str, previous: Option<String>) {
    // An idle release still closing this session's old agent process: a new one must not resume the native session
    // while the old one may still write to it
    let closing = self.state.lock().closing.get(id).cloned();
    if let Some(mut rx) = closing {
      let _ = rx.wait_for(|done| *done).await;
    }
    // Claimed before the record is read and held until the agent is up: another engine can neither start a turn on the
    // session between this look and the restore, nor finish one after the read and leave a stale running turn here
    let claim = self.leases.claim(id);
    // Not taken: the session is shown read-only (no agent, no saves) and opens once the lease can be had
    let refused = match &claim {
      Claim::Held(_) => None,
      Claim::Elsewhere => Some((t("host.sessionElsewhere"), true)),
      Claim::Failed(e) => {
        self.log(&format!("session {id}: lease failed: {e}"));
        Some((tp("host.sessionLeaseFailed", &[("error", e)]), false))
      }
    };
    let pin = match claim {
      Claim::Held(pin) => pin,
      _ => 0,
    };
    let hold = LeaseHold { leases: &self.leases, id, pin };
    let record = match self.deps.store.load_detailed(id).await {
      Ok(record) => record,
      Err(error) => {
        let missing = error.is_missing();
        self.notify_record_load_failure(id, &error);
        if missing {
          self.state.lock().index.retain(|s| s.id != id);
          self.emit_sessions();
          self.save_index();
          self.rehome(id).await;
        } else if v.active_id().as_deref() == Some(id) {
          self.set_active(v, previous.filter(|old| old != id));
        }
        return;
      }
    };
    if self.live(id).is_some() {
      return;
    }
    // A copy of a session another machine drives in this machine's folder: only that machine writes it
    if let Some(from) = record.copy_of.clone().filter(|c| c.machine != crate::remote::host_store::this_machine().0) {
      self.state.lock().copies.insert(id.to_owned());
      let name = if from.name.is_empty() { t("host.sessionCopyAnother") } else { from.name };
      self.start_mirror(id, record, tp("host.sessionCopy", &[("machine", &name)]), false);
      return;
    }
    // Another engine is mid-turn on this session (or holds it while draining): starting an agent here would race it
    if let Some((note, elsewhere)) = refused {
      // Startup's account scan is asynchronous. Learn this login's lock state before a mirror can request handover.
      if elsewhere
        && let Some(accounts) = &self.deps.accounts
      {
        accounts.check_locks(Some(&record.agent)).await;
      }
      self.start_mirror(id, record, note, elsewhere);
      return;
    }
    // The record's own controls are this session's choices. session/resume and session/load answer with the agent's
    // defaults (claude-agent-acp: Fable 5.1 Medium, Manual), and that answer must neither show nor reach the record:
    // held on screen through the restore, then replayed. Every reopen does this, not only one after an idle release in
    // this engine, since a restarted engine, a reloaded window or another engine's session has no such memory
    let settings = capture_turn_settings(&record.controls);
    let s = AcpSession::new(record, self.session_deps());
    s.hold_settings(&settings);
    self.state.lock().live.insert(id.to_owned(), s.clone());
    self.emit_session(&s);
    s.emit("session.opened", || serde_json::json!({}));
    s.start().await;
    // Also on a failed start: it releases the hold and the prompts it queued
    let picked = s.adopt_controls(settings).await;
    self.remember_picked(&s, &picked);
    // A draining engine keeps the lease of every live agent: synced before the load's own hold goes
    self.sync_leases();
    drop(hold);
  }
}
