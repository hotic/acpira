//! One session = one agent subprocess + one transcript. State machine:
//! start → (resume | load | new) → ready ⇄ prompt / cancel; failed login → auth_required; unresumable → readonly;
//! dead process → error.
//!
//! All mutable state is `Core`, behind one short-held mutex. Waiting (a prompt on the wire, a permission card, a platform
//! round trip) happens with the lock released, so a `stop` never queues behind a running `send`. This module holds the
//! state and its construction; the lifecycle, the turn, the queue, the gates, the controls, history editing and the
//! view / record projections are further `impl AcpSession` blocks in the child modules

pub mod account_switch;
pub mod attachments;
pub mod compaction;
pub mod controls;
pub mod edit;
pub mod errors;
pub mod failure;
pub mod gates;
pub mod handlers;
pub mod images;
pub mod lifecycle;
pub mod plan_build;
pub mod prompt;
pub mod queue;
pub mod restore_turns;
pub mod tasks;
pub mod turn_usage;
pub mod updates;
pub mod usage;
pub mod view;

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use anyhow::Result;
use serde_json::Value;
use tokio::sync::oneshot;

use acpira_shared::inventory::{AgentHealthStage, AgentRuntimeInfo};
use acpira_shared::model_shapes::ModelShapes;
use acpira_shared::num::Num;
use acpira_shared::slash_commands::restore_command_receipts;
use acpira_shared::transcript::*;

use crate::acp::agents::model_sources::ModelFacts;
use crate::acp::agents::pool::AgentPool;
use crate::acp::agents::registry::AgentRegistry;
use crate::acp::session::compaction::CompactionCompletion;
use crate::acp::session::failure::SessionFailure;
use crate::acp::session::images::{file_image_saver, image_saver};
use crate::acp::session::restore_turns::restore_interrupted_turns;
use crate::acp::transcript::normalize::{NormalizeState, ToolCtx, runtime_info_of};
use crate::acp::transcript::plan_snapshots::restore_plan_snapshots;
use crate::acp::transcript::subagent_tree::SubagentTree;
use crate::acp::transport::process::AgentProcess;
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::t;
use crate::store::record::{ForkedFrom, ImportedFrom, SessionRecord};
use crate::store::transcript_store::{LogFn, TranscriptStore};
use crate::util::{now_iso, random_uuid};

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
  /// Shared MCP servers for session/new, load and resume; None sends none
  pub shared_mcp: Option<crate::shared_config::McpProvider>,
  /// The `mcpServers` entry of Acpira's own MCP server (`host_mcp.rs`), sent with session/new, load and resume
  pub host_mcp: Option<crate::host_mcp::HostMcp>,
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
  pub prepared: crate::acp::session::attachments::PreparedPrompt,
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
  pub raw_questions: crate::acp::transcript::questions::RawMemory,
  pub tree: SubagentTree,
  // Prompt queue
  pub queue: Vec<QueuedEntry>,
  pub sending_id: Option<String>,
  /// The queued entry whose `_session/steering` request is on the wire; the queue holds its flush until the answer
  pub steering_id: Option<String>,
  /// The peer brackets its turns (Codex `threadStatus`), so a turn it starts on its own has an observable end
  pub thread_status_seen: bool,
  /// The running turn's peer already reported its thread idle: the prompt response is on its way and a steer would miss
  pub peer_idle: bool,
  /// A steer landed after the turn it aimed at had ended and the peer started a turn of its own (Codex `startedNewTurn`):
  /// the session stays running until the peer's thread reports idle
  pub detached: bool,
  /// A message accepted below a pre-send compaction: visible, but not the normalizer's last turn
  pub pending_prompt: Option<Turn>,
  pub building_plan: bool,
  pub compacted_at: Option<f64>,
  pub completion: Option<CompactionCompletion>,
  pub turn_failure: Option<SessionFailure>,
  pub auth_hint: Option<String>,
  /// The last restore met a native session lock held by an agent of another Acpira sidecar: its pid, for take_over
  pub lock_holder: Option<u32>,
  pub model_facts: ModelFacts,
  pub usage_revision: u64,
  pub usage_notifications: bool,
  pub auto_compact_eligible: bool,
  pub grok_usage_unavailable: bool,
  pub usage_timer: Option<tokio::task::AbortHandle>,
  pub usage_inflight: bool,
  pub pi_stamp: Option<crate::acp::vendors::pi_usage::Stamp>,
  /// Claude's last `usage_update.size` as the adapter sent it, before `claude_window` corrected it
  pub reported_window: Option<f64>,
  pub finish_usage_refresh: Option<oneshot::Sender<bool>>,
  pub syncing_thought: bool,
  pub adopting: bool,
  /// A new session's remembered choices are on screen and still to be replayed: prompts queue until `adopt_controls` ends
  pub adopt_pending: bool,
  /// The pick overlay entries (key, token) that hold remembered choices on screen while they are replayed
  pub holds: Vec<(String, u64)>,
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
      let ctx = ToolCtx {
        save_image: Some(image_saver(me.clone())),
        save_image_file: Some(file_image_saver(me.clone())),
        cwd: Some(record.cwd.clone()),
        ..Default::default()
      };
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
          steering_id: None,
          thread_status_seen: false,
          peer_idle: false,
          detached: false,
          pending_prompt: None,
          building_plan: false,
          compacted_at: None,
          completion: None,
          turn_failure: None,
          auth_hint: None,
          lock_holder: None,
          model_facts: ModelFacts::default(),
          usage_revision: 0,
          usage_notifications: false,
          auto_compact_eligible: false,
          grok_usage_unavailable: false,
          usage_timer: None,
          usage_inflight: false,
          pi_stamp: None,
          reported_window: None,
          finish_usage_refresh: None,
          syncing_thought: false,
          adopting: false,
          adopt_pending: false,
          holds: vec![],
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

  pub(crate) fn def(&self) -> crate::acp::agents::registry::AgentDef {
    self.deps.registry.get(&self.agent).cloned().unwrap_or_default()
  }


  /// Publish state, leaving updatedAt alone (streamed chunks must not reorder the list)
  pub(crate) fn touch(&self, c: &mut Core) {
    self.refine_controls(c);
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

  /// A queued prompt can join the running turn over `_session/steering` (see `vendors::steering::supported`)
  pub(crate) fn can_steer_of(&self, c: &Core) -> bool {
    c.proc.as_ref().is_some_and(|p| crate::acp::vendors::steering::supported(&p.init))
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
}

pub(crate) fn num(n: f64) -> Num {
  Num(n)
}
