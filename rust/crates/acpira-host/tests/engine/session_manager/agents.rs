//! The agent list, availability, quota, known and probed controls, health and terminal auth

use super::*;

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
  use acpira_host::acp::agents::probe_controls::probe_agent_controls;
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
  assert_eq!(args, &[fake.args(), vec!["--login".to_owned()]].concat());
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
