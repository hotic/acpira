//! Several managers over one directory and the workspace scope

use super::*;

// Two extension hosts (two windows, or VS Code + Cursor) share ~/.acpira/sessions. Each used to rewrite index.json from its own memory,
// so whichever streamed last erased the other's new sessions from the list while their records stayed on disk
#[tokio::test(flavor = "multi_thread")]
async fn two_managers_over_one_directory_see_each_others_sessions_and_honor_deletion() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "from A" })).await;
  let sa = a.active_id().unwrap();
  b.new_session(None).await;
  b.handle(json!({ "type": "send", "text": "from B" })).await;
  let sb = b.active_id().unwrap();
  // Each keeps streaming (index writes on both sides) — nothing is lost; a refresh (window focus) is when the other's work shows up
  a.handle(json!({ "type": "send", "text": "A again" })).await;
  b.handle(json!({ "type": "send", "text": "B again" })).await;
  b.m.refresh_index().await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  assert_eq!(sorted(a.session_ids()), sorted(vec![sa.clone(), sb.clone()]));
  assert_eq!(sorted(b.session_ids()), sorted(vec![sa.clone(), sb.clone()]));
  // A renames its own session: B sees the new title after its refresh, not its stale copy
  a.handle(json!({ "type": "renameSession", "id": sa, "title": "A 的会话" })).await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  assert_eq!(b.sessions().iter().find(|s| s["id"] == sa.as_str()).unwrap()["title"], "A 的会话");
  // A's window closes; B deletes A's session (live nowhere now): a host starting meanwhile does not list it, B's undo brings it back for everyone
  a.dispose().await;
  let c = Mgr::new(dir.path(), Opts::fake(&fake));
  b.handle(json!({ "type": "deleteSession", "id": sa })).await;
  c.init().await;
  assert_eq!(c.session_ids(), [sb.clone()]);
  b.handle(json!({ "type": "restoreSession", "id": sa })).await;
  c.m.refresh_index().await;
  assert_eq!(sorted(c.session_ids()), sorted(vec![sa.clone(), sb]));
  assert_eq!(c.sessions().iter().find(|s| s["id"] == sa.as_str()).unwrap()["title"], "A 的会话");
  b.dispose().await;
  c.dispose().await;
}

// The same session open in two windows: a deletion in one used to be undone by the other's next debounced save, which recreated the record
#[tokio::test(flavor = "multi_thread")]
async fn a_session_deleted_in_one_manager_closes_in_the_other_and_its_stale_save_does_not_revive_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "shared" })).await;
  let id = a.active_id().unwrap();
  // A's reconcile lands its debounced record; B picks the session up from the disk and opens it too
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  b.m.select_session_for(&b.v, &id).await;
  assert_eq!(b.active().unwrap()["id"], id.as_str());
  // B changes the record (a save is now debounced) right before A deletes it
  b.handle(json!({ "type": "renameSession", "id": id, "title": "renamed in B" })).await;
  a.handle(json!({ "type": "deleteSession", "id": id })).await;
  assert!(!dir.path().join(format!("{id}.json")).exists());
  // B's next reconcile: the pending save is dropped, the session closed, the viewer moved on
  b.m.refresh_index().await;
  assert!(!dir.path().join(format!("{id}.json")).exists());
  assert!(dir.path().join("trash").join(format!("{id}.json")).exists());
  assert!(!b.session_ids().contains(&id));
  assert!(b.active_id().is_some_and(|x| x != id));
  assert!(b.toasts().iter().any(|t| t.contains("another window") || t.contains("另一个窗口")), "{:?}", b.toasts());
  // Undo in A: the record is back in both lists as a stored session; B does not reattach to it by itself
  a.handle(json!({ "type": "restoreSession", "id": id })).await;
  b.m.refresh_index().await;
  assert!(dir.path().join(format!("{id}.json")).exists());
  assert!(a.session_ids().contains(&id));
  assert!(b.session_ids().contains(&id));
  assert!(b.view_of(&id).is_none());
  a.dispose().await;
  b.dispose().await;
}

