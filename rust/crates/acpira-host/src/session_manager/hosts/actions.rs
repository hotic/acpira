//! Requests on host sessions from this engine's pages: lookups, searches and edits, page and list actions, and the host views a page opens

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::context::{ContextContent, ContextRaw, ContextReport};
use acpira_shared::protocol::{EditTurnRequest, HostMsg, SessionHit, WebviewMsg};
use acpira_shared::remote::parse_remote_folder;
use acpira_shared::transcript::SessionSummary;

use super::{HostControls, HostRun, LIST_VIEW, NEW_WAIT, OPEN_WAIT, SessionManager, Shown, local_folder};
use crate::session_manager::{Viewer, is_forwarded, session_id_of};
use crate::store::record::SessionRecord;

impl SessionManager {
  /// None means the session belongs here; otherwise the host's actual edit result is returned to the page.
  pub(crate) async fn edit_on_host(&self, edit: &EditTurnRequest) -> Option<Result<()>> {
    let run = self.host_of(&edit.session_id)?;
    Some(run.edit(edit.clone()).await)
  }

  /// A host session's context split, asked of the host engine that owns it; None when the session is not a host's
  pub(crate) async fn host_context(&self, id: &str, detail: bool) -> Option<Option<ContextReport>> {
    let run = self.host_of(id)?;
    Some(run.context(id, detail).await)
  }

  /// One leaf's content in a host session's context report; None when the session is not a host's
  pub(crate) async fn host_context_item(&self, id: &str, rev: &str, item_id: &str) -> Option<Option<ContextContent>> {
    let run = self.host_of(id)?;
    Some(run.context_item(id, rev, item_id).await)
  }

  /// A host session's raw request payload; None when the session is not a host's
  pub(crate) async fn host_context_raw(&self, id: &str) -> Option<Result<Option<ContextRaw>, String>> {
    let run = self.host_of(id)?;
    Some(run.context_raw(id).await)
  }

  fn host_of(&self, id: &str) -> Option<Arc<HostRun>> {
    if self.locally_owned(id) {
      return None;
    }
    self.hosts.runs.lock().values().find(|r| r.owns(id)).cloned()
  }

  /// Search only on demand; the owning host supplies summaries outside the project subscription.
  pub(crate) async fn search_hosts(&self, query: &str) -> Vec<SessionHit> {
    let runs: Vec<_> = self.hosts.runs.lock().values().cloned().collect();
    let mut tasks = tokio::task::JoinSet::new();
    for run in runs {
      let query = query.to_owned();
      tasks.spawn(async move { run.search(&query).await });
    }
    let mut hits = vec![];
    while let Some(Ok(found)) = tasks.join_next().await {
      hits.extend(found.into_iter().filter(|h| !self.locally_owned(&h.id)));
    }
    hits
  }

  async fn find_host(&self, id: &str) -> Option<Arc<HostRun>> {
    if !crate::store::transcript_store::is_session_id(id) || self.locally_owned(id) {
      return None;
    }
    // A record here that the index has not caught up with yet (another window's) is this machine's too
    if self.deps.store.has_record(id).await {
      return None;
    }
    let run = match self.host_of(id) {
      Some(run) => run,
      None => {
        let runs: Vec<_> = self.hosts.runs.lock().values().cloned().collect();
        let mut tasks = tokio::task::JoinSet::new();
        for run in runs {
          let id = id.to_owned();
          tasks.spawn(async move { run.lookup(&id).await.then_some(run) });
        }
        let mut found = None;
        while let Some(reply) = tasks.join_next().await {
          if let Ok(Some(run)) = reply {
            found = Some(run);
            break;
          }
        }
        let run = found?;
        // A local record may have arrived while the host was queried.
        if self.locally_owned(id) {
          return None;
        }
        run
      }
    };
    Some(run)
  }

  /// A host session's record fetched from its host (export, copy and read_session); None when the session is not a host's
  pub(crate) async fn host_record(&self, id: &str) -> Option<Result<SessionRecord>> {
    let run = self.find_host(id).await?;
    Some(run.record(id).await)
  }

  /// Whether a session is a host's own (not this machine's, though the host may hold a copy of it)
  pub(crate) fn is_host_session(&self, id: &str) -> bool {
    self.host_of(id).is_some()
  }

