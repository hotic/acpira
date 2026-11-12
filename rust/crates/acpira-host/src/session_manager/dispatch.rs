//! A page's message: the session it targets, the read-only answers about it (edits, diffs, plans), and the dispatch
//! of every action to the code that runs it

use std::sync::Arc;

use anyhow::{Result, anyhow};

use acpira_shared::protocol::{EditTurnRequest, HostMsg, WebviewMsg};
use acpira_shared::transcript::*;

use super::{SessionManager, Viewer, is_forwarded, lower_first, session_id_of};
use crate::acp::session::AcpSession;
use crate::acp::session::errors::error_text;
use crate::i18n::{t, tp};
use crate::store::transcript_store::{UnreadOp, is_session_id};

pub(super) fn is_execution(m: &WebviewMsg) -> bool {
  use WebviewMsg as W;
  matches!(
    m,
    W::Send { .. }
      | W::Stop { .. }
      | W::Permission { .. }
      | W::Answer { .. }
      | W::PluginCard { .. }
      | W::PluginMenu { .. }
      | W::BuildPlan { .. }
      | W::SetMode { .. }
      | W::SetConfig { .. }
      | W::SelectAccount { .. }
      | W::Compact { .. }
      | W::Retry { .. }
      | W::RetryTurn { .. }
      | W::Reconnect { .. }
      | W::TakeOverSession { .. }
      | W::Dequeue { .. }
      | W::SendQueued { .. }
      | W::SteerQueued { .. }
      | W::EditQueued { .. }
      | W::Login { .. }
      | W::StopAsyncTask { .. }
      | W::StopCommand { .. }
      | W::Goal { .. }
  )
}

/// List edits of one session: written by whichever engine owns it, so forwarded there like its actions
pub(super) fn list_edit_target(m: &WebviewMsg) -> Option<&str> {
  use WebviewMsg as W;
  match m {
    W::RenameSession { id, .. } | W::PinSession { id, .. } | W::ArchiveSession { id, .. } | W::SetSessionCategory { id, .. } => Some(id),
    _ => None,
  }
}

impl SessionManager {
  pub(super) fn current(&self, v: &Viewer) -> Option<Arc<AcpSession>> {
    v.active_id().and_then(|id| self.live(&id))
  }

  /// Session-targeted messages carry the id the webview was showing; only that session may take the action
  pub(super) fn target(&self, v: &Viewer, session_id: Option<&str>) -> Option<Arc<AcpSession>> {
    let Some(sid) = session_id else { return self.current(v) };
    let s = self.live(sid);
    if s.is_none() {
      self.log(&format!("action on session {} dropped: not live", sid.chars().take(8).collect::<String>()));
    }
    s
  }

  pub async fn edit_turn(self: &Arc<Self>, edit: EditTurnRequest) -> Result<()> {
    self.promote_copy(&edit.session_id).await?;
    if let Some(result) = self.edit_on_host(&edit).await {
      return result;
    }
    if let Some(s) = self.live(&edit.session_id) {
      return s.edit_turn(edit).await;
    }
    // Shown here while another engine owns it and pushes its view: the edit runs there, like a send or a stop
    let id = edit.session_id.clone();
    if self.state.lock().remote.get(&id).is_some_and(|r| r.parts.is_some()) {
      return self.forward(&id, &WebviewMsg::EditTurn { request_id: String::new(), edit }).await;
    }
    Err(anyhow!(t("history.unavailable")))
  }

  /// The whole files behind a diff a page was sent without them: the open session (or the read-only copy shown for it)
  /// holds them. A source the Storage tab slimmed away (`omitted` in the record) has no texts left: none is sent, so the page copies
  /// what it shows
  /// A source whose texts the record keeps as blobs (`store::diff_texts`) is read back from them
  pub async fn diff_source(&self, session_id: &str, tool_call_id: &str, nth: usize, subagent: Option<&str>) -> Option<DiffSource> {
    let src = self.live(session_id).or_else(|| self.mirror_of(session_id))?.diff_source(tool_call_id, nth, subagent)?;
    self.deps.store.resolve_diff(session_id, src).await.filter(|s| !s.omitted)
  }

