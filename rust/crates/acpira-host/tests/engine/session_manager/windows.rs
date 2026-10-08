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

// The same session open in two windows: a deletion in one used to be undone by the other's next debounced save, which recreated
// the record. Now the second engine only holds a read-only copy (A has the session open), which the deletion closes
#[tokio::test(flavor = "multi_thread")]
async fn a_session_deleted_in_one_manager_closes_its_read_only_copy_in_the_other() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "shared" })).await;
  let id = a.active_id().unwrap();
  until(|| dir.path().join(format!("{id}.json")).exists(), 3000).await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  b.m.select_session_for(&b.v, &id).await;
  let shown = b.active().unwrap();
  assert_eq!(shown["id"], id.as_str());
  assert_eq!(shown["status"], "readonly", "{shown}");
  assert_eq!(shown["canTakeOver"], true, "{shown}");
  // B cannot edit what A drives; A deletes it
  b.handle(json!({ "type": "renameSession", "id": id, "title": "renamed in B" })).await;
  assert!(b.toasts().iter().any(|t| t.contains("Another Acpira engine")), "{:?}", b.toasts());
  a.handle(json!({ "type": "deleteSession", "id": id })).await;
  assert!(!dir.path().join(format!("{id}.json")).exists(), "A: {:?}", a.toasts());
  // B's next reconcile: the copy closed, the viewer moved on
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

// One session, two engines: A has it open, so B shows it read-only and cannot edit it. Taking it over stops A's turn, turns
// A's view read-only (it can take the session back) and opens the session in B with the whole transcript (store::session_lease)
#[tokio::test(flavor = "multi_thread")]
async fn a_session_open_in_one_manager_is_read_only_in_the_other_until_taken_over() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  // The two agents share a native session store, as real CLIs do, so B's agent can resume what A's started. A slow turn of
  // about 7.5 s is still running when B takes over
  let native = tempfile::tempdir().unwrap();
  let opts = || Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native.path(), "FAKE_SLOW_STEP_MS": "150" } })), "fake");
  let a = Mgr::new(dir.path(), opts());
  let b = Mgr::new(dir.path(), opts());
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "first" })).await;
  let id = a.active_id().unwrap();
  until(|| dir.path().join(format!("{id}.json")).exists(), 3000).await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  b.m.select_session_for(&b.v, &id).await;
  let shown = b.active().unwrap();
  assert_eq!(shown["status"], "readonly", "{shown}");
  assert_eq!(shown["canTakeOver"], true);
  assert_eq!(turns_in(&shown), 2);

  // A runs a long turn; B's record edits are refused meanwhile
  let running = a.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| a.active().is_some_and(|s| s["running"] == true), 5000).await;
  b.handle(json!({ "type": "renameSession", "id": id, "title": "renamed in B" })).await;
  assert!(b.toasts().iter().any(|t| t.contains("Another Acpira engine")), "{:?}", b.toasts());

  // B takes it over: A's turn stops and A keeps a read-only copy; B has the session live with A's turns on it
  b.handle(json!({ "type": "takeOverSession", "sessionId": id })).await;
  running.await.unwrap();
  let taken = b.active().unwrap();
  assert_eq!(taken["id"], id.as_str());
  assert_eq!(taken["status"], "ready", "{taken} / {:?}", b.toasts());
  assert_eq!(turns_in(&taken), 4);
  until(|| a.active().is_some_and(|s| s["status"] == "readonly"), 3000).await;
  let left = a.active().unwrap();
  assert_eq!(left["canTakeOver"], true);
  assert!(left["error"].as_str().unwrap_or_default().contains("took this session over"), "{left}");
  assert_eq!(b.m.leased(), vec![id.clone()]);
  assert!(a.m.leased().is_empty());

  // B works on it; then A takes it back
  b.handle(json!({ "type": "send", "text": "in B" })).await;
  let b_turns = turns_in(&b.active().unwrap());
  assert_eq!(b_turns, 6);
  a.handle(json!({ "type": "takeOverSession", "sessionId": id })).await;
  let back = a.active().unwrap();
  assert_eq!(back["status"], "ready", "{back} / {:?}", a.toasts());
  assert_eq!(turns_in(&back), b_turns);
  until(|| b.active().is_some_and(|s| s["status"] == "readonly"), 3000).await;
  a.dispose().await;
  b.dispose().await;
}

// A lease that cannot be taken at all (its directory is unusable) is not a free pass: turns are refused with the reason, and a
// stored session opens read-only instead of starting an agent and saving over whatever another engine might be doing
#[tokio::test(flavor = "multi_thread")]
async fn a_lease_that_cannot_be_taken_refuses_turns_and_opens_stored_sessions_read_only() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "first" })).await;
  let id = m.active_id().unwrap();
  until(|| dir.path().join(format!("{id}.json")).exists(), 3000).await;
  m.dispose().await;

  // The lease directory's place is taken by a file
  std::fs::write(dir.path().join("lease-home").join("run").join("leases.tmp"), "").unwrap();
  std::fs::remove_dir_all(dir.path().join("lease-home").join("run").join("leases")).unwrap();
  std::fs::rename(dir.path().join("lease-home").join("run").join("leases.tmp"), dir.path().join("lease-home").join("run").join("leases")).unwrap();
  let failed = |toasts: Vec<String>| toasts.iter().filter(|t| t.contains("Could not take this session")).count();

  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.m.select_session_for(&m.v, &id).await;
  let shown = m.active().unwrap();
  assert_eq!(shown["status"], "readonly", "{shown}");
  assert!(shown["error"].as_str().unwrap_or_default().contains("Could not take this session"), "{shown}");
  assert_eq!(turns_in(&shown), 2);
  // A fresh session has no record anyone else could hold, but its turns still need the lease
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "refused" })).await;
  assert_eq!(turns_in(&m.active().unwrap()), 0);
  assert_eq!(failed(m.toasts()), 1, "{:?}", m.toasts());
  m.dispose().await;
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

  // An explicit destination (dragged onto another project's group): the live idle d1 moves into project F, filed under F's
  // category in the same step; a category of another project leaves it unfiled
  let f = proj("f");
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-f", "name": "F", "cwd": f })).await;
  m.handle(json!({ "type": "categoryOp", "op": "create", "id": "c-g", "name": "G", "cwd": proj("g") })).await;
  m.handle(json!({ "type": "moveSession", "id": d1, "cwd": f, "category": "c-f" })).await;
  assert_eq!(m.active_id().as_deref(), Some(d1.as_str()));
  expect_match(m.sessions().into_iter().find(|s| s["id"] == d1.as_str()).unwrap(), json!({ "cwd": f, "category": "c-f" }));
  // The stored a1 moves out of B into G, while c-f belongs to F: it lands there unfiled
  let g = proj("g");
  m.handle(json!({ "type": "moveSession", "id": a1, "cwd": g, "category": "c-f" })).await;
  let moved = m.sessions().into_iter().find(|s| s["id"] == a1.as_str()).unwrap();
  assert_eq!(moved["cwd"], g);
  assert!(moved.get("category").is_none_or(Value::is_null), "{moved}");
  assert_eq!(store.load(&a1).await.unwrap().cwd, g);
  // A folder that no longer exists refuses, and the record stays where it was
  let gone = dir.path().join("proj").join("never").to_string_lossy().into_owned();
  m.handle(json!({ "type": "moveSession", "id": a1, "cwd": gone })).await;
  assert!(m.toasts().iter().any(|t| t.contains("never")), "{:?}", m.toasts());
  assert_eq!(store.load(&a1).await.unwrap().cwd, g);
  m.dispose().await;
}
