//! test/accounts.test.ts: the account store and vault, Devin's credential plumbing, and the account layer wired into sessions

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_host::accounts::account_manager::AccountManager;
use acpira_host::accounts::account_store::{AUTH_API_KEY, AccountCredential, AccountDraft, AccountStore, FileVault, META_AUTH, account_secret_key};
use acpira_host::accounts::devin::{DevinAccountProvider, parse_status, parse_user_status, read_credentials, toml_of};
use acpira_host::accounts::provider::{AccountProvider, LoginFlow, QuotaRead};
use acpira_host::acp::transport::process::AgentProcess;
use acpira_host::acp::transport::cancel::Cancel;
use acpira_host::acp::transport::rpc::BoxFuture;
use acpira_host::acp::session::{AcpSession, SessionAccountHooks};
use acpira_host::store::transcript_store::TranscriptStore;
use acpira_host::http::HttpStatus;
use acpira_shared::transcript::{AccountQuota, AccountQuotaIssue, StrMap};

use crate::acp_session::view;
use crate::fake_or_skip;
use crate::session_manager::{Mgr, Opts};
use crate::support::{Disposing, Harness, expect_eq, expect_match, turns_in, until, v};

mod store;
mod devin;
mod login;
mod quota;
mod sign_in;
mod exhausted;
mod plugin;

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

/// Puts the key into authenticate's _meta.api_key like Devin does; import returns one account (or waits on a gate); quota drops
/// on every read, so refreshes are observable
#[derive(Default)]
struct FakeProvider {
  import_draft: Mutex<Option<AccountDraft>>,
  import_gate: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
  imports: AtomicUsize,
  login_flows: Mutex<Vec<LoginFlow>>,
  quota_reads: AtomicUsize,
  /// Reads of the `limited-key` account, which the vendor always answers with 429
  limited_reads: AtomicUsize,
  authentications: AtomicUsize,
  auto: bool,
  /// The credential store as a keychain seen over SSH: while set, authenticate fails like a CLI that cannot read its login
  locked: AtomicBool,
  /// The plan the vendor reports on quota reads of the local login (secret `local`)
  plan: Mutex<Option<String>>,
  /// Secrets of the credentials `forget` was asked to drop
  forgotten: Mutex<Vec<String>>,
  /// The agent it signs in (`fake` unless a test needs a built-in's quirks)
  agent: &'static str,
}

impl FakeProvider {
  fn new() -> Arc<FakeProvider> {
    Self::new_for("fake")
  }

  fn new_for(agent: &'static str) -> Arc<FakeProvider> {
    let p = FakeProvider { agent, ..Default::default() };
    *p.import_draft.lock().unwrap() = Some(draft("one@example.com", Some("Max"), "good-key", None));
    Arc::new(p)
  }
}

impl AccountProvider for FakeProvider {
  fn agent(&self) -> &str {
    self.agent
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
    let flow = self.login_flows.lock().unwrap().pop();
    Box::pin(async move { flow.ok_or_else(|| anyhow!("not in test")) })
  }
  fn accepts_keys(&self) -> bool {
    true
  }
  /// `fk-bad` is refused; another key signs in as the label after its `:` (else key@example.com), one home per label
  fn key_account(&self, key: String) -> BoxFuture<Result<AccountDraft>> {
    Box::pin(async move {
      if key == "fk-bad" {
        return Err(anyhow!("refused"));
      }
      let label = key.split_once(':').map_or("key@example.com", |(_, l)| l).to_owned();
      let home = format!("/homes/{label}");
      Ok(draft(&label, None, &key, Some(&[("home", &home), (META_AUTH, AUTH_API_KEY)])))
    })
  }
  fn forget(&self, cred: AccountCredential) -> Option<BoxFuture<()>> {
    self.forgotten.lock().unwrap().push(cred.secret);
    None
  }
  fn authenticate(&self, proc: Arc<AgentProcess>, cred: AccountCredential) -> Option<BoxFuture<Result<()>>> {
    self.authentications.fetch_add(1, Ordering::SeqCst);
    let locked = self.locked.load(Ordering::SeqCst);
    Some(Box::pin(async move {
      if locked {
        return Err(anyhow!("keychain locked"));
      }
      proc.request("authenticate", json!({ "methodId": "fake.login", "_meta": { "api_key": cred.secret } })).await?;
      Ok(())
    }))
  }
  fn quota(&self, cred: AccountCredential) -> Option<BoxFuture<Result<QuotaRead>>> {
    if cred.secret == "local" {
      let plan = self.plan.lock().unwrap().clone();
      return Some(Box::pin(async move { Ok(QuotaRead { quota: None, plan }) }));
    }
    if cred.secret == "limited-key" {
      self.limited_reads.fetch_add(1, Ordering::SeqCst);
      return Some(Box::pin(async { Err(anyhow::Error::new(HttpStatus { status: 429, retry_after: None })) }));
    }
    let reads = if cred.secret == "good-key" { Some(self.quota_reads.fetch_add(1, Ordering::SeqCst) + 1) } else { None };
    Some(Box::pin(async move {
      let reads = reads.ok_or_else(|| anyhow!("invalid api key"))?;
      let quota: AccountQuota = serde_json::from_value(json!({ "windows": [{ "id": "weekly", "remaining": 1.0 - reads as f64 / 10.0, "resetsAt": "2026-09-14T00:00:00.000Z" }], "fetchedAt": acpira_host::util::now_iso() }))?;
      Ok(Some(quota).into())
    }))
  }
  fn credentials_locked(&self) -> Option<BoxFuture<bool>> {
    let locked = self.locked.load(Ordering::SeqCst);
    Some(Box::pin(async move { locked }))
  }
  fn unlock_command(&self) -> Option<(String, Vec<String>)> {
    Some(("unlock-fake".into(), vec![]))
  }
}

