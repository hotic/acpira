//! Master of all sessions: live processes, the summary list, the viewers; every
//! webview action enters here.
//!
//! Locking: the manager's own state is one short-held mutex that is never held while a session is locked or while
//! awaiting. A session's `on_change` only marks it dirty; one flush task turns dirty sessions into pushes, serializing a
//! view once for every viewer showing it

pub mod storage;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::agent_order::{AgentPrefs, arrange_agents};
use acpira_shared::chatgpt_integration::{ChatGptIntegrationStatus, ProjectMirrors};
use acpira_shared::export_transcript::{ExportInput, ExportLabels, export_file_name, export_markdown, export_text};
use acpira_shared::inventory::{AgentHealth, AgentHealthStage, AgentRuntimeInfo, HealthSource};
use acpira_shared::model_shapes::{is_model_control, learn_shape};
use acpira_shared::protocol::{
  AccountAction, AccountActionStatus, AccountActionVia, AddAccountVia, EditTurnRequest, ExportFormat, HostMsg, RawJson, SessionHit,
  WebviewMsg,
};
use acpira_shared::settings::{HiddenMap, in_workspace};
use acpira_shared::subagents::{StateSource, SubagentState};
use acpira_shared::transcript::*;
use acpira_shared::turn_settings::capture_turn_settings;

use crate::accounts::account_manager::{AccountHooks, AccountManager, RunInTerminal, Toast};
use crate::accounts::local::LocalAccounts;
use crate::acp::agents::login_path;
use crate::acp::agents::pool::AgentPool;
use crate::acp::agents::registry::AgentRegistry;
use crate::acp::agents::native_sessions::list_native_sessions;
use crate::acp::agents::probe_controls::{ProbeResult, probe_agent_controls};
use crate::acp::transport::rpc::BoxFuture;
use crate::acp::session::errors::error_text;
use crate::bus::{Bus, BusPeer, ViewFrame, apply_frame};
use acpira_shared::session_patch::ViewParts;
use crate::acp::session::{AcpSession, CompactionPolicy, SessionAccountHooks, SessionDeps, StartOutcome};
use crate::external::chatgpt_events::CHATGPT_ID;
use crate::external::chatgpt_store::ChatGptBridgeStore;
use crate::external::desktop_commander::desktop_commander_status;
use crate::i18n::{t, tp};
use crate::limits::RENAME_MAX;
use crate::store::record::{ForkedFrom, ImportedFrom, RecordSource, SessionRecord};
use crate::store::session_lease::{Claim, SessionLeases};
use crate::relay::wire::{SessionReadArgs, SessionSearchArgs};
use crate::session_library::{
  LIST_DEFAULT, LIST_MAX, LibraryCaller, LibraryFuture, SELF_READ, SessionLibrary, agent_ids, candidates, matches, render_list, resolve_ref,
  session_entry,
};
use crate::store::session_digest::{self, DigestMeta};
use crate::store::session_search::SessionSearch;
use crate::store::transcript_store::{LogFn, RecordLoadError, SessionPrefs, TranscriptStore, is_session_id, sort_index};
use crate::util::{clip, local_stamp, ms_of_iso, now_iso, random_uuid};

const TRASH_TTL: Duration = Duration::from_secs(30);
const PROBE_INTERVAL: Duration = Duration::from_secs(10);
/// Focus-triggered login shell re-runs are at least this far apart
const LOGIN_PATH_REFRESH: Duration = Duration::from_secs(30);
// How long shutdown waits for agent processes to close and exit; shells escalate to SIGTERM after 3 s, so this stays below it
const DISPOSE_GRACE: Duration = Duration::from_millis(2500);
const INDEX_DEBOUNCE: Duration = Duration::from_millis(400);
const INDEX_MAX_WAIT: Duration = Duration::from_millis(2000);
// Streamed updates coalesce into one push per session at this pace; an idle edge goes out at once
const PUSH_QUANTUM: Duration = Duration::from_millis(16);
// How often a mirrored session (driven by another engine) re-reads its record and checks whether the lease came free
const MIRROR_POLL: Duration = Duration::from_secs(1);
// How long a session another engine holds waits for that engine's pushed view before its record copy is shown. An owner on
// the bus answers a subscription with its whole view within a frame or two; without this wait the record copy (behind
// the live turn, with its "take over" card) flashed first on every switch to a session running elsewhere
const FIRST_PUSH_GRACE: Duration = Duration::from_millis(300);
// How often an engine looks for takeover requests on the sessions it has open
const TAKEOVER_POLL: Duration = Duration::from_millis(500);
// How long a takeover waits for the other engine to let go (it polls every TAKEOVER_POLL, then stops its turn)
const TAKEOVER_WAIT: Duration = Duration::from_secs(10);
// How long a yielding engine gives its cancelled turn to settle before closing the session anyway
const YIELD_SETTLE: Duration = Duration::from_secs(3);
// How long a record edit waits for another engine's short hold on the session (its load, its own edit) to end
const RECORD_HOLD_WAIT: Duration = Duration::from_secs(2);
// Mirror copies count their rev up from here: always rising, always below the 0 a live session starts at
const MIRROR_REV_BASE: i64 = -(1 << 40);
// A live session no viewer shows and with nothing in flight gives its agent process back after this long; opening it
// again resumes the native session (`release_idle`). Claude Code runs one CLI per session (150–300 MB each), so a window
// that went through a dozen conversations otherwise keeps a dozen of them alive until it closes
const IDLE_RELEASE: Duration = Duration::from_secs(20 * 60);
// Background sessions beyond the most recently seen IDLE_KEEP go sooner, once they have sat unseen for IDLE_KEEP_MIN
const IDLE_KEEP: usize = 4;
const IDLE_KEEP_MIN: Duration = Duration::from_secs(2 * 60);
// How often the idle sessions are looked at
const IDLE_SWEEP: Duration = Duration::from_secs(30);

fn safe_record_text(value: &str) -> String {
  let mut text: String = value.chars().filter(|c| !c.is_control()).take(160).collect();
  if value.chars().count() > 160 {
    text.push('…');
  }
  text
}

pub type Sink = Arc<dyn Fn(HostMsg) + Send + Sync>;

/// Why a session wants the user: a permission card or question appeared, or a turn settled (finished or failed)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttentionKind {
  Waiting,
  Done,
  Error,
}

#[derive(Debug, Clone)]
pub struct Attention {
  pub session_id: String,
  pub title: String,
  /// The agent's display name
  pub agent: String,
  pub kind: AttentionKind,
}

pub type AttentionFn = Arc<dyn Fn(Attention) + Send + Sync>;

/// How long a finished turn must stay settled before it is announced: a queued follow-up starts well within it
const ATTENTION_SETTLE: Duration = Duration::from_millis(1500);

/// A session's content at one moment (`SessionManager::snapshot`): its record, or a ChatGPT mirror's view
pub enum SessionSnapshot {
  Record(SessionRecord),
  Mirror(SessionView),
}

impl SessionSnapshot {
  pub fn title(&self) -> &str {
    match self {
      Self::Record(r) => &r.title,
      Self::Mirror(v) => &v.title,
    }
  }
  pub fn agent(&self) -> &AgentId {
    match self {
      Self::Record(r) => &r.agent,
      Self::Mirror(v) => &v.agent,
    }
  }
  pub fn cwd(&self) -> &str {
    match self {
      Self::Record(r) => &r.cwd,
      Self::Mirror(v) => &v.cwd,
    }
  }
  pub fn turns(&self) -> &[Turn] {
    match self {
      Self::Record(r) => &r.turns,
      Self::Mirror(v) => &v.turns,
    }
  }
  /// The JSON export: the record as stored, or the mirror's view
  pub fn json(&self) -> Result<String> {
    Ok(match self {
      Self::Record(r) => serde_json::to_string_pretty(r)?,
      Self::Mirror(v) => serde_json::to_string_pretty(v)?,
    })
  }
  /// A record for readers that take one (session_digest); a mirror keeps its view fields
  pub fn into_record(self) -> SessionRecord {
    match self {
      Self::Record(r) => r,
      Self::Mirror(v) => SessionRecord {
        id: v.id,
        agent: v.agent,
        account_id: v.account_id,
        acp_session_id: None,
        cwd: v.cwd,
        title: v.title,
        created_at: v.created_at,
        updated_at: v.updated_at,
        turns: v.turns,
        controls: v.controls,
        usage: v.usage,
        commands: v.commands,
        pinned: None,
        category: None,
        history_pending: false,
        forked_from: None,
        import_pending: false,
        imported_from: None,
        subagents: None,
        goal: v.goal,
      },
    }
  }
}

pub struct ManagerDeps {
  pub store: Arc<TranscriptStore>,
  pub chatgpt: Option<Arc<ChatGptBridgeStore>>,
  pub log: LogFn,
  pub cwd: Arc<dyn Fn() -> String + Send + Sync>,
  /// The shell has no single workspace and lists sessions by project folder (`ShellEnv.projects`): see `folder_for`
  pub projects: Arc<dyn Fn() -> bool + Send + Sync>,
  pub default_agent: Arc<dyn Fn() -> String + Send + Sync>,
  pub agent_prefs: Arc<dyn Fn() -> AgentPrefs + Send + Sync>,
  pub run_in_terminal: RunInTerminal,
  pub toast: Toast,
  pub accounts: Option<Arc<AccountManager>>,
  pub local_accounts: Option<Arc<LocalAccounts>>,
  pub compaction: Arc<dyn Fn() -> CompactionPolicy + Send + Sync>,
  /// Agents whose plan mode answers tool permission requests itself (`acpira.planAutoApprove`)
  pub plan_auto_approve: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
  pub hidden: Arc<dyn Fn() -> HiddenMap + Send + Sync>,
  pub scope: Arc<dyn Fn() -> String + Send + Sync>,
  /// Shared MCP servers handed to every session (`shared_config::mcp_provider`); None in tests that do not need them
  pub shared_mcp: Option<crate::shared_config::McpProvider>,
  /// Acpira's own MCP server (`acpira mcp`) for every session request; None without a known executable
  pub host_mcp: Option<crate::host_mcp::HostMcp>,
  /// ACPIRA_HOME: cross-engine session leases live under its `run/leases` (`store::session_lease`)
  pub lease_root: std::path::PathBuf,
  /// Join the engines' push bus under `lease_root` (`bus.rs`); off, sessions another engine owns are polled from disk
  pub bus: bool,
  /// Sessions that need the user or have settled (`watch_attention`); the runtime turns them into sound / system
  /// notifications per the settings, the shell decides whether the user is already looking. None: nobody listens
  pub attention: Option<AttentionFn>,
  /// SSH hosts (`remote/connections.rs`) for sessions whose folder is on one; None: no remote folders
  pub remote: Option<Arc<crate::remote::connections::Connections>>,
}

#[derive(Default)]
struct ViewerState {
  active_id: Option<String>,
  observing: Option<(String, String)>,
  last_subagent_rev: Option<i64>,
}

/// A mirrored session's view as its owner pushes it: rebuilt from the bus frames, then posted like a live view
struct Remote {
  // One line of views per subscription: a page takes the first one whole, then patches
  epoch: u64,
  parts: Option<ViewParts>,
  msg: Option<HostMsg>,
}

/// One per webview: its own active session over the shared pool and list
pub struct Viewer {
  id: u64,
  state: parking_lot::Mutex<ViewerState>,
  sink: parking_lot::Mutex<Option<Sink>>,
}

impl Viewer {
  pub fn active_id(&self) -> Option<String> {
    self.state.lock().active_id.clone()
  }

  pub fn subscribe(&self, sink: Sink) {
    *self.sink.lock() = Some(sink);
  }

  fn emit(&self, m: HostMsg) {
    let sink = self.sink.lock().clone();
    if let Some(s) = sink {
      s(m);
    }
  }
}

struct Trash {
  summary: SessionSummary,
  timer: tokio::task::AbortHandle,
}

struct State {
  registry: Arc<AgentRegistry>,
  registry_watch: u64,
  live: HashMap<String, Arc<AcpSession>>,
  index: Vec<SessionSummary>,
  trash: HashMap<String, Trash>,
  viewers: Vec<Arc<Viewer>>,
  loading: HashMap<String, tokio::sync::watch::Receiver<bool>>,
  account_actions: Vec<AccountAction>,
  probed: HashMap<String, ProbeResult>,
  health: HashMap<String, AgentHealth>,
  health_seen: HashMap<String, StartOutcome>,
  was_running: HashSet<String>,
  // Sessions whose turn ended while no viewer showed them; cleared once a viewer lands on one
  unread: HashSet<String>,
  // Attention edges (`watch_attention`): sessions inside a turn, sessions showing a card for the user, and a counter per
  // session so a settle check started for an earlier turn stays quiet once another turn has begun
  was_busy: HashSet<String>,
  was_waiting: HashSet<String>,
  settle_seq: HashMap<String, u64>,
  prefs: SessionPrefs,
  // The session list's user categories as last read from (or written to) categories.json
  categories: SessionCategories,
  probe_timer: Option<tokio::task::AbortHandle>,
  touched: HashSet<String>,
  disposed: bool,
  sync_due: Option<(tokio::time::Instant, tokio::time::Instant)>,
  sync_running: bool,
  sync_again: bool,
  dirty: HashMap<String, bool>,
  // Sessions another engine holds the lease of: read-only copies of their record, refreshed until the lease comes free
  mirrors: HashMap<String, Arc<AcpSession>>,
  // Mirrored sessions whose owner pushes their view over the bus (`bus.rs`): shown instead of the record copy, and the
  // pages' actions on them go to the owner
  remote: HashMap<String, Remote>,
  // Mirrored sessions to re-read on the next poll whatever the record's stamp (their pushed view just ended)
  refresh: HashSet<String>,
  // Since when each live session has been unseen with nothing in flight (`release_idle`)
  idle_since: HashMap<String, tokio::time::Instant>,
  // Released sessions whose old agent process is still closing: reopening one waits for it (`release`)
  closing: HashMap<String, tokio::sync::watch::Receiver<bool>>,
}

pub struct SessionManager {
  deps: ManagerDeps,
  state: parking_lot::Mutex<State>,
  pool: Arc<AgentPool>,
  wake: Arc<tokio::sync::Notify>,
  sync_lock: tokio::sync::Mutex<()>,
  // prefs.json writes run one at a time; dispose waits for the ones still in flight
  prefs_writer: tokio::sync::Mutex<()>,
  prefs_saves: parking_lot::Mutex<Vec<tokio::task::JoinHandle<()>>>,
  viewer_seq: std::sync::atomic::AtomicU64,
  // The list as last pushed: most changes (every streamed chunk) leave it as it was, and an identical list is not sent again
  last_sessions: parking_lot::Mutex<Option<Vec<SessionSummary>>>,
  // Extracted conversation text of the saved records, kept between history searches
  search: SessionSearch,
  // The last export per (session, format): an unchanged transcript hands back the same file instead of writing another one.
  // Held across the write, so rapid repeated clicks run one after another and the later ones hit the cache
  exports: tokio::sync::Mutex<HashMap<(String, ExportFormat), (u64, std::path::PathBuf)>>,
  // Which sessions this engine has open, visible to every other engine on the machine (`store::session_lease`)
  leases: SessionLeases,
  // Sessions being handed over to another engine right now (a takeover request is being honoured)
  yielding: parking_lot::Mutex<HashSet<String>>,
  // The other engines on this machine: pushes the views of sessions they show, runs the actions they forward
  bus: Bus,
  // Revs of every read-only view this engine shows (record copies and pushed views alike): one count, so a page never
  // takes a newer copy for an older one when a session moves between the two
  mirror_rev: std::sync::atomic::AtomicI64,
  // In-app agent installs (`acp/agents/installer.rs`), pushed to every webview as `agentInstalls`
  installs: Arc<crate::acp::agents::installer::Installs>,
  // Set once the first probe pass has run, which may be after `init` returned (it follows the login shell)
  first_probe: tokio::sync::watch::Sender<bool>,
  me: Weak<SessionManager>,
}

