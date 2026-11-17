use super::*;
use serde_json::json;

fn write_plugin(root: &Path, id: &str, script: &str) {
  let dir = root.join(id);
  std::fs::create_dir_all(&dir).unwrap();
  let m = json!({ "schema": 1, "id": id, "version": "0.1.0", "description": "x",
    "contributes": { "tools": [{ "name": "go", "description": "Go", "command": ["sh", "go.sh"] }] } });
  std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  std::fs::write(dir.join("go.sh"), script).unwrap();
}

fn any(_: &str) -> bool {
  true
}

#[tokio::test]
async fn a_commit_refuses_a_ring_another_engine_closed_and_a_copy_cleaned_meanwhile() {
  let tmp = tempfile::tempdir().unwrap();
  for (id, dep, optional) in [("a", "b", false), ("b", "a", true)] {
    write_plugin(tmp.path(), id, "echo");
    let path = tmp.path().join(id).join("plugin.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    m[if optional { "optionalDependencies" } else { "dependencies" }] = json!({ dep: "*" });
    std::fs::write(&path, m.to_string()).unwrap();
  }
  // Two engines, each with its own view: the second commit still sees the first through the shared state
  let (one, two) = (PluginStore::new(tmp.path().to_path_buf()), PluginStore::new(tmp.path().to_path_buf()));
  let (a, b) = (one.freeze("a", &any).unwrap(), two.freeze("b", &any).unwrap());
  one.enable_as("a", &a.hash, Provenance::user(), false).await.unwrap();
  let err = two.enable_as("b", &b.hash, Provenance::user(), false).await.unwrap_err().to_string();
  assert!(err.contains("ring: b -> a -> b"), "{err}");
  // A version the storage page removed after the freeze is refused at the commit, never enabled half there
  make_removable(&b.dir).unwrap();
  std::fs::remove_dir_all(&b.dir).unwrap();
  std::fs::remove_dir_all(tmp.path().join("a")).unwrap();
  one.remove("a", Provenance::user()).await.unwrap();
  assert!(two.enable_as("b", &b.hash, Provenance::user(), false).await.unwrap_err().to_string().starts_with(GONE));
}

#[tokio::test]
async fn freezing_is_content_addressed_and_ignores_run_leftovers() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  write_plugin(tmp.path(), "demo", "echo 1");
  let a = store.freeze("demo", &any).unwrap();
  assert!(a.findings.ok(), "{:?}", a.findings);
  assert_eq!(a.files, ["go.sh", "plugin.json"]);
  // Caches and version control are not part of the plugin
  std::fs::create_dir_all(tmp.path().join("demo/__pycache__")).unwrap();
  std::fs::write(tmp.path().join("demo/__pycache__/x.pyc"), "x").unwrap();
  std::fs::create_dir_all(tmp.path().join("demo/.git")).unwrap();
  assert_eq!(store.freeze("demo", &any).unwrap().hash, a.hash);
  // The copy is read-only and loads; a run leaving a cache in it does not spoil it
  assert!(std::fs::metadata(a.dir.join("go.sh")).unwrap().permissions().readonly());
  assert!(store.open("demo", &a.hash).is_ok());
  // An edit is a new version
  std::fs::write(tmp.path().join("demo/go.sh"), "echo 2").unwrap();
  let b = store.freeze("demo", &any).unwrap();
  assert_ne!(a.hash, b.hash);
  assert_eq!(store.source_changed("demo", &b.hash), Some(false));
  assert_eq!(store.source_changed("demo", &a.hash), Some(true));
}

#[tokio::test]
async fn a_changed_copy_is_refused_and_frozen_again() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  write_plugin(tmp.path(), "demo", "echo 1");
  let a = store.freeze("demo", &any).unwrap();
  make_writable(&a.dir).unwrap();
  std::fs::write(a.dir.join("go.sh"), "rm -rf /").unwrap();
  assert!(format!("{:#}", store.open("demo", &a.hash).unwrap_err()).contains("changed after it was frozen"));
  let again = store.freeze("demo", &any).unwrap();
  assert_eq!(again.hash, a.hash);
  assert!(store.open("demo", &a.hash).is_ok());
}

#[tokio::test]
async fn commits_count_generations_log_each_change_and_roll_back() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  write_plugin(tmp.path(), "demo", "echo 1");
  let a = store.freeze("demo", &any).unwrap().hash;
  store.enable("demo", &a, Provenance::user()).await.unwrap().unwrap();
  // The same version again is no change
  assert!(store.enable("demo", &a, Provenance::user()).await.unwrap().is_none());
  std::fs::write(tmp.path().join("demo/go.sh"), "echo 2").unwrap();
  let b = store.freeze("demo", &any).unwrap().hash;
  let by_session = Provenance { by: "session".into(), session: Some("s1".into()), agent: Some("claude".into()), model: None };
  store.enable("demo", &b, by_session.clone()).await.unwrap();
  let s = store.load().unwrap();
  assert_eq!((s.generation, s.plugins["demo"].snapshot.as_str(), s.plugins["demo"].previous.as_slice()), (2, b.as_str(), std::slice::from_ref(&a)));
  assert_eq!(s.plugins["demo"].provenance, by_session);
  assert!(tmp.path().join(".data/demo").is_dir());
  store.rollback("demo", Provenance::user()).await.unwrap();
  assert_eq!(store.load().unwrap().plugins["demo"].snapshot, a);
  assert!(store.rollback("demo", Provenance::user()).await.is_err());
  store.disable("demo", Provenance::user()).await.unwrap();
  let actions: Vec<_> = store.changes(10).into_iter().map(|c| (c.generation, c.action)).collect();
  assert_eq!(actions, [(1, "enable".into()), (2, "enable".into()), (3, "rollback".into()), (4, "disable".into())]);
}

