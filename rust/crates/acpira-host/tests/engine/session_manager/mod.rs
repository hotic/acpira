//! test/SessionManager.test.ts: the session pool, viewers, the list and its persistence, against test/fake-agent.ts

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::accounts::local::LocalAccounts;
use acpira_host::acp::agents::registry::AgentRegistry;
use acpira_host::acp::session::{AcpSession, CompactionPolicy, SessionDeps};
#[allow(unused_imports)]
use std::path::PathBuf;
use acpira_host::session_manager::{ManagerDeps, SessionManager, Viewer};
use acpira_host::store::transcript_store::TranscriptStore;
use acpira_shared::agent_order::AgentPrefs;
use acpira_shared::protocol::WebviewMsg;
use acpira_shared::settings::HiddenMap;

use crate::fake_or_skip;
use crate::support::{FakeAgent, expect_match, turns_in, until, v};

mod persistence;
mod agents;
mod viewers;
mod prefs;
mod windows;
mod fork;
mod native;

type Terminal = (String, Vec<String>, Option<BTreeMap<String, Option<String>>>);

/// What a manager is built from; every field has the TS suite's default
#[derive(Clone)]
pub struct Opts {
  pub agents: Value,
  pub default_agent: String,
  pub cwd: Arc<Mutex<String>>,
  pub scope: Arc<Mutex<String>>,
  pub prefs: Arc<Mutex<AgentPrefs>>,
  pub hidden: HiddenMap,
  pub local_accounts: Option<Arc<LocalAccounts>>,
  pub accounts: Option<Arc<acpira_host::accounts::account_manager::AccountManager>>,
}

impl Opts {
  pub fn fake(fake: &FakeAgent) -> Opts {
    Opts::with_agents(fake.setting(json!({})), "fake")
  }

  pub fn with_agents(agents: Value, default_agent: &str) -> Opts {
    Opts {
      agents,
      default_agent: default_agent.into(),
      cwd: Arc::new(Mutex::new("/tmp".into())),
      scope: Arc::new(Mutex::new("all".into())),
      prefs: Arc::new(Mutex::new(AgentPrefs::default())),
      hidden: HiddenMap::default(),
      local_accounts: None,
      accounts: None,
    }
  }

  pub fn cwd(self, cwd: &str) -> Opts {
    *self.cwd.lock().unwrap() = cwd.into();
    self
  }
}

/// A SessionManager plus the default viewer the TS manager carried (`m.newSession()`, `m.activeId`, `m.handle()`)
pub struct Mgr {
  pub m: Arc<SessionManager>,
  pub v: Arc<Viewer>,
  pub events: Arc<Mutex<Vec<Value>>>,
  pub logs: Arc<Mutex<Vec<String>>>,
  pub toasts: Arc<Mutex<Vec<String>>>,
  pub terminals: Arc<Mutex<Vec<Terminal>>>,
  pub store: Arc<TranscriptStore>,
}

pub fn record_events(v: &Viewer) -> Arc<Mutex<Vec<Value>>> {
  let events = Arc::new(Mutex::new(vec![]));
  let e = events.clone();
  v.subscribe(Arc::new(move |m| e.lock().unwrap().push(crate::support::v(&m))));
  events
}