impl SessionManager {
  pub fn new(registry: Arc<AgentRegistry>, deps: ManagerDeps) -> Arc<Self> {
    let mgr = Arc::new_cyclic(|me: &Weak<SessionManager>| {
      let leases = SessionLeases::new(&deps.lease_root, deps.log.clone());
      let bus = Bus::new(&deps.lease_root, deps.log.clone());
      let reg_me = me.clone();
      let env_me = me.clone();
      let install_me = me.clone();
      let installs = crate::acp::agents::installer::Installs::new(Arc::new(move |list| {
        if let Some(m) = install_me.upgrade() {
          m.emit(HostMsg::AgentInstalls { installs: list });
        }
      }));
      let pool = AgentPool::new(
        Arc::new(move || reg_me.upgrade().map(|m| m.registry()).unwrap_or_else(|| Arc::new(AgentRegistry::new(&Value::Null)))),
        deps.log.clone(),
        Some(Arc::new(move |agent: String, account: String| {
          let me = env_me.clone();
          Box::pin(async move {
            let m = me.upgrade()?;
            let accounts = m.deps.accounts.clone()?;
            accounts.spawn_env_for(&agent, &account).await
          }) as BoxFuture<_>
        })),
      );
      SessionManager {
        state: parking_lot::Mutex::new(State {
          registry,
          registry_watch: 0,
          live: HashMap::new(),
          index: vec![],
          trash: HashMap::new(),
          viewers: vec![],
          loading: HashMap::new(),
          account_actions: vec![],
          probed: HashMap::new(),
          health: HashMap::new(),
          health_seen: HashMap::new(),
          was_running: HashSet::new(),
          unread: HashSet::new(),
          was_busy: HashSet::new(),
          was_waiting: HashSet::new(),
          settle_seq: HashMap::new(),
          prefs: SessionPrefs::default(),
          categories: SessionCategories::default(),
          probe_timer: None,
          touched: HashSet::new(),
          disposed: false,
          sync_due: None,
          sync_running: false,
          sync_again: false,
          dirty: HashMap::new(),
          mirrors: HashMap::new(),
          remote: HashMap::new(),
          refresh: HashSet::new(),
          idle_since: HashMap::new(),
          closing: HashMap::new(),
        }),
        deps,
        pool,
        wake: Arc::new(tokio::sync::Notify::new()),
        sync_lock: tokio::sync::Mutex::new(()),
        prefs_writer: tokio::sync::Mutex::new(()),
        prefs_saves: Default::default(),
        viewer_seq: Default::default(),
        last_sessions: Default::default(),
        search: SessionSearch::default(),
        exports: Default::default(),
        leases,
        yielding: Default::default(),
        bus,
        mirror_rev: std::sync::atomic::AtomicI64::new(MIRROR_REV_BASE),
        installs,
        first_probe: tokio::sync::watch::Sender::new(false),
        me: me.clone(),
      }
    });
    // A record another engine is driving is never overwritten from here (a stale copy of a session open in two engines)
    let guard_me = mgr.me.clone();
    mgr.deps.store.set_write_guard(Arc::new(move |id: &str| guard_me.upgrade().is_none_or(|m| !m.leases.held_elsewhere(id))));
    mgr.watch_takeovers();
    if mgr.deps.bus {
      let peer: Weak<dyn BusPeer> = mgr.me.clone();
      mgr.bus.start(peer);
    }
    if let Some(a) = &mgr.deps.accounts {
      let me = mgr.me.clone();
      a.subscribe(Arc::new(move |accounts| {
        if let Some(m) = me.upgrade() {
          m.emit(HostMsg::Accounts { accounts });
        }
      }));
      let me = mgr.me.clone();
      a.on_lock_change(Arc::new(move || {
        if let Some(m) = me.upgrade() {
          m.emit_agents();
        }
      }));
    }
    if let Some(l) = &mgr.deps.local_accounts {
      let me = mgr.me.clone();
      l.subscribe(Arc::new(move || {
        if let Some(m) = me.upgrade() {
          m.emit_agents();
        }
      }));
    }
    if let Some(c) = &mgr.deps.chatgpt {
      let me = mgr.me.clone();
      c.subscribe(Arc::new(move |ids| {
        let Some(m) = me.upgrade() else { return };
        m.emit_sessions();
        for id in ids {
          match m.deps.chatgpt.as_ref().and_then(|c| c.view(&id)) {
            Some(view) => {
              let raw = RawJson::new(&view);
              for v in m.viewers_on(&id, None) {
                v.emit(HostMsg::Session { session: raw.clone(), running: view.running, parts: None });
              }
            }
            None => {
              let m2 = m.clone();
              tokio::spawn(async move { m2.rehome(&id).await });
            }
          }
        }
      }));
    }
    let reg = mgr.registry();
    mgr.watch_registry(&reg);
    let weak = mgr.me.clone();
    tokio::spawn(async move { flush_loop(weak).await });
    let weak = mgr.me.clone();
    tokio::spawn(async move { idle_loop(weak).await });
    mgr
  }

  fn log(&self, line: &str) {
    (self.deps.log)(line);
  }

  fn log_login_path(&self) {
    let added = login_path::added();
    if !added.is_empty() {
      self.log(&format!("PATH from the login shell adds: {}", added.join(":")));
    }
  }

  fn record_load_message(&self, id: &str, error: &RecordLoadError) -> String {
    let reason = match error {
      RecordLoadError::InvalidId => t("host.recordInvalidIdReason"),
      RecordLoadError::Missing => t("host.recordMissingReason"),
      RecordLoadError::StoreUnavailable => t("host.recordStoreUnavailableReason"),
      RecordLoadError::Unreadable { reason } => {
        let reason = safe_record_text(reason);
        tp("host.recordUnreadableReason", &[("error", &reason)])
      }
      RecordLoadError::Corrupt { reason } => {
        let reason = safe_record_text(reason);
        tp("host.recordCorruptReason", &[("error", &reason)])
      }
      RecordLoadError::IdMismatch { actual } => {
        let actual = safe_record_text(actual);
        tp("host.recordMismatchReason", &[("actual", &actual)])
      }
    };
    self.record_failure_message(id, error.code(), &reason)
  }

  fn record_failure_message(&self, id: &str, code: &str, reason: &str) -> String {
    let short_id: String = id.chars().take(12).collect();
    let session = if short_id.len() < id.len() { format!("{short_id}…") } else { short_id };
    tp("host.recordLoadFailed", &[("reason", reason), ("code", code), ("id", &session)])
  }

  fn notify_record_load_failure(&self, id: &str, error: &RecordLoadError) {
    self.log(&format!("session {id}: {} ({})", error.code(), error.detail()));
    (self.deps.toast)("error", &self.record_load_message(id, error));
  }

  pub fn registry(&self) -> Arc<AgentRegistry> {
    self.state.lock().registry.clone()
  }

  pub fn store(&self) -> &Arc<TranscriptStore> {
    &self.deps.store
  }

  pub async fn init(self: &Arc<Self>) {
    if let Some(c) = &self.deps.chatgpt
      && let Err(e) = c.init(true).await
    {
      self.log(&format!("ChatGPT mirror init: {e}"));
    }
    // Whatever a crashed host left in the trash had its undo window closed with it
    self.deps.store.sweep_trash(TRASH_TTL).await;
    let index = self.deps.store.load_index().await.unwrap_or_default();
    let prefs = self.deps.store.load_prefs().await;
    let categories = self.deps.store.load_categories().await;
    {
      let mut st = self.state.lock();
      st.index = index;
      st.prefs = prefs;
      st.categories = categories;
    }
    self.seed_projects().await;
    self.backfill_models();
    // The login shell's PATH decides where CLIs are found, and running the user's rc files takes a while (~0.6 s for an
    // oh-my-zsh setup, up to `FIRST_WAIT`). Nothing the first window shows needs it: a stored session is shown from its
    // record and a fresh one before its agent starts, so init (and with it the engine's handshake) does not wait for the
    // shell. Agent lookups wait for the run instead (`AgentRegistry::locate`) and the first probe pass follows it in the
    // background; until then `available` is unknown, which pages treat as available. With no shell to wait for (turned
    // off, or already run in this process) the first pass is cheap and still lands before init returns
    login_path::start();
    if login_path::is_ready() {
      self.log_login_path();
      self.registry().probe_all().await;
      self.first_probe.send_replace(true);
      self.schedule_probe();
    } else {
      let me = self.me.clone();
      tokio::spawn(async move {
        let in_time = login_path::ready_within_first_wait().await;
        let Some(m) = me.upgrade() else { return };
        if in_time {
          m.log_login_path();
        } else {
          // A slow rc file: the first pass goes ahead with the inherited PATH and the run's directories count once it ends
          let me = m.me.clone();
          tokio::spawn(async move {
            login_path::ready().await;
            if let Some(m) = me.upgrade() {
              m.log_login_path();
              m.reprobe().await;
            }
          });
        }
        m.registry().probe_all().await;
        m.first_probe.send_replace(true);
        // Init went out before this first pass, which notifies nobody of itself
        m.emit_agents();
        m.schedule_probe();
      });
    }
    if let Some(a) = self.deps.accounts.clone() {
      tokio::spawn(async move {
        a.sync_local(None).await;
        a.refresh_quotas(None, false).await;
      });
    }
    if let Some(l) = self.deps.local_accounts.clone() {
      tokio::spawn(async move { l.refresh(None, false).await });
    }
    // Warming starts now either way: its binary lookup waits for the shell itself, and the first session takes the
    // process while it is still warming (`AgentPool::take`)
    let agent = self.default_agent();
    self.warm(&agent, None);
  }

  /// A project shell's first connect: the folders sessions already ran in, newest first, become the list's first order
  async fn seed_projects(&self) {
    if !(self.deps.projects)() || self.state.lock().categories.projects.is_some() {
      return;
    }
    let mut list = self.sessions();
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    // Folders a session merely ran in are not projects: the home directory (where windows without a workspace start)
    // and folders that are gone
    let home = crate::store::data_dir::home_dir();
    let cwds: Vec<String> = list
      .into_iter()
      .map(|s| s.cwd)
      .filter(|c| !c.is_empty() && std::path::Path::new(c) != home && (acpira_shared::remote::is_remote_folder(c) || std::path::Path::new(c).is_dir()))
      .collect();
    if cwds.is_empty() {
      return;
    }
    if let Err(e) = self.category_op(&CategoryOp::SeedProjects { cwds }).await {
      self.log(&format!("seeding the project list failed: {e}"));
    }
  }

  /// The folder a view means when something needs one: a new session, warming, a probe, native sessions, @ search, the
  /// settings page's project files, ChatGPT. An explicit folder wins; a shell with a workspace means its workspace (as it
  /// always did); a project shell (`ManagerDeps.projects`) means the session the view shows, else the newest session's
  /// folder, else its first registered project, else nothing: no session starts in the home directory on its own
  pub fn folder_for(&self, v: Option<&Viewer>, explicit: Option<String>) -> Option<String> {
    if let Some(dir) = explicit.filter(|d| !d.trim().is_empty()) {
      return Some(dir);
    }
    if !(self.deps.projects)() {
      return Some((self.deps.cwd)());
    }
    if let Some(dir) = v.and_then(|v| v.active_id()).and_then(|id| self.session_cwd(&id)) {
      return Some(dir);
    }
    if let Some(s) = self.sessions().into_iter().filter(|s| !s.cwd.is_empty()).max_by(|a, b| a.updated_at.cmp(&b.updated_at)) {
      return Some(s.cwd);
    }
    self.state.lock().categories.projects.as_ref().and_then(|p| p.first().cloned())
  }

  /// A folder a project shell starts something in joins its list (at the end) the first time
  async fn register_project(&self, cwd: &str) {
    if !(self.deps.projects)() || self.state.lock().categories.projects.as_ref().is_some_and(|p| p.iter().any(|x| x == cwd)) {
      return;
    }
    if let Err(e) = self.category_op(&CategoryOp::AddProject { cwd: cwd.to_owned() }).await {
      self.log(&format!("registering project {cwd} failed: {e}"));
    }
  }

  /// Resolves once the first probe pass has run: `init` may return before it (see there)
  pub async fn agents_probed(&self) {
    let _ = self.first_probe.subscribe().wait_for(|done| *done).await;
  }

  /// Same rule as `pickDefaultAgent` (packages/shared/agentOrder.ts): the configured agent unless it is switched off or its
  /// CLI is missing, then the first enabled installed agent (the built-in one leads the registry). A missing default
  /// with nothing installed to take over stays, so the error names it; an unknown id stays (a custom agent not probed yet)
  fn default_agent(&self) -> String {
    let preferred = (self.deps.default_agent)();
    let agents = self.agents();
    let Some(own) = agents.iter().find(|a| a.id == preferred) else { return preferred };
    let disabled = own.disabled == Some(true);
    if !disabled && own.available != Some(false) {
      return preferred;
    }
    let open: Vec<&AgentInfo> = agents.iter().filter(|a| a.external != Some(true) && a.disabled != Some(true)).collect();
    let installed = open.iter().find(|a| a.available != Some(false));
    installed.or(if disabled { open.first() } else { None }).map(|a| a.id.clone()).unwrap_or(preferred)
  }

  pub fn emit_agents(&self) {
    self.emit(HostMsg::Agents { agents: self.agents() });
  }

  fn watch_registry(self: &Arc<Self>, r: &Arc<AgentRegistry>) {
    let old = {
      let st = self.state.lock();
      (st.registry.clone(), st.registry_watch)
    };
    old.0.unsubscribe(old.1);
    let me = self.me.clone();
    let id = r.subscribe(Arc::new(move || {
      if let Some(m) = me.upgrade() {
        m.emit_agents();
        m.schedule_probe();
      }
    }));
    self.state.lock().registry_watch = id;
  }

