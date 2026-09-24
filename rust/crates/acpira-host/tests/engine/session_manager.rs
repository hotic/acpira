//! test/SessionManager.test.ts: the session pool, viewers, the list and its persistence, against test/fake-agent.ts

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::accounts::local::LocalAccounts;
use acpira_host::acp::agent_registry::AgentRegistry;
use acpira_host::acp::session::{AcpSession, CompactionPolicy, SessionDeps};
#[allow(unused_imports)]
use std::path::PathBuf;
use acpira_host::session_manager::{ManagerDeps, SessionManager, Viewer};
use acpira_host::store::transcript_store::TranscriptStore;
use acpira_shared::agent_order::AgentPrefs;
use acpira_shared::protocol::WebviewMsg;
use acpira_shared::settings::HiddenMap;

use crate::fake_or_skip;
use crate::support::{FakeAgent, expect_match, until, v};

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

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_turn_is_persisted_before_shutdown_releases_the_session() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  let prompt = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| m.active().is_some_and(|a| last_turn(&a)["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["streaming"] == true))), 5000).await;
  m.dispose().await;
  prompt.await.unwrap();
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  let record = v(store.load(&id).await.unwrap());
  let last = last_turn(&record);
  expect_match(&last, json!({ "stop": "cancelled" }));
  assert!(last["endedAt"].is_number());
  assert!(last["blocks"].as_array().unwrap().iter().any(|b| b["streaming"] == false));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_agent_list_is_ordered_and_new_sessions_start_on_the_first_enabled_agent() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let mut opts = Opts::with_agents(merged(&[("fake", agent_setting(&fake, "fake", json!({}))), ("fake2", agent_setting(&fake, "fake2", json!({ "name": "Fake 2" })))]), "fake");
  opts.prefs = Arc::new(Mutex::new(AgentPrefs { order: vec!["fake2".into()], disabled: vec![] }));
  let m = Mgr::new(dir.path(), opts.clone());
  m.init().await;
  assert_eq!(m.m.agents()[0].id, "fake2");
  opts.prefs.lock().unwrap().disabled = vec!["fake".into()];
  m.m.emit_agents();
  let pushed = m.events.lock().unwrap().iter().rfind(|e| e["type"] == "agents").cloned().unwrap();
  let disabled: Vec<Value> = pushed["agents"].as_array().unwrap().iter().filter(|a| a["disabled"] == true).map(|a| a["id"].clone()).collect();
  assert_eq!(disabled, [json!("fake")]);
  m.new_session(None).await;
  assert_eq!(m.active().unwrap()["agent"], "fake2");
  // An explicit pick (a disabled agent's own Retry / new session) is still honoured
  m.new_session(Some("fake")).await;
  assert_eq!(m.active().unwrap()["agent"], "fake");
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn local_quota_updates_publish_and_refresh_after_a_turn_without_binding_an_account() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
  let c = calls.clone();
  let local = LocalAccounts::with_http(
    Arc::new(|_| [("KIMI_CODE_API_KEY".to_owned(), "test-code-key".to_owned())].into_iter().collect()),
    Arc::new(move |_url, _headers| {
      c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
      Box::pin(async { Ok(json!({ "usage": { "limit": 100, "used": 25 } })) })
    }),
  );
  let mut opts = Opts::with_agents(fake.setting_as("kimi", json!({ "name": "Kimi Code" })), "kimi");
  opts.local_accounts = Some(local);
  let m = Mgr::new(dir.path(), opts);
  m.init().await;
  m.handle(json!({ "type": "refreshQuota", "agent": "kimi" })).await;
  let kimi = m.m.agents().into_iter().find(|a| a.id == "kimi").unwrap();
  expect_match(&kimi, json!({ "localAccount": { "status": "ready", "quota": { "windows": [{ "remaining": 0.75 }] } } }));
  assert!(m.events.lock().unwrap().iter().any(|e| e["type"] == "agents"));
  assert!(m.m.accounts().is_empty());
  m.new_session(None).await;
  assert!(m.active().unwrap()["accountId"].is_null());
  let before = calls.load(std::sync::atomic::Ordering::SeqCst);
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  // A turn end refreshes the quota (forced, past the cache)
  until(|| calls.load(std::sync::atomic::Ordering::SeqCst) > before, 5000).await;
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_is_soft_restore_brings_it_back_and_rename_and_pin_land_in_the_index() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let a = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  m.new_session(None).await;
  let b = m.active_id().unwrap();
  assert_eq!(m.session_ids(), [b.clone(), a.clone()]);
  m.handle(json!({ "type": "renameSession", "id": a, "title": "第一条" })).await;
  m.handle(json!({ "type": "pinSession", "id": a, "pinned": true })).await;
  // The list follows each change on the next flush (16 ms), where the TS manager rebuilt it inside onChange
  until(|| m.sessions()[0]["id"] == a.as_str(), 2000).await;
  expect_match(&m.sessions()[0], json!({ "id": a, "title": "第一条", "pinned": true }));
  // delete the current session b → active switches to a, b's file moved to the trash (out of the live directory, so no other window lists it)
  m.handle(json!({ "type": "deleteSession", "id": b })).await;
  assert_eq!(m.session_ids(), [a.clone()]);
  assert_eq!(m.active_id().as_deref(), Some(a.as_str()));
  assert_eq!(m.active().unwrap()["title"], "第一条");
  assert!(!dir.path().join(format!("{b}.json")).exists());
  assert!(dir.path().join("trash").join(format!("{b}.json")).exists());
  m.handle(json!({ "type": "restoreSession", "id": b })).await;
  assert_eq!(sorted(m.session_ids()), sorted(vec![a.clone(), b.clone()]));
  assert!(dir.path().join(format!("{b}.json")).exists());
  // delete a again, then reopen the manager: only b left in the index
  m.handle(json!({ "type": "deleteSession", "id": a })).await;
  assert_eq!(m.active_id().as_deref(), Some(b.as_str()));
  m.dispose().await;
  assert!(!dir.path().join(format!("{a}.json")).exists());
  assert!(!dir.path().join("trash").join(format!("{a}.json")).exists());
  let m2 = Mgr::new(dir.path(), Opts::with_agents(Value::Null, "fake"));
  m2.init().await;
  assert_eq!(m2.session_ids(), [b]);
  m2.dispose().await;
}

fn session_ids_seen(events: &Arc<Mutex<Vec<Value>>>) -> Vec<String> {
  events.lock().unwrap().iter().filter(|e| e["type"] == "session").map(|e| e["session"]["id"].as_str().unwrap_or("").to_owned()).collect()
}

// Several webviews (sidebar + editor tabs) each hold their own active session over the shared list; only the viewers showing a session get its updates
#[tokio::test(flavor = "multi_thread")]
async fn viewers_hold_independent_active_sessions_and_only_get_their_sessions_events() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  let a = m.m.attach(None);
  let b = m.m.attach(None);
  let seen_a = record_events(&a);
  let seen_b = record_events(&b);
  // a fresh viewer opens a fresh session; a second fresh viewer gets its own, not a's
  m.m.ensure_active_for(&a).await;
  m.m.ensure_active_for(&b).await;
  let sa = a.active_id().unwrap();
  let sb = b.active_id().unwrap();
  assert_ne!(sa, sb);
  assert_eq!(sorted(m.session_ids()), sorted(vec![sa.clone(), sb.clone()]));
  // both viewers can look at the same session; b moving on to a new one leaves a where it was
  m.handle_on(&a, json!({ "type": "send", "text": "hi" })).await;
  m.m.select_session_for(&b, &sa).await;
  assert_eq!(turns_len(m.active_of(&b)), 2);
  m.m.new_session_for(&b, None, None).await.unwrap();
  assert!(m.session_ids().contains(&sa));
  assert_eq!(a.active_id().as_deref(), Some(sa.as_str()));
  assert_ne!(b.active_id().as_deref(), Some(sa.as_str()));
  // updates route by active session: a's turn reached a, but not b once b moved on
  seen_a.lock().unwrap().clear();
  seen_b.lock().unwrap().clear();
  m.handle_on(&a, json!({ "type": "send", "text": "again" })).await;
  tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  assert!(session_ids_seen(&seen_a).contains(&sa));
  assert!(!session_ids_seen(&seen_b).contains(&sa));
  // deleting a's session moves only a; b stays where it was
  let sb2 = b.active_id().unwrap();
  m.handle_on(&b, json!({ "type": "deleteSession", "id": sa })).await;
  assert_eq!(b.active_id().as_deref(), Some(sb2.as_str()));
  assert!(a.active_id().is_some_and(|x| x != sa));
  // a detached viewer no longer hears anything
  seen_b.lock().unwrap().clear();
  m.m.detach(&b);
  m.handle_on(&a, json!({ "type": "send", "text": "quiet" })).await;
  assert!(seen_b.lock().unwrap().is_empty());
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn probing_binaries_marks_the_fake_agent_available_and_an_uninstalled_one_not() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  assert!(m.m.agents().iter().find(|a| a.id == "fake").unwrap().available.is_none());
  m.init().await;
  assert_eq!(m.m.agents().iter().find(|a| a.id == "fake").unwrap().available, Some(true));
  m.dispose().await;
  let dir2 = tempfile::tempdir().unwrap();
  let m2 = Mgr::new(dir2.path(), Opts::with_agents(json!({ "ghost": { "name": "Ghost", "command": "/nonexistent/ghost-cli" } }), "ghost"));
  m2.init().await;
  assert_eq!(m2.m.agents().iter().find(|a| a.id == "ghost").unwrap().available, Some(false));
  m2.dispose().await;
}

