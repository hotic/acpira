//! Adding and binding accounts: imports, terminal logins, pasted keys, reloads, switching and the CLI's own login

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn import_progress_publishes_at_once_and_repeated_clicks_wait_for_it() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let (release, gate) = tokio::sync::oneshot::channel();
  *s.provider.import_gate.lock().unwrap() = Some(gate);
  let first = s.m.spawn_handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" }));
  let action_events = || s.m.events.lock().unwrap().iter().filter(|e| e["type"] == "accountActions").map(|e| e["actions"].clone()).collect::<Vec<_>>();
  until(|| !action_events().is_empty(), 2000).await;
  assert_eq!(action_events()[0], json!([{ "agent": "fake", "via": "import", "status": "pending" }]));
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  assert_eq!(s.provider.imports.load(Ordering::SeqCst), 1);
  release.send(()).unwrap();
  first.await.unwrap();
  assert_eq!(action_events().last().unwrap(), &json!([{ "agent": "fake", "via": "import", "status": "success" }]));
  s.m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_login_can_be_retried_from_a_reloaded_view_without_waiting_for_the_old_cleanup() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let (old_done, old_cleanup) = tokio::sync::oneshot::channel();
  let cancelled = Arc::new(AtomicBool::new(false));
  let saw_cancel = cancelled.clone();
  s.provider.login_flows.lock().unwrap().push(LoginFlow {
    command: "first-login".into(), args: vec![], env: Default::default(),
    collect: Box::new(move |cancel| Box::pin(async move {
      cancel.cancelled().await;
      saw_cancel.store(true, Ordering::SeqCst);
      old_cleanup.await.ok();
      None
    })),
  });
  let first = s.m.spawn_handle(json!({ "type": "addAccount", "agent": "fake", "via": "login" }));
  until(|| s.terminals.lock().unwrap().len() == 1, 2000).await;

  // Closing an external terminal sends no notification; a new page still sees the pending action.
  let viewer = s.m.m.attach(Some(acpira_shared::sidecar::InitialView::Empty { empty: true }));
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "login", "status": "pending" }]));
  let (signed_in, credential) = tokio::sync::oneshot::channel();
  s.provider.login_flows.lock().unwrap().push(LoginFlow {
    command: "replacement-login".into(), args: vec![], env: Default::default(),
    collect: Box::new(move |_| Box::pin(async move {
      credential.await.ok();
      Some(draft("retry@example.com", None, "retry-key", None))
    })),
  });
  let manager = s.m.m.clone();
  let retry = tokio::spawn(async move { manager.add_account(&viewer, "fake", acpira_shared::protocol::AddAccountVia::Login).await });
  until(|| s.terminals.lock().unwrap().len() == 2 && cancelled.load(Ordering::SeqCst), 2000).await;
  // The old cancellation must not overwrite the replacement's pending state.
  old_done.send(()).unwrap();
  first.await.unwrap();
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "login", "status": "pending" }]));
  signed_in.send(()).unwrap();
  retry.await.unwrap().unwrap();
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "login", "status": "success" }]));
  assert_eq!(s.accounts.list()[0].label, "retry@example.com");
  s.m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_import_supersedes_an_auto_login_and_a_late_credential_cannot_overwrite_it() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let local = s.provider.import_draft.lock().unwrap().take();
  let (old_done, old_cleanup) = tokio::sync::oneshot::channel();
  let cancelled = Arc::new(AtomicBool::new(false));
  let saw_cancel = cancelled.clone();
  s.provider.login_flows.lock().unwrap().push(LoginFlow {
    command: "abandoned-login".into(), args: vec![], env: Default::default(),
    collect: Box::new(move |cancel| Box::pin(async move {
      cancel.cancelled().await;
      saw_cancel.store(true, Ordering::SeqCst);
      old_cleanup.await.ok();
      // A credential read that was already in flight can still finish after cancellation.
      Some(draft("late@example.com", None, "late-key", None))
    })),
  });
  let first = s.m.spawn_handle(json!({ "type": "addAccount", "agent": "fake", "via": "auto" }));
  until(|| s.terminals.lock().unwrap().len() == 1, 2000).await;
  *s.provider.import_draft.lock().unwrap() = local;
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  until(|| cancelled.load(Ordering::SeqCst), 2000).await;
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "import", "status": "success" }]));
  old_done.send(()).unwrap();
  first.await.unwrap();
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "import", "status": "success" }]));
  assert_eq!(s.accounts.list().iter().map(|a| a.label.clone()).collect::<Vec<_>>(), ["one@example.com"]);
  s.m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