  pub fn reprobe(self: &Arc<Self>) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move {
      // Back from a terminal with something still missing: the rc files may have just gained its directory
      if me.registry().missing() && login_path::refresh(LOGIN_PATH_REFRESH).await {
        me.log_login_path();
      }
      me.registry().probe_all().await;
      me.schedule_probe();
    })
  }

  /// Poll only while something is missing
  fn schedule_probe(self: &Arc<Self>) {
    let mut st = self.state.lock();
    if let Some(h) = st.probe_timer.take() {
      h.abort();
    }
    if st.disposed || !st.registry.missing() {
      return;
    }
    let weak = self.me.clone();
    let h = tokio::spawn(async move {
      tokio::time::sleep(PROBE_INTERVAL).await;
      if let Some(m) = weak.upgrade() {
        m.state.lock().probe_timer = None;
        m.reprobe().await;
      }
    });
    st.probe_timer = Some(h.abort_handle());
  }

  fn warm(&self, agent: &str, account: Option<String>) {
    if agent == CHATGPT_ID {
      return;
    }
    let Some(cwd) = self.folder_for(None, None) else { return };
    let acc = account.or_else(|| self.default_account(agent));
    self.pool.ensure(agent, &cwd, acc.as_deref());
  }

  fn default_account(&self, agent: &str) -> Option<String> {
    self.deps.accounts.as_ref().filter(|a| a.supports(agent)).and_then(|a| a.default_for(agent)).map(|a| a.id)
  }

  pub fn last_settings(&self, agent: &str) -> Option<TurnSettings> {
    self.state.lock().prefs.last_settings.get(agent).cloned()
  }

  fn remember(&self, s: &AcpSession) {
    let config = capture_turn_settings(&s.agent_controls()).config;
    {
      let mut st = self.state.lock();
      let cur = st.prefs.last_settings.entry(s.agent.clone()).or_default();
      cur.config = config;
    }
    self.save_prefs(&s.agent);
  }

  fn remember_mode(&self, agent: &str, mode: &str) {
    {
      let mut st = self.state.lock();
      let cur = st.prefs.last_settings.entry(agent.to_owned()).or_default();
      if cur.mode_id.as_deref() == Some(mode) {
        return;
      }
      cur.mode_id = Some(mode.to_owned());
    }
    self.save_prefs(agent);
  }

  /// A ready session's agent truth tells which parameters its current model comes with
  fn learn_shape_of(&self, s: &AcpSession) {
    if s.status() != SessionStatus::Ready {
      return;
    }
    let controls = s.agent_controls();
    let next = {
      let st = self.state.lock();
      learn_shape(st.prefs.model_shapes.as_ref().and_then(|m| m.get(&s.agent)), &controls)
    };
    let Some(next) = next else { return };
    self.state.lock().prefs.model_shapes.get_or_insert_with(Default::default).insert(s.agent.clone(), next);
    self.save_prefs(&s.agent);
  }

  fn save_prefs(&self, agent: &str) {
    let Some(me) = self.me.upgrade() else { return };
    let agent = agent.to_owned();
    let task = tokio::spawn(async move {
      // Each write takes the newest prefs when its turn comes, so a late one can never put back an older snapshot
      let _turn = me.prefs_writer.lock().await;
      let prefs = me.state.lock().prefs.clone();
      if let Err(e) = me.deps.store.save_prefs(&prefs, &[agent]).await {
        me.log(&format!("prefs save failed: {e}"));
      }
    });
    let mut saves = self.prefs_saves.lock();
    saves.retain(|h| !h.is_finished());
    saves.push(task);
  }

  /// The in-memory list changed: bring the disk index along shortly (debounced)
  fn save_index(&self) {
    let now = tokio::time::Instant::now();
    let spawn = {
      let mut st = self.state.lock();
      let (first, _) = st.sync_due.unwrap_or((now, now));
      let due = (now + INDEX_DEBOUNCE).min(first + INDEX_MAX_WAIT);
      let fresh = st.sync_due.is_none();
      st.sync_due = Some((first, due));
      fresh
    };
    if !spawn {
      return;
    }
    let weak = self.me.clone();
    tokio::spawn(async move {
      loop {
        let Some(m) = weak.upgrade() else { return };
        let due = match m.state.lock().sync_due {
          Some((_, d)) => d,
          None => return,
        };
        if due > tokio::time::Instant::now() {
          drop(m);
          tokio::time::sleep_until(due).await;
          continue;
        }
        m.state.lock().sync_due = None;
        m.sync_index().await;
        return;
      }
    });
  }

  /// Index entries without a model (written before the field existed, or rewritten by an older engine that drops it)
  /// learn it from their records in the background, so the list's mark flips to the model's vendor without each session
  /// being opened first. Reading every record's controls takes a while on a large store; the list shows meanwhile
  fn backfill_models(&self) {
    let ids: Vec<String> = self.state.lock().index.iter().filter(|s| s.model.is_none() && s.external.is_none()).map(|s| s.id.clone()).collect();
    if ids.is_empty() {
      return;
    }
    let me = self.me.clone();
    tokio::spawn(async move {
      let Some(store) = me.upgrade().map(|m| m.deps.store.clone()) else { return };
      let started = std::time::Instant::now();
      let found = store.read_models(&ids).await;
      let Some(m) = me.upgrade() else { return };
      m.log(&format!("model backfill: read {} records in {} ms", ids.len(), started.elapsed().as_millis()));
      if found {
        m.sync_index().await;
      }
    });
  }

  /// Reconcile the list with the sessions directory now
  pub async fn refresh_index(self: &Arc<Self>) {
    self.state.lock().sync_due = None;
    self.sync_index().await;
    self.sync_shapes().await;
    self.sync_categories().await;
    if let Some(c) = &self.deps.chatgpt {
      c.refresh().await;
    }
  }

  /// Categories another window edited reach this one's list
  async fn sync_categories(&self) {
    let disk = self.deps.store.load_categories().await;
    self.set_categories(disk);
  }

  /// Keep `list` and push it to every page when it differs from what they have
  fn set_categories(&self, list: SessionCategories) {
    {
      let mut st = self.state.lock();
      if st.categories == list {
        return;
      }
      st.categories = list.clone();
    }
    self.emit(HostMsg::Categories { categories: list });
  }

  pub fn categories(&self) -> SessionCategories {
    self.state.lock().categories.clone()
  }

  pub async fn category_op(&self, op: &CategoryOp) -> Result<()> {
    // A folder on an SSH host is listed in its one canonical spelling, so two spellings never make two projects
    let normalized;
    let op = match op {
      CategoryOp::AddProject { cwd } if acpira_shared::remote::is_remote_folder(cwd) => {
        let canonical = acpira_shared::remote::parse_remote_folder(cwd).map(|f| f.uri()).ok_or_else(|| anyhow!(tp("host.folderMissing", &[("folder", cwd)])))?;
        normalized = CategoryOp::AddProject { cwd: canonical };
        &normalized
      }
      other => other,
    };
    let list = self.deps.store.update_categories(op).await?;
    self.set_categories(list);
    Ok(())
  }

  /// A folder something is about to start in, as it will be stored: a local folder must exist; a folder on an SSH host
  /// (`ssh://<alias>/<path>`, docs/dev/remote.md) is put in its canonical spelling and must be a folder on the host,
  /// which is connected for that. `missing`: the message key for a folder that is not there
  pub(crate) async fn check_folder(&self, dir: &str, missing: &str) -> Result<String> {
    let gone = || anyhow!(tp(missing, &[("folder", dir)]));
    if !acpira_shared::remote::is_remote_folder(dir) {
      return if tokio::fs::metadata(dir).await.is_ok_and(|m| m.is_dir()) { Ok(dir.to_owned()) } else { Err(gone()) };
    }
    let folder = acpira_shared::remote::parse_remote_folder(dir).ok_or_else(gone)?;
    let connections = self.deps.remote.clone().ok_or_else(gone)?;
    let client = connections.ensure(&folder.alias, std::time::Duration::from_secs(30)).await?;
    let stat = client.call("stat", serde_json::json!({ "path": folder.path })).await.map_err(|_| gone())?;
    if stat.value.get("kind").and_then(Value::as_str) != Some("dir") {
      return Err(gone());
    }
    Ok(folder.uri())
  }

  /// A category id the session may be filed under: one that exists and belongs to the session's project
  fn category_for(&self, category: Option<String>, cwd: &str) -> Option<Option<String>> {
    match category {
      None => Some(None),
      Some(id) => self.state.lock().categories.categories.iter().any(|c| c.id == id && c.cwd == cwd).then_some(Some(id)),
    }
  }

  /// Model shapes another window learned reach this one's history editor
  async fn sync_shapes(&self) {
    let disk = self.deps.store.load_prefs().await.model_shapes.unwrap_or_default();
    let changed: Vec<String> = {
      let mut st = self.state.lock();
      let mine = st.prefs.model_shapes.clone().unwrap_or_default();
      let changed: Vec<String> = disk
        .iter()
        .filter(|(agent, models)| models.iter().any(|(model, shape)| mine.get(*agent).and_then(|m| m.get(model)) != Some(shape)))
        .map(|(a, _)| a.clone())
        .collect();
      if changed.is_empty() {
        return;
      }
      let shapes = st.prefs.model_shapes.get_or_insert_with(Default::default);
      for agent in &changed {
        let entry = shapes.entry(agent.clone()).or_default();
        for (k, v) in &disk[agent] {
          entry.insert(k.clone(), v.clone());
        }
      }
      changed
    };
    for s in self.live_sessions() {
      if changed.contains(&s.agent) {
        s.republish();
      }
    }
  }

  fn live_sessions(&self) -> Vec<Arc<AcpSession>> {
    self.state.lock().live.values().cloned().collect()
  }

  /// A session open in this engine, for callers outside the manager (a relay grant in the engine tests)
  pub fn live_session(&self, id: &str) -> Option<Arc<AcpSession>> {
    self.live(id)
  }

  fn live(&self, id: &str) -> Option<Arc<AcpSession>> {
    self.state.lock().live.get(id).cloned()
  }

  /// One run at a time; a request mid-run schedules exactly one more. A live session whose record left the directory
  /// although the store once had it was deleted by another window: this host follows suit
  fn sync_index(self: &Arc<Self>) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move {
      let busy = {
        let mut st = me.state.lock();
        if st.sync_running {
          st.sync_again = true;
        } else {
          st.sync_running = true;
        }
        st.sync_again
      };
      if busy {
        let _wait = me.sync_lock.lock().await;
        return;
      }
      let _serial = me.sync_lock.lock().await;
      loop {
        let (mut mine, own) = {
          let mut st = me.state.lock();
          st.sync_again = false;
          let mut own: HashSet<String> = st.live.keys().cloned().collect();
          own.extend(st.touched.drain());
          (st.index.clone(), own)
        };
        // A live session's latest change may still wait for its flush: the shared file gets its current summary, not the stale copy
        for s in me.live_sessions() {
          let sum = s.summary();
          match mine.iter().position(|x| x.id == s.id) {
            Some(i) => mine[i] = sum,
            None => mine.push(sum),
          }
        }
        match me.deps.store.sync_index(&mine, &own).await {
          Ok(merged) => {
            let trash: HashSet<String> = me.state.lock().trash.keys().cloned().collect();
            let mut merged: Vec<SessionSummary> = merged.into_iter().filter(|s| !trash.contains(&s.id)).collect();
            let live = me.live_sessions();
            let mut gone: Vec<String> =
              live.iter().filter(|s| me.deps.store.knew(&s.id) && !merged.iter().any(|m| m.id == s.id)).map(|s| s.id.clone()).collect();
            // A mirrored session deleted by the engine driving it goes the same way
            let mirrors: Vec<String> = me.state.lock().mirrors.keys().cloned().collect();
            gone.extend(mirrors.into_iter().filter(|id| !merged.iter().any(|m| &m.id == id)));
            for id in &gone {
              me.forget(id);
            }
            for s in me.live_sessions() {
              let sum = s.summary();
              match merged.iter().position(|x| x.id == s.id) {
                Some(i) => merged[i] = sum,
                None => merged.push(sum),
              }
            }
            sort_index(&mut merged);
            let changed = {
              let mut st = me.state.lock();
              let changed = st.index != merged;
              if changed {
                st.index = merged;
              }
              changed
            };
            if changed {
              me.emit_sessions();
            }
            for id in gone {
              if !me.viewers_on(&id, None).is_empty() {
                (me.deps.toast)("info", &t("host.deletedElsewhere"));
              }
              me.rehome(&id).await;
            }
          }
          Err(e) => me.log(&format!("index sync failed: {e}")),
        }
        let again = {
          let mut st = me.state.lock();
          let again = st.sync_again;
          if !again {
            st.sync_running = false;
          }
          again
        };
        if !again {
          break;
        }
      }
    })
  }

  /// Swap the registry (acpira.agents changed): drop warm processes started with the old command, re-probe, re-push
  pub fn set_registry(self: &Arc<Self>, r: Arc<AgentRegistry>) {
    self.watch_registry(&r);
    self.state.lock().registry = r;
    self.pool.invalidate(None);
    let me = self.clone();
    tokio::spawn(async move {
      me.reprobe().await;
      me.emit_agents();
    });
  }

  /// Agents started from now on get a fresh process (the network route changed); running sessions keep theirs
  pub fn invalidate_warm_pool(&self) {
    self.pool.invalidate(None);
  }

  /// Every agent's latest in-app install, for the init state
  pub fn agent_installs(&self) -> Vec<acpira_shared::protocol::AgentInstallProgress> {
    self.installs.list()
  }

  pub fn agents(&self) -> Vec<AgentInfo> {
    let registry = self.registry();
    let mut all: Vec<AgentInfo> = registry
      .list()
      .into_iter()
      .filter(|a| self.deps.chatgpt.is_none() || a.id != CHATGPT_ID)
      .map(|mut a| {
        if let Some(m) = self.deps.accounts.as_ref().filter(|m| m.supports(&a.id)) {
          a.accounts = Some(true);
          a.credentials_locked = m.credentials_locked(&a.id).then_some(true);
        } else {
          a.local_account = self.deps.local_accounts.as_ref().and_then(|l| l.get(&a.id));
        }
        a
      })
      .collect();
    if self.deps.chatgpt.is_some() {
      all.push(AgentInfo {
        id: CHATGPT_ID.into(),
        name: "ChatGPT".into(),
        external: Some(true),
        available: Some(true),
        ..Default::default()
      });
    }
    arrange_agents(all, &(self.deps.agent_prefs)())
  }

  pub fn accounts(&self) -> Vec<AccountInfo> {
    self.deps.accounts.as_ref().map(|a| a.list()).unwrap_or_default()
  }

  pub fn account_actions(&self) -> Vec<AccountAction> {
    self.state.lock().account_actions.clone()
  }

  fn set_account_action(&self, action: AccountAction) {
    let actions = {
      let mut st = self.state.lock();
      match st.account_actions.iter().position(|a| a.agent == action.agent) {
        Some(i) => st.account_actions[i] = action,
        None => st.account_actions.push(action),
      }
      st.account_actions.clone()
    };
    self.emit(HostMsg::AccountActions { actions });
  }

  pub fn runtime_info(&self, agent: &str) -> Option<AgentRuntimeInfo> {
    for s in self.live_sessions() {
      if s.agent == agent
        && let Some(info) = s.runtime_info()
      {
        return Some(info);
      }
    }
    self.state.lock().probed.get(agent).map(|p| p.runtime.clone())
  }

  pub fn agent_health(&self, agent: &str) -> Option<AgentHealth> {
    self.state.lock().health.get(agent).cloned()
  }

  pub fn hidden(&self) -> HiddenMap {
    (self.deps.hidden)()
  }

  pub fn emit_hidden(&self) {
    self.emit(HostMsg::Hidden { hidden: self.hidden() });
  }

  /// The configOptions an agent offered most recently
  pub async fn known_controls(&self, agent: &str) -> Vec<ConfigControl> {
    if let Some(p) = self.state.lock().probed.get(agent).filter(|p| !p.options.is_empty()) {
      return p.options.clone();
    }
    let index = self.state.lock().index.clone();
    for s in index.iter().filter(|s| s.agent == agent) {
      let options = match self.live(&s.id) {
        Some(l) => l.agent_controls().options,
        None => self.deps.store.load_controls(&s.id).await.map(|c| c.options).unwrap_or_default(),
      };
      if !options.is_empty() {
        return options;
      }
    }
    vec![]
  }

  /// What a new session of this agent paints before session/new: the probe's or a stored session's options, and the modes
  /// a stored session showed (protocol modes are never probed)
  async fn known_start_controls(&self, agent: &str) -> SessionControls {
    let probed = self.state.lock().probed.get(agent).filter(|p| !p.options.is_empty()).map(|p| p.options.clone());
    let index = self.state.lock().index.clone();
    let mut out = SessionControls::default();
    for s in index.iter().filter(|s| s.agent == agent) {
      let controls = match self.live(&s.id) {
        Some(l) => l.agent_controls(),
        None => self.deps.store.load_controls(&s.id).await.unwrap_or_default(),
      };
      if out.modes.is_empty() && !controls.modes.is_empty() {
        out.modes = controls.modes;
        out.mode_config_id = controls.mode_config_id;
      }
      if !controls.options.is_empty() && probed.is_none() {
        out.options = controls.options;
        break;
      }
      // The probe has the options: the newest session's modes are enough, older records are not worth loading
      if probed.is_some() {
        break;
      }
    }
    if let Some(options) = probed {
      out.options = options;
    }
    out
  }

  /// The settings page's refresh button: a throwaway spawn reads the CLI's current configOptions
  pub async fn probe_controls(self: &Arc<Self>, agent: &str) -> Vec<ConfigControl> {
    if agent == CHATGPT_ID {
      return self.known_controls(agent).await;
    }
    self.pool.invalidate(Some(agent));
    let registry = self.registry();
    let Ok(def) = registry.get(agent).cloned() else { return self.known_controls(agent).await };
    let Some(bin) = registry.resolve_binary(agent).await else {
      self.log(&format!("probe {agent}: no binary"));
      return self.known_controls(agent).await;
    };
    let acc = self.default_account(agent);
    let env = match (&acc, &self.deps.accounts) {
      (Some(a), Some(m)) => m.spawn_env_for(agent, a).await,
      _ => None,
    };
    // A project shell without any project probes in the home directory, like a shell without a workspace always did
    let cwd = self.folder_for(None, None).unwrap_or_else(|| (self.deps.cwd)());
    match probe_agent_controls(&def, &bin, &cwd, env.as_ref(), self.deps.log.clone(), None).await {
      Ok(r) => {
        let options = r.options.clone();
        {
          let mut st = self.state.lock();
          st.probed.insert(agent.to_owned(), r);
          st.health.insert(
            agent.to_owned(),
            AgentHealth { stage: AgentHealthStage::Ready, at: now_iso(), error: None, source: HealthSource::Probe },
          );
        }
        self.warm(agent, acc);
        options
      }
      Err(e) => {
        self.state.lock().health.insert(
          agent.to_owned(),
          AgentHealth { stage: e.stage, at: now_iso(), error: Some(e.message.clone()), source: HealthSource::Probe },
        );
        self.log(&format!("probe {agent} failed: {}", e.message));
        self.known_controls(agent).await
      }
    }
  }

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
    let Some(cwd) = cwd else { return Ok(vec![]) };
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
      category: None,
      history_pending: false,
      forked_from: None,
      import_pending: true,
      imported_from: Some(ImportedFrom { session_id: session_id.to_owned() }),
      subagents: None,
      goal: None,
    };
    if let Err(e) = self.deps.store.flush(Arc::new(record.clone())).await {
      self.log(&format!("import flush failed: {e}"));
    }
    self.drop_empty_current(v).await;
    let s = AcpSession::new(record, self.session_deps());
    self.state.lock().live.insert(s.id.clone(), s.clone());
    self.set_active(v, Some(s.id.clone()));
    self.process_change(&s.id);
    s.start().await;
    // The agent restored the native context but replayed nothing: say what happened
    if s.status() == SessionStatus::Ready && s.turn_count() == 0 {
      (self.deps.toast)("info", &t("host.importNoHistory"));
    }
  }

  pub fn sessions(&self) -> Vec<SessionSummary> {
    let (index, live, unread) = {
      let st = self.state.lock();
      (st.index.clone(), st.live.clone(), st.unread.clone())
    };
    let mut merged: Vec<SessionSummary> = index
      .into_iter()
      .map(|mut s| {
        let live_state = live.get(&s.id).and_then(|l| l.list_state());
        let seen = !unread.contains(&s.id);
        s.state = live_state.or_else(|| (!seen).then_some(SummaryState::Unread));
        // A reply that came in while background work runs: the ring brightens instead of the finished dot showing
        s.unread = (live_state == Some(SummaryState::Background) && !seen).then_some(true);
        s
      })
      .collect();
    if let Some(c) = &self.deps.chatgpt {
      merged.extend(c.summaries());
    }
    sort_index(&mut merged);
    merged
  }

  /// The serialized view of a session (live, or a ChatGPT mirror)
  pub fn view_of(&self, id: Option<&str>) -> Option<(RawJson, bool)> {
    let id = id?;
    if self.live(id).is_none()
      && let Some(HostMsg::Session { session, running, .. }) = self.remote_msg(id)
    {
      return Some((session, running));
    }
    if let Some(s) = self.live(id).or_else(|| self.mirror_of(id)) {
      return Some(s.view_json());
    }
    let view = self.deps.chatgpt.as_ref()?.view(id)?;
    Some((RawJson::new(&view), view.running))
  }

  /// The session push for one viewer, as `emit_session` would send it
  pub fn session_msg(&self, id: Option<&str>) -> Option<HostMsg> {
    let id = id?;
    if self.live(id).is_none()
      && let Some(m) = self.remote_msg(id)
    {
      return Some(m);
    }
    if let Some(s) = self.live(id).or_else(|| self.mirror_of(id)) {
      return Some(s.view_msg());
    }
    let (session, running) = self.view_of(Some(id))?;
    Some(HostMsg::Session { session, running, parts: None })
  }

  fn has_view(&self, id: &str) -> bool {
    self.live(id).is_some() || self.mirror_of(id).is_some() || self.deps.chatgpt.as_ref().is_some_and(|c| c.view(id).is_some())
  }

  fn most_recent(&self) -> Option<String> {
    let scope = (self.deps.scope)();
    let cwd = (self.deps.cwd)();
    self.sessions().into_iter().find(|s| scope == "all" || in_workspace(&s.cwd, &cwd)).map(|s| s.id)
  }

  /// Attach a viewer: `initial` is the session it opens on, or the newest listed one
  pub fn attach(&self, initial: Option<acpira_shared::sidecar::InitialView>) -> Arc<Viewer> {
    use acpira_shared::sidecar::InitialView;
    let id = match initial {
      Some(InitialView::Id(id)) => Some(id),
      Some(InitialView::MostRecent { most_recent: true }) => self.most_recent(),
      _ => None,
    };
    let mut st = self.state.lock();
    if let Some(id) = &id {
      st.unread.remove(id);
    }
    let v = Arc::new(Viewer {
      id: self.viewer_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
      state: parking_lot::Mutex::new(ViewerState { active_id: id, ..Default::default() }),
      sink: Default::default(),
    });
    st.viewers.push(v.clone());
    v
  }

  pub fn detach(&self, v: &Viewer) {
    *v.sink.lock() = None;
    self.state.lock().viewers.retain(|x| x.id != v.id);
  }

  fn viewers(&self) -> Vec<Arc<Viewer>> {
    self.state.lock().viewers.clone()
  }

  pub fn emit(&self, m: HostMsg) {
    for v in self.viewers() {
      v.emit(m.clone());
    }
  }

  fn emit_session(&self, s: &AcpSession) {
    // A mirrored session whose owner pushes its view: that view, not the record copy
    if self.live(&s.id).is_none()
      && let Some(m) = self.remote_msg(&s.id)
    {
      for v in self.viewers_on(&s.id, None) {
        v.emit(m.clone());
      }
      return;
    }
    let mut view: Option<HostMsg> = None;
    for v in self.viewers() {
      if v.active_id().as_deref() != Some(s.id.as_str()) {
        continue;
      }
      v.emit(view.get_or_insert_with(|| s.view_msg()).clone());
      let obs = v.state.lock().observing.clone();
      let Some((sid, sub)) = obs.filter(|(sid, _)| *sid == s.id) else { continue };
      if let Some((turns, rev, running)) = s.subagent_transcript(&sub) {
        let fresh = {
          let mut vs = v.state.lock();
          let fresh = vs.last_subagent_rev != Some(rev);
          if fresh {
            vs.last_subagent_rev = Some(rev);
          }
          fresh
        };
        if fresh {
          v.emit(HostMsg::Subagent { session_id: sid, subagent_id: sub, rev, running, turns });
        }
      }
    }
    // Shown by another engine too: the same view goes out over the bus
    if self.bus.watching(&s.id)
      && self.live(&s.id).is_some_and(|l| std::ptr::eq(&*l, s))
      && let HostMsg::Session { parts: Some(p), .. } = view.unwrap_or_else(|| s.view_msg())
    {
      self.bus.publish(&s.id, &p);
    }
  }

  /// The pushed view of a mirrored session, once its owner sent one. Only while the mirror is there: a deletion or a
  /// reconcile that drops the mirror ends the pushed view with it, before the mirror's poll unsubscribes
  fn remote_msg(&self, id: &str) -> Option<HostMsg> {
    let st = self.state.lock();
    if !st.mirrors.contains_key(id) {
      return None;
    }
    st.remote.get(id).and_then(|r| r.msg.clone())
  }

  fn next_mirror_rev(&self) -> i64 {
    self.mirror_rev.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
  }

  /// A mirror is no longer shown or no longer a mirror: its owner stops pushing to this engine
  fn end_remote(&self, id: &str) {
    self.state.lock().remote.remove(id);
    self.bus.unsubscribe(id);
  }

  /// A page's action on a session another engine owns and pushes: run there
  async fn forward(&self, id: &str, m: &WebviewMsg) -> Result<()> {
    let msg = serde_json::to_value(m)?;
    self.bus.act(id, msg).await.map_err(|e| anyhow!(e))
  }

  /// Run an action a page of another engine took on a session this one owns, as if from a page showing it here
  async fn run_forwarded(self: Arc<Self>, session: String, msg: Value) -> Result<(), String> {
    let m: WebviewMsg = serde_json::from_value(msg).map_err(|e| e.to_string())?;
    if !is_forwarded(&m) || session_id_of(&m).is_some_and(|id| id != session) {
      return Err(format!("not an action on {session}"));
    }
    if self.live(&session).is_none() {
      return Err(t("history.unavailable"));
    }
    // A page of its own that nobody sees: not in the viewer list, so it neither receives pushes nor keeps the session here
    let v = Arc::new(Viewer {
      id: self.viewer_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
      state: parking_lot::Mutex::new(ViewerState { active_id: Some(session), ..Default::default() }),
      sink: Default::default(),
    });
    self.dispatch(&v, m).await.map_err(|e| error_text(&e))
  }

  /// Pages that join later get the list in their init state, so skipping an unchanged list leaves none behind. Built and
  /// posted under one lock: two racing callers can never post an older list after a newer one
  fn emit_sessions(&self) {
    let mut last = self.last_sessions.lock();
    let sessions = self.sessions();
    if last.as_ref() == Some(&sessions) {
      return;
    }
    *last = Some(sessions.clone());
    self.emit(HostMsg::Sessions { sessions });
  }

  /// The current list for one page, taken under the same lock as `emit_sessions`: whatever `send` queues lands after any
  /// list already emitted and before any later one, so the page cannot end on an older list that the next emit skips
  pub fn sessions_in_order(&self, send: impl FnOnce(Vec<SessionSummary>)) {
    let _order = self.last_sessions.lock();
    send(self.sessions());
  }

  /// Landing on a session also reads it; callers push the session list afterwards
  fn set_active(&self, v: &Viewer, id: Option<String>) {
    if let Some(id) = &id {
      self.state.lock().unread.remove(id);
    }
    let mut st = v.state.lock();
    if st.active_id != id {
      st.observing = None;
      st.last_subagent_rev = None;
    }
    st.active_id = id;
  }

  fn viewers_on(&self, id: &str, except: Option<&Viewer>) -> Vec<Arc<Viewer>> {
    self.viewers().into_iter().filter(|v| except.is_none_or(|e| e.id != v.id) && v.active_id().as_deref() == Some(id)).collect()
  }

  pub(crate) fn mark_dirty(&self, id: &str, running: bool) {
    let mut st = self.state.lock();
    let prev = st.dirty.insert(id.to_owned(), running);
    // Every change reports here, so a turn shorter than one flush quantum still leaves its running edge for the idle check
    if running {
      st.was_running.insert(id.to_owned());
      st.was_busy.insert(id.to_owned());
    }
    drop(st);
    // A fresh entry or an idle edge wakes the flusher at once
    if prev.is_none() || !running {
      self.wake.notify_one();
    }
  }

  /// What TS onChange did, for one session: list entry, disk, pushes, health, quota, warm pool
  fn process_change(&self, id: &str) {
    let Some(s) = self.live(id) else { return };
    self.learn_shape_of(&s);
    let sum = s.summary();
    {
      let mut st = self.state.lock();
      match st.index.iter().position(|x| x.id == sum.id) {
        Some(i) => st.index[i] = sum,
        None => st.index.insert(0, sum),
      }
      sort_index(&mut st.index);
    }
    self.deps.store.save(s.clone() as Arc<dyn RecordSource>);
    self.save_index();
    let running = s.is_running();
    let idle_edge = {
      let mut st = self.state.lock();
      if running {
        st.was_running.insert(id.to_owned());
        false
      } else {
        st.was_running.remove(id)
      }
    };
    // The lease outlives the turn until its final record is on disk: another engine opening it must read the ended turn.
    // Taken on any change that finds the turn over (not only the observed edge), so a pin is never left behind
    if let Some(pin) = s.take_ended_lease() {
      self.release_after_save(s.clone(), pin);
    }
    // A turn starting or ending moves this session's lease (another engine must not open it mid-turn)
    self.sync_leases();
    // Decided before the list goes out, so the push that shows the turn ending already carries the unread dot
    if idle_edge && self.viewers_on(id, None).is_empty() {
      self.state.lock().unread.insert(id.to_owned());
    }
    self.emit_session(&s);
    self.emit_sessions();
    self.watch_attention(&s);
    if let Some(outcome) = s.start_outcome() {
      let mut st = self.state.lock();
      if st.health_seen.get(id) != Some(&outcome) {
        st.health_seen.insert(id.to_owned(), outcome.clone());
        st.health.insert(
          s.agent.clone(),
          AgentHealth { stage: outcome.stage, at: outcome.at, error: outcome.error, source: HealthSource::Session },
        );
      }
    }
    if idle_edge {
      match (s.account_id(), &self.deps.accounts, &self.deps.local_accounts) {
        (Some(acc), Some(m), _) => {
          tokio::spawn(m.refresh_quota(&acc, true));
        }
        (None, _, Some(l)) => {
          let (l, agent) = (l.clone(), s.agent.clone());
          tokio::spawn(async move { l.refresh(Some(&agent), true).await });
        }
        _ => {}
      }
    }
    if s.status() == SessionStatus::Ready {
      self.pool.ensure(&s.agent, &s.cwd, s.account_id().as_deref());
    }
  }

  /// Announce the edges a user away from the session wants to hear about: a card waiting for them (at once), and a turn that
  /// has settled, once it stayed settled for ATTENTION_SETTLE (a queued follow-up or a hook's follow-up keeps it busy).
  /// Background work the agent reported counts as busy too: a turn that ends on "I'll report when it's done" is not done.
  /// A turn that failed is announced without waiting for it, since a long-lived monitor would hold the error back indefinitely.
  /// A turn the user cancelled is not announced. Whether the user is already looking is the shell's call
  fn watch_attention(&self, s: &Arc<AcpSession>) {
    let Some(announce) = self.deps.attention.clone() else { return };
    let id = s.id.clone();
    let waiting = s.list_state() == Some(SummaryState::Waiting);
    let held = |s: &AcpSession| s.turn_busy() || (s.background_running() && s.last_stop() != Some(TurnStop::Error));
    let busy = held(s);
    let (asked, settled) = {
      let mut st = self.state.lock();
      let asked = if waiting {
        st.was_waiting.insert(id.clone())
      } else {
        st.was_waiting.remove(&id);
        false
      };
      let settled = if busy {
        st.was_busy.insert(id.clone());
        None
      } else if st.was_busy.remove(&id) {
        let seq = st.settle_seq.entry(id.clone()).or_default();
        *seq += 1;
        Some(*seq)
      } else {
        None
      };
      (asked, settled)
    };
    let agent = self.registry().try_get(&s.agent).map(|d| d.name.clone()).unwrap_or_else(|| s.agent.clone());
    if asked {
      announce(Attention { session_id: id.clone(), title: s.title(), agent: agent.clone(), kind: AttentionKind::Waiting });
    }
    let Some(seq) = settled else { return };
    let me = self.me.clone();
    tokio::spawn(async move {
      tokio::time::sleep(ATTENTION_SETTLE).await;
      let Some(m) = me.upgrade() else { return };
      let Some(s) = m.live(&id) else { return };
      // Another turn began meanwhile (it announces itself when it settles), or the session now waits on the user
      if m.state.lock().settle_seq.get(&id) != Some(&seq)
        || held(&s)
        || s.list_state() == Some(SummaryState::Waiting)
      {
        return;
      }
      let kind = match s.last_stop() {
        Some(TurnStop::Error) => AttentionKind::Error,
        Some(TurnStop::Cancelled) | None => return,
        Some(_) => AttentionKind::Done,
      };
      announce(Attention { session_id: id, title: s.title(), agent, kind });
    });
  }

  fn session_deps(&self) -> SessionDeps {
    let me = self.me.clone();
    let shapes_me = self.me.clone();
    let claim_me = self.me.clone();
    let toast = self.deps.toast.clone();
    let compaction = self.deps.compaction.clone();
    let plan_auto = self.deps.plan_auto_approve.clone();
    SessionDeps {
      registry: self.registry(),
      log: self.deps.log.clone(),
      on_change: Arc::new(move |id: &str, running: bool| {
        if let Some(m) = me.upgrade() {
          m.mark_dirty(id, running);
        }
      }),
      blobs: self.deps.store.clone(),
      notify: Some(Arc::new(move |text: &str| toast("info", text))),
      accounts: self.deps.accounts.clone().map(|a| Arc::new(AccountHooks(a)) as Arc<dyn SessionAccountHooks>),
      compaction: Some(Arc::new(move || compaction())),
      plan_auto_approve: Some(Arc::new(move |agent: &str| plan_auto().iter().any(|a| a == agent))),
      pool: Some(self.pool.clone()),
      host_mcp: self.deps.host_mcp.clone(),
      model_shapes: Some(Arc::new(move |agent: &str| {
        shapes_me.upgrade().and_then(|m| m.state.lock().prefs.model_shapes.as_ref().and_then(|s| s.get(agent).cloned()))
      })),
      shared_mcp: self.deps.shared_mcp.clone(),
      claim: Some(Arc::new(move |id: &str| claim_me.upgrade().map_or(Ok(0), |m| m.claim_turn(id)))),
      remote: self.deps.remote.clone(),
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
  async fn drop_empty_current(&self, v: &Viewer) {
    let Some(cur) = self.current(v) else { return };
    if cur.turn_count() > 0 || cur.is_running() || !self.viewers_on(&cur.id, Some(v)).is_empty() {
      return;
    }
    self.forget(&cur.id);
    self.state.lock().index.retain(|x| x.id != cur.id);
    self.deps.store.remove(&cur.id).await;
    self.save_index();
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
    s.start().await;
    // Also on a failed start: it releases the hold and the prompts it queued
    s.adopt_controls(settings).await;
    // A draining engine keeps the lease of every live agent: synced before the load's own hold goes
    self.sync_leases();
    drop(hold);
  }

  /// Record edits (rename, pin, delete, move) hold the session's lease until they are on disk: refused while another
  /// engine drives the session (or this one only mirrors it), since they would race that engine's saves. A short hold
  /// (another engine opening the session or saving an edit of its own) is waited out rather than refused
  async fn hold_record<'a>(&'a self, id: &'a str) -> Result<LeaseHold<'a>> {
    if self.mirror_of(id).is_some() {
      return Err(anyhow!(t("host.sessionBusyElsewhere")));
    }
    let deadline = tokio::time::Instant::now() + RECORD_HOLD_WAIT;
    loop {
      match self.leases.claim(id) {
        Claim::Held(pin) => return Ok(LeaseHold { leases: &self.leases, id, pin }),
        Claim::Elsewhere if tokio::time::Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(50)).await,
        Claim::Elsewhere => return Err(anyhow!(t("host.sessionBusyElsewhere"))),
        Claim::Failed(e) => return Err(anyhow!(tp("host.sessionLeaseFailed", &[("error", &e)]))),
      }
    }
  }

  /// The session's `claim` hook: the lease for a turn about to start
  fn claim_turn(&self, id: &str) -> Result<u64, String> {
    match self.leases.claim(id) {
      Claim::Held(pin) => Ok(pin),
      Claim::Elsewhere => Err(t("host.sessionBusyElsewhere")),
      Claim::Failed(e) => {
        self.log(&format!("session {id}: lease failed: {e}"));
        Err(tp("host.sessionLeaseFailed", &[("error", &e)]))
      }
    }
  }

  /// A finished turn's lease pin goes once its final record is on disk. A failed save keeps the lease (another engine
  /// would read a record without the end of the turn) and is retried, until it lands or the session is closed here
  fn release_after_save(&self, s: Arc<AcpSession>, pin: u64) {
    let me = self.me.clone();
    tokio::spawn(async move {
      let mut wait = Duration::from_secs(1);
      loop {
        let Some(m) = me.upgrade() else { return };
        let closed = m.live(&s.id).is_none_or(|live| !Arc::ptr_eq(&live, &s));
        if closed {
          m.leases.release(&s.id, pin);
          return;
        }
        match m.deps.store.flush(s.clone() as Arc<dyn RecordSource>).await {
          Ok(()) => {
            m.leases.release(&s.id, pin);
            m.sync_leases();
            return;
          }
          Err(e) => m.log(&format!("session {}: final save failed, lease kept, retrying in {}s: {e}", s.id, wait.as_secs())),
        }
        drop(m);
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(30));
      }
    });
  }

  fn mirror_of(&self, id: &str) -> Option<Arc<AcpSession>> {
    self.state.lock().mirrors.get(id).cloned()
  }

  /// Hold exactly the leases of the sessions this engine has open: a session is driven from one engine at a time, every
  /// other one shows it read-only until it takes the session over (or this engine closes it, or ends)
  pub fn sync_leases(&self) {
    let wanted: HashSet<String> = self.live_sessions().into_iter().map(|s| s.id.clone()).collect();
    self.leases.sync(&wanted);
    self.bus.announce(self.leases.held());
  }

  /// Watch for other engines asking to take over a session this one has open; ends with the manager
  fn watch_takeovers(&self) {
    let me = self.me.clone();
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(TAKEOVER_POLL).await;
        let Some(m) = me.upgrade() else { return };
        if m.state.lock().disposed {
          return;
        }
        for id in m.leases.held() {
          if m.leases.takeover_requested(&id) && m.live(&id).is_some() && m.yielding.lock().insert(id.clone()) {
            let m2 = m.clone();
            tokio::spawn(async move {
              m2.yield_session(&id).await;
              m2.yielding.lock().remove(&id);
            });
          } else if m.leases.handover_requested(&id) && m.handover_ready(&id) && m.yielding.lock().insert(id.clone()) {
            let m2 = m.clone();
            tokio::spawn(async move {
              m2.hand_over_idle(&id).await;
              m2.yielding.lock().remove(&id);
            });
          }
        }
      }
    });
  }

  /// Another engine takes this session over: the running turn stops, the record is written, the agent ends and the lease
  /// goes; whoever was looking at it here keeps a read-only copy that can take it back
  async fn yield_session(self: &Arc<Self>, id: &str) {
    let Some(s) = self.live(id) else { return };
    self.log(&format!("session {id}: another engine takes it over, letting go"));
    if s.is_running() {
      s.cancel_for_handover().await;
      let t0 = tokio::time::Instant::now();
      while s.is_running() && t0.elapsed() < YIELD_SETTLE {
        tokio::time::sleep(Duration::from_millis(50)).await;
      }
    }
    if let Err(e) = self.deps.store.flush(s.clone() as Arc<dyn RecordSource>).await {
      self.log(&format!("session {id}: save before handing over failed: {e}"));
    }
    let record = s.to_record();
    self.forget(id);
    // The copy goes up before the lease goes, so the viewers never see the session vanish
    if !self.viewers_on(id, None).is_empty() {
      self.start_mirror(id, record, t("host.sessionTakenOver"), true);
    }
    self.sync_leases();
    self.emit_sessions();
  }

  /// A session another engine wants can go without asking anyone: live and idle here, and no window of this engine shows
  /// it (the usual case is a window reloaded onto a new engine after an upgrade, leaving this one with no window at all)
  fn handover_ready(&self, id: &str) -> bool {
    self.live(id).is_some_and(|s| s.releasable()) && self.viewers_on(id, None).is_empty()
  }

  /// Hand an idle, unseen session to the engine that wants it: the agent process closes like an idle release, and the
  /// lease goes once the native session is closed, so the other engine's read-only copy opens by itself
  async fn hand_over_idle(self: &Arc<Self>, id: &str) {
    let Some(s) = self.live(id) else { return };
    if self.release(&s).await {
      self.log(&format!("session {id}: another engine wants it and no window here shows it, handing it over"));
      self.sync_leases();
      self.emit_sessions();
    }
  }

  /// Take over a session another engine has open: ask it to let go, wait for the lease, then open the session here
  async fn take_over_elsewhere(self: &Arc<Self>, id: &str) -> Result<()> {
    self.leases.request_takeover(id).map_err(|e| anyhow!(tp("host.sessionLeaseFailed", &[("error", &e.to_string())])))?;
    let deadline = tokio::time::Instant::now() + TAKEOVER_WAIT;
    let pin = loop {
      match self.leases.claim(id) {
        Claim::Held(pin) => break pin,
        Claim::Elsewhere if tokio::time::Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(100)).await,
        Claim::Elsewhere => {
          self.leases.clear_takeover(id);
          return Err(anyhow!(t("host.takeOverTimeout")));
        }
        Claim::Failed(e) => {
          self.leases.clear_takeover(id);
          return Err(anyhow!(tp("host.sessionLeaseFailed", &[("error", &e)])));
        }
      }
    };
    let hold = LeaseHold { leases: &self.leases, id, pin };
    self.leases.clear_takeover(id);
    self.log(&format!("session {id}: taken over from another engine"));
    self.state.lock().mirrors.remove(id);
    // The first viewer loads it (fresh from disk, the other engine's last word); the rest find it live
    for v in self.viewers_on(id, None) {
      self.select_session_for(&v, id).await;
    }
    drop(hold);
    Ok(())
  }

  /// Some session is mid-turn (a permission card or question waiting on the user counts: the turn is still running)
  pub fn busy(&self) -> bool {
    self.live_sessions().iter().any(|s| s.is_running())
  }

  /// Sessions whose lease this engine holds (tests)
  pub fn leased(&self) -> Vec<String> {
    self.leases.held()
  }

  /// On the engines' push bus right now (tests)
  pub fn bus_connected(&self) -> bool {
    self.bus.connected()
  }

  /// Inert deps for a mirror: it never saves (the record belongs to the engine holding the lease), warms or switches
  fn mirror_deps(&self) -> SessionDeps {
    SessionDeps {
      on_change: Arc::new(|_: &str, _: bool| {}),
      notify: None,
      accounts: None,
      compaction: None,
      plan_auto_approve: None,
      pool: None,
      // A mirror never runs a turn
      claim: Some(Arc::new(|_: &str| Err(t("host.sessionBusyElsewhere")))),
      remote: None,
      ..self.session_deps()
    }
  }

  fn record_path(&self, id: &str) -> std::path::PathBuf {
    self.deps.store.dir().join(format!("{id}.json"))
  }

  /// Show `record` read-only to the viewers on `id` and poll: the copy follows the record the other engine keeps saving,
  /// and once its lease comes free the session opens here like any other (the viewers re-select it)
  fn start_mirror(self: &Arc<Self>, id: &str, record: SessionRecord, note: String, elsewhere: bool) {
    // One epoch for this copy and every copy the poll below makes: their revs grow, so a viewer is patched between them
    let epoch = crate::acp::session::next_view_epoch();
    let mirror = AcpSession::mirror(record, self.mirror_deps(), note.clone(), self.next_mirror_rev(), elsewhere, epoch);
    let fresh = self.state.lock().mirrors.insert(id.to_owned(), mirror.clone()).is_none();
    self.log(&format!("session {id}: lease not taken, mirroring its record ({note})"));
    // Held by an engine that may push its view: the record copy waits for that view a moment (`FIRST_PUSH_GRACE`)
    let await_push = fresh && elsewhere && self.bus.connected();
    if !await_push {
      self.emit_session(&mirror);
    }
    if !fresh {
      return;
    }
    // Held by another engine: ask it to hand the session over once nobody there needs it (refreshed on every poll below),
    // and to push the session's view meanwhile
    if elsewhere {
      self.want_handover(id);
      self.bus.subscribe(id);
    }
    let me = self.me.clone();
    let id = id.to_owned();
    tokio::spawn(async move {
      if await_push {
        let until = tokio::time::Instant::now() + FIRST_PUSH_GRACE;
        while tokio::time::Instant::now() < until {
          tokio::time::sleep(PUSH_QUANTUM).await;
          let Some(m) = me.upgrade() else { return };
          if m.remote_msg(&id).is_some() || m.mirror_of(&id).is_none() {
            break;
          }
        }
        // No pushed view in time (an owner from before the bus, or one not running the session): the record copy
        if let Some(m) = me.upgrade()
          && m.remote_msg(&id).is_none()
          && let Some(copy) = m.mirror_of(&id)
        {
          m.emit_session(&copy);
        }
      }
      let mut seen = None;
      loop {
        tokio::time::sleep(MIRROR_POLL).await;
        let Some(m) = me.upgrade() else { return };
        if m.state.lock().disposed {
          return;
        }
        // Replaced by the session itself (taken over, deleted): this poll is done
        if m.mirror_of(&id).is_none() {
          m.leases.clear_handover(&id);
          m.end_remote(&id);
          return;
        }
        let watchers = m.viewers_on(&id, None);
        if watchers.is_empty() {
          // Nobody looks at it any more; a later select starts over
          m.state.lock().mirrors.remove(&id);
          m.leases.clear_handover(&id);
          m.end_remote(&id);
          return;
        }
        // Free and nobody else is about to take it (a takeover in flight frees it for the requester, not for this copy)
        if !m.leases.held_elsewhere(&id) && !m.leases.takeover_requested(&id) {
          m.state.lock().mirrors.remove(&id);
          m.leases.clear_handover(&id);
          m.end_remote(&id);
          m.log(&format!("session {id}: the other engine let go, opening it here"));
          // The first watcher loads it (fresh from disk); the rest wait on that load inside select_session_for
          for v in watchers {
            let (m2, id2) = (m.clone(), id.clone());
            tokio::spawn(async move { m2.select_session_for(&v, &id2).await });
          }
          return;
        }
        if elsewhere {
          m.want_handover(&id);
        }
        // Re-read only when the other engine saved since the last look (or its pushed view just ended)
        let forced = m.state.lock().refresh.remove(&id);
        let stamp = tokio::fs::metadata(m.record_path(&id)).await.and_then(|md| md.modified()).ok();
        if stamp.is_none() || (stamp == seen && !forced) {
          continue;
        }
        seen = stamp;
        if let Ok(record) = m.deps.store.load_detailed(&id).await {
          let copy = AcpSession::mirror(record, m.mirror_deps(), note.clone(), m.next_mirror_rev(), elsewhere, epoch);
          let current = {
            let mut st = m.state.lock();
            match st.mirrors.get_mut(&id) {
              Some(slot) => {
                *slot = copy.clone();
                true
              }
              None => false,
            }
          };
          if !current {
            m.end_remote(&id);
            return;
          }
          // The copy still serves reads (diff sources, exports); the screen keeps the pushed view while there is one
          m.emit_session(&copy);
        }
      }
    });
  }

  /// Keep this engine's handover request for a mirrored session fresh
  fn want_handover(&self, id: &str) {
    if let Err(e) = self.leases.request_handover(id) {
      self.log(&format!("session {id}: handover request failed: {e}"));
    }
  }

  fn current(&self, v: &Viewer) -> Option<Arc<AcpSession>> {
    v.active_id().and_then(|id| self.live(&id))
  }

  /// Session-targeted messages carry the id the webview was showing; only that session may take the action
  fn target(&self, v: &Viewer, session_id: Option<&str>) -> Option<Arc<AcpSession>> {
    let Some(sid) = session_id else { return self.current(v) };
    let s = self.live(sid);
    if s.is_none() {
      self.log(&format!("action on session {} dropped: not live", sid.chars().take(8).collect::<String>()));
    }
    s
  }

  pub async fn edit_turn(&self, edit: EditTurnRequest) -> Result<()> {
    let s = self.live(&edit.session_id).ok_or_else(|| anyhow!(t("history.unavailable")))?;
    s.edit_turn(edit).await
  }

  /// The whole files behind a diff a page was sent without them: the open session (or the read-only copy shown for it)
  /// holds them. A ChatGPT mirror's view carries its sources whole and never asks
  /// A source the Storage tab slimmed away (`omitted` in the record) has no texts left: none is sent, so the page copies
  /// what it shows
  pub fn diff_source(&self, session_id: &str, tool_call_id: &str, nth: usize, subagent: Option<&str>) -> Option<DiffSource> {
    self.live(session_id).or_else(|| self.mirror_of(session_id))?.diff_source(tool_call_id, nth, subagent).filter(|s| !s.omitted)
  }

  /// The files the session's own edits touched with their text before its first edit of each (`AcpSession::edit_bases`);
  /// empty for a session that is neither open here nor shown as another engine's copy
  pub fn edit_bases(&self, session_id: &str) -> Vec<(String, Option<String>)> {
    self.live(session_id).or_else(|| self.mirror_of(session_id)).map(|s| s.edit_bases()).unwrap_or_default()
  }

  pub fn plan_document(&self, session_id: &str, plan_id: &str) -> Option<PlanDocumentBlock> {
    if let Some(s) = self.live(session_id) {
      return s.plan_document(plan_id);
    }
    let view = self.deps.chatgpt.as_ref()?.view(session_id)?;
    view.turns.iter().filter_map(Turn::as_agent).flat_map(|t| t.blocks.iter()).find_map(|b| match b {
      AgentBlock::PlanDocument(p) if p.id == plan_id => Some(p.clone()),
      _ => None,
    })
  }

  pub async fn handle_for(self: &Arc<Self>, v: &Arc<Viewer>, m: WebviewMsg) {
    let kind = format!("{m:?}").split([' ', '{', '(']).next().unwrap_or("").to_owned();
    if let Err(e) = self.dispatch(v, m).await {
      let text = error_text(&e);
      self.log(&format!("handle {} failed: {text}", lower_first(&kind)));
      (self.deps.toast)("error", &text);
    }
  }

  async fn dispatch(self: &Arc<Self>, v: &Arc<Viewer>, m: WebviewMsg) -> Result<()> {
    use WebviewMsg as W;
    let target_id = session_id_of(&m).or_else(|| v.active_id());
    if let Some(tid) = &target_id
      && self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(tid))
      && is_execution(&m)
    {
      return Err(anyhow!(t("chatgpt.externalOnly")));
    }
    // Shown here read-only while another engine owns it and pushes its view: the action runs there
    if let Some(tid) = &target_id
      && is_forwarded(&m)
      && self.live(tid).is_none()
      && self.state.lock().remote.get(tid).is_some_and(|r| r.parts.is_some())
    {
      return self.forward(tid, &m).await;
    }
    let valid = |id: &str| is_session_id(id);
    match m {
      W::ConnectChatgpt => self.new_session_for(v, Some(CHATGPT_ID.into()), None, None, None).await?,
      W::Send { session_id, text, attachments, steer } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          if steer == Some(true) {
            s.steer_prompt(text, attachments).await?;
          } else {
            s.prompt(text, attachments, false, None, None).await;
          }
        }
      }
      W::Stop { session_id } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          s.cancel().await;
        }
      }
      W::Permission { session_id, block_id, option_id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.resolve_permission(&block_id, &option_id);
        }
      }
      W::Answer { session_id, block_id, answers, skip } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
        {
          s.answer_questions(&block_id, &answers, skip == Some(true));
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
      W::NewSession { agent, category, cwd } => self.new_session_for(v, agent, None, category, cwd).await?,
      W::RenameSession { id, title } => self.rename_session(&id, &title).await?,
      W::DeleteSession { id } => self.delete_session(&id).await?,
      W::RestoreSession { id } => self.restore_session(&id).await?,
      W::PinSession { id, pinned } => self.pin_session(&id, pinned).await?,
      W::SetSessionCategory { id, category } => self.set_session_category(&id, category).await?,
      W::CategoryOp { op, file } => {
        self.category_op(&op).await?;
        if let (CategoryOp::Create { id, .. }, Some(session)) = (&op, file) {
          self.set_session_category(&session, Some(id.clone())).await?;
        }
      }
      // Unread dots live in this engine's memory only (a restart forgets them too), so forgetting them is the whole job
      W::MarkAllRead => {
        self.state.lock().unread.clear();
        self.emit_sessions();
      }
      W::MoveSession { id, cwd, category } => self.move_session(&id, cwd, category).await?,
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
        if let Some((turns, rev, running)) = self.live(&session_id).and_then(|s| s.subagent_transcript(&subagent_id)) {
          v.state.lock().last_subagent_rev = Some(rev);
          v.emit(HostMsg::Subagent { session_id, subagent_id, rev, running, turns });
        }
      }
      W::UnobserveSubagent { session_id, subagent_id } => {
        let mut vs = v.state.lock();
        if vs.observing.as_ref() == Some(&(session_id, subagent_id)) {
          vs.observing = None;
          vs.last_subagent_rev = None;
        }
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
      W::AddAccount { agent, via } => self.add_account(v, &agent, via).await?,
      W::RemoveAccount { id } => {
        if let Some(a) = &self.deps.accounts {
          a.remove(&id).await?;
        }
        self.pool.invalidate(None);
      }
      W::UnlockCredentials { agent } => self.unlock_credentials(&agent).await?,
      W::RefreshQuota { agent } => {
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
      W::SteerQueued { session_id, id } => {
        if valid(&session_id)
          && let Some(s) = self.live(&session_id)
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

  /// Rename / pin: a live session mutates itself; one not loaded is patched on disk
  pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.rename(id, title).await;
    }
    let t = clip(title.trim(), RENAME_MAX);
    if t.is_empty() {
      return Ok(());
    }
    if let Some(live) = self.live(id) {
      live.rename(&t);
      return self.save_now(live, hold).await;
    }
    self.patch_record(id, |r| r.title = t).await
  }

  pub async fn pin_session(&self, id: &str, pinned: bool) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.pin(id, pinned).await;
    }
    if let Some(live) = self.live(id) {
      live.set_pinned(pinned);
      return self.save_now(live, hold).await;
    }
    self.patch_record(id, |r| {
      r.pinned = pinned.then_some(true);
      if pinned {
        r.category = None;
      }
    })
    .await
  }

  /// File `id` under `category` (or take it out). A category of another project, or one gone meanwhile, is ignored.
  /// ChatGPT mirrors have no categories
  pub async fn set_session_category(&self, id: &str, category: Option<String>) -> Result<()> {
    if !is_session_id(id) || self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(id)) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if let Some(live) = self.live(id) {
      let Some(category) = self.category_for(category, &live.cwd) else { return Ok(()) };
      live.set_category(category);
      return self.save_now(live, hold).await;
    }
    let categories = self.categories();
    self
      .patch_record(id, move |r| {
        let fits = category.as_ref().is_none_or(|c| categories.categories.iter().any(|x| &x.id == c && x.cwd == r.cwd));
        if fits {
          if category.is_some() {
            r.pinned = None;
          }
          r.category = category;
        }
      })
      .await
  }

  /// A live session's edit written through now, while the edit still holds the lease (not on the debounce). A failed
  /// write is reported, and the lease stays with the retrying save: the edit is in memory and must not land after
  /// another engine took the session
  async fn save_now(&self, live: Arc<AcpSession>, hold: LeaseHold<'_>) -> Result<()> {
    match self.deps.store.flush(live.clone() as Arc<dyn RecordSource>).await {
      Ok(()) => Ok(()),
      Err(e) => {
        self.log(&format!("session {}: save failed ({e})", live.id));
        self.release_after_save(live, hold.keep());
        Err(anyhow!(tp("host.saveFailed", &[("error", &e.to_string())])))
      }
    }
  }

  async fn patch_record(&self, id: &str, patch: impl FnOnce(&mut SessionRecord)) -> Result<()> {
    let Some(mut r) = self.deps.store.load(id).await else { return Ok(()) };
    patch(&mut r);
    if let Err(e) = self.deps.store.flush(Arc::new(r.clone())).await {
      self.log(&format!("session {id}: save failed ({e})"));
      return Err(anyhow!(tp("host.saveFailed", &[("error", &e.to_string())])));
    }
    self.replace_summary(&r);
    Ok(())
  }

  /// Soft deletion with a 30-second undo window; viewers showing it move on
  pub async fn delete_session(self: &Arc<Self>, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let _hold = self.hold_record(id).await?;
    if let Some(c) = self.deps.chatgpt.clone().filter(|c| c.owns(id)) {
      c.delete(id).await?;
      self.rehome(id).await;
      return Ok(());
    }
    if let Some(live) = self.live(id) {
      self.deps.store.flush(live as Arc<dyn RecordSource>).await.ok();
    }
    self.forget(id);
    let sum = {
      let mut st = self.state.lock();
      let sum = st.index.iter().find(|s| s.id == id).cloned();
      st.index.retain(|s| s.id != id);
      sum
    };
    if let Some(summary) = sum {
      let weak = self.me.clone();
      let tid = id.to_owned();
      let timer = tokio::spawn(async move {
        tokio::time::sleep(TRASH_TTL).await;
        let Some(m) = weak.upgrade() else { return };
        m.state.lock().trash.remove(&tid);
        m.deps.store.remove(&tid).await;
      });
      self.state.lock().trash.insert(id.to_owned(), Trash { summary, timer: timer.abort_handle() });
    }
    self.deps.store.trash(id).await?;
    self.save_index();
    self.rehome(id).await;
    self.emit_sessions();
    Ok(())
  }

  /// Close a live session and drop every trace of it in memory; out of the live map first, so its dispose callback
  /// cannot write the record back
  fn forget(&self, id: &str) {
    let live = {
      let mut st = self.state.lock();
      st.was_running.remove(id);
      st.unread.remove(id);
      st.was_busy.remove(id);
      st.was_waiting.remove(id);
      st.settle_seq.remove(id);
      st.health_seen.remove(id);
      st.dirty.remove(id);
      st.mirrors.remove(id);
      st.idle_since.remove(id);
      st.live.remove(id)
    };
    if let Some(s) = live {
      if let Some(pin) = s.drop_lease() {
        self.leases.release(id, pin);
      }
      s.dispose();
    }
  }

  /// Viewers left on a session that is gone move to the newest one in scope, or a new session
  fn rehome<'a>(self: &'a Arc<Self>, id: &'a str) -> BoxFuture<()> {
    let me = self.clone();
    let id = id.to_owned();
    Box::pin(async move {
      if me.state.lock().disposed {
        return;
      }
      for v in me.viewers_on(&id, None) {
        me.set_active(&v, None);
        match me.most_recent() {
          Some(next) => me.select_session_for(&v, &next).await,
          // A project shell whose last session went and that has no project left shows its empty state
          None if me.folder_for(Some(&v), None).is_none() => v.emit(HostMsg::NoSession),
          None => {
            if let Err(e) = me.new_session_for(&v, None, None, None, None).await {
              me.log(&format!("new session failed: {e}"));
            }
          }
        }
      }
    })
  }

  /// Re-home a session into another project folder: `to` (a project the list shows), or this window's workspace folder
  /// without one. `category` files it under one of the destination project's categories in the same step; any other id
  /// leaves it unfiled there
  pub async fn move_session(self: &Arc<Self>, id: &str, to: Option<String>, category: Option<String>) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    let hold = self.hold_record(id).await?;
    if self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(id)) {
      return Err(anyhow!(t("chatgpt.projectBound")));
    }
    let cwd = match to {
      // The agent is spawned in the folder from the next open on, so a project whose folder is gone cannot take it
      Some(dir) => self.check_folder(&dir, "host.moveFolderMissing").await?,
      None => self.folder_for(None, None).ok_or_else(|| anyhow!(t("host.noProject")))?,
    };
    // A category belongs to the project the session leaves; only one of the destination's survives the move
    let category = self.category_for(category, &cwd).flatten();
    if let Some(live) = self.live(id) {
      if live.cwd == cwd {
        // Already there: a category given is still a filing
        if category.is_none() {
          return Ok(());
        }
        live.set_category(category);
        return self.save_now(live, hold).await;
      }
      if live.is_running() {
        return Err(anyhow!(t("host.moveWhileRunning")));
      }
      let mut record = live.to_record();
      self.forget(id);
      record.cwd = cwd;
      // Filing unpins, as everywhere else
      if category.is_some() {
        record.pinned = None;
      }
      record.category = category;
      self.deps.store.flush(Arc::new(record.clone())).await.ok();
      self.replace_summary(&record);
      for v in self.viewers_on(id, None) {
        self.set_active(&v, None);
        self.select_session_for(&v, id).await;
      }
      return Ok(());
    }
    self
      .patch_record(id, |r| {
        if r.cwd == cwd && category.is_none() {
          return;
        }
        r.cwd = cwd;
        if category.is_some() {
          r.pinned = None;
        }
        r.category = category;
      })
      .await
  }

  /// Fork from an agent turn: a fresh session whose transcript is the source's turns up to that reply. `agent` moves the
  /// fork onto another agent, which starts the way a new session of it does (default account, remembered settings)
  pub async fn fork_session(self: &Arc<Self>, v: &Arc<Viewer>, source_id: &str, turn_index: i64, agent: Option<String>) -> Result<()> {
    if !is_session_id(source_id) {
      return Ok(());
    }
    if self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(source_id)) {
      return Err(anyhow!(t("chatgpt.externalOnly")));
    }
    let live = self.live(source_id);
    let source = match &live {
      Some(l) => l.to_record(),
      None => match self.deps.store.load_detailed(source_id).await {
        Ok(record) => record,
        Err(error) => return Err(anyhow!(self.record_load_message(source_id, &error))),
      },
    };
    let idx = usize::try_from(turn_index)
      .ok()
      .filter(|i| matches!(source.turns.get(*i), Some(Turn::Agent(_))))
      .ok_or_else(|| anyhow!(t("host.forkStale")))?;
    if live.as_ref().is_some_and(|l| l.is_running()) && idx == source.turns.len() - 1 {
      return Err(anyhow!(t("host.forkRunning")));
    }
    // Another agent only when it is one this engine can launch; naming the source's own agent is a plain fork
    let target = agent.filter(|a| a != &source.agent);
    if let Some(id) = &target
      && !self.agents().iter().any(|a| &a.id == id && a.external != Some(true))
    {
      return Err(anyhow!(tp("host.forkAgentUnknown", &[("agent", id)])));
    }
    let model = target.as_ref().and_then(|_| fork_model_label(&source, idx));
    let mut turns: Vec<Turn> = source.turns[..=idx].to_vec();
    // Live-only state does not travel
    for turn in turns.iter_mut().filter_map(Turn::as_agent_mut) {
      turn.activity = None;
      for b in &mut turn.blocks {
        match b {
          AgentBlock::Text(x) if x.streaming == Some(true) => x.streaming = Some(false),
          AgentBlock::Thought(x) if x.streaming == Some(true) => x.streaming = Some(false),
          _ => {}
        }
      }
    }
    let now = now_iso();
    let subagents: Vec<_> = source
      .subagents
      .clone()
      .unwrap_or_default()
      .into_iter()
      .filter(|n| n.core.turn_index as usize <= idx)
      .map(|mut n| {
        if n.core.state == SubagentState::Running {
          n.core.state = SubagentState::Disconnected;
          n.core.state_source = StateSource::Local;
          n.core.ended_at = ms_of_iso(&now);
        }
        n.core.cancel_requested = None;
        n
      })
      .collect();
    // A fork onto another agent takes nothing of the source's agent setup: the target's default account, and its
    // controls come from its own start (seeded below like a new session's), not the source's model list
    let (agent_id, account_id, controls) = match &target {
      Some(id) => (id.clone(), self.default_account(id), SessionControls::default()),
      None => (source.agent.clone(), source.account_id.clone(), source.controls.clone()),
    };
    let mut record = SessionRecord {
      id: random_uuid(),
      agent: agent_id,
      account_id,
      acp_session_id: None,
      cwd: source.cwd.clone(),
      title: clip(&tp("session.forkTitle", &[("title", &source.title)]), RENAME_MAX),
      created_at: now.clone(),
      updated_at: now,
      turns,
      controls,
      usage: None,
      commands: vec![],
      pinned: None,
      // A fork stays in its source's category
      category: source.category.clone(),
      history_pending: true,
      forked_from: Some(ForkedFrom { session_id: source.id.clone(), turn_index, agent: target.as_ref().map(|_| source.agent.clone()), model }),
      import_pending: false,
      imported_from: None,
      subagents: (!subagents.is_empty()).then_some(subagents),
      goal: None,
    };
    // Every blob the copied turns point at (prompt / steer attachments and the images the agent produced, subagent
    // transcripts included) is re-saved under the fork's own blob dir; content-hash names keep the same file name
    let fork_id = record.id.clone();
    let mut slots = Vec::new();
    blob_slots(&mut record.turns, &mut slots);
    for node in record.subagents.iter_mut().flatten() {
      blob_slots(&mut node.turns, &mut slots);
    }
    // One copy per distinct blob; a name that could not be copied is dropped everywhere it appears
    let mut copied: HashMap<String, Option<String>> = HashMap::new();
    for blob in slots {
      let Some(name) = blob.clone() else { continue };
      if !copied.contains_key(&name) {
        let ext = std::path::Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let result = match self.deps.store.read_blob(&source.id, &name).await {
          Ok(bytes) => self.deps.store.save_blob(&fork_id, &ext, &bytes).await.map(|(n, _)| n),
          Err(e) => Err(e),
        };
        let next = result.map_err(|e| self.log(&format!("fork: blob {name} not copied ({e})"))).ok();
        copied.insert(name.clone(), next);
      }
      *blob = copied[&name].clone();
    }
    self.deps.store.flush(Arc::new(record.clone())).await?;
    self.drop_empty_current(v).await;
    let s = AcpSession::new(record, self.session_deps());
    // The target's controls the way new_session_for seeds them: its known controls with the remembered settings laid
    // over before the spawn (held, so a prompt sent while it spawns waits for them), adopted once it is up
    let last = target.as_ref().and_then(|id| self.last_settings(id));
    if let Some(id) = &target {
      s.preview_controls(&self.known_start_controls(id).await, last.as_ref());
      if let Some(last) = &last {
        s.hold_settings(last);
      }
    }
    self.state.lock().live.insert(s.id.clone(), s.clone());
    self.set_active(v, Some(s.id.clone()));
    self.process_change(&s.id);
    s.start().await;
    match &target {
      Some(id) => {
        self.state.lock().probed.remove(id);
        if let Some(last) = last {
          s.adopt_controls(last).await;
        }
      }
      None if s.status() == SessionStatus::Ready => s.adopt_controls(capture_turn_settings(&source.controls)).await,
      None => {}
    }
    Ok(())
  }

  /// Write the session as Markdown or JSON under exports/. Exporting a transcript that has not changed since the last export
  /// in the same format returns that file again (as long as it still exists) instead of writing a second copy
  /// A session's content as export, copy and read_session see it: the live session first, else its record, else a ChatGPT
  /// mirror. A record that cannot be read answers the same message the viewer would show
  pub async fn snapshot(&self, id: &str) -> Result<SessionSnapshot> {
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.view(id).map(SessionSnapshot::Mirror).ok_or_else(|| anyhow!(self.record_load_message(id, &RecordLoadError::Missing)));
    }
    if let Some(l) = self.live(id) {
      return Ok(SessionSnapshot::Record(l.to_record()));
    }
    match self.deps.store.load_detailed(id).await {
      Ok(record) => Ok(SessionSnapshot::Record(record)),
      Err(error) => Err(anyhow!(self.record_load_message(id, &error))),
    }
  }

  fn agent_name(&self, agent: &AgentId) -> String {
    self.agents().into_iter().find(|a| &a.id == agent).map(|a| a.name).unwrap_or_else(|| agent.clone())
  }

  /// The Markdown both "Export as Markdown" and "Copy as Markdown" produce, labels in the display language, blobs as paths
  fn snapshot_markdown(&self, id: &str, snap: &SessionSnapshot, agent_name: &str) -> String {
    let labels = ExportLabels {
      user: t("export.label.user"),
      agent: t("export.label.agent"),
      project: t("export.label.project"),
      exported: t("export.label.exported"),
      attachments: t("export.label.attachments"),
      thinking: t("export.label.thinking"),
      compacted: t("export.label.compacted"),
      auto_compact: t("export.label.autoCompact"),
      auto_continue: t("export.label.autoContinue"),
      auto_retry: t("export.label.autoRetry"),
      auto_gate: t("export.label.autoGate"),
      auto_subagents: t("export.label.autoSubagents"),
      error: t("export.label.error"),
    };
    let store = self.deps.store.clone();
    let sid = id.to_owned();
    export_markdown(
      &ExportInput { title: snap.title(), agent_name, cwd: snap.cwd(), exported_at: &now_iso(), turns: snap.turns() },
      &labels,
      &move |name| store.blob_path(&sid, name).map(|p| p.to_string_lossy().into_owned()),
    )
  }

  /// "Copy as Markdown": the export's Markdown as text, nothing written
  pub async fn session_markdown(&self, id: &str) -> Result<String> {
    let snap = self.snapshot(id).await?;
    Ok(self.snapshot_markdown(id, &snap, &self.agent_name(snap.agent())))
  }

  pub async fn export_session(&self, id: &str, format: ExportFormat) -> Result<std::path::PathBuf> {
    let mut exports = self.exports.lock().await;
    let snap = self.snapshot(id).await?;
    let json = snap.json()?;
    let agent_name = self.agent_name(snap.agent());
    // The fingerprint leaves out the export time stamped into the Markdown; the agent name and the label language are in it,
    // so a renamed agent or a switched display language still writes a fresh file
    let fingerprint = {
      use std::hash::{Hash, Hasher};
      let mut h = std::collections::hash_map::DefaultHasher::new();
      (&json, &agent_name, t("export.label.user")).hash(&mut h);
      h.finish()
    };
    let key = (id.to_owned(), format);
    if let Some((hash, path)) = exports.get(&key)
      && *hash == fingerprint
      && tokio::fs::try_exists(path).await.unwrap_or(false)
    {
      return Ok(path.clone());
    }
    let content = match format {
      ExportFormat::Json => json,
      ExportFormat::Markdown => self.snapshot_markdown(id, &snap, &agent_name),
      ExportFormat::Text => export_text(
        &ExportInput { title: snap.title(), agent_name: &agent_name, cwd: snap.cwd(), exported_at: "", turns: snap.turns() },
        &t("export.label.user"),
      ),
    };
    let path = self.deps.store.write_export(&export_file_name(snap.title(), format.extension(), &local_stamp()), &content).await?;
    exports.insert(key, (fingerprint, path.clone()));
    Ok(path)
  }

  /// The working directory of a session a viewer shows: live, a mirror of another engine's session, a ChatGPT mirror,
  /// else whatever the index recorded (a session not opened here yet)
  pub fn session_cwd(&self, id: &str) -> Option<String> {
    {
      let st = self.state.lock();
      if let Some(s) = st.live.get(id).or_else(|| st.mirrors.get(id)) {
        return Some(s.cwd.clone());
      }
    }
    if let Some(v) = self.deps.chatgpt.as_ref().and_then(|c| c.view(id)) {
      return Some(v.cwd);
    }
    let st = self.state.lock();
    st.index.iter().find(|s| s.id == id).map(|s| s.cwd.clone()).filter(|c| !c.is_empty())
  }

  pub fn blob_path(&self, session_id: &str, name: &str) -> Option<std::path::PathBuf> {
    self.deps.store.blob_path(session_id, name)
  }

  /// A record changed on disk without a live session: this host's copy wins for it
  fn replace_summary(&self, record: &SessionRecord) {
    {
      let mut st = self.state.lock();
      let sum = record.summary();
      match st.index.iter().position(|s| s.id == record.id) {
        Some(i) => st.index[i] = sum,
        None => st.index.push(sum),
      }
      sort_index(&mut st.index);
      st.touched.insert(record.id.clone());
    }
    self.save_index();
    self.emit_sessions();
  }

  /// Undo deletion: move it back out of the trash
  pub async fn restore_session(&self, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.restore(id).await;
    }
    let Some(entry) = self.state.lock().trash.remove(id) else { return Ok(()) };
    entry.timer.abort();
    self.deps.store.restore(id).await;
    {
      let mut st = self.state.lock();
      st.index.push(entry.summary);
      sort_index(&mut st.index);
      st.touched.insert(id.to_owned());
    }
    self.save_index();
    self.emit_sessions();
    Ok(())
  }

  /// Switching accounts rebinds the current session and becomes the agent's default
  pub async fn select_account(&self, v: &Viewer, account_id: &str, session_id: Option<&str>) -> Result<()> {
    let Some(accounts) = self.deps.accounts.clone() else { return Ok(()) };
    let Some(acc) = accounts.get(account_id) else { return Ok(()) };
    if let Some(cur) = self.target(v, session_id)
      && cur.agent == acc.agent
    {
      cur.rebind_account(account_id).await?;
    }
    accounts.touch(account_id).await
  }

  pub async fn add_account(&self, v: &Viewer, agent: &str, via: AddAccountVia) -> Result<()> {
    let Some(accounts) = self.deps.accounts.clone() else { return Ok(()) };
    if self.state.lock().account_actions.iter().any(|a| a.agent == agent && a.status == AccountActionStatus::Pending) {
      return Ok(());
    }
    let action = |status, error| AccountAction { agent: agent.to_owned(), via: via.into(), status, error };
    self.set_account_action(action(AccountActionStatus::Pending, None));
    let result = match via {
      AddAccountVia::Import => accounts.import(agent).await,
      AddAccountVia::Login => accounts.login(agent).await,
      AddAccountVia::Auto => accounts.add(agent).await,
    };
    match result {
      Ok(None) => {
        self.set_account_action(action(
          if via == AddAccountVia::Import { AccountActionStatus::Missing } else { AccountActionStatus::Cancelled },
          None,
        ));
        Ok(())
      }
      Ok(Some(acc)) => {
        if let Some(cur) = self.current(v)
          && cur.agent == agent
          && (cur.status() == SessionStatus::AuthRequired || cur.account_id().is_none())
          && let Err(e) = cur.rebind_account(&acc.id).await
        {
          self.set_account_action(action(AccountActionStatus::Error, Some(e.to_string())));
          return Err(e);
        }
        self.pool.invalidate(None);
        self.set_account_action(action(AccountActionStatus::Success, None));
        Ok(())
      }
      Err(e) => {
        self.set_account_action(action(AccountActionStatus::Error, Some(e.to_string())));
        Err(e)
      }
    }
  }

  /// Unlock the agent's credential store in a terminal; once it opens, warm processes started without it are dropped
  /// and every session of the agent that stopped on sign-in reconnects on the same native session
  pub async fn unlock_credentials(&self, agent: &str) -> Result<()> {
    let Some(accounts) = self.deps.accounts.clone() else { return Ok(()) };
    if self.state.lock().account_actions.iter().any(|a| a.agent == agent && a.status == AccountActionStatus::Pending) {
      return Ok(());
    }
    let action = |status, error| AccountAction { agent: agent.to_owned(), via: AccountActionVia::Unlock, status, error };
    self.set_account_action(action(AccountActionStatus::Pending, None));
    match accounts.unlock(agent).await {
      Ok(false) => self.set_account_action(action(AccountActionStatus::Cancelled, None)),
      Ok(true) => {
        self.pool.invalidate(Some(agent));
        for s in self.live_sessions().into_iter().filter(|s| s.agent == agent && s.status() == SessionStatus::AuthRequired) {
          tokio::spawn(async move {
            let _ = s.reconnect().await;
          });
        }
        self.set_account_action(action(AccountActionStatus::Success, None));
      }
      Err(e) => {
        self.set_account_action(action(AccountActionStatus::Error, Some(e.to_string())));
        return Err(e);
      }
    }
    Ok(())
  }

  /// A terminal method runs the agent binary itself in a terminal; other methods go through ACP authenticate, falling
  /// back to the registry's login command
  async fn login(&self, s: Option<Arc<AcpSession>>, method_id: Option<&str>) -> Result<()> {
    let Some(s) = s else { return Ok(()) };
    let registry = self.registry();
    let def = registry.get(&s.agent)?.clone();
    let title = tp("host.loginTerminalTitle", &[("agent", &def.name)]);
    if let Some(term) = s.auth_method(method_id).and_then(|m| m.terminal) {
      let bin = registry.resolve_binary(&s.agent).await;
      let mut env: std::collections::BTreeMap<String, Option<String>> =
        def.env.clone().unwrap_or_default().into_iter().map(|(k, v)| (k, Some(v))).collect();
      for (k, v) in term.env.unwrap_or_default() {
        env.insert(k, Some(v));
      }
      let mut args = def.args.clone();
      args.extend(term.args);
      (self.deps.run_in_terminal)(title, bin.unwrap_or_else(|| def.command.clone()), args, (!env.is_empty()).then_some(env));
      (self.deps.toast)("info", &tp("host.loginThenRetry", &[("agent", &def.name)]));
      return Ok(());
    }
    let result: Result<()> = async {
      s.authenticate(method_id).await?;
      s.retry().await
    }
    .await;
    if let Err(e) = result {
      self.log(&format!("authenticate failed: {e}"));
      let Some(login) = &def.login else { return Err(e) };
      let bin = if login.command == def.command { registry.resolve_binary(&s.agent).await } else { None };
      (self.deps.run_in_terminal)(title, bin.unwrap_or_else(|| login.command.clone()), login.args.clone(), None);
      (self.deps.toast)("info", &tp("host.loginThenRetry", &[("agent", &def.name)]));
    }
    Ok(())
  }

  /// Run the registry's install line in the background on this engine's machine (`installer`): progress and log go to
  /// every webview, and a finished install is looked for at once instead of on the next poll
  fn install(&self, agent: &str) {
    let registry = self.registry();
    let Ok(def) = registry.get(agent) else { return };
    let Some(command) = registry.install(agent).and_then(|i| i.command) else { return };
    let name = def.name.clone();
    let messages = crate::acp::agents::installer::Messages {
      node_missing: t("host.installNeedsNode"),
      failed: Box::new(|code| match code {
        Some(code) => tp("host.installFailedCode", &[("code", &code.to_string())]),
        None => t("host.installFailed"),
      }),
      spawn: Box::new(|why| tp("host.installSpawnFailed", &[("error", why)])),
    };
    let me = self.me.clone();
    let id = agent.to_owned();
    let os = crate::platform::command::Os::current();
    self.installs.start(agent, command, os, messages, move |ok| {
      let Some(m) = me.upgrade() else { return };
      tokio::spawn(async move {
        // The installer may have just written the rc file that puts its directory on PATH
        if ok && login_path::refresh(LOGIN_PATH_REFRESH).await {
          m.log_login_path();
        }
        let found = m.registry().resolve_binary(&id).await.is_some();
        m.emit_agents();
        let key = match (ok, found) {
          (true, true) => Some(("info", "host.installDone")),
          (true, false) => Some(("info", "host.installDoneNotFound")),
          _ => None,
        };
        if let Some((level, key)) = key {
          (m.deps.toast)(level, &tp(key, &[("agent", &name)]));
        }
      });
    });
  }

  /// The install line in an IDE terminal, for an installer that wants a person at the keyboard; the poll notices the
  /// new executable
  fn install_in_terminal(&self, agent: &str) {
    let registry = self.registry();
    let Ok(def) = registry.get(agent) else { return };
    let Some(command) = registry.install(agent).and_then(|i| i.command) else { return };
    let (shell, flag) = if cfg!(windows) { ("powershell", "-Command") } else { ("bash", "-c") };
    (self.deps.run_in_terminal)(tp("host.installTerminalTitle", &[("agent", &def.name)]), shell.into(), vec![flag.into(), command], None);
    (self.deps.toast)("info", &tp("host.installThenDetect", &[("agent", &def.name)]));
  }

  /// End the agent processes of sessions nobody looks at: a live session no viewer shows, with nothing in flight
  /// (`AcpSession::releasable`), goes once it has been like that for `after`; beyond the `keep` most recently seen ones,
  /// once it has been like that for IDLE_KEEP_MIN (or `after`, when shorter). Its record is written, its lease released
  /// and the list entry stays; opening it again resumes the native session like after a window reload and replays its
  /// controls (`load_session`). Returns the ids released
  pub async fn release_idle(self: &Arc<Self>, after: Duration, keep: usize) -> Vec<String> {
    let now = tokio::time::Instant::now();
    let live = self.live_sessions();
    let shown: HashSet<String> = self.viewers().iter().filter_map(|v| v.active_id()).collect();
    let yielding = self.yielding.lock().clone();
    // Read outside the manager lock: a session reports its changes into the manager while holding its own lock
    let quiet: Vec<(Arc<AcpSession>, bool)> = live
      .into_iter()
      .map(|s| {
        let quiet = !shown.contains(&s.id) && !yielding.contains(&s.id) && s.releasable();
        (s, quiet)
      })
      .collect();
    let mut idle: Vec<(tokio::time::Instant, Arc<AcpSession>)> = vec![];
    {
      let mut st = self.state.lock();
      if st.disposed {
        return vec![];
      }
      let ids: HashSet<String> = st.live.keys().cloned().collect();
      st.idle_since.retain(|id, _| ids.contains(id));
      for (s, quiet) in quiet {
        if quiet {
          idle.push((*st.idle_since.entry(s.id.clone()).or_insert(now), s));
        } else {
          st.idle_since.remove(&s.id);
        }
      }
    }
    // Most recently seen first: the first `keep` of them only go after the full wait
    idle.sort_by_key(|(since, _)| std::cmp::Reverse(*since));
    let mut released = vec![];
    for (i, (since, s)) in idle.into_iter().enumerate() {
      let unseen = now.duration_since(since);
      if (unseen >= after || (i >= keep && unseen >= IDLE_KEEP_MIN.min(after))) && self.release(&s).await {
        released.push(s.id.clone());
      }
    }
    if !released.is_empty() {
      self.sync_leases();
      self.emit_sessions();
    }
    released
  }

  /// Close one idle session's agent process; false when a viewer landed on it or it changed meanwhile. Until the old
  /// process has closed the native session and exited, this engine keeps the lease (no other engine resumes it) and a
  /// reopen here waits (`load_session`)
  async fn release(self: &Arc<Self>, s: &Arc<AcpSession>) -> bool {
    if let Err(e) = self.deps.store.flush(s.clone() as Arc<dyn RecordSource>).await {
      self.log(&format!("session {}: idle release skipped, save failed: {e}", s.id));
      return false;
    }
    if !s.releasable() {
      return false;
    }
    let (closed_tx, closed_rx) = tokio::sync::watch::channel(false);
    {
      // A viewer selecting the session sets its active id before it looks the session up here: either this check sees
      // the viewer, or the lookup finds the session gone and loads it from the record just written
      let mut st = self.state.lock();
      let current = st.live.get(&s.id).is_some_and(|l| Arc::ptr_eq(l, s));
      if !current || st.viewers.iter().any(|v| v.active_id().as_deref() == Some(s.id.as_str())) {
        return false;
      }
      st.live.remove(&s.id);
      st.was_running.remove(&s.id);
      st.was_busy.remove(&s.id);
      st.was_waiting.remove(&s.id);
      st.settle_seq.remove(&s.id);
      st.health_seen.remove(&s.id);
      st.dirty.remove(&s.id);
      st.idle_since.remove(&s.id);
      st.closing.insert(s.id.clone(), closed_rx);
    }
    // A pin of its own holds the lease across the close; the session's turn pin (if any) can go now
    let close_pin = match self.leases.claim(&s.id) {
      Claim::Held(pin) => Some(pin),
      _ => None,
    };
    if let Some(pin) = s.drop_lease() {
      self.leases.release(&s.id, pin);
    }
    let (_, closing) = s.shutdown();
    let me = self.clone();
    let id = s.id.clone();
    tokio::spawn(async move {
      if let Some(f) = closing {
        f.await;
      }
      if let Some(pin) = close_pin {
        me.leases.release(&id, pin);
      }
      me.state.lock().closing.remove(&id);
      let _ = closed_tx.send(true);
    });
    self.log(&format!("session {}: idle, agent process released", s.id));
    true
  }

  pub async fn dispose(self: &Arc<Self>) {
    self.installs.cancel_all();
    self.bus.stop();
    {
      let mut st = self.state.lock();
      st.disposed = true;
      if let Some(h) = st.probe_timer.take() {
        h.abort();
      }
    }
    if let Some(c) = &self.deps.chatgpt {
      c.dispose().await;
    }
    // Agent processes are ended before the host returns (it exits right after): each session closes and kills its process, the pool
    // kills its idle ones, all at once; whatever has not exited within DISPOSE_GRACE is SIGKILLed so nothing outlives the host
    let live: Vec<Arc<AcpSession>> = self.state.lock().live.drain().map(|(_, s)| s).collect();
    let mut procs = Vec::new();
    let mut exits = tokio::task::JoinSet::new();
    for s in live {
      let (proc, closing) = s.shutdown();
      procs.extend(proc);
      if let Some(f) = closing {
        exits.spawn(f);
      }
      if let Err(e) = self.deps.store.flush(s as Arc<dyn RecordSource>).await {
        self.log(&format!("dispose flush failed: {e}"));
      }
    }
    let (pooled, pool_exit) = self.pool.dispose();
    exits.spawn(pool_exit);
    if tokio::time::timeout(DISPOSE_GRACE, async { while exits.join_next().await.is_some() {} }).await.is_err() {
      self.log("agent processes still running at shutdown; killing them");
      procs.extend(pooled.lock().iter().cloned());
      for p in procs {
        p.kill_hard();
      }
    }
    let trash: Vec<(String, Trash)> = self.state.lock().trash.drain().collect();
    for (id, t) in trash {
      t.timer.abort();
      self.deps.store.remove(&id).await;
    }
    self.state.lock().mirrors.clear();
    self.leases.release_all();
    let saves: Vec<_> = self.prefs_saves.lock().drain(..).collect();
    for h in saves {
      let _ = h.await;
    }
    self.state.lock().sync_due = None;
    self.deps.store.dispose().await;
    self.sync_index().await;
  }
}

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