impl Mgr {
  pub fn new(dir: &Path, opts: Opts) -> Mgr {
    let (logs, toasts, terminals) = (Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(vec![])), Arc::new(Mutex::new(vec![])));
    let (l, t, tt) = (logs.clone(), toasts.clone(), terminals.clone());
    let log: acpira_host::store::transcript_store::LogFn = Arc::new(move |line: &str| l.lock().unwrap().push(line.to_owned()));
    let store = TranscriptStore::new(dir.to_path_buf(), log.clone(), None);
    let (cwd, scope, prefs, hidden, default_agent) = (opts.cwd.clone(), opts.scope.clone(), opts.prefs.clone(), opts.hidden.clone(), opts.default_agent.clone());
    let m = SessionManager::new(
      Arc::new(AgentRegistry::new(&opts.agents)),
      ManagerDeps {
        store: store.clone(),
        chatgpt: None,
        log,
        cwd: Arc::new(move || cwd.lock().unwrap().clone()),
        default_agent: Arc::new(move || default_agent.clone()),
        agent_prefs: Arc::new(move || prefs.lock().unwrap().clone()),
        run_in_terminal: Arc::new(move |_title, command, args, env| tt.lock().unwrap().push((command, args, env))),
        toast: Arc::new(move |_level, text| t.lock().unwrap().push(text.to_owned())),
        accounts: opts.accounts.clone(),
        local_accounts: opts.local_accounts.clone(),
        compaction: Arc::new(|| CompactionPolicy { at_tokens: 300_000.0, auto: false }),
        hidden: Arc::new(move || hidden.clone()),
        scope: Arc::new(move || scope.lock().unwrap().clone()),
        host_mcp: None,
      },
    );
    let v = m.attach(None);
    let events = record_events(&v);
    Mgr { m, v, events, logs, toasts, terminals, store }
  }

  pub async fn init(&self) {
    self.m.init().await;
  }

  pub fn active_id(&self) -> Option<String> {
    self.v.active_id()
  }

  pub fn view_of(&self, id: &str) -> Option<Value> {
    self.m.view_of(Some(id)).map(|(raw, _)| serde_json::from_str(raw.get()).unwrap())
  }

  pub fn active(&self) -> Option<Value> {
    self.view_of(&self.active_id()?)
  }

  pub fn active_of(&self, viewer: &Viewer) -> Option<Value> {
    self.view_of(&viewer.active_id()?)
  }

  pub async fn new_session(&self, agent: Option<&str>) {
    self.m.new_session_for(&self.v, agent.map(str::to_owned), None).await.unwrap();
  }

  pub async fn handle(&self, msg: Value) {
    self.handle_on(&self.v, msg).await;
  }

  pub async fn handle_on(&self, viewer: &Arc<Viewer>, msg: Value) {
    let m: WebviewMsg = serde_json::from_value(msg).expect("webview message");
    self.m.handle_for(viewer, m).await;
  }

  /// A send left running (the TS `const sending = m.handle({ type: 'send', … })`)
  pub fn spawn_handle(&self, msg: Value) -> tokio::task::JoinHandle<()> {
    let (m, viewer) = (self.m.clone(), self.v.clone());
    let msg: WebviewMsg = serde_json::from_value(msg).expect("webview message");
    tokio::spawn(async move { m.handle_for(&viewer, msg).await })
  }

  pub fn sessions(&self) -> Vec<Value> {
    self.m.sessions().iter().map(v).collect()
  }

  pub fn session_ids(&self) -> Vec<String> {
    self.m.sessions().into_iter().map(|s| s.id).collect()
  }

  pub fn toasts(&self) -> Vec<String> {
    self.toasts.lock().unwrap().clone()
  }

  pub fn logs(&self) -> Vec<String> {
    self.logs.lock().unwrap().clone()
  }

  pub async fn dispose(&self) {
    self.m.dispose().await;
  }
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
  v.sort();
  v
}

fn agent_setting(fake: &FakeAgent, id: &str, extra: Value) -> Value {
  fake.setting_as(id, extra)[id].clone()
}

fn merged(parts: &[(&str, Value)]) -> Value {
  Value::Object(parts.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

fn last_turn(view: &Value) -> Value {
  view["turns"].as_array().and_then(|t| t.last().cloned()).unwrap_or(Value::Null)
}

fn turns_len(view: Option<Value>) -> usize {
  view.and_then(|x| x["turns"].as_array().map(|t| t.len())).unwrap_or(0)
}

/// A session some other process ran on the fake's native store, never seen by the manager
async fn foreign_native_session(fake: &FakeAgent, native: &Path, store: &Arc<TranscriptStore>, cwd: &str) -> (String, acpira_host::store::record::SessionRecord) {
  let deps = SessionDeps {
    registry: Arc::new(AgentRegistry::new(&fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })))),
    log: Arc::new(|_: &str| {}),
    on_change: Arc::new(|_, _| {}),
    blobs: store.clone(),
    notify: None,
    accounts: None,
    compaction: None,
    pool: None,
    model_shapes: None,
    host_mcp: None,
  };
  let s = AcpSession::fresh("fake", cwd, deps, None);
  s.start().await;
  let record = s.to_record();
  s.dispose();
  (record.acp_session_id.clone().unwrap(), record)
}
