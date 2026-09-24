//! Master of all sessions (mirror of src/host/SessionManager.ts): live processes, the summary list, the viewers; every
//! webview action enters here.
//!
//! Locking: the manager's own state is one short-held mutex that is never held while a session is locked or while
//! awaiting. A session's `on_change` only marks it dirty; one flush task turns dirty sessions into pushes, serializing a
//! view once for every viewer showing it

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::agent_order::{AgentPrefs, arrange_agents};
use acpira_shared::chatgpt_integration::{ChatGptIntegrationStatus, ProjectMirrors};
use acpira_shared::export_transcript::{ExportInput, ExportLabels, export_file_name, export_markdown};
use acpira_shared::inventory::{AgentHealth, AgentHealthStage, AgentRuntimeInfo, HealthSource};
use acpira_shared::model_shapes::learn_shape;
use acpira_shared::protocol::{
  AccountAction, AccountActionStatus, AddAccountVia, EditTurnRequest, ExportFormat, HostMsg, RawJson, WebviewMsg,
};
use acpira_shared::settings::{HiddenMap, in_workspace};
use acpira_shared::subagents::{StateSource, SubagentState};
use acpira_shared::transcript::*;
use acpira_shared::turn_settings::capture_turn_settings;

use crate::accounts::account_manager::{AccountHooks, AccountManager, RunInTerminal, Toast};
use crate::accounts::local::LocalAccounts;
use crate::acp::agent_pool::AgentPool;
use crate::acp::agent_registry::AgentRegistry;
use crate::acp::native_sessions::list_native_sessions;
use crate::acp::probe_controls::{ProbeResult, probe_agent_controls};
use crate::acp::rpc::BoxFuture;
use crate::acp::session::{AcpSession, CompactionPolicy, SessionAccountHooks, SessionDeps, StartOutcome};
use crate::external::chatgpt_events::CHATGPT_ID;
use crate::external::chatgpt_store::ChatGptBridgeStore;
use crate::external::desktop_commander::desktop_commander_status;
use crate::i18n::{t, tp};
use crate::limits::RENAME_MAX;
use crate::store::record::{ForkedFrom, ImportedFrom, RecordSource, SessionRecord};
use crate::store::transcript_store::{LogFn, SessionPrefs, TranscriptStore, is_session_id, sort_index};
use crate::util::{clip, local_stamp, ms_of_iso, now_iso, random_uuid};

const TRASH_TTL: Duration = Duration::from_secs(30);
const PROBE_INTERVAL: Duration = Duration::from_secs(10);
// How long shutdown waits for agent processes to close and exit; shells escalate to SIGTERM after 3 s, so this stays below it
const DISPOSE_GRACE: Duration = Duration::from_millis(2500);
const INDEX_DEBOUNCE: Duration = Duration::from_millis(400);
const INDEX_MAX_WAIT: Duration = Duration::from_millis(2000);
// Streamed updates coalesce into one push per session at this pace; an idle edge goes out at once
const PUSH_QUANTUM: Duration = Duration::from_millis(16);

pub type Sink = Arc<dyn Fn(HostMsg) + Send + Sync>;

pub struct ManagerDeps {
  pub store: Arc<TranscriptStore>,
  pub chatgpt: Option<Arc<ChatGptBridgeStore>>,
  pub log: LogFn,
  pub cwd: Arc<dyn Fn() -> String + Send + Sync>,
  pub default_agent: Arc<dyn Fn() -> String + Send + Sync>,
  pub agent_prefs: Arc<dyn Fn() -> AgentPrefs + Send + Sync>,
  pub run_in_terminal: RunInTerminal,
  pub toast: Toast,
  pub accounts: Option<Arc<AccountManager>>,
  pub local_accounts: Option<Arc<LocalAccounts>>,
  pub compaction: Arc<dyn Fn() -> CompactionPolicy + Send + Sync>,
  pub hidden: Arc<dyn Fn() -> HiddenMap + Send + Sync>,
  pub scope: Arc<dyn Fn() -> String + Send + Sync>,
}

#[derive(Default)]
struct ViewerState {
  active_id: Option<String>,
  observing: Option<(String, String)>,
  last_subagent_rev: Option<i64>,
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
  prefs: SessionPrefs,
  probe_timer: Option<tokio::task::AbortHandle>,
  touched: HashSet<String>,
  disposed: bool,
  sync_due: Option<(tokio::time::Instant, tokio::time::Instant)>,
  sync_running: bool,
  sync_again: bool,
  dirty: HashMap<String, bool>,
}