struct Setup {
  m: Mgr,
  accounts: Arc<AccountManager>,
  store: Arc<AccountStore>,
  provider: Arc<FakeProvider>,
  toasts: Arc<Mutex<Vec<String>>>,
  /// Commands the account layer opened a terminal for
  terminals: Arc<Mutex<Vec<String>>>,
  dir: tempfile::TempDir,
}

fn setup(fake: &crate::support::FakeAgent, load_only: bool) -> Setup {
  setup_env(fake, if load_only { json!({ "FAKE_LOAD_ONLY": "1" }) } else { json!({}) })
}

fn setup_env(fake: &crate::support::FakeAgent, extra: Value) -> Setup {
  setup_env_as(fake, "fake", extra)
}

/// The same under a built-in agent's id, with that agent's quirks over the fake's wire
fn setup_env_as(fake: &crate::support::FakeAgent, agent: &'static str, extra: Value) -> Setup {
  let dir = tempfile::tempdir().unwrap();
  let cwd = dir.path().join("needs-auth");
  std::fs::create_dir(&cwd).unwrap();
  let provider = FakeProvider::new_for(agent);
  let store = Arc::new(store_in(dir.path()));
  let toasts = Arc::new(Mutex::new(vec![]));
  let t = toasts.clone();
  let terminals = Arc::new(Mutex::new(vec![]));
  let term = terminals.clone();
  let accounts = AccountManager::new(
    store.clone(),
    vec![provider.clone()],
    log(),
    Arc::new(move |_, command, _, _| term.lock().unwrap().push(command)),
    Arc::new(move |_l: &str, x: &str| t.lock().unwrap().push(x.to_owned())),
  );
  let mut env = json!({ "FAKE_SESSION_DIR": dir.path() });
  for (k, x) in extra.as_object().unwrap() {
    env[k] = x.clone();
  }
  let mut opts = Opts::with_agents(fake.setting_as(agent, json!({ "env": env })), agent).cwd(cwd.to_str().unwrap());
  opts.accounts = Some(accounts.clone());
  let m = Mgr::new(&dir.path().join("sessions"), opts);
  Setup { m, accounts, store, provider, toasts, terminals, dir }
}

fn actions(m: &Mgr) -> Value {
  v(m.m.account_actions())
}

fn first_quota(accounts: &AccountManager) -> Option<f64> {
  accounts.list().first().and_then(|a| a.quota.as_ref()).and_then(|q| v(q)["windows"][0]["remaining"].as_f64())
}

/// Lists one account for the fake agent at start, and lets go of it when Acpira passes on the user's removal
#[cfg(unix)]
const LISTER: &str = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
const list = accounts => send({ jsonrpc: '2.0', id: 'list-' + accounts.length, method: 'accounts/set', params: { accounts } });
let removing;
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') {
    send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
    return list([{ id: 'r1', agent: 'fake', label: 'team', badge: 'Rented', env: { FAKE_INIT_LOG: '@LOG@' },
      quota: { windows: [{ id: 'weekly', remaining: 0.25 }] } }]);
  }
  // A removal answers only once the host took the shorter list, as a plugin awaiting `ctx.accounts.set` does
  if (m.method === 'rpc/account.remove') { removing = m; return list([]); }
  if (m.id === 'list-0' && removing) return send({ jsonrpc: '2.0', id: removing.id, result: { removed: removing.params.params.id } });
  if (m.method && m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, result: null });
}).on('close', () => process.exit(0));
"#;
