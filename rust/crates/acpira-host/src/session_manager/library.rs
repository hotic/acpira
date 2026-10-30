//! Sessions read from outside a session's own view: the session library agents call (`session_library.rs`),
//! conversation search, and the CLIs' native sessions listed and imported

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::protocol::SessionHit;
use acpira_shared::transcript::*;

use super::fork::holds_unseen_turns;
use super::{SessionManager, Viewer};
use crate::acp::agents::native_sessions::list_native_sessions;
use crate::acp::session::AcpSession;
use crate::i18n::{t, tp};
use crate::limits::RENAME_MAX;
use crate::relay::wire::{SessionReadArgs, SessionSearchArgs};
use crate::session_library::{
  LIST_DEFAULT, LIST_MAX, LibraryCaller, LibraryFuture, SELF_READ, SessionLibrary, agent_ids, candidates, matches, render_list,
  resolve_ref, session_entry,
};
use crate::store::record::{ImportedFrom, SessionRecord};
use crate::store::session_digest::{self, DigestMeta};
use crate::util::{clip, now_iso, random_uuid};

// Agents reading other sessions (`session_library.rs`): this side supplies the summaries, snapshots and conversation hits
impl SessionLibrary for SessionManager {
  fn search_sessions<'a>(&'a self, caller: &'a LibraryCaller, args: SessionSearchArgs) -> LibraryFuture<'a> {
    Box::pin(async move {
      let sessions = self.sessions();
      let query = args.query.as_deref().unwrap_or("").trim().to_owned();
      let limit = args.limit.unwrap_or(LIST_DEFAULT).clamp(1, LIST_MAX);
      // Inside one session: a fixed-string search over its record
      if let Some(r) = args.session.as_deref().filter(|r| !r.trim().is_empty()) {
        let id = resolve_ref(r, &sessions).ok_or_else(|| format!("No Acpira session is called {r:?}; search_sessions without `session` lists them."))?;
        if query.is_empty() {
          return Err("Give a query to search inside a session; read_session reads it.".into());
        }
        let snap = self.snapshot(&id).await.map_err(|e| e.to_string())?;
        let record = snap.into_record();
        if caller.is_self_session(&id) && !holds_unseen_turns(&record) {
          return Ok(SELF_READ.to_owned());
        }
        return Ok(session_digest::grep(&record, &query, args.limit.unwrap_or(session_digest::GREP_HITS)));
      }
      let all = args.scope.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("all"));
      let agents = self.agents();
      let filter = args.agent.as_deref().filter(|a| !a.trim().is_empty()).map(|a| agent_ids(a, &agents));
      let names: HashMap<String, String> = agents.iter().map(|a| (a.id.clone(), a.name.clone())).collect();
      let cands = candidates(&sessions, caller, all, filter.as_ref());
      let place = if all { "in all projects" } else { "in this project" };
      if query.is_empty() {
        let entries: Vec<String> = cands.iter().take(limit).map(|s| session_entry(s, None, &caller.cwd, &names)).collect();
        return Ok(render_list(&format!("Most recent sessions {place}:"), &entries, cands.len(), &format!("No sessions {place}.")));
      }
      // A query that is itself a reference (a pasted link, the agent's own session id) names that session
      if !query.contains(char::is_whitespace)
        && let Some(id) = resolve_ref(&query, &sessions)
        && let Some(s) = sessions.iter().find(|s| s.id == id && !caller.is_self_session(&s.id))
      {
        return Ok(render_list("That reference is this session:", &[session_entry(s, None, &caller.cwd, &names)], 1, ""));
      }
      let ids: HashSet<String> = cands.iter().map(|s| s.id.clone()).collect();
      let body = self.search.tool_hits(self.deps.store.dir().to_path_buf(), &query, Some(ids)).await;
      let found = matches(&cands, &query, &body);
      let entries: Vec<String> = found.iter().take(limit).map(|s| session_entry(s, body.get(&s.id), &caller.cwd, &names)).collect();
      let empty = if all { "No sessions match.".to_owned() } else { "No sessions in this project match; scope: \"all\" searches every project.".to_owned() };
      Ok(render_list(&format!("Sessions {place} matching {query:?}:"), &entries, found.len(), &empty))
    })
  }

  fn read_session<'a>(&'a self, caller: &'a LibraryCaller, args: SessionReadArgs) -> LibraryFuture<'a> {
    Box::pin(async move {
      let sessions = self.sessions();
      let r = args.session.trim();
      if r.is_empty() {
        return Err("Name the session: its link, its id, or the agent's own session id. search_sessions finds them.".into());
      }
      let id = resolve_ref(r, &sessions).ok_or_else(|| format!("No Acpira session is called {r:?}; search_sessions lists them."))?;
      let snap = self.snapshot(&id).await.map_err(|e| e.to_string())?;
      let agent_name = self.agent_name(snap.agent());
      let record = snap.into_record();
      if caller.is_self_session(&id) && !holds_unseen_turns(&record) {
        return Ok(SELF_READ.to_owned());
      }
      let fork_agent = record.forked_from.as_ref().and_then(|f| f.agent.as_ref()).map(|a| self.agent_name(a));
      let meta = DigestMeta { agent_name: &agent_name, live: self.live(&id).is_some_and(|s| s.is_running()), fork_agent: fork_agent.as_deref() };
      Ok(match (args.from, args.to) {
        (None, None) => session_digest::outline(&record, &meta),
        (from, to) => session_digest::range(&record, &meta, from.unwrap_or(1), to, args.offset.unwrap_or(0), args.detail.as_deref() == Some("full")),
      })
    })
  }
}