pub struct SessionManager {
  deps: ManagerDeps,
  state: parking_lot::Mutex<State>,
  pool: Arc<AgentPool>,
  wake: Arc<tokio::sync::Notify>,
  sync_lock: tokio::sync::Mutex<()>,
  viewer_seq: std::sync::atomic::AtomicU64,
  me: Weak<SessionManager>,
}

impl SessionManager {
  pub fn new(registry: Arc<AgentRegistry>, deps: ManagerDeps) -> Arc<Self> {
    let mgr = Arc::new_cyclic(|me: &Weak<SessionManager>| {
      let reg_me = me.clone();
      let env_me = me.clone();
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
          prefs: SessionPrefs::default(),
          probe_timer: None,
          touched: HashSet::new(),
          disposed: false,
          sync_due: None,
          sync_running: false,
          sync_again: false,
          dirty: HashMap::new(),
        }),
        deps,
        pool,
        wake: Arc::new(tokio::sync::Notify::new()),
        sync_lock: tokio::sync::Mutex::new(()),
        viewer_seq: Default::default(),
        me: me.clone(),
      }
    });
    if let Some(a) = &mgr.deps.accounts {
      let me = mgr.me.clone();
      a.subscribe(Arc::new(move |accounts| {
        if let Some(m) = me.upgrade() {
          m.emit(HostMsg::Accounts { accounts });
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
                v.emit(HostMsg::Session { session: raw.clone(), running: view.running });
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
    mgr
  }

  fn log(&self, line: &str) {
    (self.deps.log)(line);
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
    {
      let mut st = self.state.lock();
      st.index = index;
      st.prefs = prefs;
    }
    self.registry().probe_all().await;
    self.schedule_probe();
    if let Some(a) = self.deps.accounts.clone() {
      tokio::spawn(async move { a.refresh_quotas(None, false).await });
    }
    if let Some(l) = self.deps.local_accounts.clone() {
      tokio::spawn(async move { l.refresh(None, false).await });
    }
    let agent = self.default_agent();
    self.warm(&agent, None);
  }

  fn default_agent(&self) -> String {
    let preferred = (self.deps.default_agent)();
    let agents = self.agents();
    if !agents.iter().find(|a| a.id == preferred).is_some_and(|a| a.disabled == Some(true)) {
      return preferred;
    }
    let open: Vec<&AgentInfo> = agents.iter().filter(|a| a.external != Some(true) && a.disabled != Some(true)).collect();
    open.iter().find(|a| a.available != Some(false)).or(open.first()).map(|a| a.id.clone()).unwrap_or(preferred)
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
    let acc = account.or_else(|| self.default_account(agent));
    self.pool.ensure(agent, &(self.deps.cwd)(), acc.as_deref());
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
    let prefs = self.state.lock().prefs.clone();
    let store = self.deps.store.clone();
    let log = self.deps.log.clone();
    let agent = agent.to_owned();
    tokio::spawn(async move {
      if let Err(e) = store.save_prefs(&prefs, &[agent]).await {
        log(&format!("prefs save failed: {e}"));
      }
    });
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

  /// Reconcile the list with the sessions directory now
  pub async fn refresh_index(self: &Arc<Self>) {
    self.state.lock().sync_due = None;
    self.sync_index().await;
    self.sync_shapes().await;
    if let Some(c) = &self.deps.chatgpt {
      c.refresh().await;
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
        let (mine, own) = {
          let mut st = me.state.lock();
          st.sync_again = false;
          let mut own: HashSet<String> = st.live.keys().cloned().collect();
          own.extend(st.touched.drain());
          (st.index.clone(), own)
        };
        match me.deps.store.sync_index(&mine, &own).await {
          Ok(merged) => {
            let trash: HashSet<String> = me.state.lock().trash.keys().cloned().collect();
            let mut merged: Vec<SessionSummary> = merged.into_iter().filter(|s| !trash.contains(&s.id)).collect();
            let live = me.live_sessions();
            let gone: Vec<String> =
              live.iter().filter(|s| me.deps.store.knew(&s.id) && !merged.iter().any(|m| m.id == s.id)).map(|s| s.id.clone()).collect();
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

  pub fn agents(&self) -> Vec<AgentInfo> {
    let registry = self.registry();
    let mut all: Vec<AgentInfo> = registry
      .list()
      .into_iter()
      .filter(|a| self.deps.chatgpt.is_none() || a.id != CHATGPT_ID)
      .map(|mut a| {
        if self.deps.accounts.as_ref().is_some_and(|m| m.supports(&a.id)) {
          a.accounts = Some(true);
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
        None => self.deps.store.load(&s.id).await.map(|r| r.controls.options).unwrap_or_default(),
      };
      if !options.is_empty() {
        return options;
      }
    }
    vec![]
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
    match probe_agent_controls(&def, &bin, &(self.deps.cwd)(), env.as_ref(), self.deps.log.clone(), None).await {
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

  pub async fn list_native_sessions(&self, agent: &str) -> Result<Vec<NativeSessionInfo>> {
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
    let listed = list_native_sessions(&def, &bin, &(self.deps.cwd)(), env.as_ref(), self.deps.log.clone(), None).await?;
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
      history_pending: false,
      forked_from: None,
      import_pending: true,
      imported_from: Some(ImportedFrom { session_id: session_id.to_owned() }),
      subagents: None,
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
    let (index, live) = {
      let st = self.state.lock();
      (st.index.clone(), st.live.clone())
    };
    let mut merged: Vec<SessionSummary> = index
      .into_iter()
      .map(|mut s| {
        s.state = live.get(&s.id).and_then(|l| l.list_state());
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
    if let Some(s) = self.live(id) {
      return Some(s.view_json());
    }
    let view = self.deps.chatgpt.as_ref()?.view(id)?;
    Some((RawJson::new(&view), view.running))
  }

  fn has_view(&self, id: &str) -> bool {
    self.live(id).is_some() || self.deps.chatgpt.as_ref().is_some_and(|c| c.view(id).is_some())
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
    let v = Arc::new(Viewer {
      id: self.viewer_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
      state: parking_lot::Mutex::new(ViewerState { active_id: id, ..Default::default() }),
      sink: Default::default(),
    });
    self.state.lock().viewers.push(v.clone());
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
    let mut view: Option<(RawJson, bool)> = None;
    for v in self.viewers() {
      if v.active_id().as_deref() != Some(s.id.as_str()) {
        continue;
      }
      let (raw, running) = view.get_or_insert_with(|| s.view_json()).clone();
      v.emit(HostMsg::Session { session: raw, running });
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
  }

  fn emit_sessions(&self) {
    self.emit(HostMsg::Sessions { sessions: self.sessions() });
  }

  fn set_active(&self, v: &Viewer, id: Option<String>) {
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
    self.emit_session(&s);
    self.emit_sessions();
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

  fn session_deps(&self) -> SessionDeps {
    let me = self.me.clone();
    let shapes_me = self.me.clone();
    let toast = self.deps.toast.clone();
    let compaction = self.deps.compaction.clone();
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
      pool: Some(self.pool.clone()),
      model_shapes: Some(Arc::new(move |agent: &str| {
        shapes_me.upgrade().and_then(|m| m.state.lock().prefs.model_shapes.as_ref().and_then(|s| s.get(agent).cloned()))
      })),
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
      None => {
        if let Err(e) = self.new_session_for(v, None, None).await {
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
  ) -> BoxFuture<Result<()>> {
    let me = self.clone();
    let v = v.clone();
    Box::pin(async move {
      let id = agent.unwrap_or_else(|| me.default_agent());
      if id == CHATGPT_ID {
        let view = me.connect_chatgpt(None, "ChatGPT").await?;
        me.drop_empty_current(&v).await;
        me.set_active(&v, Some(view.id.clone()));
        v.emit(HostMsg::Session { session: RawJson::new(&view), running: view.running });
        return Ok(());
      }
      let acc = if me.deps.accounts.as_ref().is_some_and(|a| a.supports(&id)) { account.or_else(|| me.default_account(&id)) } else { None };
      let cwd = (me.deps.cwd)();
      if let Some(cur) = me.current(&v)
        && keep_empty(&cur, &id, acc.as_deref(), &cwd)
      {
        return Ok(());
      }
      me.drop_empty_current(&v).await;
      // Inheritable settings are snapped before the session starts spawning
      let last = me.last_settings(&id);
      let s = AcpSession::fresh(&id, &cwd, me.session_deps(), acc);
      s.preview_controls(&me.known_controls(&id).await, last.as_ref());
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

  pub async fn chatgpt_status(&self) -> ChatGptIntegrationStatus {
    if let Some(c) = &self.deps.chatgpt {
      c.refresh().await;
    }
    let raw_cwd = (self.deps.cwd)();
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

  pub async fn connect_chatgpt(&self, source_key: Option<String>, title: &str) -> Result<SessionView> {
    let c = self.deps.chatgpt.as_ref().ok_or_else(|| anyhow!("ChatGPT bridge is unavailable in this host"))?;
    c.open(&source_key.unwrap_or_else(random_uuid), &(self.deps.cwd)(), title).await
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
          (me.deps.toast)("error", &t("host.recordLost"));
          return;
        };
        me.set_active(&v, Some(id.clone()));
        v.emit(HostMsg::Session { session: RawJson::new(&view), running: view.running });
        me.emit_sessions();
        return;
      }
      if v.active_id().as_deref() == Some(id.as_str()) && me.live(&id).is_some() {
        return;
      }
      me.set_active(&v, Some(id.clone()));
      if let Some(live) = me.live(&id) {
        let (raw, running) = live.view_json();
        v.emit(HostMsg::Session { session: raw, running });
        me.emit_sessions();
        return;
      }
      // A load already running for this record: wait for it instead of building a second session on the same id
      let pending = me.state.lock().loading.get(&id).cloned();
      if let Some(mut rx) = pending {
        let _ = rx.wait_for(|done| *done).await;
        if let Some(s) = me.live(&id)
          && v.active_id().as_deref() == Some(id.as_str())
        {
          let (raw, running) = s.view_json();
          v.emit(HostMsg::Session { session: raw, running });
          me.emit_sessions();
        }
        return;
      }
      let (tx, rx) = tokio::sync::watch::channel(false);
      me.state.lock().loading.insert(id.clone(), rx);
      me.load_session(&id).await;
      me.state.lock().loading.remove(&id);
      let _ = tx.send(true);
    })
  }

  async fn load_session(self: &Arc<Self>, id: &str) {
    let Some(record) = self.deps.store.load(id).await else {
      (self.deps.toast)("error", &t("host.recordLost"));
      self.state.lock().index.retain(|s| s.id != id);
      self.emit_sessions();
      self.save_index();
      return;
    };
    if self.live(id).is_some() {
      return;
    }
    let s = AcpSession::new(record, self.session_deps());
    self.state.lock().live.insert(id.to_owned(), s.clone());
    self.emit_session(&s);
    s.start().await;
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
      let text = e.to_string();
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
    let valid = |id: &str| is_session_id(id);
    match m {
      W::ConnectChatgpt => self.new_session_for(v, Some(CHATGPT_ID.into()), None).await?,
      W::Send { session_id, text, attachments } => {
        if let Some(s) = self.target(v, session_id.as_deref()) {
          s.prompt(text, attachments, false, None, None).await;
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
      W::NewSession { agent } => self.new_session_for(v, agent, None).await?,
      W::RenameSession { id, title } => self.rename_session(&id, &title).await?,
      W::DeleteSession { id } => self.delete_session(&id).await?,
      W::RestoreSession { id } => self.restore_session(&id).await?,
      W::PinSession { id, pinned } => self.pin_session(&id, pinned).await?,
      W::MoveSession { id } => self.move_session(&id).await?,
      W::ForkSession { session_id, turn_index } => self.fork_session(v, &session_id, turn_index).await?,
      W::ObserveSubagent { session_id, subagent_id } => {
        {
          let mut vs = v.state.lock();
          vs.observing = Some((session_id.clone(), subagent_id.clone()));
          vs.last_subagent_rev = None;
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
      W::RefreshQuota { agent } => {
        let a = self.deps.accounts.clone();
        let l = self.deps.local_accounts.clone();
        let (ag1, ag2) = (agent.clone(), agent);
        tokio::join!(
          async move {
            if let Some(a) = a {
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
      _ => {}
    }
    Ok(())
  }

  /// Rename / pin: a live session mutates itself; one not loaded is patched on disk
  pub async fn rename_session(&self, id: &str, title: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.rename(id, title).await;
    }
    let t = clip(title.trim(), RENAME_MAX);
    if t.is_empty() {
      return Ok(());
    }
    if let Some(live) = self.live(id) {
      live.rename(&t);
      return Ok(());
    }
    self.patch_record(id, |r| r.title = t).await;
    Ok(())
  }

  pub async fn pin_session(&self, id: &str, pinned: bool) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    if let Some(c) = self.deps.chatgpt.as_ref().filter(|c| c.owns(id)) {
      return c.pin(id, pinned).await;
    }
    if let Some(live) = self.live(id) {
      live.set_pinned(pinned);
      return Ok(());
    }
    self.patch_record(id, |r| r.pinned = pinned.then_some(true)).await;
    Ok(())
  }

  async fn patch_record(&self, id: &str, patch: impl FnOnce(&mut SessionRecord)) {
    let Some(mut r) = self.deps.store.load(id).await else { return };
    patch(&mut r);
    if let Err(e) = self.deps.store.flush(Arc::new(r.clone())).await {
      self.log(&format!("session {id}: save failed ({e})"));
    }
    self.replace_summary(&r);
  }

  /// Soft deletion with a 30-second undo window; viewers showing it move on
  pub async fn delete_session(self: &Arc<Self>, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
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
      st.health_seen.remove(id);
      st.dirty.remove(id);
      st.live.remove(id)
    };
    if let Some(s) = live {
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
          None => {
            if let Err(e) = me.new_session_for(&v, None, None).await {
              me.log(&format!("new session failed: {e}"));
            }
          }
        }
      }
    })
  }

  /// Re-home a session into this window's workspace folder
  pub async fn move_session(self: &Arc<Self>, id: &str) -> Result<()> {
    if !is_session_id(id) {
      return Ok(());
    }
    if self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(id)) {
      return Err(anyhow!(t("chatgpt.projectBound")));
    }
    let cwd = (self.deps.cwd)();
    if let Some(live) = self.live(id) {
      if live.cwd == cwd {
        return Ok(());
      }
      if live.is_running() {
        return Err(anyhow!(t("host.moveWhileRunning")));
      }
      let mut record = live.to_record();
      self.forget(id);
      record.cwd = cwd;
      self.deps.store.flush(Arc::new(record.clone())).await.ok();
      self.replace_summary(&record);
      for v in self.viewers_on(id, None) {
        self.set_active(&v, None);
        self.select_session_for(&v, id).await;
      }
      return Ok(());
    }
    self.patch_record(id, |r| r.cwd = cwd).await;
    Ok(())
  }

  /// Fork from an agent turn: a fresh session whose transcript is the source's turns up to that reply
  pub async fn fork_session(self: &Arc<Self>, v: &Arc<Viewer>, source_id: &str, turn_index: i64) -> Result<()> {
    if !is_session_id(source_id) {
      return Ok(());
    }
    if self.deps.chatgpt.as_ref().is_some_and(|c| c.owns(source_id)) {
      return Err(anyhow!(t("chatgpt.externalOnly")));
    }
    let live = self.live(source_id);
    let source = match &live {
      Some(l) => Some(l.to_record()),
      None => self.deps.store.load(source_id).await,
    }
    .ok_or_else(|| anyhow!(t("host.recordLost")))?;
    let idx = usize::try_from(turn_index)
      .ok()
      .filter(|i| matches!(source.turns.get(*i), Some(Turn::Agent(_))))
      .ok_or_else(|| anyhow!(t("host.forkStale")))?;
    if live.as_ref().is_some_and(|l| l.is_running()) && idx == source.turns.len() - 1 {
      return Err(anyhow!(t("host.forkRunning")));
    }
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
    let mut record = SessionRecord {
      id: random_uuid(),
      agent: source.agent.clone(),
      account_id: source.account_id.clone(),
      acp_session_id: None,
      cwd: source.cwd.clone(),
      title: clip(&tp("session.forkTitle", &[("title", &source.title)]), RENAME_MAX),
      created_at: now.clone(),
      updated_at: now,
      turns,
      controls: source.controls.clone(),
      usage: None,
      commands: vec![],
      pinned: None,
      history_pending: true,
      forked_from: Some(ForkedFrom { session_id: source.id.clone(), turn_index }),
      import_pending: false,
      imported_from: None,
      subagents: (!subagents.is_empty()).then_some(subagents),
    };
    // Attachment blobs are re-saved under the fork's own blob dir (content-hash names keep the same file name)
    let fork_id = record.id.clone();
    for turn in &mut record.turns {
      let Turn::User(u) = turn else { continue };
      for a in u.attachments.iter_mut().flatten() {
        let blob = match a {
          Attachment::Image { blob, .. } | Attachment::Text { blob, .. } => blob,
          Attachment::File { .. } => continue,
        };
        let Some(name) = blob.clone() else { continue };
        let ext = std::path::Path::new(&name).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        match self.deps.store.read_blob(&source.id, &name).await {
          Ok(bytes) => match self.deps.store.save_blob(&fork_id, &ext, &bytes).await {
            Ok((n, _)) => *blob = Some(n),
            Err(e) => {
              self.log(&format!("fork: blob {name} not copied ({e})"));
              *blob = None;
            }
          },
          Err(e) => {
            self.log(&format!("fork: blob {name} not copied ({e})"));
            *blob = None;
          }
        }
      }
    }
    self.deps.store.flush(Arc::new(record.clone())).await?;
    self.drop_empty_current(v).await;
    let s = AcpSession::new(record, self.session_deps());
    self.state.lock().live.insert(s.id.clone(), s.clone());
    self.set_active(v, Some(s.id.clone()));
    self.process_change(&s.id);
    s.start().await;
    if s.status() == SessionStatus::Ready {
      s.adopt_controls(capture_turn_settings(&source.controls)).await;
    }
    Ok(())
  }

  /// Write the session as Markdown or JSON under exports/
  pub async fn export_session(&self, id: &str, format: ExportFormat) -> Result<std::path::PathBuf> {
    let record = match self.live(id) {
      Some(l) => Some(l.to_record()),
      None => self.deps.store.load(id).await,
    };
    let (title, agent, cwd, turns, json) = match record {
      Some(r) => (r.title.clone(), r.agent.clone(), r.cwd.clone(), r.turns.clone(), serde_json::to_string_pretty(&r)?),
      None => {
        let view =
          self.deps.chatgpt.as_ref().filter(|c| c.owns(id)).and_then(|c| c.view(id)).ok_or_else(|| anyhow!(t("host.recordLost")))?;
        let json = serde_json::to_string_pretty(&view)?;
        (view.title, view.agent, view.cwd, view.turns, json)
      }
    };
    let agent_name = self.agents().into_iter().find(|a| a.id == agent).map(|a| a.name).unwrap_or(agent);
    let content = match format {
      ExportFormat::Json => json,
      ExportFormat::Markdown => {
        let labels = ExportLabels {
          user: t("export.label.user"),
          agent: t("export.label.agent"),
          project: t("export.label.project"),
          exported: t("export.label.exported"),
          attachments: t("export.label.attachments"),
          thinking: t("export.label.thinking"),
          compacted: t("export.label.compacted"),
          auto_compact: t("export.label.autoCompact"),
          error: t("export.label.error"),
        };
        let store = self.deps.store.clone();
        let sid = id.to_owned();
        export_markdown(
          &ExportInput { title: &title, agent_name: &agent_name, cwd: &cwd, exported_at: &now_iso(), turns: &turns },
          &labels,
          &move |name| store.blob_path(&sid, name).map(|p| p.to_string_lossy().into_owned()),
        )
      }
    };
    self.deps.store.write_export(&export_file_name(&title, format == ExportFormat::Markdown, &local_stamp()), &content).await
  }

  /// The working directory of a session a viewer shows (live, or a ChatGPT mirror)
  pub fn session_cwd(&self, id: &str) -> Option<String> {
    if let Some(s) = self.live(id) {
      return Some(s.cwd.clone());
    }
    self.deps.chatgpt.as_ref().and_then(|c| c.view(id)).map(|v| v.cwd)
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
    let action = |status, error| AccountAction { agent: agent.to_owned(), via, status, error };
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

  /// Run the vendor's install line in a terminal; the poll notices the new executable
  fn install(&self, agent: &str) {
    let registry = self.registry();
    let Ok(def) = registry.get(agent) else { return };
    let Some(command) = registry.install(agent).and_then(|i| i.command) else { return };
    let (shell, flag) = if cfg!(windows) { ("powershell", "-Command") } else { ("bash", "-c") };
    (self.deps.run_in_terminal)(tp("host.installTerminalTitle", &[("agent", &def.name)]), shell.into(), vec![flag.into(), command], None);
    (self.deps.toast)("info", &tp("host.installThenDetect", &[("agent", &def.name)]));
  }

  pub async fn dispose(self: &Arc<Self>) {
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
    self.state.lock().sync_due = None;
    self.deps.store.dispose().await;
    self.sync_index().await;
  }
}

/// Same agent / account / cwd and still empty: keep the process instead of spawning another
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
    | W::Dequeue { session_id, .. }
    | W::SendQueued { session_id, .. }
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
      | W::Dequeue { .. }
      | W::SendQueued { .. }
      | W::EditQueued { .. }
      | W::Login { .. }
      | W::StopAsyncTask { .. }
  )
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