// A CLI installed while the window is open: the registry's notification re-pushes agents to every viewer, and the poll keeps looking while
// something is missing; the install action runs the vendor line through the shell in a host terminal
#[tokio::test(flavor = "multi_thread")]
async fn a_cli_appearing_after_init_reaches_the_viewers_and_install_agent_runs_the_vendor_line() {
  let dir = tempfile::tempdir().unwrap();
  let bin = dir.path().join("ghost-cli");
  let store_dir = tempfile::tempdir().unwrap();
  // `never` stays missing so the poll keeps its timer armed regardless of which built-in CLIs this machine has
  let m = Mgr::new(store_dir.path(), Opts::with_agents(json!({
    "ghost": { "name": "Ghost", "command": bin, "install": { "command": "curl -fsSL https://example.com/i.sh | bash" } },
    "never": { "name": "Never", "command": "/nonexistent/never-cli" },
  }), "never"));
  m.init().await;
  let viewer = m.m.attach(None);
  let seen = record_events(&viewer);
  let last = || seen.lock().unwrap().iter().rfind(|e| e["type"] == "agents").map(|e| e["agents"].as_array().unwrap().iter().find(|a| a["id"] == "ghost").unwrap()["available"].clone());
  expect_match(m.m.agents().into_iter().find(|a| a.id == "ghost").unwrap(), json!({ "available": false, "install": { "command": "curl -fsSL https://example.com/i.sh | bash" } }));
  let make = || {
    std::fs::write(&bin, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::PermissionsExt::set_mode(&mut std::fs::metadata(&bin).unwrap().permissions(), 0o755);
    #[cfg(unix)]
    std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
  };
  // The settings page's rescan path: a direct lookup finds the new binary and the list is pushed at once
  make();
  assert_eq!(m.m.registry().resolve_binary("ghost").await.as_deref(), Some(bin.to_str().unwrap()));
  assert_eq!(last(), Some(json!(true)));
  // Removed again: the next poll tick (every 10 s) notices, and it keeps polling while missing, so a reinstall shows up on its own
  std::fs::remove_file(&bin).unwrap();
  until(|| last() == Some(json!(false)), 15_000).await;
  make();
  until(|| last() == Some(json!(true)), 15_000).await;
  m.handle_on(&viewer, json!({ "type": "installAgent", "agent": "ghost" })).await;
  let (shell, flag) = if cfg!(windows) { ("powershell", "-Command") } else { ("bash", "-c") };
  let terminals = m.terminals.lock().unwrap().clone();
  assert_eq!(terminals.iter().map(|(c, a, _)| (c.clone(), a.clone())).collect::<Vec<_>>(), [(shell.to_owned(), vec![flag.to_owned(), "curl -fsSL https://example.com/i.sh | bash".to_owned()])]);
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn hidden_options_are_read_from_the_host_and_re_pushed_on_emit_hidden() {
  let dir = tempfile::tempdir().unwrap();
  let mut opts = Opts::with_agents(Value::Null, "fake");
  opts.hidden = serde_json::from_value(json!({ "devin": { "model": ["GLM-5.2"] } })).unwrap();
  let m = Mgr::new(dir.path(), opts);
  assert_eq!(v(m.m.hidden()), json!({ "devin": { "model": ["GLM-5.2"] } }));
  m.m.emit_hidden();
  let events: Vec<Value> = m.events.lock().unwrap().iter().filter(|e| e["type"] == "hidden").map(|e| e["hidden"].clone()).collect();
  assert_eq!(events, [json!({ "devin": { "model": ["GLM-5.2"] } })]);
}

fn control_ids(cs: &[acpira_shared::transcript::ConfigControl]) -> Vec<String> {
  cs.iter().map(|c| c.id.clone()).collect()
}

fn model_ids(cs: &[acpira_shared::transcript::ConfigControl]) -> Vec<String> {
  cs.iter().find(|c| c.id == "model").map(|c| c.options.iter().map(|o| o.id.clone()).collect()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn known_controls_come_from_the_agents_latest_session_even_after_the_process_is_gone() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  assert!(m.m.known_controls("fake").await.is_empty());
  m.new_session(None).await;
  assert_eq!(control_ids(&m.m.known_controls("fake").await), ["model", "effort"]);
  m.dispose().await;
  let m2 = Mgr::new(dir.path(), Opts::with_agents(Value::Null, "fake"));
  m2.init().await;
  assert_eq!(control_ids(&m2.m.known_controls("fake").await), ["model", "effort"]);
  assert!(m2.m.known_controls("ghost").await.is_empty());
  m2.dispose().await;
}

// The refresh button: a throwaway spawn runs initialize + session/new so a CLI config change (FAKE_MODELS here) shows up without a
// real session; knownControls prefers the probed list until the next real session reads its own
#[tokio::test(flavor = "multi_thread")]
async fn probe_controls_reads_the_current_config_options_and_is_preferred_until_the_next_session() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  assert_eq!(model_ids(&m.m.known_controls("fake").await), ["m1", "m2"]);
  // The CLI's configuration changes (the TS suite set FAKE_MODELS in the environment every spawn inherits)
  m.m.set_registry(Arc::new(AgentRegistry::new(&fake.setting(json!({ "env": { "FAKE_MODELS": "m3" } })))));
  let probed = m.m.probe_controls("fake").await;
  assert_eq!(model_ids(&probed), ["m1", "m2", "m3"]);
  assert_eq!(model_ids(&m.m.known_controls("fake").await), ["m1", "m2", "m3"]);
  expect_match(m.m.runtime_info("fake").unwrap(), json!({ "name": "fake" }));
  // Fill the current session so newSession cannot reuse it (keepEmpty), then a real session reads m3 itself and retires the probe
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  m.new_session(None).await;
  let options = m.active().unwrap()["controls"]["options"].clone();
  let models: Vec<Value> = options.as_array().unwrap().iter().find(|c| c["id"] == "model").unwrap()["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect();
  assert_eq!(models, [json!("m1"), json!("m2"), json!("m3")]);
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn probe_controls_answers_empty_for_an_agent_without_a_binary() {
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::with_agents(json!({ "ghost": { "name": "Ghost", "command": "/nonexistent/ghost-cli" } }), "ghost"));
  m.init().await;
  assert!(m.m.probe_controls("ghost").await.is_empty());
  m.dispose().await;
}

// The agent page's status line: each launch stage records its own outcome — probe (spawn / handshake / session/new auth)
// and real sessions (ready / auth_required). Newest wins, a failed probe keeps the last known controls
#[tokio::test(flavor = "multi_thread")]
async fn health_is_ready_after_a_probe_and_a_real_session_overwrites_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.m.probe_controls("fake").await;
  expect_match(m.m.agent_health("fake").unwrap(), json!({ "stage": "ready", "source": "probe" }));
  m.new_session(None).await;
  until(|| m.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  until(|| m.m.agent_health("fake").is_some_and(|h| v(&h)["source"] == "session"), 2000).await;
  expect_match(m.m.agent_health("fake").unwrap(), json!({ "stage": "ready", "source": "session" }));
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn health_distinguishes_auth_required_from_a_failed_handshake() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let auth_dir = dir.path().join("needs-auth");
  std::fs::create_dir_all(&auth_dir).unwrap();
  let opts = Opts::fake(&fake).cwd(auth_dir.to_str().unwrap());
  let m = Mgr::new(&dir.path().join("sessions"), opts.clone());
  m.init().await;
  m.m.probe_controls("fake").await;
  expect_match(m.m.agent_health("fake").unwrap(), json!({ "stage": "auth_required", "source": "probe" }));
  m.m.set_registry(Arc::new(AgentRegistry::new(&fake.setting(json!({ "env": { "FAKE_INIT_FAIL": "1" } })))));
  *opts.cwd.lock().unwrap() = dir.path().to_string_lossy().into_owned();
  m.m.probe_controls("fake").await;
  expect_match(m.m.agent_health("fake").unwrap(), json!({ "stage": "handshake_failed", "source": "probe" }));
  m.dispose().await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_binary_that_cannot_exec_is_spawn_failed_and_a_failed_probe_keeps_the_last_controls() {
  use acpira_host::acp::probe_controls::probe_agent_controls;
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  // A path that resolves (execute bit, regular file) but cannot exec — no interpreter behind the shebang
  let bad = dir.path().join("bad-cli");
  std::fs::write(&bad, "#!/nonexistent/definitely-missing-interp\n").unwrap();
  std::fs::set_permissions(&bad, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
  let agents = merged(&[("fake", agent_setting(&fake, "fake", json!({}))), ("broken", json!({ "name": "Broken", "command": bad }))]);
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(agents, "fake").cwd(dir.path().to_str().unwrap()));
  m.init().await;
  let before = m.m.probe_controls("fake").await;
  assert!(!before.is_empty());
  // Directly against the probe and through the manager both land on spawn_failed
  let def = m.m.registry().get("broken").unwrap().clone();
  let Err(failure) = probe_agent_controls(&def, bad.to_str().unwrap(), dir.path().to_str().unwrap(), None, Arc::new(|_: &str| {}), None).await else {
    panic!("a binary without its interpreter cannot exec");
  };
  assert_eq!(v(failure.stage), json!("spawn_failed"));
  m.m.probe_controls("broken").await;
  expect_match(m.m.agent_health("broken").unwrap(), json!({ "stage": "spawn_failed", "source": "probe" }));
  // A failed probe falls back to the last known controls instead of emptying the list
  let agents = merged(&[("fake", agent_setting(&fake, "fake", json!({ "env": { "FAKE_INIT_FAIL": "1" } }))), ("broken", json!({ "name": "Broken", "command": bad }))]);
  m.m.set_registry(Arc::new(AgentRegistry::new(&agents)));
  m.m.probe_controls("fake").await;
  expect_match(m.m.agent_health("fake").unwrap(), json!({ "stage": "handshake_failed", "source": "probe" }));
  assert_eq!(v(m.m.known_controls("fake").await), v(before));
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_auth_method_runs_the_agent_binary_in_a_terminal_and_never_reaches_authenticate() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let auth_log = dir.path().join("auth.log");
  let agents = fake.setting(json!({ "env": { "FAKE_TERMINAL_AUTH": auth_log, "FAKE_FLAG": "agent", "FAKE_OTHER": "agent" } }));
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(agents, "fake").cwd(dir.path().to_str().unwrap()));
  m.init().await;
  m.new_session(None).await;
  until(|| m.active().is_some_and(|a| a["status"] == "auth_required"), 5000).await;
  // The terminal method only exists because the host advertised auth.terminal at initialize
  let methods = m.active().unwrap()["authMethods"].clone();
  let term = methods.as_array().unwrap().iter().find(|x| x["id"] == "term-login").cloned().expect("terminal method");
  expect_match(&term, json!({ "terminal": { "args": ["--login"], "env": { "FAKE_LOGIN": "1", "FAKE_FLAG": "method" } } }));
  m.handle(json!({ "type": "login", "methodId": "term-login" })).await;
  // The resolved binary + the agent's own args + the method's args; env is the agent's launch env with the method's on top
  let node = m.m.registry().resolve_binary("fake").await.unwrap();
  let terminals = m.terminals.lock().unwrap().clone();
  assert_eq!(terminals.len(), 1);
  let (command, args, env) = &terminals[0];
  assert_eq!(command, &node);
  assert_eq!(args, &vec!["--import".to_owned(), fake.loader.to_string_lossy().into_owned(), fake.script.to_string_lossy().into_owned(), "--login".to_owned()]);
  let env: BTreeMap<String, Option<String>> = env.clone().unwrap();
  let want: BTreeMap<String, Option<String>> = [("FAKE_TERMINAL_AUTH", auth_log.to_string_lossy().as_ref()), ("FAKE_OTHER", "agent"), ("FAKE_FLAG", "method"), ("FAKE_LOGIN", "1")]
    .into_iter().map(|(k, x)| (k.to_owned(), Some(x.to_owned()))).collect();
  assert_eq!(env, want);
  // authenticate never went to the agent for the terminal method
  tokio::time::sleep(std::time::Duration::from_millis(150)).await;
  assert!(!auth_log.exists());
  expect_match(m.m.agent_health("fake").unwrap(), json!({ "stage": "auth_required", "source": "session" }));
  m.dispose().await;
}

fn option_values(view: &Value) -> Vec<Value> {
  view["controls"]["options"].as_array().unwrap().iter().map(|c| c["value"].clone()).collect()
}

// The fake agent's process starts every session on model m1 / effort high / mode agent; the option values and the mode picked last
// come back on the next new session. Only the user's own picks count: a mode the agent switches by itself is not a choice
#[tokio::test(flavor = "multi_thread")]
async fn the_last_chosen_config_values_and_picked_mode_are_remembered_per_agent_and_replayed() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  assert_eq!(option_values(&m.active().unwrap()), [json!("m1"), json!("high")]);
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "low" })).await;
  assert_eq!(v(m.m.last_settings("fake")), json!({ "config": { "model": "m2", "effort": "low" } }));
  m.handle(json!({ "type": "setMode", "id": "plan" })).await;
  assert_eq!(v(m.m.last_settings("fake")), json!({ "modeId": "plan", "config": { "model": "m2", "effort": "low" } }));
  // a mode the agent switches by itself is that session's business — the memory keeps what the user picked, and a later config
  // pick must not overwrite it either
  m.handle(json!({ "type": "send", "text": "mode:agent" })).await;
  assert_eq!(m.active().unwrap()["controls"]["modeId"], "agent");
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "high" })).await;
  assert_eq!(v(m.m.last_settings("fake")), json!({ "modeId": "plan", "config": { "model": "m2", "effort": "high" } }));
  m.new_session(None).await;
  assert_eq!(option_values(&m.active().unwrap()), [json!("m2"), json!("high")]);
  assert_eq!(m.active().unwrap()["controls"]["modeId"], "plan");
  // the applied choices are what the new session's first turn records
  m.handle(json!({ "type": "send", "text": "inspect-history" })).await;
  let markdown = last_turn(&m.active().unwrap())["blocks"].to_string();
  assert!(markdown.contains("\\\"model\\\":\\\"m2\\\"") && markdown.contains("\\\"mode\\\":\\\"plan\\\""), "{markdown}");
  m.dispose().await;

  // reload: the memory is on disk; a stale value (no longer in the agent's list) is passed over while the others still apply
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  let mut prefs = store.load_prefs().await;
  assert_eq!(v(&prefs.last_settings["fake"]), json!({ "modeId": "plan", "config": { "model": "m2", "effort": "high" } }));
  prefs.last_settings.insert("fake".into(), serde_json::from_value(json!({ "modeId": "plan", "config": { "model": "gone", "effort": "low" } })).unwrap());
  store.save_prefs(&prefs, &["fake".into()]).await.unwrap();
  let m2 = Mgr::new(dir.path(), Opts::fake(&fake));
  m2.init().await;
  m2.new_session(None).await;
  assert_eq!(option_values(&m2.active().unwrap()), [json!("m1"), json!("low")]);
  assert_eq!(m2.active().unwrap()["controls"]["modeId"], "plan");
  // a remembered mode the agent no longer offers is skipped like any other stale value
  m2.dispose().await;
  prefs.last_settings.insert("fake".into(), serde_json::from_value(json!({ "modeId": "gone", "config": {} })).unwrap());
  store.save_prefs(&prefs, &["fake".into()]).await.unwrap();
  let m3 = Mgr::new(dir.path(), Opts::fake(&fake));
  m3.init().await;
  m3.new_session(None).await;
  assert_eq!(m3.active().unwrap()["controls"]["modeId"], "agent");
  m3.dispose().await;
}

