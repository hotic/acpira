//! Opening sessions: a new one for a viewer (or the empty one it already has), the ChatGPT mirror, selecting a
//! listed session and loading its record into a live agent

use std::sync::Arc;

use anyhow::{Result, anyhow};

use acpira_shared::chatgpt_integration::{ChatGptIntegrationStatus, ProjectMirrors};
use acpira_shared::protocol::{HostMsg, RawJson};
use acpira_shared::settings::in_workspace;
use acpira_shared::transcript::*;
use acpira_shared::turn_settings::capture_turn_settings;

use super::leases::LeaseHold;
use super::{SessionManager, Viewer};
use crate::acp::session::AcpSession;
use crate::acp::transport::rpc::BoxFuture;
use crate::external::chatgpt_events::CHATGPT_ID;
use crate::external::desktop_commander::desktop_commander_status;
use crate::i18n::{t, tp};
use crate::store::session_lease::Claim;
use crate::store::transcript_store::is_session_id;
use crate::util::{now_iso, random_uuid};

fn keep_empty(cur: &AcpSession, agent: &str, account: Option<&str>, cwd: &str) -> bool {
  if cur.agent != agent || cur.account_id().as_deref() != account || cur.cwd != cwd {
    return false;
  }
  if cur.turn_count() > 0 || cur.is_running() {
    return false;
  }
  matches!(cur.status(), SessionStatus::Starting | SessionStatus::Ready)
}

impl SessionManager {
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
      // A project shell's "new in this category" starts in the category's project
      let explicit = cwd.or_else(|| {
        let id = category.as_ref().filter(|_| (me.deps.projects)())?;
        me.state.lock().categories.categories.iter().find(|c| &c.id == id).map(|c| c.cwd.clone())
      });
      let explicit = match explicit {
        Some(dir) => Some(me.check_folder(&dir, "host.folderMissing").await?),
        None => None,
      };
      let Some(cwd) = me.folder_for(Some(&v), explicit) else {
        return Err(anyhow!(t("host.noProject")));
      };
      let id = agent.unwrap_or_else(|| me.default_agent());
      if id == CHATGPT_ID {
        let view = me.connect_chatgpt(None, "ChatGPT", &cwd).await?;
        me.drop_empty_current(&v).await;
        me.set_active(&v, Some(view.id.clone()));
        v.emit(HostMsg::Session { session: RawJson::new(&view), running: view.running, parts: None });
        return Ok(());
      }
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
      me.register_project(&cwd).await;
      // Inheritable settings are snapped before the session starts spawning
      let last = me.last_settings(&id);
      let s = AcpSession::fresh(&id, &cwd, me.session_deps(), acc);
      if category.is_some() {
        s.set_category(category);
      }
      s.preview_controls(&me.known_start_controls(&id).await, last.as_ref());
      if let Some(last) = &last {
        s.hold_settings(last);
      }
      me.state.lock().live.insert(s.id.clone(), s.clone());
      me.set_active(&v, Some(s.id.clone()));
      me.process_change(&s.id);
      s.emit("session.created", || serde_json::json!({ "how": "new" }));
      s.start().await;
      // A real session just read the current configOptions; the probe snapshot retires
      me.state.lock().probed.remove(&id);
      if let Some(last) = last {
        s.adopt_controls(last).await;
      }
      Ok(())
    })
  }

  /// `folder`: the project the view means (`folder_for`); a project shell without one counts no mirrors
  pub async fn chatgpt_status(&self, folder: Option<String>) -> ChatGptIntegrationStatus {
    if let Some(c) = &self.deps.chatgpt {
      c.refresh().await;
    }
    let raw_cwd = folder.unwrap_or_default();
    let cwd = tokio::fs::canonicalize(&raw_cwd).await.map(|p| p.to_string_lossy().into_owned()).unwrap_or(raw_cwd);
    let views: Vec<SessionView> = self
      .deps
      .chatgpt
      .as_ref()
      .map(|c| c.summaries().into_iter().filter(|s| in_workspace(&s.cwd, &cwd)).filter_map(|s| c.view(&s.id)).collect())
      .unwrap_or_default();
    let observed: Vec<&SessionView> = views.iter().filter(|v| !v.turns.is_empty()).collect();
    let latest = observed.first().copied().or(views.first());
    ChatGptIntegrationStatus {
      checked_at: now_iso(),
      bridge_available: self.deps.chatgpt.as_ref().is_some_and(|c| c.available()),
      desktop_commander: desktop_commander_status().await,
      project: ProjectMirrors {
        mirrors: views.len() as u64,
        observed_mirrors: observed.len() as u64,
        latest_session_id: latest.map(|v| v.id.clone()),
        last_event_at: observed.first().and_then(|v| v.external.as_ref()).map(|e| e.last_event_at.clone()),
      },
    }
  }

  pub async fn connect_chatgpt(&self, source_key: Option<String>, title: &str, cwd: &str) -> Result<SessionView> {
    let c = self.deps.chatgpt.as_ref().ok_or_else(|| anyhow!("ChatGPT bridge is unavailable in this host"))?;
    c.open(&source_key.unwrap_or_else(random_uuid), cwd, title).await
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
      if let Some(c) = me.deps.chatgpt.clone().filter(|c| c.owns(&id)) {
        c.refresh().await;
        let Some(view) = c.view(&id) else {
          let reason = t("host.recordMirrorMissingReason");
          (me.deps.toast)("error", &me.record_failure_message(&id, "SESSION_MIRROR_MISSING", &reason));
          return;
        };
        me.set_active(&v, Some(id.clone()));
        v.emit(HostMsg::Session { session: RawJson::new(&view), running: view.running, parts: None });
        me.emit_sessions();
        return;
      }
      // A host's own session (`hosts.rs`): its view comes from the host's engine, a page's pick or a restored one alike
      if me.open_on_host(&v, &id) {
        return;
      }
      if v.active_id().as_deref() == Some(id.as_str()) && me.live(&id).is_some() {
        return;
      }
      let previous = v.active_id();
      me.set_active(&v, Some(id.clone()));
      if let Some(live) = me.live(&id) {
        v.emit(live.view_msg());
        me.emit_sessions();
        return;
      }
      // Already mirrored for another viewer (or this one a moment ago): its poll keeps this one current too. The owner's
      // pushed view when there is one, as `emit_session` sends it
      if let Some(mirror) = me.mirror_of(&id) {
        v.emit(me.remote_msg(&id).unwrap_or_else(|| mirror.view_msg()));
        me.emit_sessions();
        return;
      }
      // A load already running for this record: wait for it instead of building a second session on the same id
      let pending = me.state.lock().loading.get(&id).cloned();
      if let Some(mut rx) = pending {
        let _ = rx.wait_for(|done| *done).await;
        if let Some(s) = me.live(&id).or_else(|| me.mirror_of(&id))
          && v.active_id().as_deref() == Some(id.as_str())
        {
          v.emit(s.view_msg());
          me.emit_sessions();
        } else if v.active_id().as_deref() == Some(id.as_str()) {
          me.set_active(&v, previous.filter(|old| old != &id));
        }
        return;
      }
      let (tx, rx) = tokio::sync::watch::channel(false);
      me.state.lock().loading.insert(id.clone(), rx);
      me.load_session(&v, &id, previous).await;
      me.state.lock().loading.remove(&id);
      let _ = tx.send(true);
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
    s.adopt_controls(settings).await;
    // A draining engine keeps the lease of every live agent: synced before the load's own hold goes
    self.sync_leases();
    drop(hold);
  }
}
