//! One session = one agent subprocess + one transcript. State machine:
//! start → (resume | load | new) → ready ⇄ prompt / cancel; failed login → auth_required; unresumable → readonly;
//! dead process → error.
//!
//! All mutable state is `Core`, behind one short-held mutex. Waiting (a prompt on the wire, a permission card, a platform
//! round trip) happens with the lock released, so a `stop` never queues behind a running `send`. This module holds the
//! state and its construction; the lifecycle, the turn, the queue, the gates, the controls, history editing and the
//! view / record projections are further `impl AcpSession` blocks in the child modules

pub mod access;
pub mod account_switch;
pub mod attachments;
pub mod compaction;
pub mod context;
pub mod context_claude;
pub mod context_claude_sdk;
pub mod context_estimate;
pub mod controls;
pub mod edit;
pub mod errors;
pub mod failure;
pub mod gates;
pub mod goal;
pub mod handlers;
pub mod hooks;
pub mod host_sidekick;
pub mod images;
pub mod late_reply;
pub mod lifecycle;
pub mod plan_build;
pub mod plugin_card;
pub mod plugin_events;
pub mod plugin_hooks;
pub mod prompt;
pub mod queue;
pub mod relay;
pub mod remote;
pub mod restore_turns;
pub mod stand;
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
use acpira_shared::model_shapes::{ModelMemory, ModelShapes};
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
use crate::acp::transcript::normalize::{NormalizeState, ToolCtx, runtime_info_of, sync_idle};
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
  /// The saved account a new session of `agent` binds (the last one chosen); None = the CLI's own credentials
  fn default_account(&self, _agent: String) -> Option<String> {
    None
  }
}

#[derive(Debug, Clone, Copy)]
pub struct CompactionPolicy {
  /// 0 = the model's window: the session's reported context size less room for an answer
  pub at_tokens: f64,
  pub auto: bool,
}

/// What a view shows of the agent's prefs: its model shapes and model memory
pub type ViewPrefs = (Option<ModelShapes>, Option<ModelMemory>);

pub type OnChange = Arc<dyn Fn(&str, bool) + Send + Sync>;

/// What a session asks its manager while it runs. Each default is the inert answer: no automatic compaction, no plan
/// auto-approval, no saved model shapes, no leases (tests)
pub trait SessionPolicy: Send + Sync {
  /// The automatic-compaction setting; None: no policy, nothing compacts on its own
  fn compaction(&self) -> Option<CompactionPolicy> {
    None
  }
  /// Whether `agent`'s plan mode answers tool permission requests itself (`acpira.planAutoApprove`)
  fn plan_auto_approve(&self, _agent: &str) -> bool {
    false
  }
  /// The model shapes saved for `agent`
  fn model_shapes(&self, _agent: &str) -> Option<ModelShapes> {
    None
  }
  /// What the user last ran each of `agent`'s models with
  fn model_memory(&self, _agent: &str) -> Option<ModelMemory> {
    None
  }
  /// The CLIs a host-owned sidekick can run in, with the models each last offered (`host_sidekick.rs`)
  fn sidekick_sources(&self) -> BoxFuture<Vec<crate::acp::session::host_sidekick::SidekickSource>> {
    Box::pin(async { vec![] })
  }
  /// Option families hidden from the composer menus (`acpira.hiddenOptions`)
  fn hidden_options(&self) -> acpira_shared::settings::HiddenMap {
    Default::default()
  }
  /// Takes the session's cross-engine lease for a turn about to start (`store::session_lease`) and returns its pin; Err:
  /// the notice to show, another engine is running one or the lease could not be taken, and this turn must not start.
  /// None: no leases
  fn claim(&self, _session: &str) -> Option<Result<u64, String>> {
    None
  }
  /// The other folders of the project `cwd` belongs to (`SessionCategories::other_folders`), sent as ACP
  /// `additionalDirectories`; none outside a project of several folders
  fn other_folders(&self, _cwd: &str) -> Vec<String> {
    vec![]
  }
}

/// Every answer the inert default
pub struct InertPolicy;
impl SessionPolicy for InertPolicy {}