  /// Whether a session open here (or shown as another engine's copy) shows the file at `path` (`AcpSession::shows_file`)
  pub fn shows_file(&self, session_id: &str, path: &str) -> bool {
    self.live(session_id).or_else(|| self.mirror_of(session_id)).is_some_and(|s| s.shows_file(path))
  }

  /// The session's context split (`acp::session::context`); a session not open here (another engine's copy, or one only
  /// on disk) is estimated from its transcript (`context_estimate.rs`)
  pub async fn session_context(&self, session_id: &str, detail: bool) -> Option<acpira_shared::context::ContextReport> {
    if let Some(s) = self.live(session_id).or_else(|| self.mirror_of(session_id)) {
      return s.context_report(detail).await;
    }
    let record = self.deps.store.load_detailed(session_id).await.ok()?;
    let known = record_known(self.registry(), &record).await;
    crate::acp::session::context_estimate::report_with(&record.turns, record.usage.as_ref(), detail, known.as_ref())
  }

  pub async fn session_context_item(&self, session_id: &str, rev: &str, id: &str) -> Option<acpira_shared::context::ContextContent> {
    if let Some(s) = self.live(session_id).or_else(|| self.mirror_of(session_id)) {
      return s.context_item(rev, id).await;
    }
    let record = self.deps.store.load_detailed(session_id).await.ok()?;
    let known = if id.starts_with("summary") || id.starts_with(crate::acp::session::context_estimate::DOC_PREFIX) { record_known(self.registry(), &record).await } else { None };
    if let Some(doc) = crate::acp::session::context_estimate::doc_content(known.as_ref(), id).await {
      return Some(doc);
    }
    crate::acp::session::context_estimate::content(&record.turns, record.usage.as_ref(), rev, id, known.as_ref())
  }

  pub async fn session_context_raw(&self, session_id: &str) -> Result<Option<acpira_shared::context::ContextRaw>, String> {
    match self.live(session_id) {
      Some(s) => s.context_raw().await,
      None => Ok(None),
    }
  }

  /// The files the session's own edits touched with their text before its first edit of each (`AcpSession::edit_bases`);
  /// empty for a session that is neither open here nor shown as another engine's copy
  pub async fn edit_bases(&self, session_id: &str) -> Vec<(String, Option<String>)> {
    let Some(s) = self.live(session_id).or_else(|| self.mirror_of(session_id)) else { return vec![] };
    let mut out = vec![];
    for (path, src) in s.edit_bases() {
      let base = match src {
        Some(src) => self.deps.store.resolve_diff(session_id, src).await.filter(|s| !s.omitted).map(|s| s.old_text),
        None => None,
      };
      out.push((path, base));
    }
    out
  }

  pub fn plan_document(&self, session_id: &str, plan_id: &str) -> Option<PlanDocumentBlock> {
    self.live(session_id)?.plan_document(plan_id)
  }

  pub async fn handle_for(self: &Arc<Self>, v: &Arc<Viewer>, m: WebviewMsg) {
    let kind = format!("{m:?}").split([' ', '{', '(']).next().unwrap_or("").to_owned();
    let delete_request = match &m {
      WebviewMsg::DeleteSession { request_id, .. } => request_id.clone(),
      _ => None,
    };
    let moves = matches!(
      m,
      WebviewMsg::SelectSession { .. }
        | WebviewMsg::DeselectSession { .. }
        | WebviewMsg::NewSession { .. }
        | WebviewMsg::DeleteSession { .. }
    );
    let done = match self.host_action(v, &m).await {
      Some(r) => r,
      None => self.dispatch(v, m).await,
    };
    // A viewer that moved off a host session lets its view there go
    if moves {
      self.prune_host_views();
    }
    if let Err(e) = done {
      let text = error_text(&e);
      self.log(&format!("handle {} failed: {text}", lower_first(&kind)));
      if let Some(request_id) = delete_request {
        v.emit(HostMsg::DeleteSessionResult { request_id, undo: false, error: Some(text.clone()) });
      }
      (self.deps.toast)("error", &text);
    }
  }