// The Rust provider's import has no error channel (a local login is there or it is not), so the TS suite's rejected-import case has
// no counterpart; the missing and the retried outcomes do
async fn a_missing_local_login_is_reported_and_another_attempt_is_allowed() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let provider = s.provider.import_draft.lock().unwrap().take();
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "import", "status": "missing" }]));
  *s.provider.import_draft.lock().unwrap() = provider;
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "import", "status": "success" }]));
  s.m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pasted_key_is_checked_before_it_is_stored_and_the_same_identity_takes_it_in_place() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  assert_eq!(s.m.m.agents().iter().find(|a| a.id == "fake").and_then(|a| a.account_keys), Some(true));
  // A refused key ends the action with the reason and stores nothing
  s.m.handle(json!({ "type": "addAccountKey", "agent": "fake", "key": "fk-bad" })).await;
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "key", "status": "error", "error": "refused" }]));
  assert!(s.accounts.list().is_empty());
  // A key the vendor takes: a key account, the key in the vault and never in accounts.json
  s.m.handle(json!({ "type": "addAccountKey", "agent": "fake", "key": "fk-good" })).await;
  assert_eq!(actions(&s.m), json!([{ "agent": "fake", "via": "key", "status": "success" }]));
  let list = s.accounts.list();
  assert_eq!(list.iter().map(|a| (a.label.as_str(), a.key_auth)).collect::<Vec<_>>(), [("key@example.com", Some(true))]);
  assert!(!std::fs::read_to_string(s.dir.path().join("accounts.json")).unwrap().contains("fk-good"));
  assert!(std::fs::read_to_string(s.dir.path().join("secrets.json")).unwrap().contains("fk-good"));
  // The same identity imported as a login takes the key in place, and what that login kept outside the vault goes
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  s.m.handle(json!({ "type": "addAccountKey", "agent": "fake", "key": "fk-1:one@example.com" })).await;
  assert_eq!(*s.provider.forgotten.lock().unwrap(), ["good-key"]);
  // A key pasted again for that identity (a rotated key) keeps its home: nothing more is dropped
  s.m.handle(json!({ "type": "addAccountKey", "agent": "fake", "key": "fk-2:one@example.com" })).await;
  assert_eq!(*s.provider.forgotten.lock().unwrap(), ["good-key"]);
  let one = s.accounts.list().into_iter().find(|a| a.label == "one@example.com").unwrap();
  assert_eq!((one.key_auth, s.accounts.list().len()), (Some(true), 2));
  assert_eq!(s.store.credential(&one.id).await.unwrap().unwrap().secret, "fk-2:one@example.com");
  s.m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reload_keeps_the_imported_account_binding_and_authenticates_the_replacement_process() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  s.m.new_session(None).await;
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && !a["accountId"].is_null()), 5000).await;
  let record = s.m.active().unwrap();
  s.m.dispose().await;
  // Empty Devin sessions can disappear when the process exits; the fake's gone cwd models that response
  let transcripts = TranscriptStore::new(s.dir.path().join("sessions"), log(), None);
  let mut saved = transcripts.load(record["id"].as_str().unwrap()).await.unwrap();
  let cwd = s.dir.path().join("needs-auth-gone");
  std::fs::create_dir(&cwd).unwrap();
  saved.cwd = cwd.to_string_lossy().into_owned();
  transcripts.flush(Arc::new(saved)).await.unwrap();
  let store = Arc::new(store_in(s.dir.path()));
  store.load().await.unwrap();
  let provider = FakeProvider::new();
  let accounts = AccountManager::new(store, vec![provider.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
  let mut opts = Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": s.dir.path() } })), "fake");
  opts.accounts = Some(accounts);
  let restored = Mgr::new(&s.dir.path().join("sessions"), opts);
  restored.init().await;
  // A window reopening lands on the most recent session, like the TS manager's own viewer did
  let viewer = restored.m.attach(Some(acpira_shared::sidecar::InitialView::MostRecent { most_recent: true }));
  restored.m.ensure_active_for(&viewer).await;
  until(|| restored.active_of(&viewer).is_some_and(|a| a["status"] == "ready"), 5000).await;
  assert_eq!(provider.authentications.load(Ordering::SeqCst), 1);
  expect_match(restored.active_of(&viewer).unwrap(), json!({ "id": record["id"], "accountId": record["accountId"], "status": "ready" }));
  restored.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_old_session_whose_account_was_removed_opens_on_a_remaining_one_instead_of_asking_to_sign_in() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  s.m.new_session(None).await;
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && !a["accountId"].is_null()), 5000).await;
  let record = s.m.active().unwrap();
  let id = record["id"].as_str().unwrap().to_owned();
  s.m.dispose().await;
  // Another window removes the session's account; a second one is left
  let store = Arc::new(store_in(s.dir.path()));
  store.load().await.unwrap();
  let two = store.add("fake", draft("two@example.com", None, "good-key", None)).await.unwrap();
  store.remove(record["accountId"].as_str().unwrap()).await.unwrap();
  let provider = FakeProvider::new();
  let accounts = AccountManager::new(store, vec![provider.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
  let mut opts = Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": s.dir.path() } })), "fake");
  opts.accounts = Some(accounts);
  let restored = Mgr::new(&s.dir.path().join("sessions"), opts);
  restored.init().await;
  let viewer = restored.m.attach(Some(acpira_shared::sidecar::InitialView::MostRecent { most_recent: true }));
  restored.m.ensure_active_for(&viewer).await;
  until(|| restored.active_of(&viewer).is_some_and(|a| a["status"] != "starting"), 5000).await;
  expect_match(restored.active_of(&viewer).unwrap(), json!({ "id": id, "accountId": two.id, "status": "ready" }));
  restored.dispose().await;
  // The new binding is the session's from now on
  let transcripts = TranscriptStore::new(s.dir.path().join("sessions"), log(), None);
  assert_eq!(transcripts.load(&id).await.unwrap().account_id, Some(two.id));
}

#[tokio::test(flavor = "multi_thread")]
async fn devins_generic_missing_credential_stderr_turns_into_login_guidance() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_AUTH_HINT": "devin" } }));
  let cwd = tempfile::Builder::new().prefix("acpira-needs-auth-").tempdir().unwrap();
  let s = Disposing(h.session(cwd.path().to_str().unwrap()));
  s.start().await;
  expect_match(view(&s), json!({ "status": "auth_required", "error": null }));
}