/// A fork's agent was handed its copied turns as a clipped (possibly trimmed) history, never as its own conversation:
/// reading its own session is how it gets them in full. Any other session's own agent has its conversation in context
fn holds_unseen_turns(record: &SessionRecord) -> bool {
  record.forked_from.as_ref().is_some_and(|f| f.turn_index >= 0)
}

/// Same agent / account / cwd and still empty: keep the process instead of spawning another
/// The model the forked reply came from, as the source showed it: the prompt's recorded model resolved against the
/// source's model list, else the id the turn's usage reported, else the source's current model
fn fork_model_label(source: &SessionRecord, idx: usize) -> Option<String> {
  let control = source.controls.options.iter().find(|c| is_model_control(c));
  let name_of = |id: &str| control.and_then(|c| c.options.iter().find(|o| o.id == id)).map(|o| o.name.clone()).unwrap_or_else(|| id.to_owned());
  let prompt = source.turns[..idx].iter().rev().find_map(|t| match t {
    Turn::User(u) => Some(u),
    _ => None,
  });
  let chosen = control.and_then(|c| prompt.and_then(|u| u.settings.as_ref()).and_then(|s| s.config.get(&c.id)));
  let reported = source.turns[idx].as_agent().and_then(|a| a.usage.as_ref()).and_then(|u| u.model.as_deref());
  chosen.map(|id| name_of(id)).or_else(|| reported.map(name_of)).or_else(|| control.and_then(|c| c.value.as_deref()).map(name_of))
}