#[derive(Clone)]
pub struct SessionDeps {
  pub registry: Arc<AgentRegistry>,
  pub log: LogFn,
  /// (session id, running): must not block and must not lock the session back
  pub on_change: OnChange,
  pub blobs: Arc<TranscriptStore>,
  pub notify: Option<LogFn>,
  pub accounts: Option<Arc<dyn SessionAccountHooks>>,
  /// Compaction, plan auto-approval, model shapes and leases, asked of the manager (`InertPolicy` in tests)
  pub policy: Arc<dyn SessionPolicy>,
  pub pool: Option<Arc<AgentPool>>,
  /// Shared MCP servers for session/new, load and resume; None sends none
  pub shared_mcp: Option<crate::shared_config::McpProvider>,
  /// The plugins' hooks, chained after the project's for a session on this machine; None: no plugins (tests)
  pub plugin_hooks: Option<crate::hooks::PluginHooks>,
  /// The plugins' prompt folders, sent to the built-in agent when a session opens; None: no plugins (tests)
  pub plugin_prompts: Option<crate::hooks::PluginPrompts>,
  /// Acpira's data home, whose `prompts/` overrides the texts the engine writes for agents (`AcpSession::prompts`);
  /// None: the built-in texts and the project's (tests)
  pub data_home: Option<std::path::PathBuf>,
  /// The plugins' resident processes, which hear the session's events; None: no plugins (tests)
  pub plugin_runtime: Option<std::sync::Arc<crate::plugins::runtime::Runtimes>>,
  /// The `mcpServers` entry of Acpira's own MCP server (`host_mcp.rs`), sent with session/new, load and resume
  pub host_mcp: Option<crate::host_mcp::HostMcp>,
  /// SSH hosts: a session whose folder is on one runs the built-in agent's project tools there (`remote.rs`). None:
  /// no remote folders (tests, shells without the feature)
  pub remote: Option<Arc<crate::remote::connections::Connections>>,
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
  /// An edit sent while a turn ran stopped that turn and waits for it to settle: the queue holds until the edit takes the
  /// session (or gives up), else a queued prompt would take the freed turn first
  pub edit_waiting: bool,
  pub edit_notifications: Vec<Value>,
}

/// Where a session came from, when that still changes what it does: a fork's copied history not yet sent, an import's
/// first restore, and the agent's own titles being ignored
#[derive(Default)]
pub(crate) struct Lineage {
  /// The retained transcript has not reached the peer: its next prompt carries it as context.
  pub history_pending: bool,
  /// The peer lacks the full recorded history even after the handoff; keep self-read and the existing title.
  pub history_context: bool,
  pub forked_from: Option<ForkedFrom>,
  /// An imported native session not restored yet: the first restore prefers session/load and seals the replay
  pub import_pending: bool,
  pub imported_from: Option<ImportedFrom>,
  pub agent_title_muted: bool,
  /// The user renamed the session (`SessionRecord::renamed`): the agent's titles stay ignored for good
  pub renamed: bool,
}

impl Lineage {
  /// The transcript was cut back to `len` turns: a fork's copied prefix shrinks with it (-1 when none of it is left).
  /// The origin itself stays, since it still mutes the agent's titles
  pub fn truncated(&mut self, len: usize) {
    if len == 0 {
      self.history_pending = false;
    }
    if let Some(f) = self.forked_from.as_mut()
      && f.turn_index >= len as i64
    {
      f.turn_index = len as i64 - 1;
    }
  }
}