  /// The host sessions for the list; ids this engine has itself win
  pub(crate) fn host_summaries(&self, local: &[SessionSummary]) -> Vec<SessionSummary> {
    let runs: Vec<Arc<HostRun>> = self.hosts.runs.lock().values().cloned().collect();
    let mut out = vec![];
    for r in runs {
      out.extend(r.items.lock().iter().filter(|s| !local.iter().any(|l| l.id == s.id)).cloned());
    }
    out
  }

  /// A message to every connected host's list view (a list action that is not about one session, such as "mark all as read")
  pub(crate) fn hosts_send_list(&self, message: Value) {
    let runs: Vec<Arc<HostRun>> = self.hosts.runs.lock().values().cloned().collect();
    for run in runs {
      run.send_to_view(LIST_VIEW, message.clone());
    }
  }

  /// A shown host session's current push
  pub(crate) fn host_msg(&self, id: &str) -> Option<HostMsg> {
    let run = self.host_of(id)?;
    run.shown.lock().get(id).and_then(|s| s.msg.clone())
  }

  /// A host session whose view was asked of its host and has not come back yet
  pub fn awaits_host_view(&self, id: &str) -> bool {
    self.host_of(id).is_some_and(|run| run.shown.lock().get(id).is_some_and(|s| s.msg.is_none()))
  }

  pub(crate) fn host_cwd(&self, id: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Head { cwd: String }
    let run = self.host_of(id)?;
    let shown = run.shown.lock();
    let head = &shown.get(id)?.parts.as_ref()?.0.head;
    serde_json::from_str::<Head>(head.get()).ok().map(|h| h.cwd)
  }

  /// A page's action that belongs to a host session (or starts one in a host folder): sent there. None for the rest
  /// What a new session of `agent` in `cwd` offers when it would start in a host's engine (the same choice as
  /// `host_action`): that engine's answer to `controls`, read from the host's own CLI config, or its last one while the
  /// host is away. None when the session would start here, so the caller answers from this machine
  pub async fn host_controls(&self, agent: &str, cwd: &str, fresh: bool) -> Option<HostControls> {
    let folder = parse_remote_folder(cwd)?;
    if self.runs_here_on_ssh(agent, &folder.alias).await {
      return None;
    }
    // The built-in agent on a host that keeps this machine's settings (as the last push found, not merely offered): a
    // session this machine starts there reads them, so this machine answers
    if agent == crate::acp::agents::registry::SELF_AGENT_ID
      && self.deps.remote.as_ref().is_some_and(|c| c.models_on(&folder.alias) && c.settings_apart(&folder.alias) == Some(true))
    {
      return None;
    }
    let run = self.hosts.runs.lock().get(&folder.alias).cloned()?;
    let reply = if *run.up.borrow() {
      // Keyed by agent: a second ask for the same agent replaces the first, which falls back to the last answer
      let message = json!({ "type": "controls", "agent": agent, "fresh": fresh.then_some(true) });
      run
        .request(format!("controls-{agent}"), || {
          run.send_to_view(LIST_VIEW, message);
        })
        .await
    } else {
      None
    };
    let msg = match reply {
      Some(msg) => {
        run.controls.lock().insert(agent.to_owned(), msg.clone());
        msg
      }
      None => run.controls.lock().get(agent).cloned().unwrap_or(Value::Null),
    };
    let field = |k: &str| msg.get(k).cloned().unwrap_or(Value::Null);
    Some(HostControls {
      controls: serde_json::from_value(field("controls")).unwrap_or_default(),
      shapes: serde_json::from_value(field("shapes")).unwrap_or_default(),
      commands: serde_json::from_value(field("commands")).unwrap_or_default(),
    })
  }

  /// A new session of `agent` in a folder on the host `alias` runs on this machine: its definition supports local run
  /// (`AgentDef::local_run`), and the host says so for this agent (`Connections::local_agent`), else `sshLocalAgents`
  /// sets it explicitly, else it is an external CLI installed here (the built-in agent defaults to the host's engine)
  pub(crate) async fn runs_here_on_ssh(&self, agent: &str, alias: &str) -> bool {
    let reg = self.registry();
    if !reg.try_get(agent).is_some_and(|d| d.local_run.is_some()) {
      return false;
    }
    if let Some(on) = self.deps.remote.as_ref().and_then(|c| c.local_agent(alias, agent)) {
      return on;
    }
    let setting = (self.deps.ssh_local_agents)();
    if let Some(&on) = setting.get(agent) {
      return on;
    }
    if agent == crate::acp::agents::registry::SELF_AGENT_ID {
      return false;
    }
    match reg.available(agent) {
      Some(found) => found,
      None => reg.resolve_binary(agent).await.is_some(),
    }
  }