fn keep_empty(cur: &AcpSession, agent: &str, account: Option<&str>, cwd: &str) -> bool {
  if cur.agent != agent || cur.account_id().as_deref() != account || cur.cwd != cwd {
    return false;
  }
  if cur.turn_count() > 0 || cur.is_running() {
    return false;
  }
  matches!(cur.status(), SessionStatus::Starting | SessionStatus::Ready)
}

fn session_id_of(m: &WebviewMsg) -> Option<String> {
  use WebviewMsg as W;
  match m {
    W::Send { session_id, .. }
    | W::Stop { session_id }
    | W::SetMode { session_id, .. }
    | W::SetConfig { session_id, .. }
    | W::SelectAccount { session_id, .. }
    | W::Compact { session_id }
    | W::Login { session_id, .. }
    | W::Retry { session_id }
    | W::RetryTurn { session_id }
    | W::Reconnect { session_id }
    | W::TakeOverSession { session_id }
    | W::OpenInEditor { session_id } => session_id.clone(),
    W::Permission { session_id, .. }
    | W::Answer { session_id, .. }
    | W::BuildPlan { session_id, .. }
    | W::OpenPlan { session_id, .. }
    | W::ForkSession { session_id, .. }
    | W::ObserveSubagent { session_id, .. }
    | W::UnobserveSubagent { session_id, .. }
    | W::CancelSubagent { session_id, .. }
    | W::StopAsyncTask { session_id, .. }
    | W::Goal { session_id, .. }
    | W::Dequeue { session_id, .. }
    | W::SendQueued { session_id, .. }
    | W::SteerQueued { session_id, .. }
    | W::EditQueued { session_id, .. }
    | W::OpenFile { session_id, .. }
    | W::OpenBlob { session_id, .. }
    | W::ImportNativeSession { session_id, .. } => Some(session_id.clone()),
    _ => None,
  }
}

