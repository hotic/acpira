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
  // The record copy, as with an engine off the push bus (the pushed view closes the same way: it lives only with the copy)
  let a = Mgr::new(dir.path(), Opts::fake(&fake).no_bus());
  let b = Mgr::new(dir.path(), Opts::fake(&fake).no_bus());
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
  // about 7.5 s is still running when B takes over. Off the push bus, as with an engine from before it: B polls the record
  let native = tempfile::tempdir().unwrap();
  let opts = || Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native.path(), "FAKE_SLOW_STEP_MS": "150" } })), "fake").no_bus();
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
  // File links resolve against the session's folder before B opens it (the index) and on B's read-only copy
  let cwd = a.m.session_cwd(&id).unwrap();
  assert_eq!(b.m.session_cwd(&id).as_deref(), Some(cwd.as_str()));
  b.m.select_session_for(&b.v, &id).await;
  let shown = b.active().unwrap();
  assert_eq!(shown["status"], "readonly", "{shown}");
  assert_eq!(shown["canTakeOver"], true);
  assert_eq!(turns_in(&shown), 2);
  assert_eq!(b.m.session_cwd(&id).as_deref(), Some(cwd.as_str()));

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

// The push bus (`bus.rs`, Codex's model): a session one engine runs shows live in another, whose page acts on it without a
// takeover; the actions run in the owner, which keeps the session. Once the owner lets go, the other engine opens it itself
#[tokio::test(flavor = "multi_thread")]
async fn a_session_another_engine_owns_streams_here_and_takes_actions_without_a_takeover() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = tempfile::tempdir().unwrap();
  let opts = || Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native.path(), "FAKE_SLOW_STEP_MS": "150" } })), "fake");
  let a = Mgr::new(dir.path(), opts());
  let b = Mgr::new(dir.path(), opts());
  a.init().await;
  b.init().await;
  until(|| a.m.bus_connected() && b.m.bus_connected(), 3000).await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "first" })).await;
  let id = a.active_id().unwrap();
  until(|| dir.path().join(format!("{id}.json")).exists(), 3000).await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;

  // B shows A's own view: not read-only, nothing to take over. Nor for a moment on the way: the record copy with its
  // "take over" card used to go out first and flash before the pushed view replaced it
  let seen = record_events(&b.v);
  let flashed = |seen: &Arc<Mutex<Vec<Value>>>| -> Vec<Value> {
    seen.lock().unwrap().iter().filter(|e| e["type"] == "session" && e["session"]["id"] == id.as_str() && e["session"]["status"] == "readonly").cloned().collect()
  };
  b.m.select_session_for(&b.v, &id).await;
  until(|| b.active().is_some_and(|s| s["id"] == id.as_str() && s["status"] != "readonly"), 3000).await;
  let shown = b.active().unwrap();
  assert_ne!(shown["canTakeOver"], true, "{shown}");
  assert_eq!(turns_in(&shown), 2);
  assert!(flashed(&seen).is_empty(), "read-only copy shown first: {:?}", flashed(&seen));
  // Away and straight back (the copy is still there): the pushed view again, not the record copy
  b.new_session(None).await;
  b.m.select_session_for(&b.v, &id).await;
  until(|| b.active().is_some_and(|s| s["id"] == id.as_str()), 3000).await;
  assert!(flashed(&seen).is_empty(), "read-only copy shown on return: {:?}", flashed(&seen));

  // A's slow turn streams into B while it runs, and ends there too. B's session list shows it running as well (the
  // state goes over the bus with the views): it used to show the transcript moving next to an idle list entry
  let state_in_b = |b: &Mgr| b.sessions().into_iter().find(|s| s["id"] == id.as_str()).map(|s| s["state"].clone());
  let running = a.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| b.active().is_some_and(|s| s["running"] == true), 5000).await;
  until(|| state_in_b(&b) == Some(json!("working")), 5000).await;
  running.await.unwrap();
  until(|| b.active().is_some_and(|s| s["running"] == false && turns_in(&s) == 4), 5000).await;
  until(|| state_in_b(&b).is_some_and(|s| s.is_null()), 5000).await;

  // B sends: the turn runs in A, both show it, A keeps the session
  b.handle(json!({ "type": "send", "sessionId": id, "text": "from B" })).await;
  until(|| a.active().is_some_and(|s| turns_in(&s) == 6), 5000).await;
  until(|| b.active().is_some_and(|s| turns_in(&s) == 6 && s["running"] == false), 5000).await;
  assert!(b.toasts().is_empty(), "{:?}", b.toasts());
  assert_eq!(a.m.leased(), vec![id.clone()]);
  assert!(!b.m.leased().contains(&id));

  // A ends: B hears the session is gone and opens it here
  a.dispose().await;
  until(|| b.m.leased().contains(&id), 8000).await;
  until(|| b.active().is_some_and(|s| s["status"] == "ready" && turns_in(&s) == 6), 5000).await;
  b.dispose().await;
}

