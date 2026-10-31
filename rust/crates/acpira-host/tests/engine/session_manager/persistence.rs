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
async fn a_corrupt_record_stays_listed_and_reports_a_diagnostic_error() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let first = Mgr::new(dir.path(), Opts::fake(&fake));
  first.init().await;
  first.new_session(None).await;
  let id = first.active_id().unwrap();
  first.handle(json!({ "type": "send", "text": "hi" })).await;
  first.dispose().await;

  let second = Mgr::new(dir.path(), Opts::fake(&fake));
  second.init().await;
  assert!(second.session_ids().contains(&id));
  std::fs::write(dir.path().join(format!("{id}.json")), "{ broken").unwrap();
  second.m.select_session_for(&second.v, &id).await;
  assert!(second.session_ids().contains(&id));
  assert!(second.toasts().iter().any(|text| text.contains("SESSION_RECORD_CORRUPT")), "{:?}", second.toasts());
  second.dispose().await;
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

// Deleting a session ends its agent process right away (session/close, then the kill), the one on screen and a
// background one alike, without waiting for the undo window or the idle release; undo reopens it through the ordinary
// restore
#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_session_closes_its_agent_process_at_once() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let close_log = dir.path().join("close.log");
  let env = json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_CLOSE_LOG": close_log } });
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(fake.setting(env), "fake"));
  m.init().await;
  let acp_id = |id: &str| m.sessions().iter().find(|s| s["id"] == id).unwrap()["acpSessionId"].as_str().unwrap().to_owned();
  let closed = |acp: &str| std::fs::read_to_string(&close_log).unwrap_or_default().lines().any(|l| l == acp);
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let a = m.active_id().unwrap();
  let a_acp = acp_id(&a);
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hello" })).await;
  let b = m.active_id().unwrap();
  let b_acp = acp_id(&b);
  // A background session: gone from the live map, its native session closed and its lease returned
  m.handle(json!({ "type": "deleteSession", "id": a })).await;
  assert!(m.view_of(&a).is_none());
  until(|| closed(&a_acp), 5000).await;
  // The finished turn's pin goes from its post-save task, which may still be in flight when the process is gone
  until(|| !m.m.leased().contains(&a), 5000).await;
  // The one on screen: the viewer moves on and the process still goes
  m.handle(json!({ "type": "deleteSession", "id": b })).await;
  assert_ne!(m.active_id().as_deref(), Some(b.as_str()));
  until(|| closed(&b_acp), 5000).await;
  until(|| !m.m.leased().contains(&b), 5000).await;
  // Undo brings the entry back; opening it resumes the same native session
  m.handle(json!({ "type": "restoreSession", "id": a })).await;
  m.m.select_session_for(&m.v, &a).await;
  assert_eq!(m.active().unwrap()["status"], "ready", "{:#?}", m.logs());
  assert_eq!(acp_id(&a), a_acp);
  assert_eq!(turns_len(m.active()), 2);
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
  m.m.list_native_sessions("fake", m.m.folder_for(None, None)).await.unwrap();
  let index = || -> Option<String> {
    let list: Value = serde_json::from_str(&std::fs::read_to_string(sessions.join("index.json")).ok()?).ok()?;
    list.as_array()?.iter().find(|s| s["id"] == record.id.as_str())?["acpSessionId"].as_str().map(str::to_owned)
  };
  until(|| index().as_deref() == Some(acp_id.as_str()), 5000).await;
  m.dispose().await;
}

// A host folder's session keeps one writer across machines (`remote/host_store.rs`): the copy another machine left in
// this store opens read-only, saying where it runs, and nothing done here reaches its record; a copy this machine
// made itself (its record here was lost) opens as its own
#[tokio::test(flavor = "multi_thread")]
async fn a_copy_another_machine_left_here_is_read_only() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let at = "2026-10-01T00:00:00.000Z";
  let record = |id: &str, machine: &str| {
    serde_json::to_vec(&json!({ "id": id, "agent": "fake", "cwd": "/work/app", "title": id, "createdAt": at, "updatedAt": at,
      "turns": [], "controls": { "modes": [], "options": [] }, "commands": [], "copyOf": { "machine": machine, "name": "Studio" } }))
    .unwrap()
  };
  let (me, _) = acpira_host::remote::host_store::this_machine();
  std::fs::write(dir.path().join("s-far.json"), record("s-far", "machine-elsewhere")).unwrap();
  std::fs::write(dir.path().join("s-own.json"), record("s-own", me)).unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;

  // A list deletion must inspect copyOf before the record has ever been opened (and entered the mirrors/copies maps).
  m.handle(json!({ "type": "deleteSession", "id": "s-far" })).await;
  assert_eq!(std::fs::read(dir.path().join("s-far.json")).unwrap(), record("s-far", "machine-elsewhere"));
  assert!(m.session_ids().contains(&"s-far".to_owned()));
  assert!(!dir.path().join("trash/s-far.json").exists());
  m.handle(json!({ "type": "selectSession", "id": "s-far" })).await;
  until(|| m.active().is_some_and(|a| a["id"] == "s-far" && a["status"] == "readonly"), 5000).await;
  let shown = m.active().unwrap();
  assert!(shown["error"].as_str().unwrap_or_default().contains("Studio"), "{shown}");
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  m.handle(json!({ "type": "renameSession", "id": "s-far", "title": "Renamed here" })).await;
  m.handle(json!({ "type": "deleteSession", "id": "s-far" })).await;
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  assert_eq!(std::fs::read(dir.path().join("s-far.json")).unwrap(), record("s-far", "machine-elsewhere"), "the record is the other machine's");

  m.handle(json!({ "type": "selectSession", "id": "s-own" })).await;
  until(|| m.active().is_some_and(|a| a["id"] == "s-own" && a["status"] != "readonly"), 5000).await;
  m.dispose().await;
}