fn shape_ids(shape: &Value) -> Vec<String> {
  shape.as_array().map(|a| a.iter().map(|c| format!("{}:{}", c["id"].as_str().unwrap(), c["options"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect::<Vec<_>>().join("|"))).collect()).unwrap_or_default()
}

async fn shape_keys(dir: &Path) -> Vec<String> {
  let store = TranscriptStore::new(dir.to_path_buf(), Arc::new(|_: &str| {}), None);
  let mut keys: Vec<String> = store.load_prefs().await.model_shapes.and_then(|s| s.get("fake").map(|x| v(x).as_object().unwrap().keys().cloned().collect())).unwrap_or_default();
  keys.sort();
  keys
}

// The history editor switches models locally; what each model really offers (Devin: SWE-2 drops `speed`) is learned from live
// switches, carried on the view and kept in prefs.json for the next host
#[tokio::test(flavor = "multi_thread")]
async fn each_models_parameters_are_learned_from_live_switches_and_carried_on_the_view() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let opts = || Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SPEED": "1", "FAKE_MODELS": "x3" } })), "fake");
  let m = Mgr::new(dir.path(), opts());
  m.init().await;
  m.new_session(None).await;
  // The shape is learned from the ready session on its next flush
  until(|| m.active().is_some_and(|a| !a["modelShapes"]["m1"].is_null()), 2000).await;
  assert_eq!(shape_ids(&m.active().unwrap()["modelShapes"]["m1"]), ["effort:low|high", "speed:standard|fast"]);
  let before = m.active().unwrap()["modelShapes"].clone();
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "low" })).await;
  // A value change is not a new shape
  assert_eq!(m.active().unwrap()["modelShapes"], before);
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  until(|| m.active().is_some_and(|a| !a["modelShapes"]["m2"].is_null()), 2000).await;
  assert_eq!(shape_ids(&m.active().unwrap()["modelShapes"]["m2"]), ["effort:high"]);
  // prefs writes are fire-and-forget
  let t0 = std::time::Instant::now();
  while shape_keys(dir.path()).await != ["m1", "m2"] {
    assert!(t0.elapsed() < std::time::Duration::from_secs(5), "shapes never reached prefs.json");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
  }
  m.dispose().await;
  let next = Mgr::new(dir.path(), opts());
  let other = Mgr::new(dir.path(), opts());
  next.init().await;
  next.new_session(None).await;
  until(|| next.active().is_some_and(|a| a["modelShapes"].as_object().is_some_and(|o| o.len() >= 2)), 2000).await;
  let mut keys: Vec<String> = next.active().unwrap()["modelShapes"].as_object().unwrap().keys().cloned().collect();
  keys.sort();
  assert_eq!(keys, ["m1", "m2"]);
  // Another window that was already running picks up a shape learned here on focus / webview ready, with a newer rev
  other.init().await;
  other.new_session(None).await;
  let rev = other.active().unwrap()["rev"].as_i64().unwrap();
  next.handle(json!({ "type": "setConfig", "configId": "model", "value": "x3" })).await;
  let t0 = std::time::Instant::now();
  while !shape_keys(dir.path()).await.contains(&"x3".to_owned()) {
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
  }
  other.m.refresh_index().await;
  until(|| other.active().is_some_and(|a| !a["modelShapes"]["x3"].is_null()), 2000).await;
  assert_eq!(shape_ids(&other.active().unwrap()["modelShapes"]["x3"]), ["effort:high"]);
  assert!(other.active().unwrap()["rev"].as_i64().unwrap() > rev);
  next.dispose().await;
  other.dispose().await;
}

