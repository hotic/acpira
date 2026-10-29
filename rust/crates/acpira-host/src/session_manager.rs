//! Master of all sessions: live processes, the summary list, the viewers; every
//! webview action enters here.
//!
//! Locking: the manager's own state is one short-held mutex that is never held while a session is locked or while
//! awaiting. A session's `on_change` only marks it dirty; one flush task turns dirty sessions into pushes, serializing a
//! view once for every viewer showing it

mod accounts;
mod agents;
mod bus_peer;
mod changes;
mod dispatch;
mod export;
mod fork;
mod hosts;
mod idle;
mod leases;
mod library;
mod open;
mod projects;
mod records;
pub mod storage;
mod unread;
mod viewers;

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use acpira_shared::agent_order::AgentPrefs;
use acpira_shared::inventory::AgentHealth;
use acpira_shared::protocol::{AccountAction, ExportFormat, HostMsg, RawJson, WebviewMsg};
use acpira_shared::settings::HiddenMap;
use acpira_shared::transcript::*;

use crate::accounts::account_manager::{AccountManager, RunInTerminal, Toast};
use crate::accounts::local::LocalAccounts;
use crate::acp::agents::login_path;
use crate::acp::agents::pool::AgentPool;
use crate::acp::agents::probe_controls::ProbeResult;
use crate::acp::agents::registry::AgentRegistry;
use crate::acp::session::errors::error_text;
use crate::acp::session::{AcpSession, CompactionPolicy, StartOutcome};
use crate::acp::transport::rpc::BoxFuture;
use crate::bus::{Bus, BusPeer};
use crate::external::chatgpt_store::ChatGptBridgeStore;
use crate::i18n::{t, tp};
use crate::store::record::SessionRecord;
use crate::store::session_lease::SessionLeases;
use crate::store::session_search::SessionSearch;
use crate::store::transcript_store::{LogFn, RecordLoadError, SessionPrefs, TranscriptStore, UnreadOp};
use acpira_shared::session_patch::ViewParts;

use self::dispatch::is_execution;
use self::idle::idle_loop;
use self::viewers::flush_loop;

const TRASH_TTL: Duration = Duration::from_secs(30);
/// Under ACPIRA_HOME: one folder per chat (`chat_root`)
const CHATS_DIR: &str = "chats";
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
  /// The open card's request, the turn's error or its reply's opening (`AcpSession::attention_detail`)
  pub detail: String,
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
        copy_of: None,
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
  /// The plugins' hooks (`plugins::host`), handed to every session; None in tests that do not need them
  pub plugin_hooks: Option<crate::hooks::PluginHooks>,
  /// The plugins' prompt folders for the built-in agent's sessions; None in tests that do not need them
  pub plugin_prompts: Option<crate::hooks::PluginPrompts>,
  pub plugin_runtime: Option<Arc<crate::plugins::runtime::Runtimes>>,
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
  /// Opened with `InitialView::Empty`: shows nothing until a session is selected on it
  empty: bool,
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
  // Mirrored records that are copies of another machine's session (`remote::host_store`): never opened here
  copies: HashSet<String>,
  // Mirrored sessions whose owner pushes their view over the bus (`bus.rs`): shown instead of the record copy, and the
  // pages' actions on them go to the owner
  remote: HashMap<String, Remote>,
  // Mirrored sessions to re-read on the next poll whatever the record's stamp (their pushed view just ended)
  refresh: HashSet<String>,
  // The list entries of sessions other engines run, per engine (its connection on the bus router): this list shows them
  // as their owner does (working / waiting / failed, title, pin, model …), not only their transcripts
  elsewhere: HashMap<u64, Vec<SessionSummary>>,
  // Owner: subagents another engine's pages observe, with the transcript rev last sent there
  node_watch: HashMap<(String, String), Option<i64>>,
  // Follower: the subagent subscriptions sent for sessions shown read-only, and the transcripts their owners pushed
  node_subs: HashSet<(String, String)>,
  remote_nodes: HashMap<(String, String), (i64, bool, RawJson)>,
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
  // SSH hosts in remote run: their engines' sessions in this list (`hosts.rs`)
  hosts: hosts::Hosts,
  // Unread marks waiting to reach the shared store, written one batch at a time (`persist_unread`)
  unread_ops: parking_lot::Mutex<UnreadQueue>,
  // What other parts of the engine re-read on another engine's `touch` (`plugins` → the plugin host)
  touch_hooks: parking_lot::Mutex<Vec<(&'static str, Arc<dyn Fn() + Send + Sync>)>>,
  me: Weak<SessionManager>,
}

#[derive(Default)]
struct UnreadQueue {
  ops: Vec<UnreadOp>,
  running: bool,
}

impl SessionManager {
  /// Run `f` whenever another engine touches `what` on the bus (a shared file it changed)
  pub fn on_bus_touch(&self, what: &'static str, f: Arc<dyn Fn() + Send + Sync>) {
    self.touch_hooks.lock().push((what, f));
  }

  /// Tell the other engines that `what` changed in the shared store
  pub fn touch_others(&self, what: &str) {
    self.bus.touch(what);
  }

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
          copies: HashSet::new(),
          remote: HashMap::new(),
          refresh: HashSet::new(),
          elsewhere: HashMap::new(),
          node_watch: HashMap::new(),
          node_subs: HashSet::new(),
          remote_nodes: HashMap::new(),
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
        hosts: Default::default(),
        unread_ops: Default::default(),
        touch_hooks: Default::default(),
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
    // A plugin's `prompt` hook sends a prompt to another agent: a new session for it, here. The plugins' commands
    // changed: every open session's menu shows them
    if let Some(rt) = &mgr.deps.plugin_runtime {
      let menus = mgr.me.clone();
      rt.on_contributed(Arc::new(move || {
        if let Some(m) = menus.upgrade() {
          for s in m.live_sessions() {
            s.republish();
          }
        }
      }));
      let me = mgr.me.clone();
      rt.set_redirect(Arc::new(move |r| {
        let me = me.clone();
        Box::pin(async move {
          match me.upgrade() {
            Some(m) => m.redirect_prompt(r).await.map_err(|e| error_text(&e)),
            None => Err("the engine is closing".into()),
          }
        })
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
      // Unread marks are shared and outlive engines: a reply that came in before a restart still shows its dot
      st.unread = prefs.unread.iter().cloned().collect();
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
    self.start_hosts();
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

}

fn session_id_of(m: &WebviewMsg) -> Option<String> {
  use WebviewMsg as W;
  match m {
    W::Send { session_id, .. }
    | W::Stop { session_id, .. }
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
    | W::ImageBlob { session_id, .. }
    | W::ImportNativeSession { session_id, .. } => Some(session_id.clone()),
    _ => None,
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
  // The head comes from another engine: not an object gives None instead of a panic in `IndexMut`
  v.as_object_mut()?.insert("rev".to_owned(), Value::from(rev));
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

