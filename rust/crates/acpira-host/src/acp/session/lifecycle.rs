//! The session lifecycle: spawn or borrow the agent process, initialize, hand the account credential over, then resume /
//! load / create the native session; retry, rebind, reconnect and take over rebuild it, close ends it

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::inventory::AgentHealthStage;
use acpira_shared::transcript::*;

use crate::acp::agents::lock_holder;
use crate::acp::agents::model_sources::read_model_facts;
use crate::acp::session::errors::{AccountAuthError, RestoreFailure, classify_restore_error, error_text, is_auth};
use crate::acp::session::handlers::SessionHandlers;
use crate::acp::session::queue::{PeerTurn, PromptQueue};
use crate::acp::session::{AcpSession, Core, StartOutcome};
use crate::acp::transcript::normalize::{disconnect_async_tasks, runtime_info_of, seal_replay};
use crate::acp::transport::process::{AgentProcess, AgentSpawnError, ClientHandlers};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::{t, tp};
use crate::util::now_iso;

const CLOSE_GRACE: Duration = Duration::from_secs(3);

impl AcpSession {
  /// Params for session/new, resume and load (and an edit's fresh session): the shared MCP servers for this agent and
  /// cwd (`shared_config::mcp`, filtered by the process's `mcpCapabilities`) plus Acpira's own server (`host_mcp.rs`);
  /// Claude sessions also ask for summarized thinking (see `claude_thinking`)
  pub(crate) async fn session_request(&self, proc: &AgentProcess, acp_id: Option<&str>) -> Value {
    let mut servers = match &self.deps.shared_mcp {
      Some(provider) => {
        let caps = runtime_info_of(&proc.init).mcp;
        let (servers, line) = provider(self.agent.clone(), self.cwd.clone(), caps).await;
        if let Some(line) = line {
          self.log(&line);
        }
        servers
      }
      None => vec![],
    };
    servers.extend(self.deps.host_mcp.as_ref().and_then(|h| h.entry_for(&self.agent)));
    {
      let mut c = self.core.lock();
      if !c.mcp_skip.is_empty() {
        servers.retain(|s| !c.mcp_skip.iter().any(|n| s.get("name").and_then(Value::as_str) == Some(n)));
        self.log(&format!("MCP left out after failing to start: {}", c.mcp_skip.join(", ")));
      }
      c.mcp_sent = servers.iter().filter_map(|s| s.get("name").and_then(Value::as_str).map(str::to_owned)).collect();
    }
    let mut req = json!({ "cwd": self.cwd, "mcpServers": servers });
    if let Some(id) = acp_id {
      req["sessionId"] = json!(id);
    }
    if !self.vendor.summarized_thinking() {
      return req;
    }
    let def_env = self.def().env.and_then(|e| e.get("MAX_THINKING_TOKENS").cloned());
    let budget = def_env.or_else(|| std::env::var("MAX_THINKING_TOKENS").ok());
    crate::acp::vendors::claude_thinking::with_thinking(req, budget.as_deref())
  }

  /// Kill the current CLI; its exit / updates must not touch the session after this. session/close goes out first
  /// when the agent advertises it (close is not delete: it lets the agent flush and release its lock)
  pub(crate) fn drop_process(&self, c: &mut Core) -> Option<BoxFuture<()>> {
    let proc = c.proc.take()?;
    c.proc_gen += 1;
    c.perms.epoch += 1;
    c.peer = PeerTurn::default();
    self.disconnect_tasks(c);
    let session_id = c.acp_session_id.clone();
    let me = self.arc();
    Some(Box::pin(async move {
      let close = crate::json::truthy(proc.caps().get("sessionCapabilities").and_then(|s| s.get("close")));
      if let (Some(sid), true, true) = (session_id, proc.alive(), close) {
        match tokio::time::timeout(CLOSE_GRACE, proc.request("session/close", json!({ "sessionId": sid }))).await {
          Ok(Ok(_)) => {}
          Ok(Err(e)) => me.log(&format!("session/close before exit failed: {e}")),
          Err(_) => me.log("session/close before exit failed: session/close timed out"),
        }
      }
      proc.kill().await;
    }))
  }