// Two viewers landing on the same stored session at once used to build one AcpSession each: two processes, the second
// shadowing the first in the live map. The shared load hands both the same session
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_selects_of_the_same_stored_session_share_one_load() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "hi" })).await;
  let id = a.active_id().unwrap();
  a.dispose().await;
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  b.init().await;
  let v1 = b.m.attach(None);
  let v2 = b.m.attach(None);
  b.logs.lock().unwrap().clear();
  tokio::join!(b.m.select_session_for(&v1, &id), b.m.select_session_for(&v2, &id));
  assert_eq!(b.logs().iter().filter(|l| l.contains("spawn") || l.contains("reuse warm")).count(), 1, "{:#?}", b.logs());
  assert_eq!(v1.active_id().as_deref(), Some(id.as_str()));
  assert_eq!(v2.active_id().as_deref(), Some(id.as_str()));
  assert_eq!(turns_len(b.view_of(&id)), 2);
  b.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn new_session_on_an_empty_session_keeps_the_process() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let a = m.active_id().unwrap();
  m.new_session(None).await;
  assert_eq!(m.active_id().as_deref(), Some(a.as_str()));
  assert_eq!(m.session_ids(), [a]);
  m.dispose().await;
}

// Two extension hosts (two windows, or VS Code + Cursor) share ~/.acpira/sessions. Each used to rewrite index.json from its own memory,
// so whichever streamed last erased the other's new sessions from the list while their records stayed on disk
#[tokio::test(flavor = "multi_thread")]
async fn two_managers_over_one_directory_see_each_others_sessions_and_honor_deletion() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "from A" })).await;
  let sa = a.active_id().unwrap();
  b.new_session(None).await;
  b.handle(json!({ "type": "send", "text": "from B" })).await;
  let sb = b.active_id().unwrap();
  // Each keeps streaming (index writes on both sides) — nothing is lost; a refresh (window focus) is when the other's work shows up
  a.handle(json!({ "type": "send", "text": "A again" })).await;
  b.handle(json!({ "type": "send", "text": "B again" })).await;
  b.m.refresh_index().await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  assert_eq!(sorted(a.session_ids()), sorted(vec![sa.clone(), sb.clone()]));
  assert_eq!(sorted(b.session_ids()), sorted(vec![sa.clone(), sb.clone()]));
  // A renames its own session: B sees the new title after its refresh, not its stale copy
  a.handle(json!({ "type": "renameSession", "id": sa, "title": "A 的会话" })).await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  assert_eq!(b.sessions().iter().find(|s| s["id"] == sa.as_str()).unwrap()["title"], "A 的会话");
  // A's window closes; B deletes A's session (live nowhere now): a host starting meanwhile does not list it, B's undo brings it back for everyone
  a.dispose().await;
  let c = Mgr::new(dir.path(), Opts::fake(&fake));
  b.handle(json!({ "type": "deleteSession", "id": sa })).await;
  c.init().await;
  assert_eq!(c.session_ids(), [sb.clone()]);
  b.handle(json!({ "type": "restoreSession", "id": sa })).await;
  c.m.refresh_index().await;
  assert_eq!(sorted(c.session_ids()), sorted(vec![sa.clone(), sb]));
  assert_eq!(c.sessions().iter().find(|s| s["id"] == sa.as_str()).unwrap()["title"], "A 的会话");
  b.dispose().await;
  c.dispose().await;
}

