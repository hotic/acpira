//! Sessions stopped on sign-in: a locked credential store, locked engines, hand-offs between engines, hook failures

use super::*;

// A remote workspace over SSH sees the macOS keychain locked: the saved account stays listed, the agent reports the locked
// store, quota is not read and the session stops on sign-in. Unlocking runs the provider's command in a terminal, waits for
// the store to open, then reconnects that session on the same account
#[tokio::test(flavor = "multi_thread")]
async fn a_locked_credential_store_is_reported_and_unlocking_it_reconnects_sessions_stuck_on_sign_in() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let m = &s.m;
  m.init().await;
  let one = s.store.add("fake", draft("one@example.com", None, "good-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.provider.locked.store(true, Ordering::SeqCst);
  m.handle(json!({ "type": "refreshQuota", "agent": "fake" })).await;
  let locked = || m.m.agents().into_iter().find(|a| a.id == "fake").unwrap().credentials_locked;
  assert_eq!(locked(), Some(true));
  let pushed = m.events.lock().unwrap().iter().rfind(|e| e["type"] == "agents").cloned().unwrap();
  assert_eq!(pushed["agents"].as_array().unwrap().iter().find(|a| a["id"] == "fake").unwrap()["credentialsLocked"], true);
  assert_eq!(s.provider.quota_reads.load(Ordering::SeqCst), 0);
  m.m.new_session_for(&m.v, Some("fake".into()), Some(one.id.clone()), None, None).await.unwrap();
  assert_eq!(m.active().unwrap()["status"], "auth_required");

  let first_unlock = m.spawn_handle(json!({ "type": "unlockCredentials", "agent": "fake" }));
  until(|| s.terminals.lock().unwrap().len() == 1 && s.terminals.lock().unwrap()[0] == "unlock-fake", 2000).await;
  assert_eq!(actions(m), json!([{ "agent": "fake", "via": "unlock", "status": "pending" }]));
  // A second click while waiting supersedes the first attempt and opens a fresh terminal (in case the first was closed)
  let unlock = m.spawn_handle(json!({ "type": "unlockCredentials", "agent": "fake" }));
  until(|| s.terminals.lock().unwrap().len() == 2, 2000).await;
  first_unlock.await.unwrap();
  assert_eq!(actions(m), json!([{ "agent": "fake", "via": "unlock", "status": "pending" }]));
  // The password typed in the terminal
  s.provider.locked.store(false, Ordering::SeqCst);
  unlock.await.unwrap();
  assert_eq!(actions(m), json!([{ "agent": "fake", "via": "unlock", "status": "success" }]));
  assert_eq!(locked(), None);
  until(|| m.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  expect_match(m.active().unwrap(), json!({ "accountId": one.id }));
  until(|| first_quota(&s.accounts).is_some(), 5000).await;
  m.dispose().await;
}

// Opening an idle phone session from an SSH security session must not move it onto a locked keychain. The phone may
// already be on its list (no viewer in the owner); other viewers must still act through the authenticated owner.
#[tokio::test(flavor = "multi_thread")]
async fn a_locked_engine_viewing_a_phone_session_does_not_take_it_over_or_break_other_viewers() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  let one = s.store.add("fake", draft("one@example.com", None, "good-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.m.m.new_session_for(&s.m.v, Some("fake".into()), Some(one.id.clone()), None, None).await.unwrap();
  s.m.handle(json!({ "type": "send", "text": "from the phone" })).await;
  let id = s.m.active_id().unwrap();
  s.m.store.flush(s.m.m.live_session(&id).unwrap()).await.unwrap();

  let locked = FakeProvider::new();
  locked.locked.store(true, Ordering::SeqCst);
  let store = Arc::new(store_in(s.dir.path()));
  store.load().await.unwrap();
  let accounts = AccountManager::new(store, vec![locked.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_, _| {}));
  accounts.check_locks(Some("fake")).await;
  let opts = || Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": s.dir.path() } })), "fake")
    .cwd(s.dir.path().join("needs-auth").to_str().unwrap());
  let mut ssh_opts = opts();
  ssh_opts.accounts = Some(accounts.clone());
  let ssh = Mgr::new(s.m.store.dir(), ssh_opts);
  let mut desktop_opts = opts();
  desktop_opts.accounts = Some(s.accounts.clone());
  let desktop = Mgr::new(s.m.store.dir(), desktop_opts);
  ssh.init().await;
  desktop.init().await;
  until(|| s.m.m.bus_connected() && ssh.m.bus_connected() && desktop.m.bus_connected(), 5000).await;

  // The phone leaves the chat before SSH opens it: the owner's lack of viewers used to trigger an implicit handover.
  s.m.handle(json!({ "type": "deselectSession", "id": id })).await;
  ssh.m.select_session_for(&ssh.v, &id).await;
  until(|| ssh.active().is_some_and(|v| v["status"] == "ready"), 5000).await;
  // Cover both the owner's handover watcher and the follower's next mirror poll, not just its first pushed view.
  tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
  assert!(s.m.m.live_session(&id).is_some(), "viewing over SSH must retain the authenticated owner: {:?}", s.m.logs());
  assert!(ssh.m.live_session(&id).is_none(), "a locked viewer must not start the session: {:?}", ssh.logs());
  assert_eq!(locked.authentications.load(Ordering::SeqCst), 0);

  assert!(s.m.m.release_idle(std::time::Duration::ZERO, 0).await.is_empty(), "a remote viewer must keep its authenticated owner alive");

  // Even an explicit takeover must not stop a healthy owner when the requester cannot read its saved login.
  ssh.handle(json!({ "type": "takeOverSession", "sessionId": id })).await;
  assert!(s.m.m.leased().contains(&id), "a refused takeover must leave the owner alone");
  assert!(!ssh.toasts().is_empty(), "the requester must learn why takeover was refused");
  ssh.handle(json!({ "type": "send", "sessionId": id, "text": "from SSH, through the owner" })).await;
  until(|| s.m.view_of(&id).is_some_and(|v| v["status"] == "ready" && turns_in(&v) == 4), 5000).await;
  until(|| ssh.active().is_some_and(|v| v["status"] == "ready" && turns_in(&v) == 4), 5000).await;

  desktop.m.select_session_for(&desktop.v, &id).await;
  until(|| desktop.active().is_some_and(|v| v["status"] == "ready" && turns_in(&v) == 4), 5000).await;
  s.m.m.select_session_for(&s.m.v, &id).await;
  assert_eq!(s.m.active().unwrap()["status"], "ready", "the phone can return to the same session");
  assert_eq!(ssh.active().unwrap()["status"], "ready");
  assert!(s.m.m.leased().contains(&id));
  ssh.dispose().await;
  desktop.dispose().await;
  s.m.dispose().await;
}

/// A second engine on the same store whose credential store is locked, as an SSH login's keychain is
async fn locked_engine(fake: &crate::support::FakeAgent, s: &Setup) -> (Mgr, Arc<FakeProvider>, Arc<AccountManager>) {
  let locked = FakeProvider::new();
  locked.locked.store(true, Ordering::SeqCst);
  let store = Arc::new(store_in(s.dir.path()));
  store.load().await.unwrap();
  let accounts = AccountManager::new(store, vec![locked.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(|_, _| {}));
  accounts.check_locks(Some("fake")).await;
  let mut opts = Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": s.dir.path() } })), "fake")
    .cwd(s.dir.path().join("needs-auth").to_str().unwrap());
  opts.accounts = Some(accounts.clone());
  let m = Mgr::new(s.m.store.dir(), opts);
  m.init().await;
  (m, locked, accounts)
}

// The owner's engine finishing (no window left) frees the lease. A locked engine still showing the session must not
// open it then: it would stop on sign-in and hold the lease where no engine with the login reaches it.
#[tokio::test(flavor = "multi_thread")]
async fn a_locked_engine_left_with_a_freed_session_stays_read_only_until_unlocked() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  let one = s.store.add("fake", draft("one@example.com", None, "good-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.m.m.new_session_for(&s.m.v, Some("fake".into()), Some(one.id.clone()), None, None).await.unwrap();
  s.m.handle(json!({ "type": "send", "text": "from the phone" })).await;
  let id = s.m.active_id().unwrap();
  s.m.store.flush(s.m.m.live_session(&id).unwrap()).await.unwrap();

  let (ssh, locked, accounts) = locked_engine(&fake, &s).await;
  until(|| s.m.m.bus_connected() && ssh.m.bus_connected(), 15_000).await;
  ssh.m.select_session_for(&ssh.v, &id).await;
  until(|| ssh.active().is_some_and(|v| v["status"] == "ready"), 15_000).await;
  s.m.dispose().await;

  until(|| ssh.active().is_some_and(|v| v["status"] == "readonly" && v["error"].as_str().is_some_and(|e| e.contains("cannot read"))), 15_000)
    .await;
  // Several mirror polls past the freed lease
  tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
  assert!(ssh.m.live_session(&id).is_none(), "a locked engine must not open a freed session: {:?}", ssh.logs());
  assert_eq!(locked.authentications.load(Ordering::SeqCst), 0);
  assert_eq!(ssh.active().unwrap()["status"], "readonly");

  // Unlocked: the next poll opens it here, signed in
  locked.locked.store(false, Ordering::SeqCst);
  accounts.check_locks(Some("fake")).await;
  until(|| ssh.m.live_session(&id).is_some() && ssh.active().is_some_and(|v| v["status"] == "ready" && turns_in(&v) == 2), 20_000).await;
  ssh.dispose().await;
}

// A session stopped on sign-in in an engine no window shows is handed to an engine that wants it, instead of pinning
// its lease for as long as that engine lives
#[tokio::test(flavor = "multi_thread")]
async fn a_session_stalled_on_sign_in_is_handed_to_the_engine_that_wants_it() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  let one = s.store.add("fake", draft("one@example.com", None, "good-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.m.m.new_session_for(&s.m.v, Some("fake".into()), Some(one.id.clone()), None, None).await.unwrap();
  s.m.handle(json!({ "type": "send", "text": "from the desktop" })).await;
  let id = s.m.active_id().unwrap();
  s.m.handle(json!({ "type": "deselectSession", "id": id })).await;
  assert_eq!(s.m.m.release_idle(std::time::Duration::ZERO, 0).await, [id.clone()]);

  let (ssh, _locked, _accounts) = locked_engine(&fake, &s).await;
  until(|| s.m.m.bus_connected() && ssh.m.bus_connected(), 15_000).await;
  ssh.m.select_session_for(&ssh.v, &id).await;
  until(|| ssh.active().is_some_and(|v| v["status"] == "auth_required"), 15_000).await;
  // The SSH window goes away; its engine keeps running with the session stopped on sign-in
  ssh.handle(json!({ "type": "deselectSession", "id": id })).await;

  s.m.m.select_session_for(&s.m.v, &id).await;
  until(|| s.m.m.live_session(&id).is_some() && s.m.active().is_some_and(|v| v["status"] == "ready" && turns_in(&v) == 2), 20_000).await;
  assert!(ssh.m.live_session(&id).is_none(), "{:?}", ssh.logs());
  ssh.dispose().await;
  s.m.dispose().await;
}

/// Hooks answered by closures, the AcpSession side of the account layer without a manager
struct Hooks<F: Fn(u32, Arc<AgentProcess>) -> BoxFuture<Result<()>> + Send + Sync>(F, AtomicUsize);

impl<F: Fn(u32, Arc<AgentProcess>) -> BoxFuture<Result<()>> + Send + Sync> SessionAccountHooks for Hooks<F> {
  fn spawn_env(&self, _agent: String, _account: String) -> BoxFuture<Option<StrMap>> {
    Box::pin(async { None })
  }
  fn authenticate(&self, _agent: String, _account: String, proc: Arc<AgentProcess>) -> BoxFuture<Result<()>> {
    let n = self.1.fetch_add(1, Ordering::SeqCst) as u32 + 1;
    (self.0)(n, proc)
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_credential_from_the_hooks_enters_auth_required_and_keeps_the_reason() {
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  h.deps.accounts = Some(Arc::new(Hooks(|_, _| Box::pin(async { Err(anyhow!("账号 x 的凭据不在了")) }), AtomicUsize::new(0))));
  let cwd = tempfile::Builder::new().prefix("acpira-needs-auth-").tempdir().unwrap();
  let s = Disposing(AcpSession::fresh("fake", cwd.path().to_str().unwrap(), h.deps.clone(), Some("missing".into())));
  s.start().await;
  expect_match(view(&s), json!({ "status": "auth_required", "error": "账号 x 的凭据不在了", "accountId": "missing" }));
}

// Regression: the account hand-off can fail transiently (Devin timed out fetching team settings right after a window reload)
// while the process stays alive; retry must re-hand the credential instead of bouncing session/load off -32000 forever
#[tokio::test(flavor = "multi_thread")]
async fn a_transient_hand_off_failure_is_auth_required_and_retry_re_authenticates_on_the_same_process() {
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  let hooks = Arc::new(Hooks(
    |n, proc: Arc<AgentProcess>| -> BoxFuture<Result<()>> {
      Box::pin(async move {
        if n == 1 {
          return Err(anyhow!("Authentication failed: Failed to fetch team settings: fetch timed out after 10000ms"));
        }
        proc.request("authenticate", json!({ "methodId": "fake.login", "_meta": { "api_key": "good-key" } })).await?;
        Ok(())
      })
    },
    AtomicUsize::new(0),
  ));
  h.deps.accounts = Some(hooks.clone());
  let cwd = tempfile::Builder::new().prefix("acpira-needs-auth-").tempdir().unwrap();
  let s = Disposing(AcpSession::fresh("fake", cwd.path().to_str().unwrap(), h.deps.clone(), Some("acc1".into())));
  s.start().await;
  expect_match(view(&s), json!({ "status": "auth_required", "error": "Authentication failed: Failed to fetch team settings: fetch timed out after 10000ms" }));
  assert!(s.alive());
  s.retry().await.unwrap();
  assert_eq!(hooks.1.load(Ordering::SeqCst), 2);
  expect_match(view(&s), json!({ "status": "ready", "error": null }));
}
