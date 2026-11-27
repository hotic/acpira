//! Used-up accounts: handing the turn to the next account, Droid Core swaps, plugin accounts running out

use super::*;

/// A session on `one` (good-key, exhausted) with `two` (good-key-2) saved next to it
async fn exhausted_session(s: &Setup) -> (String, String) {
  exhausted_session_as(s, "fake").await
}

async fn exhausted_session_as(s: &Setup, agent: &str) -> (String, String) {
  s.m.init().await;
  s.m.new_session(None).await;
  s.m.handle(json!({ "type": "addAccount", "agent": agent, "via": "import" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  let one = s.accounts.list()[0].id.clone();
  let two = s.store.add(agent, draft("two@example.com", None, "good-key-2", None)).await.unwrap().id;
  s.accounts.reload().await;
  (one, two)
}

fn turns(m: &Mgr) -> Vec<Value> {
  m.active().unwrap()["turns"].as_array().unwrap().clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_exhausted_account_hands_the_turn_to_the_next_account_which_continues_it() {
  let fake = fake_or_skip!();
  // Devin's typed -32011 and the AIR quota_exhausted failure of codex-acp / claude-agent-acp take the same path
  for prompt in ["hi", "quota-air"] {
    let s = setup_env(&fake, json!({ "FAKE_EXHAUSTED_KEYS": "good-key" }));
    let (one, two) = exhausted_session(&s).await;
    s.m.handle(json!({ "type": "send", "text": prompt })).await;
    // Sent while the switch is under way: it waits for the continue instead of landing on the exhausted account
    s.m.handle(json!({ "type": "send", "text": "echo-blocks after" })).await;
    until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && turns_in(&a) == 6 && a["turns"][5]["stop"] == "end_turn"), 10_000).await;
    let t = turns(&s.m);
    assert_eq!(t[4]["text"], "echo-blocks after");
    expect_match(s.m.active().unwrap(), json!({ "accountId": two }));
    // The exhausted turn keeps its output; a notice row replaces the error card
    let exhausted = &t[1];
    assert!(exhausted["error"].is_null(), "{prompt}: {exhausted}");
    assert_eq!(exhausted["stop"], "end_turn");
    assert_eq!(exhausted["blocks"][0]["markdown"], "partial work");
    let notices: Vec<&Value> = exhausted["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "notice").collect();
    assert_eq!(notices.len(), 1, "{prompt}: the AIR row is replaced, not doubled");
    expect_match(notices[0], json!({ "severity": "warning", "category": "limit", "actions": [] }));
    let title = notices[0]["title"].as_str().unwrap();
    assert!(title.contains("one@example.com") && title.contains("two@example.com"), "{title}");
    // The continue is a hidden automatic turn in the host language, answered on the new account
    expect_match(&t[2], json!({ "role": "user", "auto": true, "autoReason": "accountSwitch", "text": acpira_host::i18n::t("host.autoContinuePrompt") }));
    assert!(t[3]["error"].is_null());
    assert!(!t[3]["blocks"].as_array().unwrap().is_empty());
    // Same native session: the continue reached the context the exhausted turn left
    s.m.handle(json!({ "type": "send", "text": "inspect-native-history" })).await;
    let reply = turns(&s.m).last().cloned().unwrap();
    let echoed: Value = serde_json::from_str(reply["blocks"][0]["markdown"].as_str().unwrap()).unwrap();
    let texts: Vec<&str> = echoed["prompts"].as_array().unwrap().iter().map(|p| p[0]["text"].as_str().unwrap()).collect();
    assert_eq!(texts[..3], [prompt, acpira_host::i18n::t("host.autoContinuePrompt").as_str(), "echo-blocks after"]);
    assert_eq!(s.accounts.default_for("fake").unwrap().id, two);
    assert_ne!(one, two);
    s.m.dispose().await;
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn with_every_account_exhausted_or_the_switch_off_the_error_stays() {
  let fake = fake_or_skip!();
  let s = setup_env(&fake, json!({ "FAKE_EXHAUSTED_KEYS": "good-key,good-key-2" }));
  let (_, two) = exhausted_session(&s).await;
  // one → two, two is exhausted as well, and one is parked: the second failure stays on screen
  s.m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && turns_in(&a) == 4 && a["turns"][3]["error"].is_object()), 10_000).await;
  expect_match(s.m.active().unwrap(), json!({ "accountId": two }));
  expect_match(&turns(&s.m)[3], json!({ "stop": "error", "error": { "code": -32011, "kind": "resource_exhausted" } }));
  s.m.dispose().await;

  let s = setup_env(&fake, json!({ "FAKE_EXHAUSTED_KEYS": "good-key" }));
  s.accounts.set_switch_policy(Arc::new(|_: &str| acpira_host::accounts::switch::SwitchStrategy::Off));
  let (one, _) = exhausted_session(&s).await;
  s.m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| s.m.active().is_some_and(|a| turns_in(&a) == 2 && a["turns"][1]["error"].is_object()), 5000).await;
  expect_match(s.m.active().unwrap(), json!({ "status": "ready", "accountId": one }));
  s.m.dispose().await;
}

fn model_of(m: &Mgr) -> Value {
  m.active().unwrap()["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == "model").unwrap()["value"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_droid_core_swap_moves_the_turn_to_the_next_account_on_the_picked_model() {
  let fake = fake_or_skip!();
  let env = |keys: &str| json!({ "FAKE_CORE_KEYS": keys, "FAKE_MODELS": "claude-fable-5.2,kimi-k3", "FAKE_CONFIG_ECHO": "1" });
  // One has used up its Standard usage, two still has it: the turn continues on two, on the model the user picked
  let s = setup_env_as(&fake, "droid", env("good-key"));
  let (one, two) = exhausted_session_as(&s, "droid").await;
  s.m.handle(json!({ "type": "setConfig", "configId": "model", "value": "claude-fable-5.2" })).await;
  until(|| model_of(&s.m) == "claude-fable-5.2", 5000).await;
  s.m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && turns_in(&a) == 4 && a["turns"][3]["stop"] == "end_turn"), 10_000).await;
  expect_match(s.m.active().unwrap(), json!({ "accountId": two }));
  let t = turns(&s.m);
  assert!(t[1]["error"].is_null(), "{}", t[1]);
  let notice = t[1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "notice").cloned().unwrap();
  expect_match(&notice, json!({ "category": "limit" }));
  expect_match(&t[2], json!({ "role": "user", "auto": true, "autoReason": "accountSwitch" }));
  assert_eq!(model_of(&s.m), "claude-fable-5.2");
  // The new account's process answers on the picked model, never the Droid Core one
  s.m.handle(json!({ "type": "send", "text": "which model" })).await;
  until(|| turns(&s.m).last().is_some_and(|t| t["stop"] == "end_turn" && t["blocks"][0]["markdown"].is_string()), 5000).await;
  assert_eq!(turns(&s.m).last().unwrap()["blocks"][0]["markdown"], "model=claude-fable-5.2");
  assert_ne!(one, two);
  s.m.dispose().await;

  // Every account used up: the error stays, still on the picked model
  let s = setup_env_as(&fake, "droid", env("good-key,good-key-2"));
  let (_, two) = exhausted_session_as(&s, "droid").await;
  s.m.handle(json!({ "type": "setConfig", "configId": "model", "value": "claude-fable-5.2" })).await;
  until(|| model_of(&s.m) == "claude-fable-5.2", 5000).await;
  s.m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && turns_in(&a) == 4 && a["turns"][3]["error"].is_object()), 10_000).await;
  expect_match(s.m.active().unwrap(), json!({ "accountId": two }));
  expect_match(&turns(&s.m)[3], json!({ "stop": "error", "error": { "kind": "resource_exhausted" } }));
  assert_eq!(model_of(&s.m), "claude-fable-5.2");
  s.m.dispose().await;
}