  pub(crate) async fn host_action(self: &Arc<Self>, v: &Arc<Viewer>, m: &WebviewMsg) -> Option<Result<()>> {
    use WebviewMsg as W;
    if self.hosts.runs.lock().is_empty() {
      return None;
    }
    // A new session in a host folder starts on the host, unless its agent runs here (`runs_here_on_ssh`,
    // `acp/session/remote.rs`)
    let asked_on = match m {
      // The folder resolves as `new_session_for` resolves it: a page may name only a category, or nothing (the
      // current session's folder), and either may be a host's
      W::NewSession { cwd, agent, category, chat } if *chat != Some(true) => {
        if let Some(f) = self.new_session_folder(v, cwd.clone(), category.as_ref()).as_deref().and_then(parse_remote_folder) {
          let agent = agent.clone().unwrap_or_else(|| self.default_agent());
          if self.runs_here_on_ssh(&agent, &f.alias).await {
            None
          } else {
            self.hosts.runs.lock().get(&f.alias).cloned().map(|r| (f, r))
          }
        } else {
          None
        }
      }
      _ => None,
    };
    if let (W::NewSession { agent, .. }, Some((f, run))) = (m, asked_on) {
      self.register_project(Some(v), &local_folder(&f.alias, &f.path)).await;
      // A host still being reached (the first connection deploys the helper) is waited for
      let mut up = run.up.subscribe();
      let _ = tokio::time::timeout(NEW_WAIT, up.wait_for(|u| *u)).await;
      // Keep the host's current viewer so its normal empty-session replacement runs. A shared view stays on the old
      // session for the other pages; this page then needs a separate view on the host.
      let current = v.active_id().filter(|id| self.viewers_on(id, Some(v)).is_empty());
      let reuse = current.and_then(|id| run.shown.lock().get(&id).map(|s| s.view_id.clone()));
      if let Some(view_id) = reuse {
        // Only this page follows the view to its new session; a page that opens the old one meanwhile stays there
        run.asked.lock().insert(view_id.clone(), Arc::downgrade(v));
        let sent = run.send_to_view(&view_id, json!({ "type": "newSession", "agent": agent, "cwd": f.path }));
        return Some(if sent { Ok(()) } else { Err(anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &f.alias)]))) });
      }
      let view_id = run.next_view_id();
      run.asked.lock().insert(view_id.clone(), Arc::downgrade(v));
      run.open_view(&view_id, json!({ "sessionList": { "folders": [] } }));
      // The host's categories are its own: a new session there starts unfiled
      let sent = run.send_to_view(&view_id, json!({ "type": "newSession", "agent": agent, "cwd": f.path }));
      // An empty session this page left here (an agent run here, then one run on the host) goes, as `new_session_for`
      // drops it when the next one starts here
      if sent {
        self.drop_empty_current(v).await;
      }
      return Some(if sent { Ok(()) } else { Err(anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &f.alias)]))) });
    }
    let explicit = session_id_of(m).or_else(|| list_target(m));
    let target = explicit.clone().or_else(|| v.active_id().filter(|_| is_forwarded(m)))?;
    let run = match self.host_of(&target) {
      Some(run) => run,
      None if list_target(m).is_some() && !v.has_list_scope() => self.find_host(&target).await?,
      None => return None,
    };
    let msg = serde_json::to_value(m).ok()?;
    let delete_request = match m {
      W::DeleteSession { request_id, .. } => request_id.as_ref(),
      _ => None,
    };
    if let Some(request_id) = delete_request {
      run.deletions.lock().insert(request_id.clone(), (target.clone(), Arc::downgrade(v)));
    }
    // Deletion must outlive its shown view: that view can be detached while the host closes and saves the session.
    let view_id = if matches!(m, W::DeleteSession { .. }) {
      LIST_VIEW.to_owned()
    } else {
      run.shown.lock().get(&target).map(|s| s.view_id.clone()).unwrap_or_else(|| LIST_VIEW.to_owned())
    };
    Some(if run.send_to_view(&view_id, msg) {
      Ok(())
    } else {
      if let Some(request_id) = delete_request {
        run.deletions.lock().remove(request_id);
      }
      Err(anyhow!(crate::i18n::tp("host.remoteUnreachable", &[("host", &run.alias)])))
    })
  }

  /// A viewer lands on a session: a host's own is shown through the host's engine. False when it is not a host's
  pub(crate) async fn open_on_host(self: &Arc<Self>, v: &Arc<Viewer>, id: &str) -> bool {
    if self.locally_owned(id) || v.has_list_scope() {
      return false;
    }
    let Some(run) = self.find_host(id).await else { return false };
    if v.active_id().as_deref() != Some(id) {
      return true;
    }
    self.set_active(v, Some(id.to_owned()));
    self.show_host_session(&run, id, v);
    self.emit_sessions();
    if !*run.up.borrow() {
      self.reach_for_view(run, id, v);
    }
    true
  }

  /// A host that is not connected brings no view, and the page would wait for one forever. The host is reached now, as
  /// opening a folder does (the background driver never retries a host the user disconnected, nor a failure only the
  /// user can settle), and a page still waiting when that fails is told why; once connected, the driver reopens the view
  fn reach_for_view(&self, run: Arc<HostRun>, id: &str, v: &Arc<Viewer>) {
    let Some(connections) = self.deps.remote.clone() else { return };
    let (id, viewer) = (id.to_owned(), Arc::downgrade(v));
    tokio::spawn(async move {
      let mut up = run.up.subscribe();
      let reached = tokio::time::timeout(OPEN_WAIT, async {
        connections.ensure(&run.alias, OPEN_WAIT).await?;
        // The helper channel is up: the driver links to the host's engine next
        let _ = up.wait_for(|u| *u).await;
        anyhow::Ok(())
      })
      .await;
      let detail = match reached {
        Ok(Ok(())) => return,
        Ok(Err(e)) => Some(format!("{e:#}")),
        Err(_) => None,
      };
      let Some(v) = viewer.upgrade() else { return };
      let waiting = v.active_id().as_deref() == Some(id.as_str()) && run.shown.lock().get(&id).is_some_and(|s| s.msg.is_none());
      if !waiting || *run.up.borrow() {
        return;
      }
      let unreachable = crate::i18n::tp("host.remoteUnreachable", &[("host", &run.alias)]);
      let reason = detail.map_or_else(|| unreachable.clone(), |d| format!("{unreachable}\n{d}"));
      v.emit(HostMsg::SessionUnavailable { id, reason });
    });
  }

  /// A viewer lands on a host session: its view there is opened once, and a view already pushed goes out at once
  pub(super) fn show_host_session(&self, run: &Arc<HostRun>, id: &str, v: &Viewer) {
    let current = {
      let mut shown = run.shown.lock();
      match shown.get(id) {
        Some(s) => {
          let (view_id, current) = (s.view_id.clone(), s.msg.clone());
          drop(shown);
          // A shared view can carry an explicitly kept unread mark. Landing on its cached transcript must still
          // select it on the owning host, where the mark lives.
          run.send_to_view(&view_id, json!({ "type": "selectSession", "id": id }));
          current
        }
        None => {
          let view_id = run.next_view_id();
          shown.insert(id.to_owned(), Shown::new(view_id.clone()));
          drop(shown);
          run.open_view(&view_id, json!({ "sessionList": { "folders": [] }, "session": id }));
          None
        }
      }
    };
    if let Some(msg) = current {
      v.emit(msg);
    }
  }

  /// Host views no viewer shows any more are closed there
  pub(crate) fn prune_host_views(&self) {
    let runs: Vec<Arc<HostRun>> = self.hosts.runs.lock().values().cloned().collect();
    if runs.is_empty() {
      return;
    }
    let active: Vec<String> = self.viewers().iter().filter_map(|v| v.active_id()).collect();
    for run in runs {
      let gone: Vec<String> = {
        let mut shown = run.shown.lock();
        let ids: Vec<String> = shown.keys().filter(|id| !active.contains(id)).cloned().collect();
        ids.into_iter().filter_map(|id| shown.remove(&id).map(|s| s.view_id)).collect()
      };
      for view_id in gone {
        run.send(json!({ "type": "detachView", "viewId": view_id }));
      }
    }
  }
}

/// List actions the host's engine runs on its own sessions. Categories and moves are this machine's list and folders,
/// so they are not among them
fn list_target(m: &WebviewMsg) -> Option<String> {
  use WebviewMsg as W;
  match m {
    W::RenameSession { id, .. }
    | W::DeleteSession { id, .. }
    | W::RestoreSession { id }
    | W::PinSession { id, .. }
    | W::ArchiveSession { id, .. }
    | W::SetSessionUnread { id, .. } => Some(id.clone()),
    _ => None,
  }
}
