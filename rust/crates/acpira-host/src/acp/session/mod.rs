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
pub mod goal;
pub mod handlers;
pub mod hooks;
pub mod images;
pub mod lifecycle;
pub mod plan_build;
pub mod prompt;
pub mod queue;
pub mod relay;
pub mod restore_turns;
pub mod tasks;
pub mod turn_usage;
pub mod ultracode;
pub mod updates;
pub mod usage;
pub mod view;
pub mod workflow_logs;

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use anyhow::Result;
use serde_json::Value;

use acpira_shared::inventory::{AgentHealthStage, AgentRuntimeInfo};
use acpira_shared::model_shapes::ModelShapes;
use acpira_shared::num::Num;
use acpira_shared::slash_commands::restore_command_receipts;
use acpira_shared::transcript::*;

use crate::acp::agents::model_sources::ModelFacts;
use crate::acp::agents::pool::AgentPool;
use crate::acp::agents::registry::AgentRegistry;
use crate::acp::session::compaction::CompactionState;
use crate::acp::session::controls::ControlPicks;
use crate::acp::session::gates::{PermissionGate, QuestionGate};
use crate::acp::session::queue::{PeerTurn, PromptQueue};
use crate::acp::session::usage::UsageTracker;
use crate::acp::session::failure::SessionFailure;
use crate::acp::session::images::{file_image_saver, image_saver};
use crate::acp::session::restore_turns::restore_interrupted_turns;
use crate::acp::transcript::normalize::{NormalizeState, ToolCtx, runtime_info_of};
use crate::acp::transcript::plan_snapshots::restore_plan_snapshots;
use crate::acp::transcript::subagent_tree::SubagentTree;
use crate::acp::transport::process::AgentProcess;
use crate::acp::vendors::Vendor;
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
  /// Whether an agent's plan mode answers tool permission requests itself (`acpira.planAutoApprove`); None: never
  pub plan_auto_approve: Option<Arc<dyn Fn(&str) -> bool + Send + Sync>>,
  pub pool: Option<Arc<AgentPool>>,
  pub model_shapes: Option<Arc<dyn Fn(&str) -> Option<ModelShapes> + Send + Sync>>,
  /// Shared MCP servers for session/new, load and resume; None sends none
  pub shared_mcp: Option<crate::shared_config::McpProvider>,
  /// The `mcpServers` entry of Acpira's own MCP server (`host_mcp.rs`), sent with session/new, load and resume
  pub host_mcp: Option<crate::host_mcp::HostMcp>,
  /// Takes the session's cross-engine lease for a turn about to start (`store::session_lease`) and returns its pin; Err:
  /// the notice to show, another engine is running one or the lease could not be taken, and this turn must not start.
  /// None: no leases (tests)
  pub claim: Option<Arc<dyn Fn(&str) -> Result<u64, String> + Send + Sync>>,
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

/// Where a session came from, when that still changes what it does: a fork's copied history not yet sent, an import's
/// first restore, and the agent's own titles being ignored
#[derive(Default)]
pub(crate) struct Lineage {
  /// A fork's copied transcript has never reached the peer: its first prompt carries it as retained context
  pub history_pending: bool,
  pub forked_from: Option<ForkedFrom>,
  /// An imported native session not restored yet: the first restore prefers session/load and seals the replay
  pub import_pending: bool,
  pub imported_from: Option<ImportedFrom>,
  pub agent_title_muted: bool,
}

