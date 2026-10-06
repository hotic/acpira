//! User categories of the session list: the shared categories.json, filing, the pin exclusivity and other windows

use super::*;

fn categories_of(m: &Mgr) -> Value {
  serde_json::to_value(m.m.categories()).unwrap()
}

fn summary_of(m: &Mgr, id: &str) -> Value {
  m.sessions().into_iter().find(|s| s["id"] == id).unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn categories_persist_file_sessions_of_their_own_project_and_exclude_pinning() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-ui", "name": "  UI 打磨 ", "cwd": "/tmp" })).await;
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-ssh", "name": "SSH", "cwd": "/tmp" })).await;
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-far", "name": "elsewhere", "cwd": "/elsewhere" })).await;
  // A blank rename keeps the name; reordering moves within the project only
  m.handle(json!({ "type": "categoryOp", "op": "rename", "id": "c-ssh", "name": "   " })).await;
  m.handle(json!({ "type": "categoryOp", "op": "reorder", "id": "c-ssh", "before": "c-ui" })).await;
  m.handle(json!({ "type": "categoryOp", "op": "collapse", "id": "c-ui", "collapsed": true })).await;
  m.handle(json!({ "type": "categoryOp", "op": "collapseProject", "cwd": "/elsewhere", "collapsed": true })).await;
  expect_match(
    categories_of(&m),
    json!({
      "categories": [
        { "id": "c-ssh", "name": "SSH", "cwd": "/tmp" },
        { "id": "c-ui", "name": "UI 打磨", "cwd": "/tmp", "collapsed": true },
        { "id": "c-far", "cwd": "/elsewhere" },
      ],
      "collapsedProjects": ["/elsewhere"],
    }),
  );
  // Every page got the list
  assert!(m.events.lock().unwrap().iter().any(|e| e["type"] == "categories"));

  m.new_session(None).await;
  let a = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  until(|| !summary_of(&m, &a).is_null(), 2000).await;
  // Pinned first, then filing unpins it
  m.handle(json!({ "type": "pinSession", "id": a, "pinned": true })).await;
  m.handle(json!({ "type": "setSessionCategory", "id": a, "category": "c-ui" })).await;
  until(|| summary_of(&m, &a)["category"] == "c-ui", 2000).await;
  assert!(summary_of(&m, &a).get("pinned").is_none());
  // Another project's category is refused; pinning takes it out of its category
  m.handle(json!({ "type": "setSessionCategory", "id": a, "category": "c-far" })).await;
  assert_eq!(summary_of(&m, &a)["category"], "c-ui");
  m.handle(json!({ "type": "pinSession", "id": a, "pinned": true })).await;
  until(|| summary_of(&m, &a)["pinned"] == true, 2000).await;
  assert!(summary_of(&m, &a).get("category").is_none());
  m.handle(json!({ "type": "setSessionCategory", "id": a, "category": "c-ssh" })).await;
  until(|| summary_of(&m, &a)["category"] == "c-ssh", 2000).await;

  // A category created for a session files it in the same step
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-new", "name": "", "cwd": "/tmp", "file": a })).await;
  until(|| summary_of(&m, &a)["category"] == "c-new", 2000).await;
  expect_match(&categories_of(&m)["categories"][3], json!({ "id": "c-new", "name": "New category" }));
  m.handle(json!({ "type": "setSessionCategory", "id": a, "category": "c-ssh" })).await;
  until(|| summary_of(&m, &a)["category"] == "c-ssh", 2000).await;

  // "New session in this category"
  m.handle(json!({ "type": "newSession", "agent": "fake", "category": "c-ui" })).await;
  let b = m.active_id().unwrap();
  assert_ne!(a, b);
  m.handle(json!({ "type": "send", "text": "filed from the start" })).await;
  until(|| summary_of(&m, &b)["category"] == "c-ui", 2000).await;
  m.dispose().await;

  // Both the list and the filing survive a restart; a stored record is filed through its file
  let m2 = Mgr::new(dir.path(), Opts::with_agents(Value::Null, "fake"));
  m2.init().await;
  assert_eq!(categories_of(&m2)["categories"].as_array().unwrap().len(), 4);
  expect_match(summary_of(&m2, &a), json!({ "category": "c-ssh" }));
  m2.handle(json!({ "type": "setSessionCategory", "id": a, "category": null })).await;
  assert!(summary_of(&m2, &a).get("category").is_none());
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  assert_eq!(store.load(&a).await.unwrap().category, None);
  assert_eq!(store.load(&b).await.unwrap().category.as_deref(), Some("c-ui"));
  // categories.json is not mistaken for a session record
  assert!(!m2.session_ids().iter().any(|id| id == "categories"));
  m2.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_category_edit_in_one_window_reaches_the_other_on_refresh() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let one = Mgr::new(dir.path(), Opts::fake(&fake));
  let two = Mgr::new(dir.path(), Opts::fake(&fake));
  one.init().await;
  two.init().await;
  one.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-1", "name": "one", "cwd": "/tmp" })).await;
  // Two's edit applies to what is on disk, so one's category is kept
  two.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-2", "name": "two", "cwd": "/tmp" })).await;
  expect_match(categories_of(&two), json!({ "categories": [{ "id": "c-1" }, { "id": "c-2" }] }));
  two.events.lock().unwrap().clear();
  one.events.lock().unwrap().clear();
  one.m.refresh_index().await;
  expect_match(categories_of(&one), json!({ "categories": [{ "id": "c-1" }, { "id": "c-2" }] }));
  assert!(one.events.lock().unwrap().iter().any(|e| e["type"] == "categories"));
  // Nothing changed for two: no push
  two.m.refresh_index().await;
  assert!(!two.events.lock().unwrap().iter().any(|e| e["type"] == "categories"));
  one.handle(json!({ "type": "categoryOp", "op": "delete", "id": "c-1" })).await;
  two.m.refresh_index().await;
  expect_match(categories_of(&two), json!({ "categories": [{ "id": "c-2" }] }));
  assert_eq!(categories_of(&two)["categories"].as_array().unwrap().len(), 1);
  one.dispose().await;
  two.dispose().await;
}

// "New session in this category" files the new session itself: switching back to an older session while the new one is still
// starting (here: replaying remembered choices on a slow agent) leaves the older one where it was
#[tokio::test(flavor = "multi_thread")]
async fn a_switch_while_a_new_session_starts_does_not_file_the_session_switched_to() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::with_agents(fake.setting(json!({ "env": { "FAKE_CONFIG_DELAY_MS": "300" } })), "fake"));
  m.init().await;
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-ui", "name": "UI", "cwd": "/tmp" })).await;
  m.new_session(None).await;
  // A remembered pick makes the next start replay it, one slow request
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let old = m.active_id().unwrap();
  m.handle(json!({ "type": "pinSession", "id": old, "pinned": true })).await;
  until(|| summary_of(&m, &old)["pinned"] == true, 2000).await;

  let opening = m.spawn_handle(json!({ "type": "newSession", "agent": "fake", "category": "c-ui" }));
  until(|| m.active_id().is_some_and(|id| id != old), 5000).await;
  let new = m.active_id().unwrap();
  m.handle(json!({ "type": "selectSession", "id": old })).await;
  opening.await.unwrap();

  assert_eq!(m.active_id().as_deref(), Some(old.as_str()));
  expect_match(summary_of(&m, &old), json!({ "pinned": true }));
  assert!(summary_of(&m, &old).get("category").is_none());
  until(|| summary_of(&m, &new)["category"] == "c-ui", 2000).await;
  m.dispose().await;
}