  pub(super) async fn dispatch(self: &Arc<Self>, v: &Arc<Viewer>, m: WebviewMsg) -> Result<()> {
    use WebviewMsg as W;
    let target_id = session_id_of(&m).or_else(|| v.active_id());
    // Picker changes on standby stay process-free and survive later disk updates.
    if let Some(id) = target_id.as_deref().filter(|id| self.state.lock().copies.contains(*id)) {
      let pick = match &m {
        W::SetMode { id, .. } => Some((None, id.as_str())),
        W::SetConfig { config_id, value, .. } => Some((Some(config_id.as_str()), value.as_str())),
        _ => None,
      };
      if let Some((config, value)) = pick {
        let _op = self.session_op(id).await;
        if let Some(s) = self.mirror_of(id) {
          s.standby_pick(config, value)?;
          let mut st = self.state.lock();
          let picks = st.copy_picks.entry(id.to_owned()).or_default();
          match config {
            Some(key) => { picks.config.insert(key.to_owned(), value.to_owned()); }
            None => picks.mode_id = Some(value.to_owned()),
          }
          drop(st);
          self.emit_session(&s);
          return Ok(());
        }
      }
    }
    if is_execution(&m) && !matches!(m, W::SetMode { .. } | W::SetConfig { .. } | W::Stop { .. })
      && let Some(id) = target_id.as_deref()
    {
      self.promote_copy(id).await?;
    }
    // Shown here read-only while another engine owns it and pushes its view: the action runs there
    if let Some(tid) = &target_id
      && is_forwarded(&m)
      && self.live(tid).is_none()
      && self.state.lock().remote.get(tid).is_some_and(|r| r.parts.is_some())
    {
      return self.forward(tid, &m).await;
    }
    // A list edit (rename, pin, file) of a session another engine runs: that engine writes the record and announces the new
    // entry. An owner that cannot take it (off the bus, older) leaves the edit to the refusal below
    if let Some(id) = list_edit_target(&m)
      && self.live(id).is_none()
      && !self.state.lock().copies.contains(id)
      && self.bus.connected()
      && (self.mirror_of(id).is_some() || self.leases.held_elsewhere(id))
      && self.forward(id, &m).await.is_ok()
    {
      return Ok(());
    }
    let valid = |id: &str| is_session_id(id);
    match m {
      W::Send { session_id, text, attachments, steer, client_id } => {
        if let Some(s) = self.target(v, session_id.as_deref())
          && s.claim_client_id(client_id.as_deref())
        {
          if steer == Some(true) {
            s.steer_prompt(text, attachments).await?;
          } else {
            s.prompt(text, attachments, false, None, None).await;
          }
        }
      }
      W::Stop { session_id, client_id } => {
        if let Some(s) = self.target(v, session_id.as_deref())
          && s.claim_client_id(client_id.as_deref())
        {
          s.cancel().await;
        }
      }
      W::Permission { session_id, block_id, option_id, client_id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
          && s.claim_client_id(client_id.as_deref())
        {
          s.resolve_permission(&block_id, &option_id);
        }
      }
      W::Answer { session_id, block_id, answers, images, skip, client_id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
          && s.claim_client_id(client_id.as_deref())
        {
          s.answer_questions_with(&block_id, &answers, images.as_ref(), skip == Some(true));
        }
      }
      W::PluginCard { session_id, card, op } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.plugin_card(&card, op).await;
        }
      }
      // A plugin's process may take its full time limit: off the message loop, answered to this page by id
      W::PluginCommand { id, session_id, name, arguments } => {
        let (s, v) = (valid(&session_id).then(|| self.live(&session_id)).flatten(), v.clone());
        tokio::spawn(async move {
          let outcome = match &s {
            Some(s) => s.run_plugin_command(&name, &arguments, true).await,
            None => Some(Err(t("plugin.command.noSession"))),
          };
          let (result, error) = match outcome {
            None => (None, Some(tp("plugin.command.gone", &[("name", name.as_str())]))),
            Some(Ok(text)) => (Some(serde_json::json!({ "text": text })), None),
            Some(Err(e)) => (None, Some(e)),
          };
          v.emit(HostMsg::PluginCalled { id, result, error });
        });
      }
      W::PluginMenu { session_id, name, item } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.plugin_menu_pick(&name, &item);
        }
      }
      W::BuildPlan { session_id, plan_id, option_id, model } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.build_plan(&plan_id, model.map(|x| (x.config_id, x.value)), option_id).await?;
        }
      }
      // Remembered only once the session actually shows the mode
      W::SetMode { session_id, id } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          s.select_mode(id.clone()).await?;
          if s.agent_controls().mode_id.as_deref() == Some(id.as_str()) {
            self.remember_mode(&s.agent, &id);
          }
        }
      }
      W::SetConfig { session_id, config_id, value } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          s.select_config(config_id, value).await?;
          self.remember(&s);
        }
      }
      W::SelectSession { id } => self.select_session_for(v, &id).await,
      // Only the session the view shows: a deselect that lost the race with the next select leaves that one alone
      W::DeselectSession { id } => {
        if v.active_id().as_deref() == Some(id.as_str()) {
          self.set_active(v, None);
        }
      }
      // A chat (project shells): a fresh folder of its own, unfiled
      W::NewSession { agent, chat: Some(true), .. } if self.projects_of(Some(v)) => {
        let dir = self.new_chat_folder().await?;
        self.new_session_for(v, agent, None, None, Some(dir)).await?
      }
      W::NewSession { agent, category, cwd, .. } => self.new_session_for(v, agent, None, category, cwd).await?,
      W::RenameSession { id, title } => self.rename_session(&id, &title).await?,
      W::DeleteSession { id, request_id } => {
        let undo = self.delete_record(&id).await?;
        if let Some(request_id) = request_id {
          v.emit(HostMsg::DeleteSessionResult { request_id, undo, error: None });
        }
        self.rehome(&id).await;
      }
      W::RestoreSession { id } => self.restore_session(&id).await?,
      W::PinSession { id, pinned } => self.pin_session(&id, pinned).await?,
      W::ArchiveSession { id, archived } => self.archive_session(&id, archived).await?,
      W::SetSessionUnread { id, unread } => self.set_unread(&id, unread),
      W::SetSessionCategory { id, category } => self.set_session_category(&id, category).await?,
      W::CategoryOp { op, file } => {
        self.category_op(&op).await?;
        if let (CategoryOp::Create { id, .. }, Some(session)) = (&op, file) {
          self.set_session_category(&session, Some(id.clone())).await?;
        }
      }
      // Unread marks are shared: cleared for every engine and window on the machine, and on the SSH hosts in remote run
      W::MarkAllRead => {
        {
          let mut st = self.state.lock();
          st.unread.clear();
          st.kept_unread.clear();
        }
        self.persist_unread(UnreadOp::Clear);
        self.mark_hosts_read();
        self.emit_sessions();
      }
      W::MoveSession { id, cwd, category } => self.move_session(Some(v), &id, cwd, category).await?,
      W::ForkSession { session_id, turn_index, agent } => self.fork_session(v, &session_id, turn_index, agent).await?,
      W::ObserveSubagent { session_id, subagent_id } => {
        {
          let mut vs = v.state.lock();
          vs.observing = Some((session_id.clone(), subagent_id.clone()));
          vs.last_subagent_rev = None;
        }
        if let Some(s) = self.live(&session_id) {
          // A restored Claude workflow agent without a process reads its sidechain log once (`workflow_logs.rs`)
          s.observe_subagent(&subagent_id);
        }
        // Run by another engine: its owner pushes the transcript (`bus_node_view`)
        self.sync_node_subs();
        let current = match self.live(&session_id) {
          Some(s) => s.subagent_transcript(&subagent_id),
          // What the owner pushed already. The read-only copy's only when the owner does not push (off the bus): restored
          // from the record, it shows running work as cancelled under the live transcript's rev, which the page would
          // then keep over the owner's
          None if self.state.lock().remote.get(&session_id).is_some_and(|r| r.parts.is_some()) => self.pushed_node(&session_id, &subagent_id),
          None => self.mirror_of(&session_id).and_then(|m| m.subagent_transcript(&subagent_id)),
        };
        if let Some((turns, rev, running)) = current {
          v.state.lock().last_subagent_rev = Some(rev);
          v.emit(HostMsg::Subagent { session_id, subagent_id, rev, running, turns });
        }
      }
      W::UnobserveSubagent { session_id, subagent_id } => {
        {
          let mut vs = v.state.lock();
          if vs.observing.as_ref() == Some(&(session_id, subagent_id)) {
            vs.observing = None;
            vs.last_subagent_rev = None;
          }
        }
        self.sync_node_subs();
      }
      W::CancelSubagent { session_id, subagent_id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.cancel_subagent(&subagent_id).await;
        }
      }
      W::StopAsyncTask { session_id, task_id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.stop_async_task(&task_id).await?;
        }
      }
      W::StopCommand { session_id, tool_call_id, subagent_id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.stop_command(&tool_call_id, subagent_id.as_deref()).await?;
        }
      }
      W::Goal { session_id, action, objective } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.control_goal(action, objective).await?;
        }
      }
      W::ImportNativeSession { agent, session_id, cwd, title, updated_at } => {
        self.import_native_session(v, &agent, &session_id, &cwd, title.as_deref(), updated_at.as_deref()).await
      }
      W::SelectAccount { session_id, id } => self.select_account(v, &id, session_id.as_deref()).await?,
      W::AddAccount { agent, via, .. } => self.add_account(v, &agent, via).await?,
      W::RemoveAccount { id, .. } => {
        if let Some(a) = &self.deps.accounts {
          a.remove(&id).await?;
        }
        self.pool.invalidate(None);
      }
      W::UnlockCredentials { agent, .. } => self.unlock_credentials(&agent).await?,
      W::RefreshQuota { agent, .. } => {
        let a = self.deps.accounts.clone();
        let l = self.deps.local_accounts.clone();
        let (ag1, ag2) = (agent.clone(), agent);
        tokio::join!(
          async move {
            if let Some(a) = a {
              // The account view is open: a login made in a terminal since shows up without a click
              a.sync_local(Some(&ag1)).await;
              a.refresh_quotas(Some(&ag1), false).await;
            }
          },
          async move {
            if let Some(l) = l {
              l.refresh(Some(&ag2), false).await;
            }
          }
        );
      }
      W::Compact { session_id } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          s.compact(false).await?;
        }
      }
      W::Retry { session_id } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          if let Some(a) = &self.deps.accounts {
            a.sync_local(Some(&s.agent)).await;
          }
          s.retry().await?;
        }
      }
      W::RetryTurn { session_id } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          s.retry_turn().await?;
        }
      }
      W::Reconnect { session_id } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          s.reconnect().await?;
        }
      }
      W::TakeOverSession { session_id } => {
        let id = session_id.clone().or_else(|| v.active_id());
        if let Some(id) = id.filter(|id| self.mirror_of(id).is_some_and(|m| m.can_take_over_elsewhere())) {
          self.take_over_elsewhere(&id).await?;
        } else if let Some(s) = self.target(v, session_id.as_deref()) {
          s.take_over().await?;
        }
      }
      W::Dequeue { session_id, id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.dequeue(&id);
        }
      }
      W::SendQueued { session_id, id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.send_queued(&id).await?;
        }
      }
      W::SteerQueued { session_id, id, client_id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
          && s.claim_client_id(client_id.as_deref())
        {
          s.steer_queued(&id).await?;
        }
      }
      W::EditQueued { session_id, id, text, retained_attachments, attachments } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.edit_queued(&id, text, retained_attachments, attachments).await?;
        }
      }
      W::Login { session_id, method_id } => {
        let s = self.target(v, session_id.as_deref());
        self.login(s, method_id.as_deref()).await?;
      }
      W::InstallAgent { agent } => self.install(&agent),
      W::CancelInstall { agent } => self.installs.cancel(&agent),
      W::InstallInTerminal { agent } => self.install_in_terminal(&agent),
      _ => {}
    }
    Ok(())
  }
}

/// A closed Claude session's split from Claude's own log (other agents have nothing to read without their process)
async fn record_known(
  registry: std::sync::Arc<crate::acp::agents::registry::AgentRegistry>,
  record: &crate::store::record::SessionRecord,
) -> Option<crate::acp::session::context_estimate::Known> {
  if record.agent.as_str() != "claude" {
    return None;
  }
  crate::acp::session::context::claude_known(registry, record.agent.clone(), record.cwd.clone(), record.acp_session_id.clone()?).await
}