// The same session open in two windows: a deletion in one used to be undone by the other's next debounced save, which recreated the record
#[tokio::test(flavor = "multi_thread")]
async fn a_session_deleted_in_one_manager_closes_in_the_other_and_its_stale_save_does_not_revive_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "shared" })).await;
  let id = a.active_id().unwrap();
  // A's reconcile lands its debounced record; B picks the session up from the disk and opens it too
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  b.m.select_session_for(&b.v, &id).await;
  assert_eq!(b.active().unwrap()["id"], id.as_str());
  // B changes the record (a save is now debounced) right before A deletes it
  b.handle(json!({ "type": "renameSession", "id": id, "title": "renamed in B" })).await;
  a.handle(json!({ "type": "deleteSession", "id": id })).await;
  assert!(!dir.path().join(format!("{id}.json")).exists());
  // B's next reconcile: the pending save is dropped, the session closed, the viewer moved on
  b.m.refresh_index().await;
  assert!(!dir.path().join(format!("{id}.json")).exists());
  assert!(dir.path().join("trash").join(format!("{id}.json")).exists());
  assert!(!b.session_ids().contains(&id));
  assert!(b.active_id().is_some_and(|x| x != id));
  assert!(b.toasts().iter().any(|t| t.contains("another window") || t.contains("另一个窗口")), "{:?}", b.toasts());
  // Undo in A: the record is back in both lists as a stored session; B does not reattach to it by itself
  a.handle(json!({ "type": "restoreSession", "id": id })).await;
  b.m.refresh_index().await;
  assert!(dir.path().join(format!("{id}.json")).exists());
  assert!(a.session_ids().contains(&id));
  assert!(b.session_ids().contains(&id));
  assert!(b.view_of(&id).is_none());
  a.dispose().await;
  b.dispose().await;
}