// A paired device resends what it is unsure of after a lost link with the same clientId: the engine running the session
// runs it once, whether the resend reaches it directly or forwarded over the push bus from another engine
#[tokio::test(flavor = "multi_thread")]
async fn a_resent_client_id_runs_once_here_and_over_the_bus() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  until(|| a.m.bus_connected() && b.m.bus_connected(), 3000).await;
  a.new_session(None).await;
  let send = |id: &str, cid: &str| json!({ "type": "send", "sessionId": id, "text": "hi", "clientId": cid });
  a.handle(json!({ "type": "send", "text": "first" })).await;
  let id = a.active_id().unwrap();
  until(|| a.active().is_some_and(|s| turns_in(&s) == 2 && s["running"] == false), 5000).await;
  a.handle(send(&id, "c1")).await;
  a.handle(send(&id, "c1")).await;
  until(|| a.active().is_some_and(|s| turns_in(&s) == 4 && s["running"] == false), 5000).await;

  until(|| dir.path().join(format!("{id}.json")).exists(), 3000).await;
  a.m.refresh_index().await;
  b.m.refresh_index().await;
  b.m.select_session_for(&b.v, &id).await;
  until(|| b.active().is_some_and(|s| s["id"] == id.as_str() && s["status"] != "readonly"), 3000).await;
  b.handle(send(&id, "c2")).await;
  b.handle(send(&id, "c2")).await;
  // The id A already ran, now arriving from B
  b.handle(send(&id, "c1")).await;
  until(|| a.active().is_some_and(|s| turns_in(&s) == 6 && s["running"] == false), 5000).await;
  tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  assert_eq!(turns_in(&a.active().unwrap()), 6);
  assert!(b.toasts().is_empty(), "{:?}", b.toasts());
  a.dispose().await;
  b.dispose().await;
}

// The sync model's session facts in the list: a session another engine runs shows there as its owner lists it (a new one
// at once, a rename) without waiting for a refresh, and a rename or pin made in the other engine is written by the owner
// instead of being refused as "busy elsewhere"
#[tokio::test(flavor = "multi_thread")]
async fn the_list_entry_of_a_session_another_engine_runs_follows_its_owner() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  until(|| a.m.bus_connected() && b.m.bus_connected(), 3000).await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "first" })).await;
  let id = a.active_id().unwrap();
  let entry = |m: &Mgr| m.sessions().into_iter().find(|s| s["id"] == id.as_str());
  // B never refreshes in this test: A announces its sessions
  until(|| entry(&b).is_some(), 3000).await;
  a.handle(json!({ "type": "renameSession", "id": id, "title": "A 改的名" })).await;
  until(|| entry(&b).is_some_and(|s| s["title"] == "A 改的名"), 3000).await;
  // B renames and pins it: A, the owner, writes both, and both lists show them
  b.handle(json!({ "type": "renameSession", "id": id, "title": "B 改的名" })).await;
  b.handle(json!({ "type": "pinSession", "id": id, "pinned": true })).await;
  assert!(b.toasts().is_empty(), "{:?}", b.toasts());
  until(|| entry(&a).is_some_and(|s| s["title"] == "B 改的名" && s["pinned"] == true), 3000).await;
  until(|| entry(&b).is_some_and(|s| s["title"] == "B 改的名" && s["pinned"] == true), 3000).await;
  let record = a.store.load(&id).await.unwrap();
  assert_eq!((record.title.as_str(), record.pinned), ("B 改的名", Some(true)));
  // A ends: B keeps the entry, now read from the directory, with nothing running
  a.dispose().await;
  until(|| entry(&b).is_some_and(|s| s["title"] == "B 改的名" && s["state"].is_null()), 5000).await;
  b.dispose().await;
}

