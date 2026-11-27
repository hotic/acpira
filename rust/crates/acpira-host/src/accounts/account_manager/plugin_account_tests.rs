use super::*;
use crate::accounts::account_store::{AccountDraft, FileVault};
use crate::accounts::provider::LoginFlow;
use serde_json::json;

struct Saved;
impl AccountProvider for Saved {
  fn agent(&self) -> &str {
    "codex"
  }
  fn import_local(&self) -> BoxFuture<Option<AccountDraft>> {
    Box::pin(async { None })
  }
  fn login(&self) -> BoxFuture<Result<LoginFlow>> {
    Box::pin(async { Err(anyhow!("no login here")) })
  }
}

async fn manager(dir: &std::path::Path) -> Arc<AccountManager> {
  let log: LogFn = Arc::new(|_| {});
  let vault = Arc::new(FileVault::new(dir.join("secrets.json"), log.clone()));
  let store = Arc::new(AccountStore::new(dir.join("accounts.json"), vault, log.clone()));
  store.load().await.unwrap();
  AccountManager::new(store, vec![Arc::new(Saved)], log, Arc::new(|_, _, _, _| {}), Arc::new(|_, _| {}))
}

fn rented(env: &str) -> serde_json::Value {
  json!({ "id": "r1", "agent": "codex", "label": "team", "badge": "Rented", "env": { "CODEX_HOME": env } })
}

#[tokio::test]
async fn a_plugin_lists_starts_and_lets_go_of_its_accounts() {
  let dir = tempfile::tempdir().unwrap();
  let m = manager(dir.path()).await;
  let lists = Arc::new(parking_lot::Mutex::new(0));
  let rotated = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
  let (l, r) = (lists.clone(), rotated.clone());
  m.subscribe(Arc::new(move |_| *l.lock() += 1));
  m.on_rotated(Arc::new(move |changed| r.lock().extend(changed.into_iter().map(|(_, id)| id))));

  let id = "plugin:demo:r1";
  // Not listed yet: nothing may start on it
  assert!(m.unavailable("codex", id).is_some());
  assert_eq!(m.set_plugin_accounts("demo", "Demo", &[rented("/a")], false), Ok(1));
  assert!(m.set_plugin_accounts("demo", "Demo", &[json!({ "id": "x", "agent": "grok", "label": "y" })], false).is_err());
  assert_eq!((m.unavailable("codex", id), m.unavailable("claude", id).is_some()), (None, true));
  assert_eq!(m.unavailable("codex", "e25b0f67-07b2-4d1f-aeba-4c4f255c29a2"), None);
  let listed = m.list().into_iter().find(|a| a.id == id).unwrap();
  assert_eq!((listed.badge.as_deref(), listed.plugin.as_deref(), listed.removable), (Some("Rented"), Some("Demo"), Some(false)));
  assert_eq!(m.spawn_env_for("codex", id).await.unwrap().get("CODEX_HOME").map(String::as_str), Some("/a"));
  assert_eq!(m.spawn_env_for("claude", id).await, None);
  assert!(m.default_for("codex").is_none());
  m.touch(id).await.unwrap();
  assert_eq!(m.default_for("codex").map(|a| a.id).as_deref(), Some(id));
  // The same list again tells nobody; a renewed credential tells the sessions, not the pages
  assert_eq!((*lists.lock(), rotated.lock().len()), (1, 0));
  m.set_plugin_accounts("demo", "Demo", &[rented("/a")], false).unwrap();
  assert_eq!((*lists.lock(), rotated.lock().len()), (1, 0));
  m.set_plugin_accounts("demo", "Demo", &[rented("/b")], false).unwrap();
  assert_eq!((*lists.lock(), rotated.lock().as_slice()), (1, [id.to_owned()].as_slice()));
  // Not removable without the plugin's say; a plugin leaving takes its accounts along
  assert!(m.remove(id).await.is_err());
  m.retain_plugin_accounts(&|p| p != "demo");
  assert!(m.get(id).is_none());
  assert!(m.unavailable("codex", id).is_some());
  assert_eq!((*lists.lock(), rotated.lock().len()), (2, 2));
}

#[tokio::test]
async fn the_same_quota_read_again_tells_nobody() {
  let dir = tempfile::tempdir().unwrap();
  let m = manager(dir.path()).await;
  let lists = Arc::new(parking_lot::Mutex::new(0));
  let l = lists.clone();
  m.subscribe(Arc::new(move |_| *l.lock() += 1));
  let with = |remaining: f64| {
    let mut a = rented("/a");
    a["quota"] = json!({ "windows": [{ "id": "weekly", "remaining": remaining }] });
    a
  };
  m.set_plugin_accounts("demo", "Demo", &[with(0.5)], false).unwrap();
  std::thread::sleep(Duration::from_millis(5));
  m.set_plugin_accounts("demo", "Demo", &[with(0.5)], false).unwrap();
  assert_eq!(*lists.lock(), 1);
  m.set_plugin_accounts("demo", "Demo", &[with(0.4)], false).unwrap();
  assert_eq!(*lists.lock(), 2);
}

