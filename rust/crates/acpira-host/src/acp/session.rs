//! One session = one agent subprocess + one transcript. State machine:
//! start → (resume | load | new) → ready ⇄ prompt / cancel; failed login → auth_required; unresumable → readonly;
//! dead process → error.
//!
//! All mutable state is `Core`, behind one short-held mutex. Waiting (a prompt on the wire, a permission card, a platform
//! round trip) happens with the lock released, so a `stop` never queues behind a running `send`. The gates, the queue,
//! the controls and history editing are further `impl AcpSession` blocks in sibling files

use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use acpira_shared::inventory::{AgentHealthStage, AgentRuntimeInfo};
use acpira_shared::model_shapes::ModelShapes;
use acpira_shared::model_sources::{ModelSources, apply_model_sources};
use acpira_shared::num::Num;
use acpira_shared::protocol::RawJson;
use acpira_shared::slash_commands::restore_command_receipts;
use acpira_shared::subagents::SubagentSummary;
use acpira_shared::transcript::*;

use super::agent_pool::AgentPool;
use super::agent_process::{AgentProcess, AgentSpawnError, ClientHandlers};
use super::agent_registry::AgentRegistry;
use super::cancel::Cancel;
use super::compaction::CompactionCompletion;
use super::model_sources::read_model_sources;
use super::normalize::{
  FileImageSaver, ImageSaver, NormalizeState, ToolCtx, disconnect_async_tasks, runtime_info_of, seal_replay,
};
use super::plan_snapshots::restore_plan_snapshots;
use super::restore_turns::restore_interrupted_turns;
use super::rpc::{BoxFuture, RpcError};
use super::session_errors::{AccountAuthError, RestoreFailure, auth_hint_of, classify_restore_error, is_auth};
use super::session_failure::SessionFailure;
use super::subagent_tree::SubagentTree;
use crate::i18n::{t, t_or, tp};
use crate::store::record::{ForkedFrom, ImportedFrom, RecordSource, SessionRecord};
use crate::store::transcript_store::{LogFn, TranscriptStore, blob_name};
use crate::util::{now_iso, random_uuid};

pub const USAGE_POLL_INTERVAL: Duration = Duration::from_millis(800);
const CLOSE_GRACE: Duration = Duration::from_secs(3);
pub(crate) const MODE_PICK: &str = "\0mode";

/// The hooks the account layer gives a session: environment variables before spawn, authenticate after initialize, and
/// the automatic switch when the bound account runs out of quota
pub trait SessionAccountHooks: Send + Sync {
  fn spawn_env(&self, agent: String, account: String) -> BoxFuture<Option<StrMap>>;
  fn authenticate(&self, agent: String, account: String, proc: Arc<AgentProcess>) -> BoxFuture<Result<()>>;
  /// (id, label) of the account to move to after `current` ran out of quota; None = stay and show the error
  fn fallback(&self, _agent: String, _current: Option<String>) -> BoxFuture<Option<(String, String)>> {
    Box::pin(async { None })
  }
  fn label(&self, _account: String) -> Option<String> {
    None
  }
}

#[derive(Debug, Clone, Copy)]
pub struct CompactionPolicy {
  pub at_tokens: f64,
  pub auto: bool,
}

pub type OnChange = Arc<dyn Fn(&str, bool) + Send + Sync>;