pub(crate) struct Core {
  pub account_id: Option<String>,
  pub updated_at: String,
  pub pinned: Option<bool>,
  pub category: Option<String>,
  pub acp_session_id: Option<String>,
  pub proc: Option<Arc<AgentProcess>>,
  /// Bumped with every process replacement: callbacks and answers of an older generation are ignored
  pub proc_gen: u64,
  pub task_peer: HashMap<String, String>,
  pub state: NormalizeState,
  pub tree: SubagentTree,
  pub status: SessionStatus,
  pub error: Option<String>,
  pub auth_methods: Option<Vec<AuthMethodInfo>>,
  pub auth_hint: Option<String>,
  pub start_outcome: Option<StartOutcome>,
  /// The last restore met a native session lock held by an agent of another Acpira sidecar: its pid, for take_over
  pub lock_holder: Option<u32>,
  /// The cross-engine lease pin the current (or just ended) turn holds; given back through `take_ended_lease`
  pub lease_pin: Option<u64>,
  /// A read-only copy of a session another Acpira engine has open: `takeOverSession` asks that engine to let go
  pub elsewhere: bool,
  /// MCP server names the last session request carried, and those the agent could not start (left out from then on)
  pub mcp_sent: Vec<String>,
  pub mcp_skip: Vec<String>,
  /// The settled turn asks for a fresh connection before anything else is sent (a server just joined `mcp_skip`)
  pub reconnect_after_turn: bool,
  /// A Retry is sending its prompt: the agent turn that prompt opens becomes `retried_turn`
  pub retry_pending: bool,
  /// The agent turn (its start) the last Retry opened; retrying that turn again restarts the connection first
  pub retried_turn: Option<i64>,
  pub phase: Phase,
  pub replaying: bool,
  pub startup_banner: Option<String>,
  pub perms: PermissionGate,
  pub questions: QuestionGate,
  pub queue: PromptQueue,
  pub peer: PeerTurn,
  /// A message accepted below a pre-send compaction: visible, but not the normalizer's last turn
  pub pending_prompt: Option<Turn>,
  pub turn_failure: Option<SessionFailure>,
  pub building_plan: bool,
  pub compaction: CompactionState,
  pub usage: UsageTracker,
  pub model_facts: ModelFacts,
  pub picks: ControlPicks,
  pub lineage: Lineage,
  pub rev: i64,
  /// An automatic account switch is between the exhausted turn and its continue: prompts queue, manual switches wait
  pub switching: bool,
  /// Summoned children's processes, rounds in flight and root calls waiting for their node (`relay.rs`)
  pub relays: crate::acp::session::relay::Relays,
  /// Claude's host-made Ultra effort level (`ultracode.rs`)
  pub ultracode: crate::acp::session::ultracode::UltracodeState,
  /// Claude workflow agents' sidechain logs being read into their nodes (`workflow_logs.rs`)
  pub workflow_logs: crate::acp::session::workflow_logs::WorkflowLogs,
  /// The project's workspace hooks: the running turn's snapshot, rejected edits and the gate's rounds (`hooks.rs`)
  pub hooks: crate::acp::session::hooks::HookState,
}

pub struct AcpSession {
  pub id: String,
  pub agent: String,
  pub cwd: String,
  pub created_at: String,
  /// What the agent's adapter needs handled specially, resolved once from its id
  pub(crate) vendor: Vendor,
  pub(crate) core: parking_lot::Mutex<Core>,
  pub(crate) deps: SessionDeps,
  pub(crate) me: Weak<AcpSession>,
  // Serializes composer picks: rapid clicks collapse to the last value per control
  pub(crate) pick_lock: tokio::sync::Mutex<()>,
  // Serializes polled usage reads (Grok, Pi) so slow replies never overwrite a newer snapshot
  pub(crate) usage_lock: tokio::sync::Mutex<()>,
  /// Which line of views this instance's revs count in (`next_view_epoch`): a page holding a view of the same epoch can
  /// be sent a patch, any other one gets the whole view. A reopened session counts its revs from 0 again, so rev alone
  /// cannot tell two instances apart
  pub(crate) epoch: u64,
}

