//! Saving on shutdown, soft deletion, the index and the warm process

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_turn_is_persisted_before_shutdown_releases_the_session() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  let prompt = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| m.active().is_some_and(|a| last_turn(&a)["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["streaming"] == true))), 5000).await;
  m.dispose().await;
  prompt.await.unwrap();
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  let record = v(store.load(&id).await.unwrap());
  let last = last_turn(&record);
  expect_match(&last, json!({ "stop": "cancelled" }));
  assert!(last["endedAt"].is_number());
  assert!(last["blocks"].as_array().unwrap().iter().any(|b| b["streaming"] == false));
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_is_soft_restore_brings_it_back_and_rename_and_pin_land_in_the_index() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let a = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  m.new_session(None).await;
  let b = m.active_id().unwrap();
  assert_eq!(m.session_ids(), [b.clone(), a.clone()]);
  m.handle(json!({ "type": "renameSession", "id": a, "title": "第一条" })).await;
  m.handle(json!({ "type": "pinSession", "id": a, "pinned": true })).await;
  // The list follows each change on the next flush (16 ms), where the TS manager rebuilt it inside onChange
  until(|| m.sessions()[0]["id"] == a.as_str(), 2000).await;
  expect_match(&m.sessions()[0], json!({ "id": a, "title": "第一条", "pinned": true }));
  // delete the current session b → active switches to a, b's file moved to the trash (out of the live directory, so no other window lists it)
  m.handle(json!({ "type": "deleteSession", "id": b })).await;
  assert_eq!(m.session_ids(), [a.clone()]);
  assert_eq!(m.active_id().as_deref(), Some(a.as_str()));
  assert_eq!(m.active().unwrap()["title"], "第一条");
  assert!(!dir.path().join(format!("{b}.json")).exists());
  assert!(dir.path().join("trash").join(format!("{b}.json")).exists());
  m.handle(json!({ "type": "restoreSession", "id": b })).await;
  assert_eq!(sorted(m.session_ids()), sorted(vec![a.clone(), b.clone()]));
  assert!(dir.path().join(format!("{b}.json")).exists());
  // delete a again, then reopen the manager: only b left in the index
  m.handle(json!({ "type": "deleteSession", "id": a })).await;
  assert_eq!(m.active_id().as_deref(), Some(b.as_str()));
  m.dispose().await;
  assert!(!dir.path().join(format!("{a}.json")).exists());
  assert!(!dir.path().join("trash").join(format!("{a}.json")).exists());
  let m2 = Mgr::new(dir.path(), Opts::with_agents(Value::Null, "fake"));
  m2.init().await;
  assert_eq!(m2.session_ids(), [b]);
  m2.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn new_session_on_an_empty_session_keeps_the_process() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let a = m.active_id().unwrap();
  m.new_session(None).await;
  assert_eq!(m.active_id().as_deref(), Some(a.as_str()));
  assert_eq!(m.session_ids(), [a]);
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_first_session_takes_the_warm_process_started_at_init() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  assert!(m.logs().iter().any(|l| l.contains("reuse warm")), "{:#?}", m.logs());
  assert_eq!(m.active().unwrap()["status"], "ready");
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_session_ignores_path_like_ids() {
  let fake = fake_or_skip!();
  let parent = tempfile::tempdir().unwrap();
  let dir = parent.path().join("sessions");
  let m = Mgr::new(&dir, Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  let marker = parent.path().join("keep");
  std::fs::write(&marker, "x").unwrap();
  m.handle(json!({ "type": "deleteSession", "id": ".." })).await;
  m.handle(json!({ "type": "deleteSession", "id": "/etc/passwd" })).await;
  assert_eq!(m.active_id().as_deref(), Some(id.as_str()));
  assert!(marker.exists());
  m.dispose().await;
}

// An index summary written before acpSessionId existed is patched once and the debounced index write persists it,
// so the next listing (or another window) does not re-read the record
#[tokio::test(flavor = "multi_thread")]
async fn a_stale_index_summary_gains_its_acp_session_id_and_the_index_file_is_updated() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let sessions = dir.path().join("sessions");
  let store = TranscriptStore::new(sessions.clone(), Arc::new(|_: &str| {}), None);
  let (acp_id, record) = foreign_native_session(&fake, &native, &store, "/tmp").await;
  store.flush(Arc::new(record.clone())).await.unwrap();
  // Rewrite the index the way a build without the field left it
  let mut stale = v(record.summary());
  stale.as_object_mut().unwrap().remove("acpSessionId");
  std::fs::write(sessions.join("index.json"), serde_json::to_string(&json!([stale])).unwrap()).unwrap();
  let m = Mgr::new(&sessions, Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })), "fake"));
  m.init().await;
  m.m.list_native_sessions("fake").await.unwrap();
  let index = || -> Option<String> {
    let list: Value = serde_json::from_str(&std::fs::read_to_string(sessions.join("index.json")).ok()?).ok()?;
    list.as_array()?.iter().find(|s| s["id"] == record.id.as_str())?["acpSessionId"].as_str().map(str::to_owned)
  };
  until(|| index().as_deref() == Some(acp_id.as_str()), 5000).await;
  m.dispose().await;
}
