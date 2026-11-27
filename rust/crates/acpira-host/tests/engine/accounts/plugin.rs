//! Accounts a plugin lists: starting the agent with them, removal through the plugin, sessions left without one

use super::*;

// A plugin's account sits beside the saved ones, starts the agent with the plugin's variables, and a removal goes back
// to the plugin; a session left on an account its plugin no longer lists stops instead of running on the CLI's login
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_account_is_picked_started_and_handed_back_through_its_plugin() {
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let init_log = dir.path().join("init.log");
  let store = Arc::new(store_in(dir.path()));
  store.load().await.unwrap();
  let accounts = AccountManager::new(store, vec![FakeProvider::new()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
  // The plugin, with `account.remove` among its methods
  let plugins = dir.path().join("plugins");
  let pdir = plugins.join("lister");
  std::fs::create_dir_all(&pdir).unwrap();
  std::fs::write(pdir.join("plugin.mjs"), LISTER.replace("@LOG@", init_log.to_str().unwrap())).unwrap();
  let m = json!({ "schema": 1, "id": "lister", "version": "1.0.0", "description": "d",
    "contributes": { "runtime": { "command": ["node", "plugin.mjs"], "methods": ["account.remove"] } } });
  let (manifest, _) = acpira_host::plugins::manifest::parse(m.to_string().as_bytes()).unwrap();
  let rt = acpira_host::plugins::runtime::Runtimes::new(plugins.join(".data"), log());
  rt.set_accounts(accounts.clone());
  rt.enable(None);
  rt.sync(&[Arc::new(acpira_host::plugins::host::Active { id: "lister".into(), hash: "h".into(), dir: pdir, manifest })]).await;
  let id = "plugin:lister:r1";
  until(|| accounts.get(id).is_some(), 10_000).await;
  expect_match(serde_json::to_value(accounts.get(id).unwrap()).unwrap(), json!({ "label": "team", "badge": "Rented", "plugin": "lister", "quota": { "windows": [{ "remaining": 0.25 }] } }));
  assert!(accounts.get(id).unwrap().removable.is_none());

  let cwd = dir.path().join("work");
  std::fs::create_dir(&cwd).unwrap();
  let mut opts = Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": dir.path() } })), "fake").cwd(cwd.to_str().unwrap());
  opts.accounts = Some(accounts.clone());
  let mgr = Mgr::new(&dir.path().join("sessions"), opts);
  mgr.init().await;
  mgr.new_session(None).await;
  until(|| mgr.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  assert!(!init_log.exists(), "the CLI's own login starts without the plugin's variables");
  mgr.handle(json!({ "type": "selectAccount", "id": id })).await;
  until(|| mgr.active().is_some_and(|a| a["status"] == "ready" && a["accountId"] == id) && init_log.exists(), 10_000).await;
  assert_eq!(accounts.default_for("fake").map(|a| a.id).as_deref(), Some(id));

  // Removed by the user: the plugin hears it and lists nothing, and the session on it stops with the reason
  mgr.handle(json!({ "type": "removeAccount", "id": id })).await;
  until(|| accounts.get(id).is_none(), 5000).await;
  until(|| mgr.active().is_some_and(|a| a.to_string().contains("lister")), 10_000).await;
  assert_ne!(mgr.active().unwrap()["status"], "ready");
  // Retry does not fall back to the CLI's own login either
  mgr.handle(json!({ "type": "retry" })).await;
  tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  let after = mgr.active().unwrap();
  assert_ne!(after["status"], "ready", "{after}");
  assert!(after.to_string().contains("lister"), "{after}");
  mgr.dispose().await;
  rt.stop_all().await;
}

// Reopening a session whose plugin account was removed (or that had no account bound yet) moves it to a remaining
// saved account automatically instead of stopping on sign-in
#[tokio::test(flavor = "multi_thread")]
async fn a_session_off_a_removed_plugin_account_or_unbound_opens_on_a_saved_account() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let store = Arc::new(store_in(dir.path()));
  store.load().await.unwrap();
  let own = store.add("fake", draft("own@example.com", None, "good-key", None)).await.unwrap();
  let provider = FakeProvider::new();
  let accounts = AccountManager::new(store, vec![provider.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
  let transcripts = TranscriptStore::new(dir.path().join("sessions"), log(), None);
  let cwd = dir.path().join("work");
  std::fs::create_dir(&cwd).unwrap();
  let write_record = |id: &str, account_id: Option<&str>| {
    acpira_host::store::record::SessionRecord {
      id: id.to_owned(),
      agent: "fake".into(),
      account_id: account_id.map(str::to_owned),
      acp_session_id: None,
      cwd: cwd.to_string_lossy().into_owned(),
      title: id.to_owned(),
      created_at: "2026-11-20T00:00:00Z".into(),
      updated_at: "2026-11-20T00:00:00Z".into(),
      turns: vec![],
      controls: Default::default(),
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
      client_ids: vec![],
      plugin_menus: Default::default(),
      settings_from: None,
    }
  };
  let plugin_sid = "11111111-1111-4111-8111-111111111111";
  let unbound_sid = "22222222-2222-4222-8222-222222222222";
  transcripts.flush(Arc::new(write_record(plugin_sid, Some("plugin:gateway:deleted")))).await.unwrap();
  transcripts.flush(Arc::new(write_record(unbound_sid, None))).await.unwrap();
  let mut opts = Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": dir.path() } })), "fake").cwd(cwd.to_str().unwrap());
  opts.accounts = Some(accounts);
  let mgr = Mgr::new(&dir.path().join("sessions"), opts);
  mgr.init().await;

  for sid in [plugin_sid, unbound_sid] {
    mgr.handle(json!({ "type": "selectSession", "id": sid })).await;
    until(|| mgr.active().is_some_and(|a| a["id"] == sid && a["status"] != "starting"), 5000).await;
    expect_match(mgr.active().unwrap(), json!({ "id": sid, "accountId": own.id, "status": "ready" }));
  }
  mgr.dispose().await;
  for sid in [plugin_sid, unbound_sid] {
    assert_eq!(transcripts.load(sid).await.unwrap().account_id, Some(own.id.clone()));
  }
}
