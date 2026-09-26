//! test/accounts.test.ts: the account store and vault, Devin's credential plumbing, and the account layer wired into sessions

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_host::accounts::account_manager::AccountManager;
use acpira_host::accounts::account_store::{AccountCredential, AccountDraft, AccountStore, FileVault, account_secret_key};
use acpira_host::accounts::devin::{DevinAccountProvider, parse_status, parse_user_status, read_credentials, toml_of};
use acpira_host::accounts::provider::{AccountProvider, LoginFlow};
use acpira_host::acp::agent_process::AgentProcess;
use acpira_host::acp::cancel::Cancel;
use acpira_host::acp::rpc::BoxFuture;
use acpira_host::acp::session::{AcpSession, SessionAccountHooks};
use acpira_host::store::transcript_store::TranscriptStore;
use acpira_shared::transcript::{AccountQuota, StrMap};

use crate::acp_session::view;
use crate::fake_or_skip;
use crate::session_manager::{Mgr, Opts};
use crate::support::{Disposing, Harness, expect_eq, expect_match, until, v};

fn log() -> acpira_host::store::transcript_store::LogFn {
  Arc::new(|_: &str| {})
}

fn draft(label: &str, detail: Option<&str>, secret: &str, meta: Option<&[(&str, &str)]>) -> AccountDraft {
  AccountDraft {
    label: label.into(),
    detail: detail.map(str::to_owned),
    secret: secret.into(),
    meta: meta.map(|m| m.iter().map(|(k, x)| (k.to_string(), x.to_string())).collect()),
  }
}

fn store_in(dir: &std::path::Path) -> AccountStore {
  AccountStore::new(dir.join("accounts.json"), Arc::new(FileVault::new(dir.join("secrets.json"), log())), log())
}

fn file_ids(path: &std::path::Path) -> Vec<String> {
  let list: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
  list.as_array().unwrap().iter().map(|x| x["id"].as_str().unwrap().to_owned()).collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
  v.sort();
  v
}

#[tokio::test(flavor = "multi_thread")]
async fn metadata_goes_to_json_secrets_to_the_vault_and_relogin_replaces_only_the_secret() {
  let dir = tempfile::tempdir().unwrap();
  let store = store_in(dir.path());
  store.load().await.unwrap();
  let a = store.add("devin", draft("a@x.io", Some("Max"), "s1", Some(&[("api_server_url", "https://s")]))).await.unwrap();
  let b = store.add("devin", draft("b@x.io", None, "s2", None)).await.unwrap();
  assert_eq!(store.list(Some("devin")).iter().map(|x| x.label.as_str()).collect::<Vec<_>>(), ["a@x.io", "b@x.io"]);
  let text = std::fs::read_to_string(dir.path().join("accounts.json")).unwrap();
  assert!(!text.contains("s1") && !text.contains("s2"));
  assert_eq!(store.credential(&a.id).await.unwrap(), Some(AccountCredential { secret: "s1".into(), meta: Some([("api_server_url".to_owned(), "https://s".to_owned())].into()) }));
  // the earliest added is the default; after b is used once, the default switches to b
  assert_eq!(store.default_for("devin").unwrap().id, a.id);
  store.touch(&b.id).await.unwrap();
  assert_eq!(store.default_for("devin").unwrap().id, b.id);
  // re-login with the same label: id unchanged, secret replaced
  let a2 = store.add("devin", draft("a@x.io", None, "s1-new", None)).await.unwrap();
  assert_eq!(a2.id, a.id);
  assert_eq!(store.list(Some("devin")).len(), 2);
  assert_eq!(store.credential(&a.id).await.unwrap().unwrap().secret, "s1-new");
  store.remove(&a.id).await.unwrap();
  assert_eq!(store.list(Some("devin")).iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [b.id.clone()]);
  assert!(store.credential(&a.id).await.unwrap().is_none());
  // still there after a reload
  let store2 = store_in(dir.path());
  store2.load().await.unwrap();
  assert_eq!(store2.list(None).iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [b.id]);
}

