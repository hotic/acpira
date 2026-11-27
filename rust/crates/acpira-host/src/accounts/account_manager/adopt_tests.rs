use super::*;
use crate::accounts::account_store::FileVault;
use crate::accounts::provider::LoginFlow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A CLI whose local login is copied out: each adoption is a folder under `homes`, holding `login` when the local
/// login was the adopted label's at the time
struct Adopting {
  local: parking_lot::Mutex<Option<String>>,
  homes: PathBuf,
  seq: AtomicUsize,
}

impl AccountProvider for Adopting {
  fn agent(&self) -> &str {
    "antigravity"
  }
  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    let label = self.local.lock().clone();
    Box::pin(async move { label.map(|label| AccountDraft { label, detail: None, secret: LOCAL_LOGIN.into(), meta: None }) })
  }
  fn auto_import(&self) -> bool {
    true
  }
  fn adopt_local(&self, label: String) -> Option<BoxFuture<Result<AccountDraft>>> {
    let home = self.homes.join(self.seq.fetch_add(1, Ordering::SeqCst).to_string());
    let signed_in = self.local.lock().as_deref() == Some(label.as_str());
    Some(Box::pin(async move {
      std::fs::create_dir_all(&home)?;
      if signed_in {
        std::fs::write(home.join("login"), &label)?;
      }
      Ok(AccountDraft { label, detail: None, secret: home.to_string_lossy().into_owned(), meta: None })
    }))
  }
  fn forget(&self, cred: AccountCredential) -> Option<BoxFuture<()>> {
    Some(Box::pin(async move {
      let _ = std::fs::remove_dir_all(cred.secret);
    }))
  }
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    Box::pin(async { Err(anyhow!("no login here")) })
  }
}

#[tokio::test]
async fn a_local_login_moves_to_a_home_of_its_own_and_another_identity_never_takes_the_account() {
  let dir = tempfile::tempdir().unwrap();
  let log: LogFn = Arc::new(|_| {});
  let vault = Arc::new(FileVault::new(dir.path().join("secrets.json"), log.clone()));
  let store = Arc::new(AccountStore::new(dir.path().join("accounts.json"), vault, log.clone()));
  store.load().await.unwrap();
  let on_cli = |label: &str| AccountDraft { label: label.into(), detail: None, secret: LOCAL_LOGIN.into(), meta: None };
  // An earlier build left the account on the CLI's store
  let me = store.add("antigravity", on_cli("me@x.io")).await.unwrap();
  let p = Arc::new(Adopting { local: Some("me@x.io".to_owned()).into(), homes: dir.path().join("homes"), seq: AtomicUsize::new(0) });
  let m = AccountManager::new(store.clone(), vec![p.clone()], log, Arc::new(|_, _, _, _| {}), Arc::new(|_, _| {}));
  let secret = async |id: &str| store.credential(id).await.unwrap().unwrap().secret;
  let labels = || {
    let mut l: Vec<String> = m.list().into_iter().map(|a| a.label).collect();
    l.sort();
    l
  };

  // Moved, signed in, keeping its id; settled after that
  m.sync_local(None).await;
  let home = secret(&me.id).await;
  assert!(home != LOCAL_LOGIN && Path::new(&home).join("login").exists());
  m.sync_local(None).await;
  assert_eq!((secret(&me.id).await, labels()), (home.clone(), vec!["me@x.io".to_owned()]));

  // The app signs in as someone else: a new account beside it, never a rename
  *p.local.lock() = Some("other@x.io".to_owned());
  m.sync_local(None).await;
  assert_eq!(labels(), ["me@x.io", "other@x.io"]);
  assert_eq!((m.get(&me.id).unwrap().label, secret(&me.id).await), ("me@x.io".to_owned(), home.clone()));

  // One still on the CLI's store whose identity is gone from it: kept, in a home of its own, signed out
  let stale = store.add("antigravity", on_cli("stale@x.io")).await.unwrap();
  *p.local.lock() = Some("third@x.io".to_owned());
  m.reload().await;
  m.sync_local(None).await;
  assert_eq!(labels(), ["me@x.io", "other@x.io", "stale@x.io", "third@x.io"]);
  let stale_home = secret(&stale.id).await;
  assert!(stale_home != LOCAL_LOGIN && Path::new(&stale_home).is_dir() && !Path::new(&stale_home).join("login").exists());

  // Importing on purpose again replaces the copy and drops the one it replaced
  let third = m.list().into_iter().find(|a| a.label == "third@x.io").unwrap();
  let first_copy = secret(&third.id).await;
  let again = m.import("antigravity").await.unwrap().unwrap();
  assert_eq!(again.id, third.id);
  let copy = secret(&third.id).await;
  assert!(copy != first_copy && Path::new(&copy).join("login").exists() && !Path::new(&first_copy).exists());
}

