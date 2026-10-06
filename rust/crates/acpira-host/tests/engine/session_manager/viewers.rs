//! Several viewers over one manager: independent active sessions, shared loads, addressed answers, observation

use super::*;

fn session_ids_seen(events: &Arc<Mutex<Vec<Value>>>) -> Vec<String> {
  events.lock().unwrap().iter().filter(|e| e["type"] == "session").map(|e| e["session"]["id"].as_str().unwrap_or("").to_owned()).collect()
}

// Several webviews (sidebar + editor tabs) each hold their own active session over the shared list; only the viewers showing a session get its updates
#[tokio::test(flavor = "multi_thread")]
async fn viewers_hold_independent_active_sessions_and_only_get_their_sessions_events() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  let a = m.m.attach(None);
  let b = m.m.attach(None);
  let seen_a = record_events(&a);
  let seen_b = record_events(&b);
  // a fresh viewer opens a fresh session; a second fresh viewer gets its own, not a's
  m.m.ensure_active_for(&a).await;
  m.m.ensure_active_for(&b).await;
  let sa = a.active_id().unwrap();
  let sb = b.active_id().unwrap();
  assert_ne!(sa, sb);
  assert_eq!(sorted(m.session_ids()), sorted(vec![sa.clone(), sb.clone()]));
  // both viewers can look at the same session; b moving on to a new one leaves a where it was
  m.handle_on(&a, json!({ "type": "send", "text": "hi" })).await;
  m.m.select_session_for(&b, &sa).await;
  assert_eq!(turns_len(m.active_of(&b)), 2);
  m.m.new_session_for(&b, None, None, None).await.unwrap();
  assert!(m.session_ids().contains(&sa));
  assert_eq!(a.active_id().as_deref(), Some(sa.as_str()));
  assert_ne!(b.active_id().as_deref(), Some(sa.as_str()));
  // updates route by active session: a's turn reached a, but not b once b moved on
  seen_a.lock().unwrap().clear();
  seen_b.lock().unwrap().clear();
  m.handle_on(&a, json!({ "type": "send", "text": "again" })).await;
  until(|| session_ids_seen(&seen_a).contains(&sa), 5000).await;
  assert!(!session_ids_seen(&seen_b).contains(&sa));
  // deleting a's session moves only a; b stays where it was
  let sb2 = b.active_id().unwrap();
  m.handle_on(&b, json!({ "type": "deleteSession", "id": sa })).await;
  assert_eq!(b.active_id().as_deref(), Some(sb2.as_str()));
  assert!(a.active_id().is_some_and(|x| x != sa));
  // a detached viewer no longer hears anything
  seen_b.lock().unwrap().clear();
  m.m.detach(&b);
  m.handle_on(&a, json!({ "type": "send", "text": "quiet" })).await;
  assert!(seen_b.lock().unwrap().is_empty());
  m.dispose().await;
}

// Two viewers landing on the same stored session at once used to build one AcpSession each: two processes, the second
// shadowing the first in the live map. The shared load hands both the same session
#[tokio::test(flavor = "multi_thread")]
async fn concurrent_selects_of_the_same_stored_session_share_one_load() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let a = Mgr::new(dir.path(), Opts::fake(&fake));
  a.init().await;
  a.new_session(None).await;
  a.handle(json!({ "type": "send", "text": "hi" })).await;
  let id = a.active_id().unwrap();
  a.dispose().await;
  let b = Mgr::new(dir.path(), Opts::fake(&fake));
  b.init().await;
  let v1 = b.m.attach(None);
  let v2 = b.m.attach(None);
  b.logs.lock().unwrap().clear();
  tokio::join!(b.m.select_session_for(&v1, &id), b.m.select_session_for(&v2, &id));
  assert_eq!(b.logs().iter().filter(|l| l.contains("spawn") || l.contains("reuse warm")).count(), 1, "{:#?}", b.logs());
  assert_eq!(v1.active_id().as_deref(), Some(id.as_str()));
  assert_eq!(v2.active_id().as_deref(), Some(id.as_str()));
  assert_eq!(turns_len(b.view_of(&id)), 2);
  b.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_answers_address_the_named_session_not_the_viewers_current_one() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  let perm_of = |id: &str| m.view_of(id).and_then(|x| crate::acp_session::agent_blocks(&x).into_iter().find(|b| b["type"] == "permission"));
  // handle(send) waits for the whole turn, including the permission gate — do not await it
  let send_a = m.spawn_handle(json!({ "type": "send", "text": "use tool" }));
  until(|| perm_of(&m.active_id().unwrap()).is_some(), 8000).await;
  let a = m.active_id().unwrap();
  let perm_a = perm_of(&a).unwrap();
  m.new_session(None).await;
  let send_b = m.spawn_handle(json!({ "type": "send", "text": "use tool" }));
  until(|| m.active_id().is_some_and(|x| x != a) && perm_of(&m.active_id().unwrap()).is_some(), 8000).await;
  let b = m.active_id().unwrap();
  m.handle(json!({ "type": "permission", "sessionId": a, "blockId": perm_a["id"], "optionId": "allow" })).await;
  until(|| perm_of(&a).is_none() && m.view_of(&a).is_some_and(|x| crate::acp_session::agent_blocks(&x).iter().any(|b| b["type"] == "tool_call" && b["status"] == "completed")), 8000).await;
  assert!(perm_of(&b).is_some());
  assert_eq!(m.active_id().as_deref(), Some(b.as_str()));
  send_a.await.unwrap();
  send_b.abort();
  m.dispose().await;
}

