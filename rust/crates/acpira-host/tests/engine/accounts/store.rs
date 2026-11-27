//! The account file and the secret vault: metadata versus secrets, several hosts on one file, corrupt and missing vaults

use super::*;

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
async fn a_corrupt_vault_is_logged_and_set_aside_on_the_next_store() {
  let dir = tempfile::tempdir().unwrap();
  let file = dir.path().join("secrets.json");
  std::fs::write(&file, "{ nope").unwrap();
  let logs = Arc::new(Mutex::new(Vec::<String>::new()));
  let l = logs.clone();
  let vault = FileVault::new(file.clone(), Arc::new(move |x: &str| l.lock().unwrap().push(x.to_owned())));
  vault.store("k", "v").await.unwrap();
  // The corrupt bytes survive beside the file, never overwritten in place, and the vault starts a clean table
  let aside: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok())
    .filter(|e| e.file_name().to_string_lossy().starts_with("secrets.json.corrupt-")).collect();
  assert_eq!(aside.len(), 1);
  assert_eq!(std::fs::read_to_string(aside[0].path()).unwrap(), "{ nope");
  assert_eq!(vault.get("k").await.unwrap().as_deref(), Some("v"));
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