fn is_execution(m: &WebviewMsg) -> bool {
  use WebviewMsg as W;
  matches!(
    m,
    W::Send { .. }
      | W::Stop { .. }
      | W::Permission { .. }
      | W::Answer { .. }
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
      | W::Goal { .. }
  )
}

impl BusPeer for SessionManager {
  fn bus_watch(&self, session: &str, on: bool) {
    // A new subscriber: the bus sends the next view whole, so push the current one now rather than at the next change
    if on
      && let Some(s) = self.live(session)
      && let HostMsg::Session { parts: Some(p), .. } = s.view_msg()
    {
      self.bus.publish(session, &p);
    }
  }

  fn bus_view(&self, session: &str, frame: ViewFrame) {
    let built = {
      let mut st = self.state.lock();
      // Only for a session still shown read-only here
      if !st.mirrors.contains_key(session) || st.live.contains_key(session) {
        return;
      }
      let r = st.remote.entry(session.to_owned()).or_insert_with(|| Remote {
        epoch: crate::acp::session::next_view_epoch(),
        parts: None,
        msg: None,
      });
      match apply_frame(r.parts.as_ref().map(|p| &*p.0), &frame) {
        None => None,
        Some((turns, last)) => {
          let rev = self.next_mirror_rev();
          remote_view(session, &frame.head, rev, r.epoch, turns, last).map(|(parts, msg)| {
            r.parts = Some(parts);
            r.msg = Some(msg.clone());
            msg
          })
        }
      }
    };
    match built {
      Some(msg) => {
        for v in self.viewers_on(session, None) {
          v.emit(msg.clone());
        }
      }
      // A step this engine cannot apply: subscribing again makes the owner send the whole view
      None => {
        self.log(&format!("session {session}: pushed view out of step, asking for the whole view"));
        self.bus.subscribe(session);
      }
    }
  }