#[tokio::test]
async fn login_terminal_merges_agent_env_with_provider_env_taking_precedence() {
  struct EnvLogin;
  impl AccountProvider for EnvLogin {
    fn agent(&self) -> &str {
      "claude"
    }
    fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
      Box::pin(async { None })
    }
    fn login(&self) -> BoxFuture<Result<LoginFlow>> {
      Box::pin(async {
        Ok(LoginFlow {
          command: "claude-agent-acp".into(),
          args: vec!["--cli".into(), "auth".into(), "login".into()],
          env: [("CLAUDE_CONFIG_DIR".into(), Some("/Users/dev/.acpira/accounts/claude/isolated".into()))].into(),
          collect: Box::new(|_| Box::pin(async { None })),
        })
      })
    }
  }

  let dir = tempfile::tempdir().unwrap();
  let log: LogFn = Arc::new(|_| {});
  let vault = Arc::new(FileVault::new(dir.path().join("secrets.json"), log.clone()));
  let store = Arc::new(AccountStore::new(dir.path().join("accounts.json"), vault, log.clone()));
  store.load().await.unwrap();
  let seen = Arc::new(parking_lot::Mutex::new(None::<BTreeMap<String, Option<String>>>));
  let capture = seen.clone();
  let m = AccountManager::new(
    store,
    vec![Arc::new(EnvLogin)],
    log,
    Arc::new(move |_, _, _, env| {
      *capture.lock() = env;
    }),
    Arc::new(|_, _| {}),
  );
  m.set_agent_env(Arc::new(|agent| {
    assert_eq!(agent, "claude");
    [
      ("HTTPS_PROXY".into(), "http://127.0.0.1:8080".into()),
      ("CUSTOM_FLAG".into(), "1".into()),
      ("CLAUDE_CONFIG_DIR".into(), "/Users/dev/.claude-custom".into()),
    ]
    .into()
  }));

  assert!(m.login("claude", Cancel::new()).await.unwrap().is_none());
  let env = seen.lock().clone().expect("login terminal received env");
  assert_eq!(env.get("HTTPS_PROXY"), Some(&Some("http://127.0.0.1:8080".into())));
  assert_eq!(env.get("CUSTOM_FLAG"), Some(&Some("1".into())));
  assert_eq!(env.get("CLAUDE_CONFIG_DIR"), Some(&Some("/Users/dev/.acpira/accounts/claude/isolated".into())));
}

#[tokio::test]
async fn manual_quota_provider_skips_automatic_reads_and_adopts_cache() {
  use acpira_shared::num::Num;
  use acpira_shared::transcript::QuotaWindow;

  struct ManualQuota {
    reads: Arc<AtomicUsize>,
  }
  impl AccountProvider for ManualQuota {
    fn agent(&self) -> &str {
      "antigravity"
    }
    fn manual_quota(&self) -> bool {
      true
    }
    fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
      Box::pin(async { None })
    }
    fn login(&self) -> BoxFuture<Result<LoginFlow>> {
      Box::pin(async { Err(anyhow!("no login here")) })
    }
    fn quota(&self, _cred: AccountCredential) -> Option<BoxFuture<Result<QuotaRead>>> {
      let reads = self.reads.clone();
      Some(Box::pin(async move {
        let n = reads.fetch_add(1, Ordering::SeqCst) + 1;
        Ok(
          Some(AccountQuota {
            windows: vec![QuotaWindow { id: "5hGemini".into(), remaining: Num(1.0 - n as f64 * 0.1), resets_at: None }],
            on_demand_balance_usd: None,
            fetched_at: now_iso(),
          })
          .into(),
        )
      }))
    }
  }

  let dir = tempfile::tempdir().unwrap();
  let log: LogFn = Arc::new(|_| {});
  let vault = Arc::new(FileVault::new(dir.path().join("secrets.json"), log.clone()));
  let store = Arc::new(AccountStore::new(dir.path().join("accounts.json"), vault, log.clone()));
  store.load().await.unwrap();
  let acc = store
    .add("antigravity", AccountDraft { label: "a@example.com".into(), detail: None, secret: "/tmp/h".into(), meta: None })
    .await
    .unwrap();
  let reads = Arc::new(AtomicUsize::new(0));
  let m1 = AccountManager::new(store.clone(), vec![Arc::new(ManualQuota { reads: reads.clone() })], log.clone(), Arc::new(|_, _, _, _| {}), Arc::new(|_, _| {}));
  assert!(m1.manual_quota("antigravity"));

  // Automatic reads (unforced or forced) never invoke the provider
  m1.refresh_quota(&acc.id, false).await;
  m1.refresh_quota(&acc.id, true).await;
  m1.refresh_quotas(Some("antigravity"), true).await;
  assert_eq!(reads.load(Ordering::SeqCst), 0);
  assert!(m1.get(&acc.id).unwrap().quota.is_none());

  // Explicit manual refresh reads once, and an immediate repeat stays within the built-in cooldown
  m1.refresh_quotas_manual(Some("antigravity")).await;
  assert_eq!(reads.load(Ordering::SeqCst), 1);
  assert_eq!(m1.get(&acc.id).unwrap().quota.unwrap().windows[0].remaining.0, 0.9);
  m1.refresh_quotas_manual(Some("antigravity")).await;
  assert_eq!(reads.load(Ordering::SeqCst), 1);
  assert_eq!(m1.manual_quota_cooldown_secs("antigravity"), MANUAL_QUOTA_COOLDOWN_SECS);

  // Another window adopts the shared disk cache on an automatic read without calling the provider
  let m2 = AccountManager::new(store, vec![Arc::new(ManualQuota { reads: reads.clone() })], log, Arc::new(|_, _, _, _| {}), Arc::new(|_, _| {}));
  m2.refresh_quotas(Some("antigravity"), false).await;
  assert_eq!(reads.load(Ordering::SeqCst), 1);
  assert_eq!(m2.get(&acc.id).unwrap().quota.unwrap().windows[0].remaining.0, 0.9);
}