// Unread is the user's, not an engine's: a reply that came in while nobody anywhere looked shows its dot in every
// engine's list and in an engine started later, and looking at it in any one of them clears it in all
#[tokio::test(flavor = "multi_thread")]
async fn an_unread_mark_is_shared_by_every_engine_and_outlives_them() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  until(|| a.m.bus_connected() && b.m.bus_connected(), 3000).await;
  a.new_session(None).await;
  let id = a.active_id().unwrap();
  let state = |m: &Mgr| m.sessions().into_iter().find(|s| s["id"] == id.as_str()).map(|s| s["state"].clone()).unwrap_or(Value::Null);
  // The turn ends after A's page moved on, and B shows nothing of it
  let running = a.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| a.active().is_some_and(|s| s["running"] == true), 5000).await;
  a.new_session(None).await;
  running.await.unwrap();
  until(|| state(&a) == "unread", 5000).await;
  until(|| state(&b) == "unread", 5000).await;
  // An engine started now (a window opened later, or after a restart) reads it from the shared store
  b.dispose().await;
  let b2 = Mgr::new(dir.path(), Opts::fake(&fake));
  b2.init().await;
  until(|| state(&b2) == "unread", 5000).await;
  // Looked at in that engine: read in A as well, and in the store
  until(|| b2.m.bus_connected(), 3000).await;
  until(|| dir.path().join(format!("{id}.json")).exists(), 3000).await;
  b2.m.select_session_for(&b2.v, &id).await;
  assert_eq!(state(&b2), Value::Null);
  until(|| state(&a) == Value::Null, 5000).await;
  let prefs: Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("prefs.json")).unwrap()).unwrap();
  assert!(!prefs["unread"].as_array().is_some_and(|u| u.iter().any(|x| x == id.as_str())), "{prefs}");
  a.dispose().await;
  b2.dispose().await;
}

// A subagent of a session another engine runs: observing it there gets the owner's transcript, and the owner pushes each
// change while it is observed (here the permission the other engine answered)
#[tokio::test(flavor = "multi_thread")]
async fn a_subagent_of_a_session_another_engine_runs_streams_to_its_observer() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  b.init().await;
  until(|| a.m.bus_connected() && b.m.bus_connected(), 3000).await;
  a.new_session(None).await;
  let sid = a.active_id().unwrap();
  let p = a.spawn_handle(json!({ "type": "send", "text": "subagents-native" }));
  let c1 = |m: &Mgr| m.view_of(&sid).and_then(|x| x["subagents"].as_array().and_then(|n| n.iter().find(|n| n["peer"]["sessionId"] == "c1").cloned()));
  until(|| c1(&a).is_some_and(|n| n["permissions"].as_array().is_some_and(|p| !p.is_empty())), 8000).await;
  let c1id = c1(&a).unwrap()["id"].as_str().unwrap().to_owned();
  until(|| dir.path().join(format!("{sid}.json")).exists(), 3000).await;
  b.m.select_session_for(&b.v, &sid).await;
  until(|| b.active().is_some_and(|s| s["id"] == sid.as_str() && s["status"] != "readonly"), 3000).await;
  let subs = |m: &Mgr| m.events.lock().unwrap().iter().filter(|x| x["type"] == "subagent" && x["subagentId"] == c1id.as_str()).cloned().collect::<Vec<_>>();
  let perm_in = |x: &Value| {
    x["turns"].as_array().into_iter().flatten().filter(|t| t["role"] == "agent").flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["type"] == "permission")
  };
  b.handle(json!({ "type": "observeSubagent", "sessionId": sid, "subagentId": c1id })).await;
  until(|| subs(&b).iter().any(|x| perm_in(x).is_some()), 5000).await;
  // Never the read-only copy's version, which shows the running subagent as cancelled
  assert!(subs(&b).iter().all(|x| !x["turns"].to_string().contains("cancelled")), "{:?}", subs(&b));
  let first = subs(&b).into_iter().rev().find(|x| perm_in(x).is_some()).unwrap();
  // Answered in B (the action runs in A): the newer transcript comes back
  b.handle(json!({ "type": "permission", "sessionId": sid, "blockId": perm_in(&first).unwrap()["id"], "optionId": "allow" })).await;
  until(|| subs(&b).iter().any(|x| x["rev"].as_i64() > first["rev"].as_i64()), 5000).await;
  assert!(b.toasts().is_empty(), "{:?}", b.toasts());
  b.handle(json!({ "type": "unobserveSubagent", "sessionId": sid, "subagentId": c1id })).await;
  p.await.unwrap();
  a.dispose().await;
  b.dispose().await;
}