#[tokio::test(flavor = "multi_thread")]
async fn account_switching_preserves_native_history_through_resume_or_load() {
  let fake = fake_or_skip!();
  for load_only in [false, true] {
    let s = setup(&fake, load_only);
    let m = &s.m;
    m.init().await;
    assert_eq!(m.m.agents().into_iter().find(|a| a.id == "fake").unwrap().accounts, Some(true));
    m.new_session(None).await;
    assert_eq!(m.active().unwrap()["status"], "auth_required");
    let empty = m.active_id().unwrap();
    m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
    let one = s.accounts.list()[0].clone();
    expect_match(&one, json!({ "agent": "fake", "label": "one@example.com" }));
    // the empty session stuck on login is rebound, not replaced
    assert_eq!(m.active_id().as_deref(), Some(empty.as_str()));
    assert_eq!(m.session_ids(), [empty.clone()]);
    until(|| m.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
    expect_match(m.active().unwrap(), json!({ "status": "ready", "accountId": one.id }));
    m.handle(json!({ "type": "send", "text": "hi" })).await;
    m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
    m.handle(json!({ "type": "setMode", "id": "plan" })).await;
    let before = m.active().unwrap();
    let transcripts = TranscriptStore::new(s.dir.path().join("sessions"), log(), None);
    let id = before["id"].as_str().unwrap().to_owned();
    let t0 = std::time::Instant::now();
    while transcripts.load(&id).await.and_then(|r| r.acp_session_id).is_none() {
      assert!(t0.elapsed() < std::time::Duration::from_secs(5));
      tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let native = transcripts.load(&id).await.unwrap().acp_session_id;
    let history = before["turns"].clone();
    // second account: same session, new credential; the default account switches too
    let two = s.store.add("fake", draft("two@example.com", None, "good-key", None)).await.unwrap();
    s.accounts.reload().await;
    m.handle(json!({ "type": "selectAccount", "id": two.id })).await;
    assert_eq!(m.active_id().as_deref(), Some(id.as_str()));
    assert_eq!(m.sessions().len(), 1);
    let now = m.active().unwrap();
    expect_match(&now, json!({ "status": "ready", "accountId": two.id }));
    assert_eq!(now["turns"], history);
    assert_eq!(now["controls"]["modeId"], "plan");
    assert_eq!(now["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == "model").unwrap()["value"], "m2");
    assert_eq!(s.accounts.default_for("fake").unwrap().id, two.id);
    m.handle(json!({ "type": "send", "text": "inspect-native-history" })).await;
    let reply = m.active().unwrap()["turns"].as_array().unwrap().last().cloned().unwrap();
    let echoed: Value = serde_json::from_str(reply["blocks"][0]["markdown"].as_str().unwrap()).unwrap();
    expect_eq(&echoed["prompts"], json!([[{ "type": "text", "text": "hi" }], [{ "type": "text", "text": "inspect-native-history" }]]));
    m.handle(json!({ "type": "selectAccount", "id": one.id })).await;
    expect_match(m.active().unwrap(), json!({ "id": id, "status": "ready", "accountId": one.id }));
    m.dispose().await;
    assert_eq!(transcripts.load(&id).await.unwrap().acp_session_id, native, "load_only={load_only}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_during_a_running_turn_leaves_the_session_and_default_account_unchanged() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  s.m.new_session(None).await;
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  let one = s.m.active().unwrap()["accountId"].clone();
  let two = s.store.add("fake", draft("two@example.com", None, "good-key", None)).await.unwrap();
  s.accounts.reload().await;
  let sending = s.m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| s.m.active().is_some_and(|a| a["running"] == true), 5000).await;
  s.m.handle(json!({ "type": "selectAccount", "id": two.id })).await;
  assert_eq!(s.m.active().unwrap()["accountId"], one);
  assert_eq!(json!(s.accounts.default_for("fake").unwrap().id), one);
  s.m.handle(json!({ "type": "stop" })).await;
  sending.await.unwrap();
  s.m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn plus_imports_the_local_login_once_and_falls_back_to_terminal_login_afterwards() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  // first time: the local login is not in the list yet → import directly
  expect_match(s.accounts.add("fake", Cancel::new()).await.unwrap().unwrap(), json!({ "label": "one@example.com" }));
  assert!(s.toasts.lock().unwrap().last().unwrap().contains("one@example.com"));
  // second time: already imported → goes to login (the fake provider's login fails, proving that path was taken)
  assert!(s.accounts.add("fake", Cancel::new()).await.unwrap_err().to_string().contains("not in test"));
  // not logged in locally → also goes to login
  *s.provider.import_draft.lock().unwrap() = None;
  assert!(s.accounts.add("fake", Cancel::new()).await.unwrap_err().to_string().contains("not in test"));
  assert_eq!(s.accounts.list().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_key_is_auth_required_and_removing_the_account_removes_its_credential() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  let bad = s.store.add("fake", draft("bad@example.com", None, "bad-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.m.m.new_session_for(&s.m.v, Some("fake".into()), Some(bad.id.clone()), None, None).await.unwrap();
  assert_eq!(s.m.active().unwrap()["status"], "auth_required");
  s.m.handle(json!({ "type": "removeAccount", "id": bad.id })).await;
  assert!(s.accounts.list().is_empty());
  assert!(s.store.credential(&bad.id).await.unwrap().is_none());
  s.m.dispose().await;
}

// Claude / Codex: the CLI's own login is listed without a "+" click, follows a re-login and stays gone once removed
#[tokio::test(flavor = "multi_thread")]
async fn the_local_login_is_listed_by_itself_follows_a_relogin_and_stays_removed() {
  let dir = tempfile::tempdir().unwrap();
  let store = Arc::new(store_in(dir.path()));
  store.load().await.unwrap();
  let provider = Arc::new(FakeProvider { auto: true, agent: "fake", ..FakeProvider::default() });
  let accounts = AccountManager::new(store.clone(), vec![provider.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
  let pushed = Arc::new(AtomicUsize::new(0));
  let p = pushed.clone();
  accounts.subscribe(Arc::new(move |_| {
    p.fetch_add(1, Ordering::SeqCst);
  }));
  let labels = || accounts.list().into_iter().map(|a| (a.label, a.detail)).collect::<Vec<_>>();
  let set_local = |label: Option<&str>, detail: Option<&str>| *provider.import_draft.lock().unwrap() = label.map(|l| draft(l, detail, "local", None));
  // not logged in: nothing
  accounts.sync_local(None).await;
  assert!(labels().is_empty());
  // a login made in a terminal shows up, once
  set_local(Some("me@x.io"), Some("Claude Pro"));
  accounts.sync_local(Some("fake")).await;
  accounts.sync_local(None).await;
  assert_eq!(labels(), [("me@x.io".to_owned(), Some("Claude Pro".to_owned()))]);
  let id = accounts.list()[0].id.clone();
  assert_eq!(store.credential(&id).await.unwrap().unwrap().secret, "local");
  let after_add = pushed.load(Ordering::SeqCst);
  assert!(after_add >= 1);
  // nothing changed: no push
  accounts.sync_local(None).await;
  assert_eq!(pushed.load(Ordering::SeqCst), after_add);
  // a plan change reaches the account through the vendor's quota answer, and the CLI store's login-time plan (Claude
  // Code keeps `subscriptionType` through refreshes) does not undo it on the next sync
  *provider.plan.lock().unwrap() = Some("Claude Ultra".into());
  accounts.refresh_quota(&id, true).await;
  assert_eq!(labels(), [("me@x.io".to_owned(), Some("Claude Ultra".to_owned()))]);
  accounts.sync_local(None).await;
  assert_eq!(labels(), [("me@x.io".to_owned(), Some("Claude Ultra".to_owned()))]);
  // the same plan again changes nothing on disk
  let before = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
  accounts.refresh_quota(&id, true).await;
  assert_eq!(std::fs::read_to_string(dir.path().join("accounts.json")).unwrap(), before);
  *provider.plan.lock().unwrap() = None;
  // a re-login as someone else updates the same account
  set_local(Some("other@x.io"), None);
  accounts.sync_local(None).await;
  assert_eq!(labels(), [("other@x.io".to_owned(), None)]);
  assert_eq!(accounts.list()[0].id, id);
  // an account in its own home with the local login's identity is not duplicated
  set_local(Some("home@x.io"), None);
  let home = store.add("fake", draft("home@x.io", None, "/private/home", None)).await.unwrap();
  accounts.reload().await;
  accounts.sync_local(None).await;
  assert_eq!(accounts.list().len(), 2);
  assert_eq!(accounts.get(&id).unwrap().label, "other@x.io");
  accounts.remove(&home.id).await.unwrap();
  // removed by the user: not picked up again, until imported on purpose
  set_local(Some("other@x.io"), None);
  accounts.remove(&id).await.unwrap();
  accounts.sync_local(None).await;
  assert!(labels().is_empty());
  let again = accounts.import("fake").await.unwrap().unwrap();
  accounts.remove(&again.id).await.unwrap();
  // the removal is remembered again, and a fresh import clears it once more
  accounts.sync_local(None).await;
  assert!(labels().is_empty());
  accounts.import("fake").await.unwrap().unwrap();
  store.remove(&accounts.list()[0].id).await.unwrap();
  accounts.reload().await;
  accounts.sync_local(None).await;
  assert_eq!(labels(), [("other@x.io".to_owned(), None)]);
  // a provider without auto_import is never synced
  let manual = FakeProvider::new();
  let other = tempfile::tempdir().unwrap();
  let store2 = Arc::new(store_in(other.path()));
  store2.load().await.unwrap();
  let accounts2 = AccountManager::new(store2, vec![manual], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_: &str, _: &str| {}));
  accounts2.sync_local(None).await;
  assert!(accounts2.list().is_empty());
}