// Sessions belong to the workspace folder they were opened in (their cwd). Under the workspace scope a viewer left without a session
// falls onto one of this folder's, never another project's; moving re-homes a session into the current folder
#[tokio::test(flavor = "multi_thread")]
async fn the_workspace_scope_keeps_most_recent_and_deletion_picks_inside_the_folder_and_move_re_homes() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let proj = |name: &str| {
    let p = dir.path().join("proj").join(name);
    std::fs::create_dir_all(&p).unwrap();
    p.to_string_lossy().into_owned()
  };
  let opts = Opts::fake(&fake).cwd(&proj("a"));
  *opts.scope.lock().unwrap() = "workspace".into();
  let sessions_dir = dir.path().join("sessions");
  // Project A: one session with a turn
  let m = Mgr::new(&sessions_dir, opts.clone());
  m.init().await;
  m.new_session(None).await;
  let a1 = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "in a" })).await;
  until(|| m.sessions().first().is_some_and(|s| s["id"] == a1.as_str()), 2000).await;
  expect_match(&m.sessions()[0], json!({ "id": a1, "cwd": proj("a") }));
  m.dispose().await;

  // Project B: a sidebar starting on "most recent" must not land on A's session
  *opts.cwd.lock().unwrap() = proj("b");
  let m = Mgr::new(&sessions_dir, opts.clone());
  m.init().await;
  let most_recent = || Some(acpira_shared::sidecar::InitialView::MostRecent { most_recent: true });
  assert!(m.m.attach(most_recent()).active_id().is_none());
  *opts.scope.lock().unwrap() = "all".into();
  assert_eq!(m.m.attach(most_recent()).active_id().as_deref(), Some(a1.as_str()));
  *opts.scope.lock().unwrap() = "workspace".into();
  m.new_session(None).await;
  let b1 = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "in b" })).await;
  // Listed once its first turn is flushed into the index: before that it is an empty session a new one would reuse, and no fallback
  let listed = |id: &str| m.sessions().iter().any(|s| s["id"] == id);
  until(|| listed(&b1), 5000).await;
  m.new_session(None).await;
  let b2 = m.active_id().unwrap();
  assert_ne!(b1, b2);
  m.handle(json!({ "type": "send", "text": "in b too" })).await;
  until(|| listed(&b2), 5000).await;
  // Deleting the active one falls back to B's other session, not the newer-looking A one
  m.handle(json!({ "type": "deleteSession", "id": b2 })).await;
  assert_eq!(m.active_id().as_deref(), Some(b1.as_str()));

  // Move A's stored session into B: its record and summary change folder
  m.handle(json!({ "type": "moveSession", "id": a1 })).await;
  assert_eq!(m.sessions().iter().find(|s| s["id"] == a1.as_str()).unwrap()["cwd"], proj("b"));
  let store = TranscriptStore::new(sessions_dir.clone(), Arc::new(|_: &str| {}), None);
  assert_eq!(store.load(&a1).await.unwrap().cwd, proj("b"));

  // Move a live idle session: it is reopened in the new folder (a fresh process; "gone" makes the fake report the old id swept —
  // the transcript already ran, so it stays read-only with its history); a running one refuses
  *opts.cwd.lock().unwrap() = proj("c-gone");
  m.handle(json!({ "type": "moveSession", "id": b1 })).await;
  assert_eq!(m.active_id().as_deref(), Some(b1.as_str()));
  let active = m.active().unwrap();
  assert_eq!(active["cwd"], proj("c-gone"));
  assert_eq!(active["status"], "readonly");
  assert_eq!(turns_in(&active), 2);
  // A session mid-turn refuses the move; use a fresh one, since the moved b1 is read-only now
  *opts.cwd.lock().unwrap() = proj("d");
  m.new_session(None).await;
  let d1 = m.active_id().unwrap();
  let sending = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| m.active().is_some_and(|a| a["running"] == true), 5000).await;
  *opts.cwd.lock().unwrap() = proj("e");
  m.handle(json!({ "type": "moveSession", "id": d1 })).await;
  assert!(m.toasts().iter().any(|t| t.contains("moving") || t.contains("移动")), "{:?}", m.toasts());
  m.handle(json!({ "type": "stop" })).await;
  sending.await.unwrap();
  assert_eq!(m.active().unwrap()["cwd"], proj("d"));
  m.dispose().await;
}