#[tokio::test]
async fn a_crash_between_the_two_writes_is_repaired() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  store.set_safe_mode(true, Provenance::cli()).await.unwrap();
  // The log line of the last commit was lost
  std::fs::write(tmp.path().join(CHANGES), "").unwrap();
  store.reconcile().await.unwrap();
  assert_eq!(store.changes(5).iter().map(|c| c.generation).collect::<Vec<_>>(), [1]);
  std::fs::write(tmp.path().join(CHANGES), "").unwrap();
  // A later commit appends it first
  store.set_tool_group("summon", false, Provenance::user()).await.unwrap();
  assert_eq!(store.changes(5).iter().map(|c| (c.generation, c.action.as_str())).collect::<Vec<_>>(), [(1, "safeMode"), (2, "toolGroup")]);
  let s = store.load().unwrap();
  assert!(s.safe_mode && !s.tool_group_on("summon") && s.tool_group_on("images"));
}

#[tokio::test]
async fn two_engines_committing_at_once_lose_nothing() {
  let tmp = tempfile::tempdir().unwrap();
  let a = std::sync::Arc::new(PluginStore::new(tmp.path().to_path_buf()));
  let b = std::sync::Arc::new(PluginStore::new(tmp.path().to_path_buf()));
  let mut tasks = vec![];
  for i in 0..8 {
    let store = if i % 2 == 0 { a.clone() } else { b.clone() };
    tasks.push(tokio::spawn(async move { store.set_tool_group(&format!("g{i}"), false, Provenance::user()).await.unwrap() }));
  }
  for t in tasks {
    t.await.unwrap();
  }
  let s = a.load().unwrap();
  assert_eq!((s.generation, s.tool_groups.len()), (8, 8));
  assert_eq!(a.changes(20).len(), 8);
}

#[tokio::test]
async fn unknown_fields_survive_and_a_broken_state_is_never_overwritten() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  std::fs::write(store.state_path(), json!({ "schema": 1, "generation": 4, "future": { "x": 1 }, "plugins": { "p": { "enabled": false, "snapshot": "h", "later": true } } }).to_string()).unwrap();
  store.set_safe_mode(true, Provenance::user()).await.unwrap();
  let v: Value = serde_json::from_slice(&std::fs::read(store.state_path()).unwrap()).unwrap();
  assert_eq!((v["future"]["x"].as_i64(), v["plugins"]["p"]["later"].as_bool(), v["generation"].as_u64()), (Some(1), Some(true), Some(5)));
  std::fs::write(store.state_path(), "{ half").unwrap();
  assert!(store.set_safe_mode(false, Provenance::user()).await.is_err());
  assert_eq!(std::fs::read_to_string(store.state_path()).unwrap(), "{ half");
}

#[tokio::test]
async fn the_storage_page_may_clean_what_nothing_in_effect_needs() {
  let tmp = tempfile::tempdir().unwrap();
  let root = tmp.path();
  let store = PluginStore::new(root.to_path_buf());
  // `keep`: a, then b in effect (a is its earlier version), and c frozen but never enabled
  write_plugin(root, "keep", "echo a");
  let a = store.freeze("keep", &any).unwrap().hash;
  store.enable("keep", &a, Provenance::user()).await.unwrap();
  write_plugin(root, "keep", "echo b");
  let b = store.freeze("keep", &any).unwrap().hash;
  store.enable("keep", &b, Provenance::user()).await.unwrap();
  write_plugin(root, "keep", "echo c");
  let c = store.freeze("keep", &any).unwrap();
  // `gone`: enabled, wrote data, then uninstalled
  write_plugin(root, "gone", "echo g");
  let g = store.freeze("gone", &any).unwrap().hash;
  store.enable("gone", &g, Provenance::user()).await.unwrap();
  std::fs::write(root.join(DATA).join("gone.kv.json"), "{}").unwrap();
  std::fs::write(root.join(DATA).join("keep.kv.json"), "{}").unwrap();
  store.remove("gone", Provenance::user()).await.unwrap();
  let removed = std::fs::read_dir(root.join(REMOVED)).unwrap().next().unwrap().unwrap().path();

  let found = leftovers(root, &|_| false);
  assert_eq!(
    found,
    [
      Leftover { id: "gone".into(), data: false, targets: vec![removed, root.join(VERSIONS).join("gone")] },
      Leftover { id: "keep".into(), data: false, targets: vec![c.dir.clone()] },
      Leftover { id: "gone".into(), data: true, targets: vec![root.join(DATA).join("gone"), root.join(DATA).join("gone.kv.json")] },
    ]
  );
  // A version frozen moments ago may be a card still waiting
  assert!(leftovers(root, &|_| true).iter().all(|l| !l.targets.contains(&c.dir)));
  // Read-only copies delete once their folders are writable; the versions in effect stay read-only
  for t in found.iter().filter(|l| !l.data).flat_map(|l| l.targets.iter()) {
    make_removable(t).unwrap();
    std::fs::remove_dir_all(t).unwrap();
  }
  assert!(std::fs::metadata(store.snapshot_dir("keep", &b).join("go.sh")).unwrap().permissions().readonly());
  assert!(store.open("keep", &a).is_ok() && store.open("keep", &b).is_ok());
}

