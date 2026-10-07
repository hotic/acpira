//! Forking and exporting sessions

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn forking_copies_the_prefix_records_its_origin_re_homes_blobs_and_hands_over_context() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let src = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi", "attachments": [{ "kind": "image", "mimeType": "image/png", "data": "aGVsbG8=", "name": "a.png" }] })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  assert_eq!(turns_len(m.active()), 4);
  m.handle(json!({ "type": "forkSession", "sessionId": src, "turnIndex": 1 })).await;
  let fork = m.active_id().unwrap();
  assert_ne!(fork, src);
  assert_eq!(turns_len(m.active()), 2);
  assert!(m.active().unwrap()["title"].as_str().unwrap().starts_with("Fork: "));
  assert_eq!(turns_len(m.view_of(&src)), 4);
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  let record = v(store.load(&fork).await.unwrap());
  expect_match(&record, json!({ "historyPending": true, "forkedFrom": { "sessionId": src, "turnIndex": 1 } }));
  let blob = record["turns"][0]["attachments"].as_array().unwrap().iter().find_map(|a| a["blob"].as_str().map(str::to_owned)).expect("copied blob");
  assert!(dir.path().join(&fork).join(&blob).exists());
  m.handle(json!({ "type": "send", "text": "again" })).await;
  assert!(last_turn(&m.active().unwrap())["blocks"].to_string().contains("resource:acpira://history/"));
  m.m.refresh_index().await;
  let t0 = std::time::Instant::now();
  while store.load(&fork).await.is_some_and(|r| r.history_pending) {
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  }
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forking_re_homes_the_blobs_of_a_steered_prompt_and_hands_them_over() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::with_agents(fake.setting(json!({ "env": { "FAKE_STEERING": "1" } })), "fake"));
  m.init().await;
  m.new_session(None).await;
  let src = m.active_id().unwrap();
  let running = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  let t0 = std::time::Instant::now();
  while turns_len(m.active()) < 2 {
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
  }
  m.handle(json!({ "type": "send", "text": "steer me", "attachments": [{ "kind": "image", "mimeType": "image/png", "data": "aGVsbG8=", "name": "s.png" }] })).await;
  let id = m.active().unwrap()["queued"][0]["id"].as_str().unwrap().to_owned();
  m.handle(json!({ "type": "steerQueued", "sessionId": src, "id": id })).await;
  running.await.unwrap();
  m.handle(json!({ "type": "forkSession", "sessionId": src, "turnIndex": 1 })).await;
  let fork = m.active_id().unwrap();
  assert_ne!(fork, src);
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  let record = v(store.load(&fork).await.unwrap());
  let steer = record["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "steer").cloned().expect("steer block");
  let blob = steer["attachments"][0]["blob"].as_str().expect("copied blob").to_owned();
  assert!(dir.path().join(&fork).join(&blob).exists());
  // The image reaches the fork's first prompt next to the history (the fake echoes non-text blocks)
  m.handle(json!({ "type": "send", "text": "again" })).await;
  let reply = last_turn(&m.active().unwrap())["blocks"].to_string();
  assert!(reply.contains("resource:acpira://history/") && reply.contains("image"), "{reply}");
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn forking_a_non_agent_turn_or_the_running_last_turn_is_refused() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let src = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  m.handle(json!({ "type": "forkSession", "sessionId": src, "turnIndex": 0 })).await;
  assert!(m.toasts().contains(&"That reply is no longer there to fork from.".to_owned()), "{:?}", m.toasts());
  assert_eq!(m.active_id().as_deref(), Some(src.as_str()));
  let sending = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| m.active().is_some_and(|a| a["running"] == true), 5000).await;
  let last = turns_len(m.active()) - 1;
  m.handle(json!({ "type": "forkSession", "sessionId": src, "turnIndex": last })).await;
  assert!(m.toasts().contains(&"Wait for this reply to finish before forking from it.".to_owned()), "{:?}", m.toasts());
  assert_eq!(m.active_id().as_deref(), Some(src.as_str()));
  m.handle(json!({ "type": "stop" })).await;
  sending.await.unwrap();
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_session_exports_as_markdown_and_json_under_the_sibling_exports_dir() {
  use acpira_shared::protocol::ExportFormat;
  let fake = fake_or_skip!();
  let parent = tempfile::tempdir().unwrap();
  let dir = parent.path().join("sessions");
  let m = Mgr::new(&dir, Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let title = m.active().unwrap()["title"].as_str().unwrap().to_owned();
  let md_path = m.m.export_session(&id, ExportFormat::Markdown).await.unwrap();
  // writeExport returns the realpath'd target in its ordinary spelling (no `\\?\` on Windows), so resolve the expectation the same way
  let exports = acpira_host::platform::paths::for_cli(parent.path().join("exports").canonicalize().unwrap());
  assert!(md_path.starts_with(&exports), "{}", md_path.display());
  assert!(!md_path.to_string_lossy().starts_with(r"\\?\"), "{}", md_path.display());
  let md = std::fs::read_to_string(&md_path).unwrap();
  assert!(md.contains(&format!("# {title}")) && md.contains("hello world"));
  // An unchanged transcript hands back the same file instead of writing another one
  assert_eq!(m.m.export_session(&id, ExportFormat::Markdown).await.unwrap(), md_path);
  let md_files = || std::fs::read_dir(parent.path().join("exports")).unwrap().filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "md")).count();
  assert_eq!(md_files(), 1);
  // A deleted export is written again
  std::fs::remove_file(&md_path).unwrap();
  let rewritten = m.m.export_session(&id, ExportFormat::Markdown).await.unwrap();
  assert!(rewritten.exists());
  // A new turn changes the transcript, so the next export carries it
  m.handle(json!({ "type": "send", "text": "second-question" })).await;
  let after = m.m.export_session(&id, ExportFormat::Markdown).await.unwrap();
  assert!(std::fs::read_to_string(&after).unwrap().contains("second-question"));
  let json_path = m.m.export_session(&id, ExportFormat::Json).await.unwrap();
  assert_eq!(serde_json::from_str::<Value>(&std::fs::read_to_string(&json_path).unwrap()).unwrap()["id"], id.as_str());
  assert!(m.m.export_session("missing-id", ExportFormat::Json).await.is_err());
  m.dispose().await;
}