pub(crate) struct Core {
  pub account_id: Option<String>,
  pub updated_at: String,
  pub pinned: Option<bool>,
  pub archived: Option<bool>,
  pub category: Option<String>,
  pub acp_session_id: Option<String>,
  /// The native session came from session/new and no prompt has reached it yet. Claude Code has written no transcript,
  /// so a resume would fail (claude-agent-acp answers -32002 Resource not found) and an ultracode rebuild takes a fresh
  /// session/new; the workspace hooks hand such a peer their context files, since it has never seen them
  pub unprompted: bool,
  pub proc: Option<Arc<AgentProcess>>,
  /// Bumped with every process replacement: callbacks and answers of an older generation are ignored
  pub proc_gen: u64,
  pub task_peer: HashMap<String, String>,
  /// Claude's pending `ScheduleWakeup` (`vendors::claude_wakeup`): the session reads as background work until it fires
  pub wake: crate::acp::vendors::claude_wakeup::Wake,
  /// Antigravity's replies after a background command, read back from a replay (`late_reply.rs`)
  pub late: late_reply::LateReply,
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
  /// The `/goal …` command a goal control steered into the running turn over `_session/goal`: its echo is dropped like
  /// a sent prompt's (`goal::is_command_echo`); cleared when the turn settles
  pub goal_steered: Option<String>,
  /// The last turn ended on its own with the goal still active, on an agent that leaves continuing to the client
  /// (`vendors::goal::continue_prompt`): the goal's next turn goes out once nothing of the user's waits (`goal_wake`)
  pub goal_due: bool,
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
  /// The adapter was updated on disk under this process (`adapter_updated`): reopened as soon as nothing is in flight
  pub adapter_stale: bool,
  /// Summoned children's processes, rounds in flight and root calls waiting for their node (`relay.rs`)
  pub relays: crate::acp::session::relay::Relays,
  /// Claude's host-made Ultra effort level (`ultracode.rs`)
  pub ultracode: crate::acp::session::ultracode::UltracodeState,
  /// The host's own sidekick select, for a CLI without one (`host_sidekick.rs`)
  pub sidekick: crate::acp::session::host_sidekick::HostSidekick,
  /// The permission level split off a CLI's modes and the mode the CLI is in (`access.rs`)
  pub access: crate::acp::session::access::AccessState,
  /// Claude workflow agents' sidechain logs being read into their nodes (`workflow_logs.rs`)
  pub workflow_logs: crate::acp::session::workflow_logs::WorkflowLogs,
  /// The project's workspace hooks: the running turn's snapshot, rejected edits and the gate's rounds (`hooks.rs`)
  pub hooks: crate::acp::session::hooks::HookState,
  /// What the plugins' processes were last told about each tool row (`plugin_events.rs`)
  pub plugin_events: crate::acp::session::plugin_events::EventState,
  pub plugin_state: crate::acp::session::plugin_hooks::PluginState,
  /// The last encoded view for `(rev, status, running, (model_shapes, model_memory))`: selecting an unchanged live session reuses it without re-serializing
  pub cached_view: Option<(i64, SessionStatus, bool, ViewPrefs, acpira_shared::protocol::ViewJson, acpira_shared::session_patch::ViewParts)>,
  /// What the last view's turns were encoded from: the next view encodes only the turns and blocks that changed
  pub encoded: acpira_shared::session_patch::Reuse,
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
  /// The project stdio MCP servers on the folder's SSH host this session's agent was given, by name: the only commands
  /// an `acpira mcp-host` shim holding its grant can start there (`HubOp::HostStdio`)
  pub(crate) host_stdio: parking_lot::Mutex<std::collections::HashMap<String, remote::HostStdio>>,
  /// A stand-in CLI's calls of the project tools on its SSH host, by the model's call id, until their rows complete
  /// (`stand.rs`)
  pub(crate) remote_calls: parking_lot::Mutex<std::collections::HashMap<String, stand::RemoteCall>>,
  /// Those that came with no row to bind to (an MCP call ahead of its ACP `tool_call`), so a stop still reaches them
  pub(crate) loose_calls: parking_lot::Mutex<std::collections::HashMap<String, crate::acp::transport::cancel::Cancel>>,
  /// Codex in a folder on an SSH host: the way to its exec-server there, as long as its CLI runs (`remote_exec_env`)
  pub(crate) exec_tunnel: parking_lot::Mutex<Option<crate::remote::codex_exec::ExecTunnel>>,
  /// The `clientId`s of the actions the session ran, newest last (`claim_client_id`); saved with the record
  client_ids: parking_lot::Mutex<std::collections::VecDeque<String>>,
  /// The plugin host each card offered by `plugin_activate` came through, by card id (`plugin_card.rs`)
  pub(crate) plugin_offers: parking_lot::Mutex<std::collections::HashMap<String, std::sync::Arc<crate::plugins::host::PluginHost>>>,
  /// The machine whose built-in agent settings the session reads (`adopt_sender`); saved with the record
  settings_from: parking_lot::Mutex<Option<String>>,
  /// Who sent last since the running turn started: the next turn reads their settings (`sent_by`)
  next_from: parking_lot::Mutex<SentFrom>,
}

/// Where a turn came from, for the built-in agent settings the session reads from then on (`AcpSession::sent_by`).
/// Travels with an action forwarded over the push bus (`bus::Bus::act`)
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SentFrom {
  /// Not known (an action an older engine forwarded, a page no shell connects): the session keeps what it reads
  #[default]
  Unknown,
  /// A window of this machine: its own settings
  Here,
  /// A machine driving this one over SSH, by its id (`ClientInfo::machine`): the settings it brought
  Machine(String),
}