  fn bus_gone(&self, session: &str) {
    let mut st = self.state.lock();
    if st.remote.remove(session).is_some() {
      // Back to the record copy: re-read it now, its rev counts on from the pushed views
      st.refresh.insert(session.to_owned());
    }
  }

  fn bus_act(&self, session: String, msg: Value) -> BoxFuture<Result<(), String>> {
    let me = self.me.clone();
    Box::pin(async move {
      let Some(m) = me.upgrade() else { return Err("engine closing".into()) };
      m.run_forwarded(session, msg).await
    })
  }

  fn bus_owned(&self) -> Vec<String> {
    self.leases.held()
  }

  fn bus_wanted(&self) -> Vec<String> {
    self.state.lock().mirrors.keys().cloned().collect()
  }
}

/// A pushed view as this engine posts it: the owner's head with this engine's rev, the whole view spliced from the turns
fn remote_view(
  id: &str,
  head: &serde_json::value::RawValue,
  rev: i64,
  epoch: u64,
  turns: Vec<Box<serde_json::value::RawValue>>,
  last: Option<acpira_shared::session_patch::LastTurnParts>,
) -> Option<(ViewParts, HostMsg)> {
  let mut v: Value = serde_json::from_str(head.get()).ok()?;
  v["rev"] = Value::from(rev);
  let running = v["running"].as_bool().unwrap_or(false);
  let head = serde_json::value::to_raw_value(&v).ok()?;
  let whole = crate::bus::splice(&head, "turns", &turns)?;
  let parts = ViewParts(Arc::new(acpira_shared::session_patch::ViewPartsData { id: id.to_owned(), epoch, rev, head, turns, last }));
  let msg = HostMsg::Session { session: RawJson(Arc::from(whole)), running, parts: Some(parts.clone()) };
  Some((parts, msg))
}