/// A fresh view epoch: every opened session gets one, a chain of read-only copies shares one (their revs keep growing)
pub fn next_view_epoch() -> u64 {
  static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
  NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl AcpSession {
  pub fn new(record: SessionRecord, deps: SessionDeps) -> Arc<AcpSession> {
    Self::build(record, deps, true, next_view_epoch())
  }

  /// A read-only copy of a record another engine is driving right now (`store::session_lease`): turns stay as that
  /// engine last saved them (a running turn is not sealed as interrupted), no agent is ever started, and `deps` should
  /// not save (the record belongs to the other engine). `note` is the notice the view shows. `rev` must grow from one
  /// copy to the next (pages drop a view whose rev is not newer) and stay below 0, where the live session opened after
  /// the mirror starts counting
  /// `elsewhere`: another engine has it open and can be asked to hand it over (not when the lease could not be read).
  /// `epoch` is shared by every copy of one mirror (`next_view_epoch`), so a viewer is patched from one copy to the next
  pub fn mirror(record: SessionRecord, deps: SessionDeps, note: String, rev: i64, elsewhere: bool, epoch: u64) -> Arc<AcpSession> {
    debug_assert!(rev < 0);
    let s = Self::build(record, deps, false, epoch);
    {
      let mut c = s.core.lock();
      c.status = SessionStatus::Readonly;
      c.error = Some(note);
      c.rev = rev;
      c.elsewhere = elsewhere;
    }
    s
  }

  fn build(record: SessionRecord, deps: SessionDeps, seal: bool, epoch: u64) -> Arc<AcpSession> {
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
      // A reopened session whose effort shows Ultra asks for ultracode from its first request
      let ultracode = crate::acp::session::ultracode::UltracodeState {
        on: crate::acp::vendors::claude_ultracode::shown_on(&record.controls.options),
        ..Default::default()
      };
      let turns = restore_command_receipts(restore_plan_snapshots(record.turns));
      let turns = if seal { restore_interrupted_turns(turns, &record.updated_at) } else { turns };
      let state = NormalizeState {
        turns,
        controls: record.controls,
        usage: record.usage,
        commands: record.commands,
        title: Some(record.title).filter(|t| !t.is_empty()),
        ctx: ctx.clone(),
        log: Some(tree_log.clone()),
        agent: Some(record.agent.clone()),
        goal: record.goal,
        ..Default::default()
      };
      let tree = SubagentTree::new(tree_log, ctx, record.subagents, Some(&record.updated_at));
      AcpSession {
        id: record.id,
        vendor: Vendor::of(&record.agent),
        agent: record.agent,
        cwd: record.cwd,
        created_at: record.created_at,
        core: parking_lot::Mutex::new(Core {
          account_id: record.account_id,
          updated_at: record.updated_at,
          pinned: record.pinned,
          category: record.category,
          acp_session_id: record.acp_session_id,
          proc: None,
          proc_gen: 0,
          lease_pin: None,
          elsewhere: false,
          task_peer: HashMap::new(),
          state,
          tree,
          status: SessionStatus::Starting,
          error: None,
          auth_methods: None,
          auth_hint: None,
          start_outcome: None,
          lock_holder: None,
          mcp_sent: vec![],
          mcp_skip: vec![],
          reconnect_after_turn: false,
          retry_pending: false,
          retried_turn: None,
          phase: Phase::default(),
          replaying: false,
          startup_banner: None,
          perms: PermissionGate::default(),
          questions: QuestionGate::default(),
          queue: PromptQueue::default(),
          peer: PeerTurn::default(),
          pending_prompt: None,
          turn_failure: None,
          building_plan: false,
          compaction: CompactionState::default(),
          usage: UsageTracker::default(),
          model_facts: ModelFacts::default(),
          picks: ControlPicks::default(),
          lineage: Lineage {
            history_pending: record.history_pending,
            agent_title_muted: record.forked_from.is_some(),
            forked_from: record.forked_from,
            import_pending: record.import_pending,
            imported_from: record.imported_from,
          },
          rev: 0,
          switching: false,
          relays: Default::default(),
          ultracode,
          workflow_logs: Default::default(),
          hooks: Default::default(),
        }),
        deps,
        me: me.clone(),
        pick_lock: tokio::sync::Mutex::new(()),
        usage_lock: tokio::sync::Mutex::new(()),
        epoch,
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
        category: None,
        history_pending: false,
        forked_from: None,
        import_pending: false,
        imported_from: None,
        subagents: None,
        goal: None,
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

  /// The lease check before a turn starts, called with the core locked (the hook never locks the session back) so the
  /// pin and `running` change together. A pin a finished turn still holds (its final save is not through yet) carries over
  pub(crate) fn lease_turn(&self, c: &mut Core) -> Result<(), String> {
    let Some(claim) = self.deps.claim.as_ref() else { return Ok(()) };
    if c.lease_pin.is_none() {
      c.lease_pin = Some(claim(&self.id)?);
    }
    Ok(())
  }

  /// A read-only copy of a session another engine has open, which that engine can be asked to hand over
  pub fn can_take_over_elsewhere(&self) -> bool {
    let c = self.core.lock();
    c.status == SessionStatus::Readonly && c.elsewhere
  }

  /// The lease pin, whatever the turn is doing: the session is closing
  pub fn drop_lease(&self) -> Option<u64> {
    self.core.lock().lease_pin.take()
  }

  /// The lease pin of a turn that has ended, taken out so it is released once (after the turn's final save); None while a
  /// turn runs or when no turn holds one. A prompt waiting on its pre-send compaction is still that turn, and so is a
  /// turn the workspace gate is deciding about (its follow-up starts on the same pin)
  pub fn take_ended_lease(&self) -> Option<u64> {
    let mut c = self.core.lock();
    if c.phase.running || c.pending_prompt.is_some() || c.hooks.gating { None } else { c.lease_pin.take() }
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

  /// The goal controls the running agent advertises (`vendors::goal::actions`)
  pub(crate) fn goal_actions_of(c: &Core) -> Option<Vec<GoalAction>> {
    c.proc.as_ref().and_then(|p| crate::acp::vendors::goal::actions(&p.init))
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

  /// Pinned and filed are exclusive: pinning takes the session out of its category
  pub fn set_pinned(&self, pinned: bool) {
    let mut c = self.core.lock();
    c.pinned = pinned.then_some(true);
    if pinned {
      c.category = None;
    }
    self.touch(&mut c);
  }

  /// Filing keeps the session's time (the list order does not jump) and unpins it
  pub fn set_category(&self, category: Option<String>) {
    let mut c = self.core.lock();
    if category.is_some() {
      c.pinned = None;
    }
    c.category = category;
    self.touch(&mut c);
  }
}

pub(crate) fn num(n: f64) -> Num {
  Num(n)
}