#[tokio::test(flavor = "multi_thread")]
async fn two_hosts_on_one_accounts_file_never_erase_or_resurrect_each_others_accounts() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("accounts.json");
  let a = store_in(dir.path());
  let b = store_in(dir.path());
  a.load().await.unwrap();
  b.load().await.unwrap();
  let one = a.add("devin", draft("one@x.io", None, "s1", None)).await.unwrap();
  // b never saw `one`; its own add must land next to it, not over it
  let two = b.add("devin", draft("two@x.io", None, "s2", None)).await.unwrap();
  assert_eq!(sorted(file_ids(&file)), sorted(vec![one.id.clone(), two.id.clone()]));
  let secrets: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("secrets.json")).unwrap()).unwrap();
  expect_eq(secrets, json!({ account_secret_key(&one.id): "s1", account_secret_key(&two.id): "s2" }));
  // a's cache still lists only `one`; a touch writes through what is on disk
  assert_eq!(a.list(None).iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [one.id.clone()]);
  a.touch(&one.id).await.unwrap();
  assert_eq!(file_ids(&file).len(), 2);
  assert_eq!(sorted(a.list(None).iter().map(|x| x.id.clone()).collect()), sorted(vec![one.id.clone(), two.id.clone()]));
  assert!(!a.reload().await);
  // b removes `two`; a still knows it but must not put it back when it touches `one`
  b.remove(&two.id).await.unwrap();
  a.touch(&one.id).await.unwrap();
  assert_eq!(file_ids(&file), [one.id.clone()]);
  assert!(b.credential(&two.id).await.unwrap().is_none());
  // the other host's secret is readable without a restart
  assert_eq!(b.vault().get(&account_secret_key(&one.id)).await.unwrap().as_deref(), Some("s1"));
  // concurrent adds from both hosts all survive
  let (p, q, r) = tokio::join!(a.add("devin", draft("p@x.io", None, "p", None)), b.add("devin", draft("q@x.io", None, "q", None)), a.add("devin", draft("r@x.io", None, "r", None)));
  p.unwrap(); q.unwrap(); r.unwrap();
  assert_eq!(file_ids(&file).len(), 4);
  let secrets: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("secrets.json")).unwrap()).unwrap();
  assert_eq!(secrets.as_object().unwrap().len(), 4);
  assert!(b.reload().await);
  assert_eq!(b.list(None).len(), 4);
}