/// What a page may do on a session another engine owns: everything that acts on the session itself, but not what moves
/// it here (take over, login) or binds it to this engine's accounts
fn is_forwarded(m: &WebviewMsg) -> bool {
  use WebviewMsg as W;
  (is_execution(m) && !matches!(m, W::TakeOverSession { .. } | W::Login { .. } | W::SelectAccount { .. } | W::Reconnect { .. }))
    || matches!(m, W::CancelSubagent { .. })
}

fn lower_first(s: &str) -> String {
  let mut c = s.chars();
  match c.next() {
    Some(f) => f.to_lowercase().collect::<String>() + c.as_str(),
    None => String::new(),
  }
}

/// Dirty sessions become pushes: the first change goes out at once, later ones within a quantum coalesce
async fn flush_loop(weak: Weak<SessionManager>) {
  loop {
    let Some(m) = weak.upgrade() else { return };
    let wake = m.wake.clone();
    let notified = wake.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    let batch: Vec<(String, bool)> = m.state.lock().dirty.drain().collect();
    if batch.is_empty() {
      drop(m);
      notified.await;
      continue;
    }
    for (id, _) in &batch {
      m.process_change(id);
    }
    drop(m);
    tokio::time::sleep(PUSH_QUANTUM).await;
  }
}

/// Every IDLE_SWEEP, background sessions with nothing in flight give their agent process back (`release_idle`)
async fn idle_loop(weak: Weak<SessionManager>) {
  loop {
    tokio::time::sleep(IDLE_SWEEP).await;
    let Some(m) = weak.upgrade() else { return };
    if m.state.lock().disposed {
      return;
    }
    m.release_idle(IDLE_RELEASE, IDLE_KEEP).await;
  }
}

/// One claim on a session's lease (`store::session_lease`), released when it drops
struct LeaseHold<'a> {
  leases: &'a SessionLeases,
  id: &'a str,
  pin: u64,
}

impl LeaseHold<'_> {
  /// The pin outlives this hold: whoever takes it releases it
  fn keep(mut self) -> u64 {
    std::mem::take(&mut self.pin)
  }
}

impl Drop for LeaseHold<'_> {
  fn drop(&mut self) {
    if self.pin != 0 {
      self.leases.release(self.id, self.pin);
    }
  }
}

/// Every blob reference in a transcript: prompt and steer attachments, agent image blocks, images a reply's markdown
/// embeds, and image results of tool calls
fn blob_slots<'a>(turns: &'a mut [Turn], out: &mut Vec<&'a mut Option<String>>) {
  fn image(c: &mut ToolContent) -> Option<&mut Option<String>> {
    match c {
      ToolContent::Image(r) => Some(&mut r.blob),
      _ => None,
    }
  }
  fn attachments<'a>(list: &'a mut Option<Vec<Attachment>>, out: &mut Vec<&'a mut Option<String>>) {
    for a in list.iter_mut().flatten() {
      match a {
        Attachment::Image { blob, .. } | Attachment::Text { blob, .. } | Attachment::Selection { blob, .. } => out.push(blob),
        Attachment::File { .. } | Attachment::Quote { .. } => {}
      }
    }
  }
  for turn in turns {
    match turn {
      Turn::User(u) => attachments(&mut u.attachments, out),
      Turn::Agent(a) => {
        for b in &mut a.blocks {
          match b {
            AgentBlock::Steer(s) => attachments(&mut s.attachments, out),
            AgentBlock::Image(i) => out.push(&mut i.blob),
            AgentBlock::Text(t) => out.extend(t.images.iter_mut().flatten().map(|r| &mut r.blob)),
            AgentBlock::ToolCall(c) => {
              out.extend(c.content.as_mut().and_then(image));
              out.extend(c.contents.iter_mut().flatten().filter_map(image));
            }
            _ => {}
          }
        }
      }
    }
  }
}