// The storage upkeep on a month-old record: its blob is shared with another session's copy, the record packs into a
// stub, and opening it gives back every turn and the whole diff texts; the first write after that keeps the big texts
// as blobs (one per distinct text), which a fresh engine reads back on demand
#[tokio::test(flavor = "multi_thread")]
async fn upkeep_packs_a_cold_record_and_diff_texts_come_back_from_blobs() {
  use std::time::{Duration, SystemTime};
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let (a, b, c) = ("a\n".repeat(20_000), "b\n".repeat(20_000), "c\n".repeat(20_000));
  let edit = |id: &str, old: &str, new: &str| {
    json!({ "type": "tool_call", "id": id, "kind": "edit", "verb": "Edit", "status": "completed",
      "content": { "type": "diff", "lines": [{ "kind": "add", "text": "x" }], "source": { "path": "/w/a.ts", "oldText": old, "newText": new } } })
  };
  let record = json!({ "id": "cold", "agent": "fake", "cwd": "/w", "title": "cold", "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z",
    "turns": [
      { "role": "user", "text": "edit it", "attachments": [{ "kind": "image", "blob": "abcdabcdabcdabcd.png", "mimeType": "image/png" }] },
      { "role": "agent", "blocks": [edit("t1", &a, &b), edit("t2", &b, &c)] },
    ],
    "controls": { "modes": [], "options": [] }, "commands": [] });
  let path = dir.path().join("cold.json");
  std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
  for s in ["cold", "other"] {
    std::fs::create_dir_all(dir.path().join(s)).unwrap();
    std::fs::write(dir.path().join(s).join("abcdabcdabcdabcd.png"), b"the same image").unwrap();
  }
  let month_ago = SystemTime::now() - Duration::from_secs(40 * 24 * 3600);
  std::fs::File::options().write(true).open(&path).unwrap().set_modified(month_ago).unwrap();

  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  let report = m.m.run_upkeep(SystemTime::now()).await;
  assert_eq!((report.packed_sessions, report.shared), (1, 14), "{report:?}");
  assert!(std::fs::metadata(&path).unwrap().len() < 2_000, "a stub is left");
  assert!(same_file::is_same_file(dir.path().join("cold/abcdabcdabcdabcd.png"), dir.path().join("other/abcdabcdabcdabcd.png")).unwrap());

  m.handle(json!({ "type": "selectSession", "id": "cold" })).await;
  until(|| m.active().is_some_and(|v| v["id"] == "cold" && turns_in(&v) == 2), 5000).await;
  let src = m.m.diff_source("cold", "t2", 0, None).await.unwrap();
  assert_eq!((src.old_text.len(), src.new_text == c), (b.len(), true));
  // A write of the reopened session: the whole record again, its diff texts as three blobs (b is shared by both edits)
  m.handle(json!({ "type": "renameSession", "id": "cold", "title": "renamed" })).await;
  until(|| std::fs::read_to_string(&path).is_ok_and(|t| t.contains("renamed") && t.contains("oldBlob")), 5000).await;
  let raw = std::fs::read_to_string(&path).unwrap();
  assert!(raw.len() < 10_000 && !raw.contains("acpira-packed:"), "{} bytes", raw.len());
  let texts = std::fs::read_dir(dir.path().join("cold")).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".zst")).count();
  // Plus the archive, kept (stale and unused) until the session is packed again
  assert_eq!(texts, 4);
  assert!(dir.path().join("cold/record.json.zst").exists());
  m.dispose().await;

  // A fresh engine reads the texts back from the blobs
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.handle(json!({ "type": "selectSession", "id": "cold" })).await;
  until(|| m.active().is_some_and(|v| v["id"] == "cold" && turns_in(&v) == 2), 5000).await;
  let src = m.m.diff_source("cold", "t1", 0, None).await.unwrap();
  assert!(src.old_text == a && src.new_text == b && !src.omitted);
  let bases = m.m.edit_bases("cold").await;
  assert_eq!(bases, vec![("/w/a.ts".to_owned(), Some(a.clone()))]);
  m.dispose().await;
}