// A window reloaded onto a new engine (the binary was reinstalled) used to find its session read-only behind "Take over":
// the old engine, with no window left, still held the lease. A session the holder runs nothing on and shows in no window
// now goes to the engine that opens it, without a takeover (store::session_lease, `<id>.wanted`)
#[tokio::test(flavor = "multi_thread")]
async fn an_idle_session_no_window_shows_moves_to_the_engine_that_opens_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = tempfile::tempdir().unwrap();
  let opts = || Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native.path() } })), "fake");
  let a = Mgr::new(dir.path(), opts());
  let b = Mgr::new(dir.path(), opts());
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "first" })).await;
  let id = a.active_id().unwrap();
  until(|| dir.path().join(format!("{id}.json")).exists(), 3000).await;
  // A's window moves on; the session stays live (and leased) in A
  a.new_session(None).await;
  assert!(a.m.leased().contains(&id));
  a.m.refresh_index().await;
  b.m.refresh_index().await;

  b.m.select_session_for(&b.v, &id).await;
  // Opened in B itself (A's pushed view is ready too, so the lease is what tells them apart)
  until(|| b.m.leased().contains(&id) && b.active().is_some_and(|s| s["id"] == id.as_str() && s["status"] == "ready"), 8000).await;
  assert_eq!(turns_in(&b.active().unwrap()), 2, "{:?}", b.toasts());
  assert!(!a.m.leased().contains(&id));
  assert!(a.logs().iter().any(|l| l.contains("handing it over")), "{:?}", a.logs());
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