// Sessions belong to the workspace folder they were opened in (their cwd). Under the workspace scope a viewer left without a session
// falls onto one of this folder's, never another project's; moving re-homes a session into the current folder
#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_scope_keeps_most_recent_and_deletion_picks_inside_the_folder_and_move_re_homes() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let proj = |name: &str| {
    let p = dir.path().join("proj").join(name);
    std::fs::create_dir_all(&p).unwrap();
    p.to_string_lossy().into_owned()
  };
  let opts = Opts::fake(&fake).cwd(&proj("a"));
  *opts.scope.lock().unwrap() = "workspace".into();
  let sessions_dir = dir.path().join("sessions");
  // Project A: one session with a turn
  let m = Mgr::new(&sessions_dir, opts.clone());
  m.init().await;
  m.new_session(None).await;
  let a1 = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "in a" })).await;
  until(|| m.sessions().first().is_some_and(|s| s["id"] == a1.as_str()), 2000).await;
  expect_match(&m.sessions()[0], json!({ "id": a1, "cwd": proj("a") }));
  m.dispose().await;

  // Project B: a sidebar starting on "most recent" must not land on A's session
  *opts.cwd.lock().unwrap() = proj("b");
  let m = Mgr::new(&sessions_dir, opts.clone());
  m.init().await;
  let most_recent = || Some(acpira_shared::sidecar::InitialView::MostRecent { most_recent: true });
  assert!(m.m.attach(most_recent()).active_id().is_none());
  *opts.scope.lock().unwrap() = "all".into();
  assert_eq!(m.m.attach(most_recent()).active_id().as_deref(), Some(a1.as_str()));
  *opts.scope.lock().unwrap() = "workspace".into();
  m.new_session(None).await;
  let b1 = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "in b" })).await;
  m.new_session(None).await;
  let b2 = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "in b too" })).await;
  // Deleting the active one falls back to B's other session, not the newer-looking A one
  m.handle(json!({ "type": "deleteSession", "id": b2 })).await;
  assert_eq!(m.active_id().as_deref(), Some(b1.as_str()));

  // Move A's stored session into B: its record and summary change folder
  m.handle(json!({ "type": "moveSession", "id": a1 })).await;
  assert_eq!(m.sessions().iter().find(|s| s["id"] == a1.as_str()).unwrap()["cwd"], proj("b"));
  let store = TranscriptStore::new(sessions_dir.clone(), Arc::new(|_: &str| {}), None);
  assert_eq!(store.load(&a1).await.unwrap().cwd, proj("b"));

  // Move a live idle session: it is reopened in the new folder (a fresh process; "gone" makes the fake report the old id swept —
  // the transcript already ran, so it stays read-only with its history); a running one refuses
  *opts.cwd.lock().unwrap() = proj("c-gone");
  m.handle(json!({ "type": "moveSession", "id": b1 })).await;
  assert_eq!(m.active_id().as_deref(), Some(b1.as_str()));
  let active = m.active().unwrap();
  assert_eq!(active["cwd"], proj("c-gone"));
  assert_eq!(active["status"], "readonly");
  assert_eq!(active["turns"].as_array().unwrap().len(), 2);
  // A session mid-turn refuses the move; use a fresh one, since the moved b1 is read-only now
  *opts.cwd.lock().unwrap() = proj("d");
  m.new_session(None).await;
  let d1 = m.active_id().unwrap();
  let sending = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| m.active().is_some_and(|a| a["running"] == true), 5000).await;
  *opts.cwd.lock().unwrap() = proj("e");
  m.handle(json!({ "type": "moveSession", "id": d1 })).await;
  assert!(m.toasts().iter().any(|t| t.contains("moving") || t.contains("移动")), "{:?}", m.toasts());
  m.handle(json!({ "type": "stop" })).await;
  sending.await.unwrap();
  assert_eq!(m.active().unwrap()["cwd"], proj("d"));
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_session_takes_the_warm_process_started_at_init() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  assert!(m.logs().iter().any(|l| l.contains("reuse warm")), "{:#?}", m.logs());
  assert_eq!(m.active().unwrap()["status"], "ready");
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_session_ignores_path_like_ids() {
  let fake = fake_or_skip!();
  let parent = tempfile::tempdir().unwrap();
  let dir = parent.path().join("sessions");
  let m = Mgr::new(&dir, Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  let marker = parent.path().join("keep");
  std::fs::write(&marker, "x").unwrap();
  m.handle(json!({ "type": "deleteSession", "id": ".." })).await;
  m.handle(json!({ "type": "deleteSession", "id": "/etc/passwd" })).await;
  assert_eq!(m.active_id().as_deref(), Some(id.as_str()));
  assert!(marker.exists());
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_answers_address_the_named_session_not_the_viewers_current_one() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let perm_of = |id: &str| m.view_of(id).and_then(|x| crate::acp_session::agent_blocks(&x).into_iter().find(|b| b["type"] == "permission"));
  // handle(send) waits for the whole turn, including the permission gate — do not await it
  let send_a = m.spawn_handle(json!({ "type": "send", "text": "use tool" }));
  until(|| perm_of(&m.active_id().unwrap()).is_some(), 8000).await;
  let a = m.active_id().unwrap();
  let perm_a = perm_of(&a).unwrap();
  m.new_session(None).await;
  let send_b = m.spawn_handle(json!({ "type": "send", "text": "use tool" }));
  until(|| m.active_id().is_some_and(|x| x != a) && perm_of(&m.active_id().unwrap()).is_some(), 8000).await;
  let b = m.active_id().unwrap();
  m.handle(json!({ "type": "permission", "sessionId": a, "blockId": perm_a["id"], "optionId": "allow" })).await;
  until(|| perm_of(&a).is_none() && m.view_of(&a).is_some_and(|x| crate::acp_session::agent_blocks(&x).iter().any(|b| b["type"] == "tool_call" && b["status"] == "completed")), 8000).await;
  assert!(perm_of(&b).is_some());
  assert_eq!(m.active_id().as_deref(), Some(b.as_str()));
  send_a.await.unwrap();
  send_b.abort();
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forking_copies_the_prefix_records_its_origin_re_homes_blobs_and_hands_over_context() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let src = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi", "attachments": [{ "kind": "image", "mimeType": "image/png", "data": "aGVsbG8=", "name": "a.png" }] })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  assert_eq!(turns_len(m.active()), 4);
  m.handle(json!({ "type": "forkSession", "sessionId": src, "turnIndex": 1 })).await;
  let fork = m.active_id().unwrap();
  assert_ne!(fork, src);
  assert_eq!(turns_len(m.active()), 2);
  assert!(m.active().unwrap()["title"].as_str().unwrap().starts_with("Fork: "));
  assert_eq!(turns_len(m.view_of(&src)), 4);
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  let record = v(store.load(&fork).await.unwrap());
  expect_match(&record, json!({ "historyPending": true, "forkedFrom": { "sessionId": src, "turnIndex": 1 } }));
  let blob = record["turns"][0]["attachments"].as_array().unwrap().iter().find_map(|a| a["blob"].as_str().map(str::to_owned)).expect("copied blob");
  assert!(dir.path().join(&fork).join(&blob).exists());
  m.handle(json!({ "type": "send", "text": "again" })).await;
  assert!(last_turn(&m.active().unwrap())["blocks"].to_string().contains("resource:acpira://history/"));
  m.m.refresh_index().await;
  let t0 = std::time::Instant::now();
  while store.load(&fork).await.is_some_and(|r| r.history_pending) {
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forking_a_non_agent_turn_or_the_running_last_turn_is_refused() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let src = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  m.handle(json!({ "type": "forkSession", "sessionId": src, "turnIndex": 0 })).await;
  assert!(m.toasts().contains(&"That reply is no longer there to fork from.".to_owned()), "{:?}", m.toasts());
  assert_eq!(m.active_id().as_deref(), Some(src.as_str()));
  let sending = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| m.active().is_some_and(|a| a["running"] == true), 5000).await;
  let last = turns_len(m.active()) - 1;
  m.handle(json!({ "type": "forkSession", "sessionId": src, "turnIndex": last })).await;
  assert!(m.toasts().contains(&"Wait for this reply to finish before forking from it.".to_owned()), "{:?}", m.toasts());
  assert_eq!(m.active_id().as_deref(), Some(src.as_str()));
  m.handle(json!({ "type": "stop" })).await;
  sending.await.unwrap();
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_exports_as_markdown_and_json_under_the_sibling_exports_dir() {
  use acpira_shared::protocol::ExportFormat;
  let fake = fake_or_skip!();
  let parent = tempfile::tempdir().unwrap();
  let dir = parent.path().join("sessions");
  let m = Mgr::new(&dir, Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let title = m.active().unwrap()["title"].as_str().unwrap().to_owned();
  let md_path = m.m.export_session(&id, ExportFormat::Markdown).await.unwrap();
  // writeExport returns the realpath'd target, so resolve the expectation the same way
  assert!(md_path.starts_with(parent.path().join("exports").canonicalize().unwrap()), "{}", md_path.display());
  let md = std::fs::read_to_string(&md_path).unwrap();
  assert!(md.contains(&format!("# {title}")) && md.contains("hello world"));
  let json_path = m.m.export_session(&id, ExportFormat::Json).await.unwrap();
  assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&json_path).unwrap()).unwrap()["id"], id.as_str());
  assert!(m.m.export_session("missing-id", ExportFormat::Json).await.is_err());
  m.dispose().await;
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
  };
  let s = AcpSession::fresh("fake", cwd, deps, None);
  s.start().await;
  let record = s.to_record();
  s.dispose();
  (record.acp_session_id.clone().unwrap(), record)
}