// The store serializes reload behind every mutation (one queue), so a reload racing an add or a remove can neither wipe the one
// nor restore the other
#[tokio::test(flavor = "multi_thread")]
async fn a_reload_racing_an_add_or_a_remove_cannot_undo_it() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("accounts.json");
  let store = store_in(dir.path());
  store.load().await.unwrap();
  let (added, _) = tokio::join!(store.add("devin", draft("a@x.io", None, "s1", None)), store.reload());
  let added = added.unwrap();
  assert_eq!(file_ids(&file), [added.id.clone()]);
  assert_eq!(store.list(None).iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [added.id.clone()]);
  assert_eq!(store.vault().get(&account_secret_key(&added.id)).await.unwrap().as_deref(), Some("s1"));
  let (removed, _) = tokio::join!(store.remove(&added.id), store.reload());
  removed.unwrap();
  assert!(file_ids(&file).is_empty());
  assert!(store.list(None).is_empty());
  assert!(store.vault().get(&account_secret_key(&added.id)).await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn load_drops_the_legacy_name_tail_from_stored_details_and_rewrites_the_file() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("accounts.json");
  std::fs::write(&file, json!([
    { "id": "x", "agent": "devin", "label": "a@x.io", "detail": "Devin Max · Someone", "addedAt": "2026-01-01T00:00:00.000Z" },
    { "id": "y", "agent": "devin", "label": "b@x.io", "detail": "Devin Max", "addedAt": "2026-01-01T00:00:00.000Z" },
  ]).to_string()).unwrap();
  let store = store_in(dir.path());
  store.load().await.unwrap();
  assert_eq!(store.list(None).iter().map(|a| a.detail.clone().unwrap()).collect::<Vec<_>>(), ["Devin Max", "Devin Max"]);
  assert!(!std::fs::read_to_string(&file).unwrap().contains("Someone"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_vault_round_trips_secrets_with_mode_600() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("secrets.json");
  let vault = FileVault::new(file.clone(), log());
  vault.store(&account_secret_key("a"), "s1").await.unwrap();
  assert_eq!(vault.get(&account_secret_key("a")).await.unwrap().as_deref(), Some("s1"));
  expect_eq(serde_json::from_str::<Value>(&std::fs::read_to_string(&file).unwrap()).unwrap(), json!({ account_secret_key("a"): "s1" }));
  #[cfg(unix)]
  assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&file).unwrap().permissions()) & 0o777, 0o600);
  vault.delete(&account_secret_key("a")).await.unwrap();
  assert!(vault.get(&account_secret_key("a")).await.unwrap().is_none());
  assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&file).unwrap()).unwrap(), json!({}));
  let again = FileVault::new(file, log());
  again.store(&account_secret_key("b"), "s2").await.unwrap();
  assert_eq!(again.get(&account_secret_key("b")).await.unwrap().as_deref(), Some("s2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_vault_file_is_an_empty_table() {
  let dir = tempfile::tempdir().unwrap();
  assert!(FileVault::new(dir.path().join("secrets.json"), log()).get("k").await.unwrap().is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_corrupt_vault_is_logged_and_never_overwritten() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("secrets.json");
  std::fs::write(&file, "{ nope").unwrap();
  let logs = Arc::new(Mutex::new(Vec::<String>::new()));
  let l = logs.clone();
  let vault = FileVault::new(file.clone(), Arc::new(move |x: &str| l.lock().unwrap().push(x.to_owned())));
  vault.store("k", "v").await.ok();
  assert_eq!(std::fs::read_to_string(&file).unwrap(), "{ nope");
  assert!(vault.get("k").await.ok().flatten().is_none());
  assert!(logs.lock().unwrap().iter().any(|l| l.contains("unreadable")), "{:?}", logs.lock().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_vault_sees_secrets_another_vault_wrote_and_rotated() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("secrets.json");
  let (a, b) = (FileVault::new(file.clone(), log()), FileVault::new(file, log()));
  a.store("k", "v1").await.unwrap();
  assert_eq!(b.get("k").await.unwrap().as_deref(), Some("v1"));
  a.store("k", "v2").await.unwrap();
  assert_eq!(b.get("k").await.unwrap().as_deref(), Some("v2"));
  a.delete("k").await.unwrap();
  assert!(b.get("k").await.unwrap().is_none());
  a.store(&account_secret_key("same"), "old-key").await.unwrap();
  assert_eq!(b.get(&account_secret_key("same")).await.unwrap().as_deref(), Some("old-key"));
  a.store(&account_secret_key("same"), "new-key").await.unwrap();
  assert_eq!(b.get(&account_secret_key("same")).await.unwrap().as_deref(), Some("new-key"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_devin_terminal_login_waits_for_the_toml_collects_it_and_cleans_up() {
  let scratch = tempfile::tempdir().unwrap();
  let p = DevinAccountProvider::new(scratch.path().to_path_buf(), Arc::new(|| Box::pin(async { Some("/nonexistent/devin".to_owned()) })));
  let flow = p.login().await.unwrap();
  assert_eq!(flow.command, "/nonexistent/devin");
  assert_eq!(flow.args, ["auth", "login"]);
  assert_eq!(flow.env.get("ACP_BACKEND"), Some(&None));
  let dir = std::path::PathBuf::from(flow.env["XDG_DATA_HOME"].clone().unwrap());
  assert!(dir.starts_with(scratch.path()));
  let cancel = Cancel::new();
  let collecting = tokio::spawn((flow.collect)(cancel.clone()));
  // simulate the CLI login writing to disk
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  std::fs::create_dir_all(dir.join("devin")).unwrap();
  std::fs::write(dir.join("devin").join("credentials.toml"), toml_of(&AccountCredential { secret: "devin-key-wxyz".into(), meta: Some([("api_server_url".to_owned(), "https://s".to_owned())].into()) })).unwrap();
  let got = collecting.await.unwrap().expect("collected");
  assert_eq!(got, draft("Devin …wxyz", None, "devin-key-wxyz", Some(&[("api_server_url", "https://s")])));
  assert!(!dir.exists());
  // abort: nothing written, collect returns nothing and the dir is cleaned up
  let flow2 = p.login().await.unwrap();
  let dir2 = std::path::PathBuf::from(flow2.env["XDG_DATA_HOME"].clone().unwrap());
  let cancel2 = Cancel::new();
  let c2 = cancel2.clone();
  tokio::spawn(async move {
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    c2.cancel();
  });
  assert!((flow2.collect)(cancel2).await.is_none());
  assert!(!dir2.exists());
}

#[test]
fn on_demand_usd_balances_are_kept_without_inventing_missing_or_malformed_amounts() {
  let balance = |micros: Value| parse_user_status(&json!({ "userStatus": { "planStatus": { "overageBalanceMicros": micros } } })).map(v);
  // Live Free-seat response matches the billing page's $68.37 display
  expect_match(balance(json!("68373043")), json!({ "windows": [], "onDemandBalanceUsd": 68.373043 }));
  expect_match(balance(json!("0")), json!({ "onDemandBalanceUsd": 0 }));
  expect_match(balance(json!(0)), json!({ "onDemandBalanceUsd": 0 }));
  expect_match(balance(json!("-503099")), json!({ "onDemandBalanceUsd": -0.503099 }));
  for bad in [Value::Null, json!(""), json!("bad"), json!("1.5"), json!(1.5), json!("9007199254740992")] {
    assert!(balance(bad.clone()).is_none(), "{bad}");
  }
  expect_match(parse_user_status(&json!({ "userStatus": { "planStatus": {
    "planInfo": { "billingStrategy": "BILLING_STRATEGY_QUOTA", "hideDailyQuota": true },
    "weeklyQuotaRemainingPercent": 87, "overageBalanceMicros": "68373043",
  } } })), json!({ "windows": [{ "id": "weekly", "remaining": 0.87 }], "onDemandBalanceUsd": 68.373043 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_credentials_toml_round_trips_and_status_output_yields_the_label_and_tier() {
  let dir = tempfile::tempdir().unwrap();
  let cred = AccountCredential { secret: "devin-abc".into(), meta: Some([
    ("api_server_url".to_owned(), "https://server.codeium.com".to_owned()),
    ("devin_webapp_host".to_owned(), "app.devin.ai".to_owned()),
    ("devin_api_url".to_owned(), "https://api.devin.ai".to_owned()),
  ].into()) };
  std::fs::write(dir.path().join("credentials.toml"), toml_of(&cred)).unwrap();
  assert_eq!(read_credentials(&dir.path().join("credentials.toml")).await, Some(cred));
  assert!(read_credentials(&dir.path().join("nope.toml")).await.is_none());
  let out = "Logged in (via Devin).\n\nUser:\n  Name:              Someone\n  Email:             someone@example.com\n\nAccount:\n  Tier:              Devin Max\n  Plan:              Max\n";
  assert_eq!(parse_status(out), Some(("someone@example.com".to_owned(), Some("Devin Max".to_owned()))));
  assert!(parse_status("Not logged in.").is_none());
}

// Shape of GetUserStatus in JSON encoding as the seat-management service returned it for a Max seat (2026-09): whole-number percents,
// int64 reset times as strings, hideDailyQuota on the plan
#[test]
fn get_user_status_exposes_the_windows_the_plan_has() {
  let max = json!({ "userStatus": { "planStatus": { "planInfo": { "planName": "Max", "billingStrategy": "BILLING_STRATEGY_QUOTA", "hideDailyQuota": true },
    "dailyQuotaRemainingPercent": 100, "weeklyQuotaRemainingPercent": 94, "dailyQuotaResetAtUnix": "1788854400", "weeklyQuotaResetAtUnix": "1789286400" } } });
  let q = v(parse_user_status(&max).unwrap());
  expect_eq(&q["windows"], json!([{ "id": "weekly", "remaining": 0.94, "resetsAt": "2026-09-13T08:00:00.000Z" }]));
  assert!(acpira_host::util::ms_of_iso(q["fetchedAt"].as_str().unwrap()).is_some());
  // Pro: both windows; the weekly one is exhausted, which proto3 JSON expresses by omitting the zero-valued percent (and reset time)
  let pro = json!({ "userStatus": { "planStatus": { "planInfo": { "planName": "Pro", "billingStrategy": "BILLING_STRATEGY_QUOTA" }, "dailyQuotaRemainingPercent": 37, "dailyQuotaResetAtUnix": "1788854400" } } });
  expect_eq(&v(parse_user_status(&pro).unwrap())["windows"], json!([
    { "id": "daily", "remaining": 0.37, "resetsAt": "2026-09-08T08:00:00.000Z" },
    { "id": "weekly", "remaining": 0 },
  ]));
  // Not billed by quota and no percents at all → nothing to show; an explicit zero on such a plan still counts
  assert!(parse_user_status(&json!({ "userStatus": { "planStatus": { "planInfo": { "billingStrategy": "BILLING_STRATEGY_CREDITS" } } } })).is_none());
  expect_eq(&v(parse_user_status(&json!({ "userStatus": { "planStatus": { "planInfo": {}, "weeklyQuotaRemainingPercent": 0 } } })).unwrap())["windows"], json!([{ "id": "weekly", "remaining": 0 }]));
  assert!(parse_user_status(&json!({})).is_none());
  assert!(parse_user_status(&Value::Null).is_none());
}

/// Puts the key into authenticate's _meta.api_key like Devin does; import returns one account (or waits on a gate); quota drops
/// on every read, so refreshes are observable
#[derive(Default)]
struct FakeProvider {
  import_draft: Mutex<Option<AccountDraft>>,
  import_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
  imports: AtomicUsize,
  quota_reads: AtomicUsize,
  authentications: AtomicUsize,
  auto: bool,
}

impl FakeProvider {
  fn new() -> Arc<FakeProvider> {
    let p = FakeProvider::default();
    *p.import_draft.lock().unwrap() = Some(draft("one@example.com", Some("Max"), "good-key", None));
    Arc::new(p)
  }
}

impl AccountProvider for FakeProvider {
  fn agent(&self) -> &str {
    "fake"
  }
  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    self.imports.fetch_add(1, Ordering::SeqCst);
    let gate = self.import_gate.lock().unwrap().take();
    let draft = self.import_draft.lock().unwrap().clone();
    Box::pin(async move {
      if let Some(g) = gate {
        g.await.ok();
      }
      draft
    })
  }
  fn auto_import(&self) -> bool {
    self.auto
  }
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    Box::pin(async { Err(anyhow!("not in test")) })
  }
  fn authenticate(&self, proc: Arc<AgentProcess>, cred: AccountCredential) -> Option<BoxFuture<Result<()>>> {
    self.authentications.fetch_add(1, Ordering::SeqCst);
    Some(Box::pin(async move {
      proc.request("authenticate", json!({ "methodId": "fake.login", "_meta": { "api_key": cred.secret } })).await?;
      Ok(())
    }))
  }
  fn quota(&self, cred: AccountCredential) -> Option<BoxFuture<Result<Option<AccountQuota>>>> {
    let reads = if cred.secret == "good-key" { Some(self.quota_reads.fetch_add(1, Ordering::SeqCst) + 1) } else { None };
    Some(Box::pin(async move {
      let reads = reads.ok_or_else(|| anyhow!("invalid api key"))?;
      Ok(Some(serde_json::from_value(json!({ "windows": [{ "id": "weekly", "remaining": 1.0 - reads as f64 / 10.0, "resetsAt": "2026-09-14T00:00:00.000Z" }], "fetchedAt": acpira_host::util::now_iso() }))?))
    }))
  }
}

struct Setup {
  m: Mgr,
  accounts: Arc<AccountManager>,
  store: Arc<AccountStore>,
  provider: Arc<FakeProvider>,
  toasts: Arc<Mutex<Vec<String>>>,
  dir: tempfile::TempDir,
}

fn setup(fake: &crate::support::FakeAgent, load_only: bool) -> Setup {
  setup_env(fake, if load_only { json!({ "FAKE_LOAD_ONLY": "1" }) } else { json!({}) })
}

fn setup_env(fake: &crate::support::FakeAgent, extra: Value) -> Setup {
  let dir = tempfile::tempdir().unwrap();
  let cwd = dir.path().join("needs-auth");
  std::fs::create_dir(&cwd).unwrap();
  let provider = FakeProvider::new();
  let store = Arc::new(store_in(dir.path()));
  let toasts = Arc::new(Mutex::new(vec![]));
  let t = toasts.clone();
  let accounts = AccountManager::new(store.clone(), vec![provider.clone()], log(), Arc::new(|_, _, _, _| {}), Arc::new(move |_l: &str, x: &str| t.lock().unwrap().push(x.to_owned())));
  let mut env = json!({ "FAKE_SESSION_DIR": dir.path() });
  for (k, x) in extra.as_object().unwrap() {
    env[k] = x.clone();
  }
  let mut opts = Opts::with_agents(fake.setting(json!({ "env": env })), "fake").cwd(cwd.to_str().unwrap());
  opts.accounts = Some(accounts.clone());
  let m = Mgr::new(&dir.path().join("sessions"), opts);
  Setup { m, accounts, store, provider, toasts, dir }
}

fn actions(m: &Mgr) -> Value {
  v(m.m.account_actions())
}

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

fn first_quota(accounts: &AccountManager) -> Option<f64> {
  accounts.list().first().and_then(|a| a.quota.as_ref()).and_then(|q| v(q)["windows"][0]["remaining"].as_f64())
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_is_fetched_after_the_hand_off_and_after_turns_and_dropped_with_the_account() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let m = &s.m;
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  // the session is ready before the vendor answers; the quota lands on the account list afterwards
  until(|| first_quota(&s.accounts) == Some(0.9), 5000).await;
  expect_eq(v(s.accounts.list()[0].quota.as_ref().unwrap())["windows"].clone(), json!([{ "id": "weekly", "remaining": 0.9, "resetsAt": "2026-09-14T00:00:00.000Z" }]));
  let pushed = || m.events.lock().unwrap().iter().filter(|e| e["type"] == "accounts").filter_map(|e| e["accounts"][0]["quota"]["windows"][0]["remaining"].as_f64()).collect::<Vec<_>>();
  until(|| pushed().contains(&0.9), 2000).await;
  // the webview asking right away costs no request
  m.handle(json!({ "type": "refreshQuota", "agent": "fake" })).await;
  assert_eq!(s.provider.quota_reads.load(Ordering::SeqCst), 1);
  // a finished turn forces a re-read
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| first_quota(&s.accounts) == Some(0.8), 5000).await;
  assert_eq!(m.active().unwrap()["accountId"], s.accounts.list()[0].id.as_str());
  // an account whose key the vendor rejects has no quota, and nothing is thrown
  let bad = s.store.add("fake", draft("bad@example.com", None, "bad-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.accounts.refresh_quota(&bad.id, true).await;
  assert!(s.accounts.get(&bad.id).unwrap().quota.is_none());
  let good = s.accounts.list()[0].id.clone();
  m.handle(json!({ "type": "removeAccount", "id": good })).await;
  assert!(!s.accounts.list().iter().any(|a| a.quota.is_some()));
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn quota_is_fetched_as_soon_as_an_account_is_stored() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  let a = s.accounts.add("fake").await.unwrap().unwrap();
  assert!(a.quota.is_none());
  until(|| s.accounts.get(&a.id).and_then(|x| x.quota).is_some_and(|q| v(q)["windows"][0]["remaining"] == 0.9), 5000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn plus_imports_the_local_login_once_and_falls_back_to_terminal_login_afterwards() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  // first time: the local login is not in the list yet → import directly
  expect_match(s.accounts.add("fake").await.unwrap().unwrap(), json!({ "label": "one@example.com" }));
  assert!(s.toasts.lock().unwrap().last().unwrap().contains("one@example.com"));
  // second time: already imported → goes to login (the fake provider's login fails, proving that path was taken)
  assert!(s.accounts.add("fake").await.unwrap_err().to_string().contains("not in test"));
  // not logged in locally → also goes to login
  *s.provider.import_draft.lock().unwrap() = None;
  assert!(s.accounts.add("fake").await.unwrap_err().to_string().contains("not in test"));
  assert_eq!(s.accounts.list().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invalid_key_is_auth_required_and_removing_the_account_removes_its_credential() {
  let fake = fake_or_skip!();
  let s = setup(&fake, false);
  s.m.init().await;
  let bad = s.store.add("fake", draft("bad@example.com", None, "bad-key", None)).await.unwrap();
  s.accounts.reload().await;
  s.m.m.new_session_for(&s.m.v, Some("fake".into()), Some(bad.id.clone())).await.unwrap();
  assert_eq!(s.m.active().unwrap()["status"], "auth_required");
  s.m.handle(json!({ "type": "removeAccount", "id": bad.id })).await;
  assert!(s.accounts.list().is_empty());
  assert!(s.store.credential(&bad.id).await.unwrap().is_none());
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

/// A session on `one` (good-key, exhausted) with `two` (good-key-2) saved next to it
async fn exhausted_session(s: &Setup) -> (String, String) {
  s.m.init().await;
  s.m.new_session(None).await;
  s.m.handle(json!({ "type": "addAccount", "agent": "fake", "via": "import" })).await;
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  let one = s.accounts.list()[0].id.clone();
  let two = s.store.add("fake", draft("two@example.com", None, "good-key-2", None)).await.unwrap().id;
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
    until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && a["turns"].as_array().unwrap().len() == 6 && a["turns"][5]["stop"] == "end_turn"), 10_000).await;
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
  until(|| s.m.active().is_some_and(|a| a["status"] == "ready" && a["turns"].as_array().unwrap().len() == 4 && a["turns"][3]["error"].is_object()), 10_000).await;
  expect_match(s.m.active().unwrap(), json!({ "accountId": two }));
  expect_match(&turns(&s.m)[3], json!({ "stop": "error", "error": { "code": -32011, "kind": "resource_exhausted" } }));
  s.m.dispose().await;

  let s = setup_env(&fake, json!({ "FAKE_EXHAUSTED_KEYS": "good-key" }));
  s.accounts.set_switch_policy(Arc::new(|_: &str| acpira_host::accounts::switch::SwitchStrategy::Off));
  let (one, _) = exhausted_session(&s).await;
  s.m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| s.m.active().is_some_and(|a| a["turns"].as_array().unwrap().len() == 2 && a["turns"][1]["error"].is_object()), 5000).await;
  expect_match(s.m.active().unwrap(), json!({ "status": "ready", "accountId": one }));
  s.m.dispose().await;
}

// Claude / Codex: the CLI's own login is listed without a "+" click, follows a re-login and stays gone once removed
#[tokio::test(flavor = "multi_thread")]
async fn the_local_login_is_listed_by_itself_follows_a_relogin_and_stays_removed() {
  let dir = tempfile::tempdir().unwrap();
  let store = Arc::new(store_in(dir.path()));
  store.load().await.unwrap();
  let provider = Arc::new(FakeProvider { auto: true, ..FakeProvider::default() });
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
  // a plan change and then a re-login as someone else update the same account
  set_local(Some("me@x.io"), Some("Claude Max"));
  accounts.sync_local(None).await;
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