// A turn that ends while no viewer shows its session leaves an unread dot until a viewer lands on it; one watched to the end leaves none
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_ending_unwatched_marks_the_session_unread_until_it_is_opened() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  let state_of = |id: &str| m.sessions().iter().find(|s| s["id"] == id).map(|s| s["state"].clone()).unwrap_or(Value::Null);
  m.new_session(None).await;
  let watched = m.active_id().unwrap();
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  assert_eq!(state_of(&watched), Value::Null);

  m.new_session(None).await;
  let perm_of = |id: &str| m.view_of(id).and_then(|x| crate::acp_session::agent_blocks(&x).into_iter().find(|b| b["type"] == "permission"));
  let send = m.spawn_handle(json!({ "type": "send", "text": "use tool" }));
  until(|| m.active_id().is_some_and(|x| x != watched) && perm_of(&m.active_id().unwrap()).is_some(), 8000).await;
  let away = m.active_id().unwrap();
  let perm = perm_of(&away).unwrap();
  m.handle(json!({ "type": "selectSession", "id": watched })).await;
  m.handle(json!({ "type": "permission", "sessionId": away, "blockId": perm["id"], "optionId": "allow" })).await;
  send.await.unwrap();
  until(|| state_of(&away) == "unread", 5000).await;

  m.handle(json!({ "type": "selectSession", "id": away })).await;
  assert_eq!(state_of(&away), Value::Null);
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn observe_subagent_streams_only_to_the_observing_viewer_until_unobserved() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  let a = m.m.attach(None);
  let b = m.m.attach(None);
  let subs_a = record_events(&a);
  let subs_b = record_events(&b);
  let subs = |e: &Arc<Mutex<Vec<Value>>>| e.lock().unwrap().iter().filter(|x| x["type"] == "subagent").cloned().collect::<Vec<_>>();
  m.m.ensure_active_for(&a).await;
  let sid = a.active_id().unwrap();
  m.m.select_session_for(&b, &sid).await;
  let p = {
    let (mm, a) = (m.m.clone(), a.clone());
    tokio::spawn(async move { mm.handle_for(&a, serde_json::from_value(json!({ "type": "send", "text": "subagents-native" })).unwrap()).await })
  };
  let c1 = || m.active_of(&a).and_then(|x| x["subagents"].as_array().and_then(|n| n.iter().find(|n| n["peer"]["sessionId"] == "c1").cloned()));
  until(|| c1().is_some_and(|n| n["permissions"].as_array().is_some_and(|p| !p.is_empty())), 8000).await;
  let c1id = c1().unwrap()["id"].as_str().unwrap().to_owned();
  m.handle_on(&a, json!({ "type": "observeSubagent", "sessionId": sid, "subagentId": c1id })).await;
  until(|| !subs(&subs_a).is_empty(), 2000).await;
  expect_match(&subs(&subs_a)[0], json!({ "sessionId": sid, "subagentId": c1id }));
  assert!(subs(&subs_b).is_empty());
  // answering the child's permission through the viewer's normal route bumps the stream's rev
  let perm = subs(&subs_a)[0]["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "agent").flat_map(|t| t["blocks"].as_array().unwrap().clone()).find(|x| x["type"] == "permission").unwrap();
  m.handle_on(&a, json!({ "type": "permission", "sessionId": sid, "blockId": perm["id"], "optionId": "allow" })).await;
  until(|| subs(&subs_a).len() >= 2, 5000).await;
  let seen = subs(&subs_a);
  assert!(seen.last().unwrap()["rev"].as_i64() > seen[0]["rev"].as_i64());
  m.handle_on(&a, json!({ "type": "unobserveSubagent", "sessionId": sid, "subagentId": c1id })).await;
  let count = subs(&subs_a).len();
  p.await.unwrap();
  tokio::time::sleep(std::time::Duration::from_millis(50)).await;
  assert_eq!(subs(&subs_a).len(), count);
  assert!(subs(&subs_b).is_empty());
  // observing another session's subagent id is a no-op, not an error
  m.handle_on(&a, json!({ "type": "observeSubagent", "sessionId": sid, "subagentId": "nonexistent" })).await;
  assert_eq!(subs(&subs_a).len(), count);
  m.dispose().await;
}