/// A plugin listing Droid accounts (`LISTER` under `droid`, `accounts` replacing its one entry) beside a saved account
/// of the user's, and a manager whose Droid session runs the fake with two models to pick from
#[cfg(unix)]
async fn droid_plugin_setup(fake: &crate::support::FakeAgent, accounts: Option<&str>) -> (tempfile::TempDir, Arc<AccountManager>, Arc<acpira_host::plugins::runtime::Runtimes>, Mgr) {
  let dir = tempfile::tempdir().unwrap();
  let init_log = dir.path().join("init.log");
  let store = Arc::new(store_in(dir.path()));
  store.load().await.unwrap();
  store.add("droid", draft("own@example.com", None, "good-key-2", None)).await.unwrap();
  let manager = AccountManager::new(store, vec![FakeProvider::new_for("droid")], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
  manager.reload().await;
  let plugins = dir.path().join("plugins");
  let pdir = plugins.join("lister");
  std::fs::create_dir_all(&pdir).unwrap();
  let one = "[{ id: 'r1', agent: 'fake', label: 'team', badge: 'Rented', env: { FAKE_INIT_LOG: '@LOG@' },\n      quota: { windows: [{ id: 'weekly', remaining: 0.25 }] } }]";
  assert!(LISTER.contains(one));
  let script = LISTER.replace(one, accounts.unwrap_or(one)).replace("agent: 'fake'", "agent: 'droid'").replace("@LOG@", init_log.to_str().unwrap());
  std::fs::write(pdir.join("plugin.mjs"), script).unwrap();
  let m = json!({ "schema": 1, "id": "lister", "version": "1.0.0", "description": "d",
    "contributes": { "runtime": { "command": ["node", "plugin.mjs"], "methods": ["account.remove"] } } });
  let (manifest, _) = acpira_host::plugins::manifest::parse(m.to_string().as_bytes()).unwrap();
  let rt = acpira_host::plugins::runtime::Runtimes::new(plugins.join(".data"), log());
  rt.set_accounts(manager.clone());
  rt.enable(None);
  rt.sync(&[Arc::new(acpira_host::plugins::host::Active { id: "lister".into(), hash: "h".into(), dir: pdir, manifest })]).await;
  until(|| manager.get("plugin:lister:r1").is_some(), 10_000).await;
  let cwd = dir.path().join("work");
  std::fs::create_dir(&cwd).unwrap();
  let env = json!({ "FAKE_SESSION_DIR": dir.path(), "FAKE_MODELS": "claude-fable-5.2,kimi-k3", "FAKE_CONFIG_ECHO": "1" });
  let mut opts = Opts::with_agents(fake.setting_as("droid", json!({ "env": env })), "droid").cwd(cwd.to_str().unwrap());
  opts.accounts = Some(manager.clone());
  let mgr = Mgr::new(&dir.path().join("sessions"), opts);
  mgr.init().await;
  mgr.new_session(None).await;
  until(|| mgr.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  (dir, manager, rt, mgr)
}

/// Bind the session to `id` and pick Fable on it
#[cfg(unix)]
async fn droid_on(mgr: &Mgr, id: &str) {
  mgr.handle(json!({ "type": "selectAccount", "id": id })).await;
  until(|| mgr.active().is_some_and(|a| a["status"] == "ready" && a["accountId"] == id), 10_000).await;
  mgr.handle(json!({ "type": "setConfig", "configId": "model", "value": "claude-fable-5.2" })).await;
  until(|| model_of(mgr) == "claude-fable-5.2", 5000).await;
}

#[cfg(unix)]
fn last_of(mgr: &Mgr) -> Value {
  mgr.active().unwrap()["turns"].as_array().unwrap().last().cloned().unwrap()
}

// A plugin's account: with no other account of that plugin, a Droid Core swap on it stops the turn
// with the error and keeps the picked model, even with a saved account of the user's next to it
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_droid_core_swap_on_a_plugin_account_stops_without_switching_or_changing_model() {
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let (_dir, _accounts, rt, mgr) = droid_plugin_setup(&fake, None).await;
  let id = "plugin:lister:r1";
  droid_on(&mgr, id).await;
  mgr.handle(json!({ "type": "send", "text": "droid core swap" })).await;
  until(|| mgr.active().is_some_and(|a| a["status"] == "ready" && last_of(&mgr)["error"].is_object()), 10_000).await;
  expect_match(mgr.active().unwrap(), json!({ "accountId": id }));
  expect_match(last_of(&mgr), json!({ "stop": "error", "error": { "kind": "resource_exhausted" } }));
  assert_eq!(model_of(&mgr), "claude-fable-5.2");
  // Nothing hands it on: no automatic continue follows
  tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  assert_eq!(mgr.active().unwrap()["turns"].as_array().unwrap().len(), 2);
  mgr.handle(json!({ "type": "send", "text": "which model" })).await;
  until(|| last_of(&mgr)["stop"] == "end_turn", 5000).await;
  assert_eq!(last_of(&mgr)["blocks"][0]["markdown"], "model=claude-fable-5.2");
  mgr.dispose().await;
  rt.stop_all().await;
}

// Several accounts of one plugin: the used-up one hands the turn to the plugin's next account (never the user's own),
// which continues it on the picked model
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_used_up_plugin_account_hands_the_turn_to_the_same_plugins_next_account() {
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let two = "[{ id: 'r1', agent: 'fake', label: 'team', badge: 'Rented', env: { FAKE_INIT_LOG: '@LOG@', FAKE_CORE_ALWAYS: '1' },
      quota: { windows: [{ id: 'weekly', remaining: 0.25 }] } },
    { id: 'r2', agent: 'fake', label: 'team 2', badge: 'Rented', env: {}, quota: { windows: [{ id: 'weekly', remaining: 0.9 }] } }]";
  let (_dir, accounts, rt, mgr) = droid_plugin_setup(&fake, Some(two)).await;
  until(|| accounts.get("plugin:lister:r2").is_some(), 5000).await;
  droid_on(&mgr, "plugin:lister:r1").await;
  mgr.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| mgr.active().is_some_and(|a| a["status"] == "ready" && a["turns"].as_array().unwrap().len() == 4 && a["turns"][3]["stop"] == "end_turn"), 10_000).await;
  let a = mgr.active().unwrap();
  expect_match(&a, json!({ "accountId": "plugin:lister:r2" }));
  assert!(a["turns"][1]["error"].is_null(), "{}", a["turns"][1]);
  expect_match(&a["turns"][2], json!({ "role": "user", "auto": true, "autoReason": "accountSwitch" }));
  assert_eq!(model_of(&mgr), "claude-fable-5.2");
  mgr.handle(json!({ "type": "send", "text": "which model" })).await;
  until(|| last_of(&mgr)["stop"] == "end_turn" && last_of(&mgr)["blocks"][0]["markdown"].is_string(), 5000).await;
  assert_eq!(last_of(&mgr)["blocks"][0]["markdown"], "model=claude-fable-5.2");
  mgr.dispose().await;
  rt.stop_all().await;
}