// "Import from <agent>": session/list on a throwaway process marks the ids this window already holds; importing a
// foreign one opens a record whose transcript the session/load replay fills
#[tokio::test(flavor = "multi_thread")]
async fn native_sessions_list_marks_imported_ones_import_replays_and_re_import_selects_the_record() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })), "fake"));
  m.init().await;
  // (a) a session this manager runs is listed with localId pointing back at its record
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let own_id = m.active_id().unwrap();
  let own = m.m.list_native_sessions("fake").await.unwrap().into_iter().find(|s| s.local_id.as_deref() == Some(own_id.as_str())).expect("own session listed");
  // The fake stores the canonical cwd (codex-acp canonicalizes thread cwd the same way)
  assert_eq!(own.cwd, std::fs::canonicalize("/tmp").unwrap().to_string_lossy());
  assert!(own.title.as_deref().is_some_and(|t| t.starts_with("Fake ")));
  assert!(own.updated_at.is_some());
  // (b) a native session another process owns is listed bare; importing it replays the native history
  let other = {
    let deps = SessionDeps {
      registry: Arc::new(AgentRegistry::new(&fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })))),
      log: Arc::new(|_: &str| {}),
      on_change: Arc::new(|_, _| {}),
      blobs: m.store.clone(),
      notify: None,
      accounts: None,
      compaction: None,
      pool: None,
      model_shapes: None,
    };
    let s = AcpSession::fresh("fake", "/tmp", deps, None);
    s.start().await;
    s.prompt("hi".into(), vec![], false, None, None).await;
    let id = s.to_record().acp_session_id.unwrap();
    s.dispose();
    id
  };
  let target = m.m.list_native_sessions("fake").await.unwrap().into_iter().find(|s| s.session_id == other).expect("foreign listed");
  assert!(target.local_id.is_none());
  let count = m.sessions().len();
  let viewer = m.m.attach(None);
  m.m.import_native_session(&viewer, "fake", &target.session_id, &target.cwd, target.title.as_deref(), target.updated_at.as_deref()).await;
  until(|| m.sessions().len() == count + 1, 2000).await;
  let imported = viewer.active_id().unwrap();
  let view = m.view_of(&imported).unwrap();
  assert_eq!(view["status"], "ready");
  assert!(crate::acp_session::agent_blocks(&view).iter().any(|b| b["markdown"].as_str().is_some_and(|t| t.contains("NATIVE_REPLAY"))));
  assert_eq!(m.sessions().iter().find(|s| s["id"] == imported.as_str()).unwrap()["acpSessionId"], other.as_str());
  let t0 = std::time::Instant::now();
  let record = loop {
    if let Some(r) = m.store.load(&imported).await.filter(|r| !r.import_pending) {
      break r;
    }
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  };
  assert_eq!(record.acp_session_id.as_deref(), Some(other.as_str()));
  assert_eq!(v(&record.imported_from), json!({ "sessionId": other }));
  // (c) importing the same native id again lands the viewer on the existing record instead of making a second one
  m.m.import_native_session(&viewer, "fake", &target.session_id, &target.cwd, target.title.as_deref(), target.updated_at.as_deref()).await;
  assert_eq!(m.sessions().len(), count + 1);
  assert_eq!(viewer.active_id().as_deref(), Some(imported.as_str()));
  m.dispose().await;
}