  pub(crate) fn disconnect_tasks(&self, c: &mut Core) {
    disconnect_async_tasks(&mut c.state);
    for (_, st) in c.tree.states_mut() {
      disconnect_async_tasks(st);
    }
    c.task_peer.clear();
  }

  /// Spawn the process + initialize + create / resume the session
  pub fn start(self: &Arc<Self>) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move {
      let closing = {
        let mut c = me.core.lock();
        c.status = SessionStatus::Starting;
        c.error = None;
        c.auth_hint = None;
        c.lock_holder = None;
        me.touch(&mut c);
        me.drop_process(&mut c)
      };
      let result: Result<()> = async {
        // Native session stores can hold a process lock until the old CLI exits
        if let Some(f) = closing {
          f.await;
        }
        me.connect().await?;
        me.open_session().await?;
        // The value the agent started with may be one the catalogue narrowed away
        if let Err(e) = me.sync_thought().await {
          me.log(&format!("effort correction refused: {e}"));
        }
        me.refresh_context_usage().await;
        let (plan, proc, sid) = {
          let c = me.core.lock();
          (
            c.status == SessionStatus::Ready && me.synthetic_modes().is_some() && c.state.controls.mode_id.as_deref() == Some("plan"),
            c.proc.clone(),
            c.acp_session_id.clone(),
          )
        };
        if plan
          && let (Some(proc), Some(sid)) = (proc, sid)
          && let Err(e) = proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": "plan" })).await
        {
          me.log(&format!("Failed to restore plan mode: {e}"));
        }
        Ok(())
      }
      .await;
      let ready = {
        let mut c = me.core.lock();
        if let Err(e) = result {
          me.fail(&mut c, &e);
        }
        let ready = c.status == SessionStatus::Ready;
        if ready {
          c.start_outcome = Some(StartOutcome { stage: AgentHealthStage::Ready, at: now_iso(), error: None });
        }
        me.touch(&mut c);
        ready
      };
      if ready {
        me.flush_queue();
      }
    })
  }

  async fn connect(self: &Arc<Self>) -> Result<()> {
    let def = self.deps.registry.get(&self.agent)?.clone();
    let (gen_id, account) = {
      let mut c = self.core.lock();
      c.usage.reset_for_process();
      c.compaction.auto_eligible = false;
      (c.proc_gen, c.account_id.clone())
    };
    let facts = read_model_facts(&def, &self.cwd).await;
    self.core.lock().model_facts = facts;
    let handlers: Arc<dyn ClientHandlers> = Arc::new(SessionHandlers { session: self.me.clone(), gen_id });
    let account_note = account.as_ref().map(|a| format!(" account {}", a.chars().take(8).collect::<String>())).unwrap_or_default();
    let borrowed = match &self.deps.pool {
      Some(pool) => pool.take(&self.agent, &self.cwd, account.as_deref(), handlers.clone()).await,
      None => None,
    };
    let proc = match borrowed {
      Some(p) => {
        self.log(&format!("reuse warm {} (cwd {}){account_note}", def.command, self.cwd));
        p
      }
      None => {
        let bin = self
          .deps
          .registry
          .resolve_binary(&self.agent)
          .await
          .ok_or_else(|| anyhow!(tp("host.notFound", &[("command", &def.command), ("agent", &def.name)])))?;
        self.log(&format!("spawn {bin} {} (cwd {}){account_note}", def.args.join(" "), self.cwd));
        let env = match (&account, &self.deps.accounts) {
          (Some(a), Some(h)) => h.spawn_env(self.agent.clone(), a.clone()).await,
          _ => None,
        };
        AgentProcess::spawn(&def, &bin, &self.cwd, handlers, env.as_ref(), None).await?
      }
    };
    let info = proc.init.get("agentInfo");
    let info_note = match info {
      Some(i) if i.is_object() => format!(
        " · {} {}",
        i.get("name").and_then(Value::as_str).unwrap_or("undefined"),
        i.get("version").and_then(Value::as_str).unwrap_or("undefined")
      ),
      _ => String::new(),
    };
    self.log(&format!(
      "initialize ok: protocol {}{info_note}",
      proc.init.get("protocolVersion").map(|v| v.to_string()).unwrap_or_else(|| "undefined".into())
    ));
    {
      let mut c = self.core.lock();
      c.tree.reindex();
      c.auth_methods = proc.init.get("authMethods").and_then(Value::as_array).map(|methods| {
        methods
          .iter()
          .map(|m| AuthMethodInfo {
            id: m.get("id").and_then(Value::as_str).unwrap_or("").to_owned(),
            name: m.get("name").and_then(Value::as_str).unwrap_or("").to_owned(),
            description: m.get("description").and_then(Value::as_str).map(str::to_owned),
            terminal: (m.get("type").and_then(Value::as_str) == Some("terminal")).then(|| TerminalAuth {
              args: m
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default(),
              env: m
                .get("env")
                .and_then(Value::as_object)
                .map(|e| e.iter().filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned()))).collect()),
            }),
          })
          .collect()
      });
      c.proc = Some(proc);
    }
    self.handoff().await
  }

  /// With an account bound, hand the credential over before opening the session; failure = login required
  pub(crate) async fn handoff(self: &Arc<Self>) -> Result<()> {
    let (account, proc) = {
      let c = self.core.lock();
      (c.account_id.clone(), c.proc.clone())
    };
    let (Some(account), Some(hooks), Some(proc)) = (account, self.deps.accounts.clone(), proc) else { return Ok(()) };
    match hooks.authenticate(self.agent.clone(), account, proc).await {
      Ok(()) => {
        self.log("authenticate ok (account)");
        Ok(())
      }
      Err(e) => Err(anyhow::Error::new(AccountAuthError(e.to_string()))),
    }
  }

  fn note_startup_banner(c: &mut Core, meta: Option<&Value>) {
    if let Some(v) = meta.and_then(|m| m.get("piAcp")).and_then(|p| p.get("startupInfo")).and_then(Value::as_str).filter(|v| !v.is_empty())
    {
      c.startup_banner = Some(v.to_owned());
    }
  }

  async fn open_session(self: &Arc<Self>) -> Result<()> {
    let (proc, acp_id, importing) = {
      let c = self.core.lock();
      (c.proc.clone().ok_or_else(|| anyhow!("no process"))?, c.acp_session_id.clone(), c.lineage.import_pending)
    };
    let caps = proc.caps().clone();
    if let Some(acp_id) = acp_id {
      let req = self.session_request(&proc, Some(&acp_id)).await;
      let (mut gone, mut failed, mut locked, mut unresumable): (bool, Option<anyhow::Error>, bool, bool) = (false, None, false, false);
      let mut holder: Option<u32> = None;
      let attempts: [&str; 2] = if importing { ["load", "resume"] } else { ["resume", "load"] };
      let has_resume = crate::json::truthy(caps.get("sessionCapabilities").and_then(|s| s.get("resume")));
      let has_load = crate::json::truthy(caps.get("loadSession"));
      let outcome: Result<bool> = async {
        for attempt in attempts {
          if gone {
            break;
          }
          let method = match attempt {
            "resume" if has_resume => "session/resume",
            "load" if has_load => "session/load",
            _ => continue,
          };
          if method == "session/load" {
            let mut c = self.core.lock();
            c.replaying = !c.state.turns.is_empty();
          }
          let mut result = proc.request_ordered(method, req.clone()).await;
          // An agent that cannot start Acpira's MCP server fails a restore that carries it, and the refusal noted by
          // session/new does not outlive the sidecar: retry once without it, as session/new does
          if let Err(e) = &result
            && crate::host_mcp::has_server(&req)
            && !is_auth(&anyhow::Error::new(e.clone()))
          {
            self.log(&format!("{method} with the Acpira MCP server failed ({e}); retrying without it"));
            result = proc.request_ordered(method, crate::host_mcp::without_server(req.clone())).await;
            if result.is_ok()
              && let Some(h) = &self.deps.host_mcp
            {
              h.refuse(&self.agent);
            }
          }
          match result {
            Ok((r, handoff)) => {
              let mut c = self.core.lock();
              c.replaying = false;
              Self::note_startup_banner(&mut c, r.get("_meta"));
              if method == "session/load" && c.lineage.import_pending {
                seal_replay(&mut c.state);
              }
              self.apply_controls(&mut c, r.get("modes"), r.get("configOptions"));
              c.status = SessionStatus::Ready;
              drop(c);
              drop(handoff);
              self.log(&format!("{method} ok"));
              return Ok(true);
            }
            Err(e) => {
              self.core.lock().replaying = false;
              self.log(&format!("{method} failed: {e}"));
              let e = anyhow::Error::new(e);
              if is_auth(&e) {
                return Err(e);
              }
              match classify_restore_error(&e) {
                Some(RestoreFailure::Gone) => gone = true,
                Some(RestoreFailure::Locked) => {
                  holder = holder.or(lock_holder::holder_pid(&e));
                  failed = Some(e);
                  locked = true;
                }
                Some(RestoreFailure::Unresumable) => {
                  failed = Some(e);
                  unresumable = true;
                }
                Some(RestoreFailure::Failed) => failed = Some(e),
                None => {}
              }
            }
          }
        }
        Ok(false)
      }
      .await;
      {
        // One restore pass per import; afterwards the record behaves like any other session of this agent
        let mut c = self.core.lock();
        if c.lineage.import_pending {
          c.lineage.import_pending = false;
          self.touch(&mut c);
        }
      }
      if outcome? {
        return Ok(());
      }
      if locked
        && let Some(pid) = holder
      {
        let sibling = lock_holder::held_by_sibling(pid).await;
        self.log(&format!("session held by pid {pid}{}", if sibling { " (another acpira sidecar's agent)" } else { "" }));
        self.core.lock().lock_holder = sibling.then_some(pid);
      }
      if !gone {
        let mut c = self.core.lock();
        if unresumable {
          c.status = SessionStatus::Readonly;
          c.error = Some(tp("host.notResumable", &[("error", &failed.map(|e| e.to_string()).unwrap_or_default())]));
          return Ok(());
        }
        if let Some(f) = failed {
          return Err(anyhow!(tp(if locked { "host.sessionLocked" } else { "host.resumeFailed" }, &[("error", &f.to_string())])));
        }
        c.status = SessionStatus::Readonly;
        c.error = Some(t("host.cannotResume"));
        return Ok(());
      }
      if importing {
        return Err(anyhow!(t("host.importGone")));
      }
      let mut c = self.core.lock();
      if !c.state.turns.is_empty() {
        c.status = SessionStatus::Readonly;
        c.error = Some(t("host.sessionGone"));
        drop(c);
        self.log("peer no longer has this session; history kept read-only");
        return Ok(());
      }
      drop(c);
      self.log("Peer swept this empty session; starting a new one");
      self.core.lock().acp_session_id = None;
    }
    // A fresh native session starts with no command inventory; cleared before the request because peers advertise
    // commands while session/new is still in flight
    self.core.lock().state.commands = vec![];
    // Ordered: pi-acp re-sends the startup banner as a chunk right after this response, which must meet the recorded banner
    let req = self.session_request(&proc, None).await;
    let (r, handoff) = match proc.request_ordered("session/new", req.clone()).await {
      // An agent that cannot start Acpira's MCP server must still get a session: retry once without it, and leave it out
      // of this agent's later requests for the life of the sidecar
      Err(e) if crate::host_mcp::has_server(&req) => {
        self.log(&format!("session/new with the Acpira MCP server failed ({e}); retrying without it"));
        let (r, h) = proc.request_ordered("session/new", crate::host_mcp::without_server(req)).await.map_err(anyhow::Error::new)?;
        if let Some(h) = &self.deps.host_mcp {
          h.refuse(&self.agent);
        }
        (r, h)
      }
      other => other.map_err(anyhow::Error::new)?,
    };
    let mut c = self.core.lock();
    let sid = r.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
    c.acp_session_id = Some(sid.clone());
    Self::note_startup_banner(&mut c, r.get("_meta"));
    self.apply_controls(&mut c, r.get("modes"), r.get("configOptions"));
    c.status = SessionStatus::Ready;
    let opts: Vec<String> = c.state.controls.options.iter().map(|o| format!("{}({})", o.id, o.options.len())).collect();
    let line = format!(
      "session/new ok: {sid} · modes {} · options {}",
      c.state.controls.modes.len(),
      if opts.is_empty() { "-".into() } else { opts.join(" ") }
    );
    drop(c);
    drop(handoff);
    self.log(&line);
    Ok(())
  }

  pub(crate) fn fail(&self, c: &mut Core, e: &anyhow::Error) {
    if is_auth(e) {
      c.status = SessionStatus::AuthRequired;
      c.start_outcome = Some(StartOutcome { stage: AgentHealthStage::AuthRequired, at: now_iso(), error: None });
      c.error = if let Some(a) = e.downcast_ref::<AccountAuthError>() { Some(a.0.clone()) } else { c.auth_hint.clone() };
      let note = c.error.as_ref().map(|x| format!(": {x}")).unwrap_or_default();
      self.log(&format!("auth required{note}"));
    } else {
      c.status = SessionStatus::Error;
      let text = error_text(e);
      c.error = Some(text.clone());
      let stage =
        if e.downcast_ref::<AgentSpawnError>().is_some() { AgentHealthStage::SpawnFailed } else { AgentHealthStage::HandshakeFailed };
      c.start_outcome = Some(StartOutcome { stage, at: now_iso(), error: Some(text.clone()) });
      self.log(&format!("error: {text}"));
    }
  }

  /// The advertised sign-in method authenticate would pick: the named one, or the first
  pub fn auth_method(&self, method_id: Option<&str>) -> Option<AuthMethodInfo> {
    let c = self.core.lock();
    let methods = c.auth_methods.as_ref()?;
    let id = method_id.map(str::to_owned).or_else(|| methods.first().map(|m| m.id.clone()))?;
    methods.iter().find(|m| m.id == id).cloned()
  }

  /// ACP authenticate goes to the agent itself; terminal-style methods are the caller's to run in a terminal
  pub async fn authenticate(self: &Arc<Self>, method_id: Option<&str>) -> Result<()> {
    let Some(proc) = self.core.lock().proc.clone() else { return Ok(()) };
    let method = self.auth_method(method_id).ok_or_else(|| anyhow!(t("host.noAuthMethod")))?;
    if method.terminal.is_some() {
      return Err(anyhow!(tp("host.terminalAuthMethod", &[("id", &method.id)])));
    }
    proc.request("authenticate", json!({ "methodId": method.id })).await.map_err(anyhow::Error::new)?;
    Ok(())
  }

  /// Retry establishing the session (after login / an error); a live process stuck on auth gets the credential again
  pub async fn retry(self: &Arc<Self>) -> Result<()> {
    let stuck = {
      let mut c = self.core.lock();
      let stuck = c.proc.as_ref().is_some_and(|p| p.alive()) && c.status == SessionStatus::AuthRequired;
      if stuck {
        c.status = SessionStatus::Starting;
        c.error = None;
        c.auth_hint = None;
        self.touch(&mut c);
      }
      stuck
    };
    if !stuck {
      self.start().await;
      return Ok(());
    }
    let result: Result<()> = async {
      self.handoff().await?;
      self.open_session().await?;
      self.refresh_context_usage().await;
      Ok(())
    }
    .await;
    let ready = {
      let mut c = self.core.lock();
      if let Err(e) = result {
        self.fail(&mut c, &e);
      }
      let ready = c.status == SessionStatus::Ready;
      if ready {
        c.start_outcome = Some(StartOutcome { stage: AgentHealthStage::Ready, at: now_iso(), error: None });
      }
      self.touch(&mut c);
      ready
    };
    if ready {
      self.flush_queue();
    }
    Ok(())
  }

  pub(crate) fn busy(c: &Core) -> bool {
    c.phase.running || c.phase.editing || c.phase.staging || c.switching || c.status == SessionStatus::Starting
  }

  /// Re-authenticate a replacement process, then resume / load the same native session
  pub async fn rebind_account(self: &Arc<Self>, account_id: &str) -> Result<()> {
    {
      let mut c = self.core.lock();
      if c.account_id.as_deref() == Some(account_id) && c.proc.as_ref().is_some_and(|p| p.alive()) && c.status == SessionStatus::Ready {
        return Ok(());
      }
      if Self::busy(&c) {
        return Err(anyhow!(t("history.unavailable")));
      }
      c.account_id = Some(account_id.to_owned());
    }
    self.reopen().await;
    Ok(())
  }

  /// The automatic switch's rebind: the session is already reserved by `switching`, which `busy` would refuse
  pub(crate) async fn rebind_reserved(self: &Arc<Self>, account_id: &str) {
    self.core.lock().account_id = Some(account_id.to_owned());
    self.reopen().await;
  }

  /// Rebuild the connection under a session whose prompts keep failing on a live process
  pub async fn reconnect(self: &Arc<Self>) -> Result<()> {
    {
      let c = self.core.lock();
      if c.status == SessionStatus::Closed {
        return Ok(());
      }
      if Self::busy(&c) {
        return Err(anyhow!(t("history.unavailable")));
      }
    }
    self.reopen().await;
    Ok(())
  }

  /// The native session is locked by an agent another Acpira sidecar left running (an extension host orphaned by a dropped
  /// remote connection): end that process, then start over like Retry. The holder is checked again right before the signal
  pub async fn take_over(self: &Arc<Self>) -> Result<()> {
    let holder = {
      let c = self.core.lock();
      if c.status != SessionStatus::Error {
        return Ok(());
      }
      c.lock_holder
    };
    if let Some(pid) = holder
      && lock_holder::held_by_sibling(pid).await
    {
      self.log(&format!("taking the session over from pid {pid}"));
      lock_holder::terminate(pid).await;
    }
    self.start().await;
    Ok(())
  }

  async fn reopen(self: &Arc<Self>) {
    let settings = acpira_shared::turn_settings::capture_turn_settings(&self.core.lock().state.controls);
    self.start().await;
    if self.status() == SessionStatus::Ready {
      self.adopt_controls(settings).await;
    }
  }

  pub fn dispose(self: &Arc<Self>) {
    if let (_, Some(f)) = self.close() {
      tokio::spawn(f);
    }
  }

  /// Closed like dispose, for a host about to exit: the agent process (for a hard kill if waiting runs out) and the future that
  /// closes its ACP session and ends it
  pub fn shutdown(self: &Arc<Self>) -> (Option<Arc<AgentProcess>>, Option<BoxFuture<()>>) {
    self.close()
  }

  fn close(self: &Arc<Self>) -> (Option<Arc<AgentProcess>>, Option<BoxFuture<()>>) {
    let proc = self.core.lock().proc.clone();
    let closing = {
      let mut c = self.core.lock();
      c.usage.clear_timer();
      c.perms.epoch += 1;
      c.status = SessionStatus::Closed;
      c.queue = PromptQueue::default();
      c.peer.detached = false;
      c.tree.settle("disposed");
      self.drain_terminal(&mut c);
      if c.phase.running {
        self.settle(&mut c, TurnStop::Cancelled, None);
      }
      self.cancel_all_permissions(&mut c);
      self.cancel_all_questions(&mut c);
      self.drop_process(&mut c)
    };
    (proc, closing)
  }
}