impl SessionManager {
  /// Which local record already holds a given native session id; summaries lacking the field are patched once
  async fn native_owners(&self, agent: &str) -> HashMap<String, String> {
    let mut local = HashMap::new();
    for s in self.live_sessions() {
      if s.agent == agent
        && let Some(acp) = s.acp_session_id()
      {
        local.insert(acp, s.id.clone());
      }
    }
    let index = self.state.lock().index.clone();
    let mut patched = false;
    for sum in index.iter().filter(|s| s.agent == agent) {
      let mut acp = sum.acp_session_id.clone();
      if acp.is_none()
        && let Some(rec) = self.deps.store.load(&sum.id).await
        && let Some(a) = rec.acp_session_id
      {
        let mut st = self.state.lock();
        if let Some(entry) = st.index.iter_mut().find(|x| x.id == sum.id) {
          entry.acp_session_id = Some(a.clone());
        }
        st.touched.insert(sum.id.clone());
        patched = true;
        acp = Some(a);
      }
      if let Some(a) = acp {
        local.insert(a, sum.id.clone());
      }
    }
    if patched {
      self.save_index();
    }
    local
  }

  /// Sessions whose saved conversation contains every term of `query`. Records reach the disk within the save debounce,
  /// so a reply still streaming is found a moment later; ChatGPT mirrors have no record here and match by title only
  pub async fn search_sessions(&self, query: &str) -> Vec<SessionHit> {
    self.search.search(self.deps.store.dir().to_path_buf(), query).await
  }

  /// `cwd`: the folder to list for (`folder_for`); nothing to list without one
  pub async fn list_native_sessions(&self, agent: &str, cwd: Option<String>) -> Result<Vec<NativeSessionInfo>> {
    // A folder on an SSH host has no native sessions here
    let Some(cwd) = cwd.filter(|c| !crate::workspace::is_remote(c)) else { return Ok(vec![]) };
    let registry = self.registry();
    let def = registry.get(agent)?.clone();
    let bin = registry
      .resolve_binary(agent)
      .await
      .ok_or_else(|| anyhow!(tp("host.notFound", &[("command", &def.command), ("agent", &def.name)])))?;
    let acc = self.default_account(agent);
    let env = match (&acc, &self.deps.accounts) {
      (Some(a), Some(m)) => m.spawn_env_for(agent, a).await,
      _ => None,
    };
    let listed = list_native_sessions(&def, &bin, &cwd, env.as_ref(), self.deps.log.clone(), None).await?;
    let local = self.native_owners(agent).await;
    let mut sessions: Vec<NativeSessionInfo> = listed
      .iter()
      .map(|s| {
        let sid = s.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
        NativeSessionInfo {
          local_id: local.get(&sid).cloned(),
          session_id: sid,
          cwd: s.get("cwd").and_then(Value::as_str).unwrap_or("").to_owned(),
          title: s.get("title").and_then(Value::as_str).map(str::to_owned),
          updated_at: s.get("updatedAt").and_then(Value::as_str).map(str::to_owned),
        }
      })
      .collect();
    sessions.sort_by(|a, b| b.updated_at.as_deref().unwrap_or("").cmp(a.updated_at.as_deref().unwrap_or("")));
    Ok(sessions)
  }

  /// A record whose transcript is filled by the session/load replay on first open
  pub async fn import_native_session(
    self: &Arc<Self>,
    v: &Arc<Viewer>,
    agent: &str,
    session_id: &str,
    cwd: &str,
    title: Option<&str>,
    updated_at: Option<&str>,
  ) {
    if let Some(existing) = self.native_owners(agent).await.get(session_id).cloned() {
      self.select_session_for(v, &existing).await;
      return;
    }
    let now = now_iso();
    let title = title.map(str::trim).filter(|x| !x.is_empty()).map(str::to_owned).unwrap_or_else(|| t("session.importedTitle"));
    let record = SessionRecord {
      id: random_uuid(),
      agent: agent.to_owned(),
      account_id: self.default_account(agent),
      acp_session_id: Some(session_id.to_owned()),
      cwd: cwd.to_owned(),
      title: clip(&title, RENAME_MAX),
      created_at: now.clone(),
      updated_at: updated_at.map(str::to_owned).unwrap_or(now),
      turns: vec![],
      controls: SessionControls::default(),
      usage: None,
      commands: vec![],
      pinned: None,
      archived: None,
      category: None,
      renamed: false,
      history_pending: false,
      forked_from: None,
      import_pending: true,
      imported_from: Some(ImportedFrom { session_id: session_id.to_owned() }),
      subagents: None,
      goal: None,
      copy_of: None,
    };
    if let Err(e) = self.deps.store.flush(Arc::new(record.clone())).await {
      self.log(&format!("import flush failed: {e}"));
    }
    self.drop_empty_current(v).await;
    let s = AcpSession::new(record, self.session_deps());
    self.state.lock().live.insert(s.id.clone(), s.clone());
    self.set_active(v, Some(s.id.clone()));
    self.process_change(&s.id);
    s.emit("session.created", || serde_json::json!({ "how": "import" }));
    s.start().await;
    // The agent restored the native context but replayed nothing: say what happened
    if s.status() == SessionStatus::Ready && s.turn_count() == 0 {
      (self.deps.toast)("info", &t("host.importNoHistory"));
    }
  }
}