#[derive(Clone)]
pub struct SessionDeps {
  pub registry: Arc<AgentRegistry>,
  pub log: LogFn,
  /// (session id, running): must not block and must not lock the session back
  pub on_change: OnChange,
  pub blobs: Arc<TranscriptStore>,
  pub notify: Option<LogFn>,
  pub accounts: Option<Arc<dyn SessionAccountHooks>>,
  pub compaction: Option<Arc<dyn Fn() -> CompactionPolicy + Send + Sync>>,
  pub pool: Option<Arc<AgentPool>>,
  pub model_shapes: Option<Arc<dyn Fn(&str) -> Option<ModelShapes> + Send + Sync>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StartOutcome {
  pub stage: AgentHealthStage,
  pub at: String,
  pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct Phase {
  pub running: bool,
  pub staging: bool,
  pub staging_aborted: bool,
  pub editing: bool,
  pub edit_notifications: Vec<Value>,
}

pub(crate) struct PendingPermission {
  pub tx: oneshot::Sender<Value>,
  pub block_id: String,
  pub options: Vec<Value>,
  pub plan_id: Option<String>,
  pub node_id: Option<String>,
}

pub(crate) enum QuestionReply {
  Form { schema: Value },
  Grok,
}

pub(crate) struct PendingQuestion {
  pub tx: oneshot::Sender<Value>,
  pub block_id: String,
  pub questions: Vec<Question>,
  pub node_id: Option<String>,
  pub reply: QuestionReply,
}

pub(crate) struct QueuedEntry {
  pub id: String,
  pub text: String,
  pub prepared: super::attachments::PreparedPrompt,
}

pub(crate) struct Core {
  pub account_id: Option<String>,
  pub updated_at: String,
  pub pinned: Option<bool>,
  pub acp_session_id: Option<String>,
  pub proc_gen: u64,
  pub task_peer: HashMap<String, String>,
  pub state: NormalizeState,
  pub status: SessionStatus,
  pub error: Option<String>,
  pub auth_methods: Option<Vec<AuthMethodInfo>>,
  pub start_outcome: Option<StartOutcome>,
  pub phase: Phase,
  pub replaying: bool,
  pub startup_banner: Option<String>,
  pub proc: Option<Arc<AgentProcess>>,
  // Permission gate
  pub perms: Vec<PendingPermission>,
  pub perm_seq: u64,
  pub perm_epoch: u64,
  pub auto_approve: bool,
  // Question gate
  pub questions: Vec<PendingQuestion>,
  pub question_seq: u64,
  pub raw_questions: super::questions::RawMemory,
  pub tree: SubagentTree,
  // Prompt queue
  pub queue: Vec<QueuedEntry>,
  pub sending_id: Option<String>,
  /// A message accepted below a pre-send compaction: visible, but not the normalizer's last turn
  pub pending_prompt: Option<Turn>,
  pub building_plan: bool,
  pub compacted_at: Option<f64>,
  pub completion: Option<CompactionCompletion>,
  pub turn_failure: Option<SessionFailure>,
  pub auth_hint: Option<String>,
  pub model_sources: ModelSources,
  pub usage_revision: u64,
  pub usage_notifications: bool,
  pub auto_compact_eligible: bool,
  pub grok_usage_unavailable: bool,
  pub usage_timer: Option<tokio::task::AbortHandle>,
  pub usage_inflight: bool,
  pub pi_stamp: Option<super::pi_usage::Stamp>,
  pub finish_usage_refresh: Option<oneshot::Sender<bool>>,
  pub syncing_thought: bool,
  pub adopting: bool,
  pub picks: HashMap<String, (String, u64)>,
  pub pick_seq: u64,
  pub rev: i64,
  pub history_pending: bool,
  pub forked_from: Option<ForkedFrom>,
  pub import_pending: bool,
  pub imported_from: Option<ImportedFrom>,
  pub agent_title_muted: bool,
  /// An automatic account switch is between the exhausted turn and its continue: prompts queue, manual switches wait
  pub switching: bool,
}

pub struct AcpSession {
  pub id: String,
  pub agent: String,
  pub cwd: String,
  pub created_at: String,
  pub(crate) core: parking_lot::Mutex<Core>,
  pub(crate) deps: SessionDeps,
  pub(crate) me: Weak<AcpSession>,
  // Serializes composer picks: rapid clicks collapse to the last value per control
  pub(crate) pick_lock: tokio::sync::Mutex<()>,
  // Serializes polled usage reads (Grok, Pi) so slow replies never overwrite a newer snapshot
  pub(crate) usage_lock: tokio::sync::Mutex<()>,
}

impl AcpSession {
  pub fn new(record: SessionRecord, deps: SessionDeps) -> Arc<AcpSession> {
    Arc::new_cyclic(|me: &Weak<AcpSession>| {
      let log_prefix = format!("[{} {}] ", record.agent, record.id.chars().take(8).collect::<String>());
      let log = deps.log.clone();
      let tree_log: LogFn = Arc::new(move |line: &str| log(&format!("{log_prefix}{line}")));
      let ctx =
        ToolCtx { save_image: Some(image_saver(me.clone())), save_image_file: Some(file_image_saver(me.clone())), ..Default::default() };
      let turns = restore_interrupted_turns(restore_command_receipts(restore_plan_snapshots(record.turns)), &record.updated_at);
      let state = NormalizeState {
        turns,
        controls: record.controls,
        usage: record.usage,
        commands: record.commands,
        title: Some(record.title).filter(|t| !t.is_empty()),
        ctx: ctx.clone(),
        log: Some(tree_log.clone()),
        agent: Some(record.agent.clone()),
        ..Default::default()
      };
      let tree = SubagentTree::new(tree_log, ctx, record.subagents, Some(&record.updated_at));
      AcpSession {
        id: record.id,
        agent: record.agent,
        cwd: record.cwd,
        created_at: record.created_at,
        core: parking_lot::Mutex::new(Core {
          account_id: record.account_id,
          updated_at: record.updated_at,
          pinned: record.pinned,
          acp_session_id: record.acp_session_id,
          proc_gen: 0,
          task_peer: HashMap::new(),
          state,
          status: SessionStatus::Starting,
          error: None,
          auth_methods: None,
          start_outcome: None,
          phase: Phase::default(),
          replaying: false,
          startup_banner: None,
          proc: None,
          perms: vec![],
          perm_seq: 0,
          perm_epoch: 0,
          auto_approve: false,
          questions: vec![],
          question_seq: 0,
          raw_questions: Default::default(),
          tree,
          queue: vec![],
          sending_id: None,
          pending_prompt: None,
          building_plan: false,
          compacted_at: None,
          completion: None,
          turn_failure: None,
          auth_hint: None,
          model_sources: ModelSources::new(),
          usage_revision: 0,
          usage_notifications: false,
          auto_compact_eligible: false,
          grok_usage_unavailable: false,
          usage_timer: None,
          usage_inflight: false,
          pi_stamp: None,
          finish_usage_refresh: None,
          syncing_thought: false,
          adopting: false,
          picks: HashMap::new(),
          pick_seq: 0,
          rev: 0,
          history_pending: record.history_pending,
          agent_title_muted: record.forked_from.is_some(),
          forked_from: record.forked_from,
          import_pending: record.import_pending,
          imported_from: record.imported_from,
          switching: false,
        }),
        deps,
        me: me.clone(),
        pick_lock: tokio::sync::Mutex::new(()),
        usage_lock: tokio::sync::Mutex::new(()),
      }
    })
  }

  pub fn fresh(agent: &str, cwd: &str, deps: SessionDeps, account_id: Option<String>) -> Arc<AcpSession> {
    let now = now_iso();
    AcpSession::new(
      SessionRecord {
        id: random_uuid(),
        agent: agent.to_owned(),
        account_id,
        acp_session_id: None,
        cwd: cwd.to_owned(),
        title: t("session.untitled"),
        created_at: now.clone(),
        updated_at: now,
        turns: vec![],
        controls: SessionControls::default(),
        usage: None,
        commands: vec![],
        pinned: None,
        history_pending: false,
        forked_from: None,
        import_pending: false,
        imported_from: None,
        subagents: None,
      },
      deps,
    )
  }

  pub(crate) fn arc(&self) -> Arc<AcpSession> {
    self.me.upgrade().expect("session alive while in use")
  }

  pub(crate) fn log(&self, line: &str) {
    (self.deps.log)(&format!("[{} {}] {line}", self.agent, self.id.chars().take(8).collect::<String>()));
  }

  pub(crate) fn notify(&self, text: &str) {
    if let Some(n) = &self.deps.notify {
      n(text);
    }
  }

  pub(crate) fn def(&self) -> super::agent_registry::AgentDef {
    self.deps.registry.get(&self.agent).cloned().unwrap_or_default()
  }

  /// Publish state, leaving updatedAt alone (streamed chunks must not reorder the list)
  pub(crate) fn touch(&self, c: &mut Core) {
    apply_model_sources(&self.agent, &mut c.state.controls.options, &c.model_sources);
    c.rev += 1;
    (self.deps.on_change)(&self.id, c.phase.running);
  }

  /// A user-initiated message moves the session to the top of the list
  pub(crate) fn bump(&self, c: &mut Core) {
    c.updated_at = now_iso();
    self.touch(c);
  }

  pub fn republish(&self) {
    let mut c = self.core.lock();
    self.touch(&mut c);
  }

  pub(crate) fn title_of(c: &Core) -> String {
    c.state.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| t("session.untitled"))
  }

  pub fn title(&self) -> String {
    Self::title_of(&self.core.lock())
  }

  pub fn is_running(&self) -> bool {
    self.core.lock().phase.running
  }

  pub fn alive(&self) -> bool {
    self.core.lock().proc.as_ref().is_some_and(|p| p.alive())
  }

  pub fn status(&self) -> SessionStatus {
    self.core.lock().status
  }

  pub fn account_id(&self) -> Option<String> {
    self.core.lock().account_id.clone()
  }

  pub fn start_outcome(&self) -> Option<StartOutcome> {
    self.core.lock().start_outcome.clone()
  }

  pub(crate) fn can_compact_of(c: &Core) -> bool {
    c.state.commands.iter().any(|x| x.name == "compact")
  }

  pub fn runtime_info(&self) -> Option<AgentRuntimeInfo> {
    self.core.lock().proc.as_ref().map(|p| runtime_info_of(&p.init))
  }

  /// What the agent last confirmed, without the optimistic overlay
  pub fn agent_controls(&self) -> SessionControls {
    self.core.lock().state.controls.clone()
  }

  pub fn acp_session_id(&self) -> Option<String> {
    self.core.lock().acp_session_id.clone()
  }

  pub fn turn_count(&self) -> usize {
    let c = self.core.lock();
    c.state.turns.len() + usize::from(c.pending_prompt.is_some())
  }

  /// The list entry of this session
  pub fn summary(&self) -> SessionSummary {
    let c = self.core.lock();
    SessionSummary {
      id: self.id.clone(),
      external: None,
      title: Self::title_of(&c),
      agent: self.agent.clone(),
      account_id: c.account_id.clone(),
      acp_session_id: c.acp_session_id.clone(),
      cwd: self.cwd.clone(),
      updated_at: c.updated_at.clone(),
      pinned: c.pinned,
      state: None,
    }
  }

  /// The session list's state dot: working, waiting on the user, or ended in error
  pub fn list_state(&self) -> Option<SummaryState> {
    let c = self.core.lock();
    if c.phase.running {
      return Some(SummaryState::Working);
    }
    let waiting = visible_turns(&c).any(|t| {
      t.as_agent().is_some_and(|a| {
        a.blocks.iter().any(|b| matches!(b, AgentBlock::Permission(_)) || matches!(b, AgentBlock::Question(q) if q.outcome.is_none()))
      })
    });
    if waiting {
      return Some(SummaryState::Waiting);
    }
    match visible_turns(&c).last() {
      Some(Turn::Agent(a)) if a.stop == Some(TurnStop::Error) => Some(SummaryState::Error),
      _ => None,
    }
  }

  /// The whole view, serialized once for every viewer showing this session
  pub fn view_json(&self) -> (RawJson, bool) {
    let shapes = self.deps.model_shapes.as_ref().and_then(|f| f(&self.agent));
    let mut c = self.core.lock();
    let subagents = if c.tree.is_empty() { None } else { Some(c.tree.summaries()) };
    let picked = (!c.picks.is_empty()).then(|| picked_controls(&c));
    let queued = queue_snapshot(&c);
    let view = ViewRef {
      id: &self.id,
      agent: &self.agent,
      account_id: c.account_id.as_deref(),
      title: Self::title_of(&c),
      cwd: &self.cwd,
      status: c.status,
      error: c.error.as_deref(),
      auth_methods: c.auth_methods.as_deref(),
      turns: TurnsRef { turns: &c.state.turns, pending: c.pending_prompt.as_ref() },
      running: c.phase.running,
      rev: c.rev,
      controls: picked.as_ref().unwrap_or(&c.state.controls),
      model_shapes: shapes.as_ref(),
      usage: c.state.usage.as_ref(),
      commands: &c.state.commands,
      queued,
      subagents: subagents.as_deref(),
      created_at: &self.created_at,
      updated_at: &c.updated_at,
    };
    (RawJson::new(&view), c.phase.running)
  }

  /// An owned view, for callers that inspect it (tests, plan lookups)
  pub fn view(&self) -> SessionView {
    serde_json::from_str(self.view_json().0.get()).expect("view round trip")
  }

  pub fn to_record(&self) -> SessionRecord {
    serde_json::from_slice(&self.record_json()).expect("record round trip")
  }

  pub fn subagent_transcript(&self, id: &str) -> Option<(RawJson, i64, bool)> {
    let c = self.core.lock();
    let (turns, rev, running) = c.tree.transcript(id)?;
    Some((RawJson::new(&turns), rev, running))
  }

  pub fn plan_document(&self, plan_id: &str) -> Option<PlanDocumentBlock> {
    let c = self.core.lock();
    visible_turns(&c).filter_map(Turn::as_agent).flat_map(|t| t.blocks.iter()).find_map(|b| match b {
      AgentBlock::PlanDocument(p) if p.id == plan_id => Some(p.clone()),
      _ => None,
    })
  }

  pub(crate) fn synthetic_modes(&self) -> Option<Vec<SessionOption>> {
    self.def().modes.map(|modes| {
      modes
        .into_iter()
        .map(|mut m| {
          m.description = m.description.map(|d| t_or(&d));
          m
        })
        .collect()
    })
  }

  /// Kill the current CLI; its exit / updates must not touch the session after this. session/close goes out first
  /// when the agent advertises it (close is not delete: it lets the agent flush and release its lock)
  pub(crate) fn drop_process(&self, c: &mut Core) -> Option<BoxFuture<()>> {
    let proc = c.proc.take()?;
    c.proc_gen += 1;
    c.perm_epoch += 1;
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
      c.usage_notifications = false;
      c.auto_compact_eligible = false;
      c.grok_usage_unavailable = false;
      c.pi_stamp = None;
      clear_usage_timer(&mut c);
      (c.proc_gen, c.account_id.clone())
    };
    let sources = read_model_sources(&self.agent, &self.cwd).await;
    self.core.lock().model_sources = sources;
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

  /// Every session/new / resume / load response: synthetic modes fill in, a resumed session keeps its persisted mode
  pub(crate) fn apply_controls(&self, c: &mut Core, modes: Option<&Value>, config_options: Option<&Value>) {
    let wanted = c.state.controls.mode_id.clone();
    if self.protocol_controls(&mut c.state.controls, modes, config_options) {
      return;
    }
    let Some(syn) = self.synthetic_modes() else { return };
    if !c.state.controls.modes.is_empty() {
      return;
    }
    c.state.controls.mode_id = Some(match wanted {
      Some(w) if syn.iter().any(|m| m.id == w) => w,
      _ => "default".into(),
    });
    c.state.controls.modes = syn;
    c.auto_approve = c.state.controls.mode_id.as_deref() == Some("yolo");
  }

  async fn open_session(self: &Arc<Self>) -> Result<()> {
    let (proc, acp_id, importing) = {
      let c = self.core.lock();
      (c.proc.clone().ok_or_else(|| anyhow!("no process"))?, c.acp_session_id.clone(), c.import_pending)
    };
    let caps = proc.caps().clone();
    if let Some(acp_id) = acp_id {
      let req = json!({ "sessionId": acp_id, "cwd": self.cwd, "mcpServers": [] });
      let (mut gone, mut failed, mut locked, mut unresumable): (bool, Option<anyhow::Error>, bool, bool) = (false, None, false, false);
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
          match proc.request_ordered(method, req.clone()).await {
            Ok((r, handoff)) => {
              let mut c = self.core.lock();
              c.replaying = false;
              Self::note_startup_banner(&mut c, r.get("_meta"));
              if method == "session/load" && c.import_pending {
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
        if c.import_pending {
          c.import_pending = false;
          self.touch(&mut c);
        }
      }
      if outcome? {
        return Ok(());
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
    let (r, handoff) = proc.request_ordered("session/new", json!({ "cwd": self.cwd, "mcpServers": [] })).await.map_err(anyhow::Error::new)?;
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
      let text = e.to_string();
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

  async fn reopen(self: &Arc<Self>) {
    let settings = acpira_shared::turn_settings::capture_turn_settings(&self.core.lock().state.controls);
    self.start().await;
    if self.status() == SessionStatus::Ready {
      self.adopt_controls(settings).await;
    }
  }

  /// Paint last-known chips before session/new returns so the composer isn't empty during start
  pub fn preview_controls(&self, options: &[ConfigControl], settings: Option<&TurnSettings>) {
    let mut c = self.core.lock();
    if let Some(syn) = self.synthetic_modes().filter(|s| !s.is_empty()) {
      c.state.controls.mode_id = Some(match settings.and_then(|s| s.mode_id.clone()) {
        Some(m) if syn.iter().any(|x| x.id == m) => m,
        _ => syn[0].id.clone(),
      });
      c.state.controls.modes = syn;
      c.auto_approve = c.state.controls.mode_id.as_deref() == Some("yolo");
    }
    if options.is_empty() {
      return;
    }
    let mut next = options.to_vec();
    for ctl in &mut next {
      if let Some(v) = settings.and_then(|s| s.config.get(&ctl.id))
        && ctl.options.iter().any(|o| &o.id == v)
      {
        ctl.value = Some(v.clone());
      }
    }
    c.state.controls.options = next;
  }

  /// Rename / pin: only the record, and updatedAt stays
  pub fn rename(&self, title: &str) {
    let t = title.trim();
    if t.is_empty() {
      return;
    }
    let mut c = self.core.lock();
    c.state.title = Some(crate::util::clip(t, crate::limits::RENAME_MAX));
    self.touch(&mut c);
  }

  pub fn set_pinned(&self, pinned: bool) {
    let mut c = self.core.lock();
    c.pinned = pinned.then_some(true);
    self.touch(&mut c);
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
      clear_usage_timer(&mut c);
      c.perm_epoch += 1;
      c.status = SessionStatus::Closed;
      c.queue.clear();
      c.sending_id = None;
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

pub(crate) fn clear_usage_timer(c: &mut Core) {
  if let Some(h) = c.usage_timer.take() {
    h.abort();
  }
}

pub(crate) fn visible_turns(c: &Core) -> impl DoubleEndedIterator<Item = &Turn> + '_ {
  c.state.turns.iter().chain(c.pending_prompt.iter())
}

/// Controls with in-flight picks overlaid
pub(crate) fn picked_controls(c: &Core) -> SessionControls {
  let mut out = c.state.controls.clone();
  if let Some((v, _)) = c.picks.get(MODE_PICK) {
    out.mode_id = Some(v.clone());
  }
  for o in &mut out.options {
    if let Some((v, _)) = c.picks.get(&o.id) {
      o.value = Some(v.clone());
    }
  }
  out
}

pub(crate) fn queue_snapshot(c: &Core) -> Option<Vec<QueuedPrompt>> {
  if c.queue.is_empty() {
    return None;
  }
  Some(
    c.queue
      .iter()
      .map(|q| QueuedPrompt {
        id: q.id.clone(),
        text: q.text.clone(),
        attachments: q.prepared.attachments.clone(),
        sending: (c.sending_id.as_deref() == Some(q.id.as_str())).then_some(true),
      })
      .collect(),
  )
}

/// The visible turns: the transcript plus a message accepted below a pre-send compaction
#[derive(Clone, Copy)]
pub(crate) struct TurnsRef<'a> {
  pub turns: &'a [Turn],
  pub pending: Option<&'a Turn>,
}

impl Serialize for TurnsRef<'_> {
  fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(self.turns.len() + usize::from(self.pending.is_some())))?;
    for t in self.turns {
      seq.serialize_element(t)?;
    }
    if let Some(p) = self.pending {
      seq.serialize_element(p)?;
    }
    seq.end()
  }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewRef<'a> {
  id: &'a str,
  agent: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  account_id: Option<&'a str>,
  title: String,
  cwd: &'a str,
  status: SessionStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  auth_methods: Option<&'a [AuthMethodInfo]>,
  turns: TurnsRef<'a>,
  running: bool,
  rev: i64,
  controls: &'a SessionControls,
  #[serde(skip_serializing_if = "Option::is_none")]
  model_shapes: Option<&'a ModelShapes>,
  #[serde(skip_serializing_if = "Option::is_none")]
  usage: Option<&'a Usage>,
  commands: &'a [SlashCommand],
  #[serde(skip_serializing_if = "Option::is_none")]
  queued: Option<Vec<QueuedPrompt>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  subagents: Option<&'a [SubagentSummary]>,
  created_at: &'a str,
  updated_at: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordRef<'a> {
  id: &'a str,
  agent: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  account_id: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  acp_session_id: Option<&'a str>,
  cwd: &'a str,
  title: String,
  created_at: &'a str,
  updated_at: &'a str,
  turns: TurnsRef<'a>,
  controls: &'a SessionControls,
  #[serde(skip_serializing_if = "Option::is_none")]
  usage: Option<&'a Usage>,
  commands: &'a [SlashCommand],
  #[serde(skip_serializing_if = "Option::is_none")]
  pinned: Option<bool>,
  #[serde(skip_serializing_if = "std::ops::Not::not")]
  history_pending: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  forked_from: Option<&'a ForkedFrom>,
  #[serde(skip_serializing_if = "std::ops::Not::not")]
  import_pending: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  imported_from: Option<&'a ImportedFrom>,
  #[serde(skip_serializing_if = "Option::is_none")]
  subagents: Option<Vec<super::subagent_tree::RecordRef<'a>>>,
}

impl RecordSource for AcpSession {
  fn record_id(&self) -> String {
    self.id.clone()
  }

  fn record_json(&self) -> Vec<u8> {
    let c = self.core.lock();
    let r = RecordRef {
      id: &self.id,
      agent: &self.agent,
      account_id: c.account_id.as_deref(),
      acp_session_id: c.acp_session_id.as_deref(),
      cwd: &self.cwd,
      title: Self::title_of(&c),
      created_at: &self.created_at,
      updated_at: &c.updated_at,
      turns: TurnsRef { turns: &c.state.turns, pending: c.pending_prompt.as_ref() },
      controls: &c.state.controls,
      usage: c.state.usage.as_ref(),
      commands: &c.state.commands,
      pinned: c.pinned,
      history_pending: c.history_pending,
      forked_from: c.forked_from.as_ref(),
      import_pending: c.import_pending,
      imported_from: c.imported_from.as_ref(),
      subagents: (!c.tree.is_empty()).then(|| c.tree.record_refs()),
    };
    serde_json::to_vec(&r).unwrap_or_default()
  }

  fn record(&self) -> SessionRecord {
    self.to_record()
  }
}

/// An agent-emitted image payload → the session blob store; the content-hash name is known before the write lands
fn image_saver(me: Weak<AcpSession>) -> ImageSaver {
  use base64::Engine;
  Arc::new(move |data: &str, mime: &str| {
    let s = me.upgrade()?;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(data.as_bytes()) {
      Ok(b) => b,
      Err(e) => {
        s.log(&format!("image payload rejected: {e}"));
        return None;
      }
    };
    let ext = acpira_shared::attachments::ext_of_mime(mime);
    let name = blob_name(ext, &bytes);
    let blobs = s.deps.blobs.clone();
    let (id, n2) = (s.id.clone(), name.clone());
    tokio::spawn(async move {
      if let Err(e) = blobs.save_blob(&id, ext, &bytes).await {
        s.log(&format!("image blob {n2}: {e}"));
      }
    });
    Some(name)
  })
}

/// A tool's resource_link to a local image: read synchronously so the blob name is known before the block renders
fn file_image_saver(me: Weak<AcpSession>) -> FileImageSaver {
  Arc::new(move |path: &str| {
    let s = me.upgrade()?;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || acpira_shared::attachments::image_mime_of(path).is_none() || meta.len() > crate::limits::MAX_OUT_IMAGE_BYTES {
      return None;
    }
    let ext = std::path::Path::new(path).extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
    let bytes = match std::fs::read(path) {
      Ok(b) => b,
      Err(e) => {
        s.log(&format!("image file {path}: {e}"));
        return None;
      }
    };
    let name = blob_name(&ext, &bytes);
    let blobs = s.deps.blobs.clone();
    let (id, n2) = (s.id.clone(), name.clone());
    tokio::spawn(async move {
      if let Err(e) = blobs.save_blob(&id, &ext, &bytes).await {
        s.log(&format!("image blob {n2}: {e}"));
      }
    });
    Some(name)
  })
}

/// The client handlers of one process generation: a replaced process's late callbacks are ignored
pub(crate) struct SessionHandlers {
  pub session: Weak<AcpSession>,
  pub gen_id: u64,
}

impl SessionHandlers {
  fn live(&self) -> Option<Arc<AcpSession>> {
    let s = self.session.upgrade()?;
    let current = s.core.lock().proc_gen;
    (current == self.gen_id).then_some(s)
  }
}

impl ClientHandlers for SessionHandlers {
  fn on_update(&self, params: Value) {
    if let Some(s) = self.live() {
      s.on_update(params);
    }
  }

  fn on_permission(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    Box::pin(async move {
      match s {
        Some(s) => s.on_permission(req, cancel).await,
        None => Ok(json!({ "outcome": { "outcome": "cancelled" } })),
      }
    })
  }

  fn on_elicitation(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_elicitation(req, cancel).await),
        None => Ok(json!({ "action": "cancel" })),
      }
    })
  }

  fn on_grok_question(&self, req: Value, cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let s = self.session.upgrade();
    Box::pin(async move {
      match s {
        Some(s) => Ok(s.on_grok_question(req, cancel).await),
        None => Ok(json!({ "outcome": "skip_interview" })),
      }
    })
  }

  fn on_stderr(&self, line: &str) {
    let Some(s) = self.live() else { return };
    s.log(&format!("stderr: {line}"));
    if let Some(hint) = auth_hint_of(line) {
      let mut c = s.core.lock();
      // stderr and the -32000 on stdout are separate pipes: a reason read after the failure still reaches the view
      if c.status == SessionStatus::AuthRequired && c.error.is_none() {
        c.error = Some(hint.clone());
        s.touch(&mut c);
      }
      c.auth_hint = Some(hint);
    }
  }

  fn on_exit(&self, code: Option<i32>, signal: Option<String>) {
    let Some(s) = self.session.upgrade() else { return };
    s.log(&format!(
      "exit code={} signal={}",
      code.map(|c| c.to_string()).unwrap_or_else(|| "null".into()),
      signal.as_deref().unwrap_or("null")
    ));
    let mut c = s.core.lock();
    if c.proc_gen != self.gen_id || c.status == SessionStatus::Closed {
      return;
    }
    let def = s.def();
    c.status = SessionStatus::Error;
    if c.error.is_none() {
      let why = code.map(|x| x.to_string()).or(signal).unwrap_or_else(|| "?".into());
      c.error = Some(tp("host.exited", &[("agent", &def.name), ("code", &why)]));
    }
    c.tree.settle("connection-lost");
    s.drain_terminal(&mut c);
    s.disconnect_tasks(&mut c);
    s.settle(&mut c, TurnStop::Cancelled, None);
    s.touch(&mut c);
  }
}

pub(crate) fn num(n: f64) -> Num {
  Num(n)
}