#[tokio::test]
async fn a_removed_account_hands_its_sessions_to_the_strategy_pick() {
  let dir = tempfile::tempdir().unwrap();
  let m = manager(dir.path()).await;
  let draft = |label: &str| AccountDraft { label: label.into(), detail: None, secret: "s".into(), meta: None };
  let a = m.store.add("codex", draft("a@x.io")).await.unwrap();
  let b = m.store.add("codex", draft("b@x.io")).await.unwrap();
  let c = m.store.add("codex", draft("c@x.io")).await.unwrap();
  // Still saved, still listed by its plugin, or an agent off the account layer: the session keeps its binding
  assert!(m.replacement("codex", &a.id).await.is_none());
  m.set_plugin_accounts("demo", "Demo", &[rented("/a")], false).unwrap();
  assert!(m.replacement("codex", "plugin:demo:r1").await.is_none());
  m.retain_plugin_accounts(&|_| false);
  assert!(m.replacement("grok", &a.id).await.is_none());
  m.remove(&a.id).await.unwrap();
  m.set_switch_policy(Arc::new(|_: &str| SwitchStrategy::ListOrder));
  assert_eq!(m.replacement("codex", &a.id).await.map(|x| x.id), Some(b.id.clone()));
  // Strategy off: the default a new session would take
  m.set_switch_policy(Arc::new(|_: &str| SwitchStrategy::Off));
  m.touch(&c.id).await.unwrap();
  assert_eq!(m.replacement("codex", &a.id).await.map(|x| x.id), Some(c.id.clone()));
  // Removed by another host: this one's cached list still has it until the re-read
  let other = AccountStore::new(dir.path().join("accounts.json"), m.store.vault().clone(), Arc::new(|_| {}));
  other.load().await.unwrap();
  other.remove(&c.id).await.unwrap();
  assert_eq!(m.replacement("codex", &c.id).await.map(|x| x.id), Some(b.id.clone()));
  // Nothing left to move to
  m.remove(&b.id).await.unwrap();
  assert!(m.replacement("codex", &b.id).await.is_none());
}

#[tokio::test]
async fn a_vanished_plugin_account_moves_within_its_plugin_then_to_saved_accounts() {
  let dir = tempfile::tempdir().unwrap();
  let m = manager(dir.path()).await;
  let draft = |label: &str| AccountDraft { label: label.into(), detail: None, secret: "s".into(), meta: None };
  let own = m.store.add("codex", draft("own@x.io")).await.unwrap();
  let r2 = json!({ "id": "r2", "agent": "codex", "label": "team 2", "badge": "Rented", "env": { "CODEX_HOME": "/b" } });
  // Another account from the same plugin wins over the user's saved account
  m.set_plugin_accounts("demo", "Demo", &[r2], false).unwrap();
  assert_eq!(m.replacement("codex", "plugin:demo:r1").await.map(|a| a.id), Some("plugin:demo:r2".to_owned()));
  // Once the plugin has none left (deleted or unloaded), move to the user's saved account
  m.retain_plugin_accounts(&|_| false);
  assert_eq!(m.replacement("codex", "plugin:demo:r1").await.map(|a| a.id), Some(own.id.clone()));
  // A plugin still starting at engine boot gets a chance to list the account before replacement moves off it
  let m_wait = m.clone();
  m.set_plugin_waiter(Arc::new(move |plugin: String| {
    let m = m_wait.clone();
    Box::pin(async move {
      if plugin == "demo" {
        m.set_plugin_accounts("demo", "Demo", &[rented("/a")], false).unwrap();
      }
    })
  }));
  assert!(m.replacement("codex", "plugin:demo:r1").await.is_none());
  m.retain_plugin_accounts(&|_| false);
  m.set_plugin_waiter(Arc::new(|_| Box::pin(async {})));
  // With no saved or plugin accounts left at all, stop rather than falling back to the CLI's unmanaged login
  m.remove(&own.id).await.unwrap();
  assert!(m.replacement("codex", "plugin:demo:r1").await.is_none());
}

#[tokio::test]
async fn a_stored_account_follows_to_the_plugin_that_lists_it_now() {
  let dir = tempfile::tempdir().unwrap();
  let m = manager(dir.path()).await;
  let kept = m.store.add("codex", AccountDraft { label: "team".into(), detail: None, secret: "s".into(), meta: None }).await.unwrap();
  let mut listed = rented("/a");
  listed["id"] = json!(kept.id);
  m.set_plugin_accounts("demo", "Demo", &[listed], false).unwrap();
  m.remove(&kept.id).await.unwrap();
  assert_eq!(m.replacement("codex", &kept.id).await.map(|a| a.id), Some(format!("plugin:demo:{}", kept.id)));
}

#[tokio::test]
async fn removal_goes_through_the_plugin() {
  let dir = tempfile::tempdir().unwrap();
  let m = manager(dir.path()).await;
  let asked = Arc::new(parking_lot::Mutex::new(None));
  let a = asked.clone();
  m.set_plugin_remover(Arc::new(move |plugin, id, agent| {
    *a.lock() = Some((plugin, id, agent));
    Box::pin(async { Ok(()) })
  }));
  m.set_plugin_accounts("demo", "Demo", &[rented("/a")], true).unwrap();
  assert_eq!(m.get("plugin:demo:r1").unwrap().removable, None);
  m.remove("plugin:demo:r1").await.unwrap();
  assert_eq!(asked.lock().clone(), Some(("demo".into(), "r1".into(), "codex".into())));
  assert!(m.get("plugin:demo:r1").is_none());
}