// The Storage tab's batch delete goes through delete_session one by one: a session another engine has open is passed
// over with the reason, the rest go into the undo window like a history-list delete
#[tokio::test(flavor = "multi_thread")]
async fn storage_batch_delete_skips_a_session_another_engine_holds_and_says_why() {
  use acpira_shared::storage::{StorageAction, StorageBlock, StorageDetail};
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let opts = || Opts::with_agents(fake.setting(json!({})), "fake");
  let a = Mgr::new(dir.path(), opts());
  let b = Mgr::new(dir.path(), opts());
  a.init().await;
  b.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "in A" })).await;
  let held = a.active_id().unwrap();
  b.new_session(None).await;
  b.handle(json!({ "type": "send", "text": "in B" })).await;
  let mine = b.active_id().unwrap();
  until(|| dir.path().join(format!("{held}.json")).exists() && dir.path().join(format!("{mine}.json")).exists(), 3000).await;
  b.m.refresh_index().await;

  let view = b.m.storage_view(&[StorageDetail::Sessions]).await;
  let rows = view.sessions.unwrap();
  assert_eq!(rows.iter().find(|r| r.id == held).unwrap().blocked, Some(StorageBlock::Elsewhere));
  assert_eq!(rows.iter().find(|r| r.id == mine).unwrap().blocked, None);

  let reply = b.m.storage_action(StorageAction::DeleteSessions { ids: vec![held.clone(), mine.clone()] }, &[StorageDetail::Sessions]).await;
  let out = reply.outcome.unwrap();
  assert_eq!(out.done, vec![mine.clone()]);
  assert_eq!(out.skipped.len(), 1);
  assert_eq!(out.skipped[0].id, held);
  assert!(out.skipped[0].reason.contains("Another Acpira engine"), "{:?}", out.skipped);
  assert!(dir.path().join("trash").join(format!("{mine}.json")).exists());
  assert!(dir.path().join(format!("{held}.json")).exists());
  let after = reply.view.sessions.unwrap();
  assert!(after.iter().all(|r| r.id != mine));

  let back = b.m.storage_action(StorageAction::RestoreSessions { ids: vec![mine.clone()] }, &[]).await;
  assert_eq!(back.outcome.unwrap().done, vec![mine.clone()]);
  assert!(dir.path().join(format!("{mine}.json")).exists());
  a.dispose().await;
  b.dispose().await;
}

// Slimming rewrites a stored record without its whole-file diff texts: the diff lines stay, the source reads omitted,
// and a session open in this engine is left alone (its transcript in memory would write the texts back)
#[tokio::test(flavor = "multi_thread")]
async fn storage_slim_drops_diff_texts_and_leaves_open_sessions_alone() {
  use acpira_shared::storage::{StorageAction, StorageDetail};
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::with_agents(fake.setting(json!({})), "fake"));
  a.init().await;
  let text = "x".repeat(20_000);
  let lines = json!([{ "kind": "hunk", "text": "@@ -1 +1 @@" }, { "kind": "del", "text": "a" }, { "kind": "add", "text": "b" }]);
  let tool = json!({ "type": "tool_call", "id": "t1", "kind": "edit", "verb": "Edit", "status": "completed",
    "content": { "type": "diff", "lines": lines, "source": { "path": "/w/a.ts", "oldText": text, "newText": text } } });
  let record = json!({ "id": "stored", "agent": "fake", "cwd": "/w", "title": "stored", "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z",
    "turns": [{ "role": "agent", "blocks": [tool] }], "controls": { "modes": [], "options": [] }, "commands": [] });
  std::fs::write(dir.path().join("stored.json"), serde_json::to_vec(&record).unwrap()).unwrap();
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "open here" })).await;
  let open = a.active_id().unwrap();
  until(|| dir.path().join(format!("{open}.json")).exists(), 3000).await;
  a.m.refresh_index().await;

  let view = a.m.storage_view(&[StorageDetail::Sessions, StorageDetail::Slim]).await;
  let rows = view.sessions.unwrap();
  let stored = rows.iter().find(|r| r.id == "stored").unwrap();
  assert!(stored.slim.unwrap() >= 40_000, "{stored:?}");
  assert!(!stored.open);
  assert!(rows.iter().find(|r| r.id == open).unwrap().open);

  let reply = a.m.storage_action(StorageAction::SlimSessions { ids: vec!["stored".into(), open.clone()] }, &[StorageDetail::Sessions]).await;
  let out = reply.outcome.unwrap();
  assert_eq!(out.done, vec!["stored".to_owned()]);
  assert!(out.freed >= 40_000, "{out:?}");
  assert_eq!(out.skipped.len(), 1);
  assert!(out.skipped[0].reason.contains("Open in this window"), "{:?}", out.skipped);
  let back: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("stored.json")).unwrap()).unwrap();
  let content = &back["turns"][0]["blocks"][0]["content"];
  assert_eq!(content["lines"], lines);
  assert_eq!(content["source"], json!({ "path": "/w/a.ts", "omitted": true }));
  assert_eq!(back["updatedAt"], "2026-01-01T00:00:00.000Z");
  a.dispose().await;
}