#[tokio::test]
async fn proposals_wait_and_removal_moves_the_source_aside() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  write_plugin(tmp.path(), "demo", "echo 1");
  let h = store.freeze("demo", &any).unwrap().hash;
  store.propose("demo", &h, Provenance::cli()).await.unwrap().unwrap();
  assert!(store.propose("demo", &h, Provenance::cli()).await.unwrap().is_none());
  let s = store.load().unwrap();
  assert!(s.proposals.contains_key("demo") && !s.plugins.contains_key("demo"));
  store.enable("demo", &h, Provenance::user()).await.unwrap();
  assert!(store.load().unwrap().proposals.is_empty());
  store.remove("demo", Provenance::user()).await.unwrap();
  assert!(!tmp.path().join("demo").exists() && store.load().unwrap().plugins.is_empty());
  assert_eq!(std::fs::read_dir(tmp.path().join(REMOVED)).unwrap().count(), 1);
  assert_eq!(store.sources(), Vec::<String>::new());
}

#[test]
fn links_and_huge_folders_are_not_frozen() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  write_plugin(tmp.path(), "demo", "echo 1");
  #[cfg(unix)]
  {
    std::os::unix::fs::symlink("/etc/hosts", tmp.path().join("demo/hosts")).unwrap();
    assert!(store.freeze("demo", &any).unwrap_err().to_string().contains("symbolic link"));
    std::fs::remove_file(tmp.path().join("demo/hosts")).unwrap();
  }
  for i in 0..=MAX_FILES {
    std::fs::write(tmp.path().join(format!("demo/f{i}")), "").unwrap();
  }
  assert!(store.freeze("demo", &any).unwrap_err().to_string().contains("more than"));
  assert!(store.freeze("Nope", &any).is_err());
}

#[test]
fn node_modules_freezes_with_the_plugin_and_versions_share_its_files() {
  let tmp = tempfile::tempdir().unwrap();
  let store = PluginStore::new(tmp.path().to_path_buf());
  write_plugin(tmp.path(), "demo", "echo 1");
  let pkg = tmp.path().join("demo/node_modules/left-pad");
  std::fs::create_dir_all(&pkg).unwrap();
  std::fs::write(pkg.join("index.js"), "module.exports = 1;").unwrap();
  std::fs::write(tmp.path().join("demo/package.json"), r#"{ "dependencies": { "left-pad": "1" } }"#).unwrap();
  std::fs::write(tmp.path().join("demo/package-lock.json"), "{}").unwrap();
  #[cfg(unix)]
  {
    // npm's links to package binaries are left out; any other link in node_modules says how to install instead
    std::fs::create_dir_all(tmp.path().join("demo/node_modules/.bin")).unwrap();
    std::os::unix::fs::symlink("../left-pad/index.js", tmp.path().join("demo/node_modules/.bin/left-pad")).unwrap();
  }
  let a = store.freeze("demo", &any).unwrap();
  assert!(a.findings.ok(), "{:?}", a.findings);
  assert!(a.files.iter().any(|f| f == "node_modules/left-pad/index.js"), "{:?}", a.files);
  assert!(!a.files.iter().any(|f| f.contains(".bin")));
  // A new version shares the package's bytes with the first
  std::fs::write(tmp.path().join("demo/go.sh"), "echo 2").unwrap();
  let b = store.freeze("demo", &any).unwrap();
  assert_ne!(a.hash, b.hash);
  #[cfg(unix)]
  {
    use std::os::unix::fs::MetadataExt;
    let ia = std::fs::metadata(a.dir.join("node_modules/left-pad/index.js")).unwrap().ino();
    let ib = std::fs::metadata(b.dir.join("node_modules/left-pad/index.js")).unwrap().ino();
    assert_eq!(ia, ib);
    std::os::unix::fs::symlink("index.js", pkg.join("main.js")).unwrap();
    assert!(store.freeze("demo", &any).unwrap_err().to_string().contains("node-linker"));
  }
  // Without a lockfile the check refuses it
  #[cfg(unix)]
  std::fs::remove_file(pkg.join("main.js")).unwrap();
  std::fs::remove_file(tmp.path().join("demo/package-lock.json")).unwrap();
  let c = store.freeze("demo", &any).unwrap();
  assert!(c.findings.text().contains("lockfile"), "{:?}", c.findings);
}