// codex-acp stores the canonicalized thread cwd and filters each page by string compare, so a project reached
// through a symlink produced only empty pages with cursors; the host pages through and retries with the realpath
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn native_sessions_of_a_symlinked_project_still_list() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let target = dir.path().join("proj-real");
  std::fs::create_dir(&target).unwrap();
  let link = dir.path().join("proj-link");
  std::os::unix::fs::symlink(&target, &link).unwrap();
  let other_dir = dir.path().join("other");
  std::fs::create_dir(&other_dir).unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let agents = fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_LIST_PAGE": "1" } }));
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(agents, "fake").cwd(link.to_str().unwrap()));
  m.init().await;
  let mk = |cwd: PathBuf| {
    let (fake_ref, native, store) = (&fake, native.clone(), m.store.clone());
    async move {
      let deps = SessionDeps {
        registry: Arc::new(AgentRegistry::new(&fake_ref.setting(json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_LIST_PAGE": "1" } })))),
        log: Arc::new(|_: &str| {}),
        on_change: Arc::new(|_, _| {}),
        blobs: store,
        notify: None,
        accounts: None,
        compaction: None,
        pool: None,
        model_shapes: None,
      };
      let s = AcpSession::fresh("fake", cwd.to_str().unwrap(), deps, None);
      s.start().await;
      let id = s.to_record().acp_session_id.unwrap();
      s.dispose();
      id
    }
  };
  let target_id = mk(link.clone()).await;
  let foreign = [mk(other_dir.clone()).await, mk(other_dir.clone()).await];
  // Deterministic order: the foreign sessions lead (filtered-out pages), the project's trails on the last page
  let stamp = |id: &str, secs: i64| {
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs as u64);
    std::fs::File::options().write(true).open(native.join(format!("{id}.json"))).unwrap().set_modified(t).unwrap();
  };
  stamp(&target_id, 1);
  for (i, id) in foreign.iter().enumerate() {
    stamp(id, 2 + i as i64);
  }
  let listed = m.m.list_native_sessions("fake").await.unwrap();
  assert_eq!(listed.iter().map(|s| s.session_id.clone()).collect::<Vec<_>>(), [target_id]);
  assert_eq!(listed[0].cwd, std::fs::canonicalize(&link).unwrap().to_string_lossy());
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_native_sessions_of_an_agent_without_the_capability_is_unsupported() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  // no FAKE_SESSION_DIR: the fixture advertises no list capability
  let m = Mgr::new(&dir.path().join("sessions"), Opts::fake(&fake));
  m.init().await;
  let err = m.m.list_native_sessions("fake").await.expect_err("unsupported");
  assert!(err.to_string().contains("does not list its sessions"), "{err}");
  m.dispose().await;
}

// An index summary written before acpSessionId existed is patched once and the debounced index write persists it,
// so the next listing (or another window) does not re-read the record
#[tokio::test(flavor = "multi_thread")]
async fn a_stale_index_summary_gains_its_acp_session_id_and_the_index_file_is_updated() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let sessions = dir.path().join("sessions");
  let store = TranscriptStore::new(sessions.clone(), Arc::new(|_: &str| {}), None);
  let (acp_id, record) = foreign_native_session(&fake, &native, &store, "/tmp").await;
  store.flush(Arc::new(record.clone())).await.unwrap();
  // Rewrite the index the way a build without the field left it
  let mut stale = v(record.summary());
  stale.as_object_mut().unwrap().remove("acpSessionId");
  std::fs::write(sessions.join("index.json"), serde_json::to_string(&json!([stale])).unwrap()).unwrap();
  let m = Mgr::new(&sessions, Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })), "fake"));
  m.init().await;
  m.m.list_native_sessions("fake").await.unwrap();
  let index = || -> Option<String> {
    let list: Value = serde_json::from_str(&std::fs::read_to_string(sessions.join("index.json")).ok()?).ok()?;
    list.as_array()?.iter().find(|s| s["id"] == record.id.as_str())?["acpSessionId"].as_str().map(str::to_owned)
  };
  until(|| index().as_deref() == Some(acp_id.as_str()), 5000).await;
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn observe_subagent_streams_only_to_the_observing_viewer_until_unobserved() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  let a = m.m.attach(None);
  let b = m.m.attach(None);
  let subs_a = record_events(&a);
  let subs_b = record_events(&b);
  let subs = |e: &Arc<Mutex<Vec<Value>>>| e.lock().unwrap().iter().filter(|x| x["type"] == "subagent").cloned().collect::<Vec<_>>();
  m.m.ensure_active_for(&a).await;
  let sid = a.active_id().unwrap();
  m.m.select_session_for(&b, &sid).await;
  let p = {
    let (mm, a) = (m.m.clone(), a.clone());
    tokio::spawn(async move { mm.handle_for(&a, serde_json::from_value(json!({ "type": "send", "text": "subagents-native" })).unwrap()).await })
  };
  let c1 = || m.active_of(&a).and_then(|x| x["subagents"].as_array().and_then(|n| n.iter().find(|n| n["peer"]["sessionId"] == "c1").cloned()));
  until(|| c1().is_some_and(|n| n["permissions"].as_array().is_some_and(|p| !p.is_empty())), 8000).await;
  let c1id = c1().unwrap()["id"].as_str().unwrap().to_owned();
  m.handle_on(&a, json!({ "type": "observeSubagent", "sessionId": sid, "subagentId": c1id })).await;
  until(|| !subs(&subs_a).is_empty(), 2000).await;
  expect_match(&subs(&subs_a)[0], json!({ "sessionId": sid, "subagentId": c1id }));
  assert!(subs(&subs_b).is_empty());
  // answering the child's permission through the viewer's normal route bumps the stream's rev
  let perm = subs(&subs_a)[0]["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "agent").flat_map(|t| t["blocks"].as_array().unwrap().clone()).find(|x| x["type"] == "permission").unwrap();
  m.handle_on(&a, json!({ "type": "permission", "sessionId": sid, "blockId": perm["id"], "optionId": "allow" })).await;
  until(|| subs(&subs_a).len() >= 2, 5000).await;
  let seen = subs(&subs_a);
  assert!(seen.last().unwrap()["rev"].as_i64() > seen[0]["rev"].as_i64());
  m.handle_on(&a, json!({ "type": "unobserveSubagent", "sessionId": sid, "subagentId": c1id })).await;
  let count = subs(&subs_a).len();
  p.await.unwrap();
  tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  assert_eq!(subs(&subs_a).len(), count);
  assert!(subs(&subs_b).is_empty());
  // observing another session's subagent id is a no-op, not an error
  m.handle_on(&a, json!({ "type": "observeSubagent", "sessionId": sid, "subagentId": "nonexistent" })).await;
  assert_eq!(subs(&subs_a).len(), count);
  m.dispose().await;
}