/// How many `clientId`s a session remembers: a device resends what it wrote in the last seconds before losing its link,
/// never dozens of actions later. Every record carries them, so the bound stays small
const CLIENT_IDS: usize = 32;

/// A fresh view epoch: every opened session gets one, a chain of read-only copies shares one (their revs keep growing)
pub fn next_view_epoch() -> u64 {
  static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
  NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl AcpSession {
  pub fn new(record: SessionRecord, deps: SessionDeps) -> Arc<AcpSession> {
    Self::build(record, deps, true, next_view_epoch())
  }

  /// A foreign copy is writable in the UI but stays process-free until the manager promotes it.
  pub fn standby(record: SessionRecord, deps: SessionDeps, rev: i64, epoch: u64) -> Arc<AcpSession> {
    let s = Self::build(record, deps, false, epoch);
    let mut c = s.core.lock();
    c.status = SessionStatus::Ready;
    c.rev = rev;
    drop(c);
    s
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
      let sidekick = crate::acp::session::host_sidekick::restored(&record.controls.options);
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
      let tree = SubagentTree::new(tree_log, ctx, record.subagents, Some(&record.updated_at), Vendor::of(&record.agent).resumed_child_logs());
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
          archived: record.archived,
          category: record.category,
          acp_session_id: record.acp_session_id,
          proc: None,
          proc_gen: 0,
          lease_pin: None,
          elsewhere: false,
          task_peer: HashMap::new(),
          wake: Default::default(),
          late: Default::default(),
          state,
          tree,
          status: SessionStatus::Starting,
          error: None,
          auth_methods: None,
          auth_hint: None,
          start_outcome: None,
          lock_holder: None,
          mcp_sent: vec![],
          unprompted: false,
          mcp_skip: vec![],
          reconnect_after_turn: false,
          retry_pending: false,
          retried_turn: None,
          phase: Phase::default(),
          replaying: false,
          startup_banner: None,
          goal_steered: None,
          goal_due: false,
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
            history_context: record.history_context || record.history_pending,
            agent_title_muted: record.forked_from.is_some() || record.history_context || record.history_pending,
            renamed: record.renamed,
            forked_from: record.forked_from,
            import_pending: record.import_pending,
            imported_from: record.imported_from,
          },
          rev: 0,
          switching: false,
          adapter_stale: false,
          relays: Default::default(),
          ultracode,
          sidekick,
          workflow_logs: Default::default(),
          access: Default::default(),
          hooks: Default::default(),
          plugin_events: Default::default(),
          plugin_state: plugin_hooks::PluginState { menus: record.plugin_menus, ..Default::default() },
          cached_view: None,
          encoded: Default::default(),
        }),
        deps,
        me: me.clone(),
        pick_lock: tokio::sync::Mutex::new(()),
        usage_lock: tokio::sync::Mutex::new(()),
        epoch,
        host_stdio: Default::default(),
        remote_calls: Default::default(),
        loose_calls: Default::default(),
        exec_tunnel: Default::default(),
        client_ids: parking_lot::Mutex::new(record.client_ids.iter().cloned().collect()),
        plugin_offers: Default::default(),
        settings_from: parking_lot::Mutex::new(record.settings_from),
        next_from: Default::default(),
      }
    })
  }

  /// Whether an action carrying `client_id` should run: a paired device that lost its link resends what it is unsure of
  /// with the same id, and each id runs once. Actions without one always run. Checked where the session is live, so an
  /// action forwarded over the push bus is counted by the engine that runs it. The ids are saved with the record: an
  /// engine that exited while the device was away (idle with no window) left them for the one the device wakes
  pub fn claim_client_id(&self, client_id: Option<&str>) -> bool {
    let Some(id) = client_id else { return true };
    let mut ids = self.client_ids.lock();
    if ids.iter().any(|x| x == id) {
      return false;
    }
    if ids.len() == CLIENT_IDS {
      ids.pop_front();
    }
    ids.push_back(id.to_owned());
    true
  }

  /// The machine whose built-in agent settings this session reads, None for this machine's own: the one that sent its
  /// last turn over SSH (`sent_by`)
  pub fn settings_from(&self) -> Option<String> {
    self.settings_from.lock().clone()
  }

  /// "Whoever sends decides": a message sent from another machine makes the next turn read that machine's built-in agent
  /// settings, one sent from a window here this machine's own. Noted only: the turn that starts next takes it
  /// (`adopt_sender`), so a send that queues or steers never changes the running turn, its staging or its children
  pub fn sent_by(&self, from: &SentFrom) {
    if self.agent != crate::acp::agents::registry::SELF_AGENT_ID || *from == SentFrom::Unknown {
      return;
    }
    *self.next_from.lock() = from.clone();
  }

  /// A turn starting (`claim`), or a session opened for a page before its `session/new`: whoever sent last since
  /// becomes the machine whose settings it reads
  pub fn adopt_sender(&self) {
    let from = std::mem::take(&mut *self.next_from.lock());
    match from {
      SentFrom::Unknown => {}
      SentFrom::Here => *self.settings_from.lock() = None,
      SentFrom::Machine(m) => *self.settings_from.lock() = Some(m),
    }
  }

  /// A built-in agent request names the machine whose settings the session reads (`CLIENT_SETTINGS_META`); a child of
  /// this session reads the same
  pub(crate) fn with_client_settings(&self, req: &mut serde_json::Value, agent: &str) {
    if agent == crate::acp::agents::registry::SELF_AGENT_ID
      && let Some(m) = self.settings_from()
    {
      req["_meta"][acpira_agent::acp::CLIENT_SETTINGS_META] = serde_json::Value::String(m);
    }
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
        archived: None,
        category: None,
        renamed: false,
        history_pending: false,
        history_context: false,
        forked_from: None,
        import_pending: false,
        imported_from: None,
        subagents: None,
        goal: None,
        copy_of: None,
        client_ids: Vec::new(),
        plugin_menus: Default::default(),
        settings_from: None,
      },
      deps,
    )
  }

  #[expect(clippy::expect_used, reason = "every session is built by Arc::new_cyclic and only reached through a live Arc, never from its Drop")]
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
    if !c.replaying {
      let bg = c.tree.any_running() || Self::relay_background(c) || c.wake.pending(crate::util::now_ms()).is_some();
      sync_idle(&mut c.state, bg);
    }
    self.refine_controls(c);
    c.rev += 1;
    c.cached_view = None;
    (self.deps.on_change)(&self.id, c.phase.running);
  }


  /// A user-initiated message moves the session to the top of the list, and back out of the archive
  pub(crate) fn bump(&self, c: &mut Core) {
    c.updated_at = now_iso();
    c.archived = None;
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
    if c.lease_pin.is_none()
      && let Some(pin) = self.deps.policy.claim(&self.id)
    {
      c.lease_pin = Some(pin?);
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
  /// turn the workspace gate is deciding about (its follow-up starts on the same pin), and so are the summoned children
  /// it left running: another engine must not take the session while they still write into it
  pub fn take_ended_lease(&self) -> Option<u64> {
    let mut c = self.core.lock();
    if c.phase.running || c.pending_prompt.is_some() || c.hooks.gating || Self::relay_background(&c) { None } else { c.lease_pin.take() }
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

  /// A queued prompt can join the running turn: over `_session/steering` (`vendors::steering::supported`), or as a
  /// `session/prompt` of its own on an agent that takes one as a steer (`Vendor::steers_by_prompt`)
  pub(crate) fn can_steer_of(&self, c: &Core) -> bool {
    c.proc.as_ref().is_some_and(|p| crate::acp::vendors::steering::supported(&p.init) || self.vendor.steers_by_prompt(&p.init))
  }

  /// A running command row can be stopped on its own (`stop_command`): stand-in sessions' project tools on an SSH host
  /// are the engine's own calls, any other command is found under the agent's process, which needs a Unix process table
  pub(crate) fn can_stop_commands_of(&self, c: &Core) -> bool {
    if self.remote_folder().is_some() {
      return self.local_run().is_some_and(|l| l.stand().is_some());
    }
    // A live process to search below: the session's own agent, or a summoned child's while its round runs
    cfg!(unix) && (c.proc.as_ref().is_some_and(|p| p.pid.is_some()) || c.relays.procs.values().any(|rp| rp.proc.pid.is_some()))
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

  /// The slash commands the agent advertised (available_commands_update), without the plugins' own
  pub fn agent_commands(&self) -> Vec<SlashCommand> {
    self.core.lock().state.commands.clone()
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
    c.lineage.renamed = true;
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

  /// Archiving keeps the session's time, pin and category, so bringing it back puts it where it was
  pub fn set_archived(&self, archived: bool) {
    let mut c = self.core.lock();
    c.archived = archived.then_some(true);
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
