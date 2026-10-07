//! Idle release: background sessions with nothing in flight give their agent process back and resume when opened again

use std::time::Duration;

use super::*;

fn released_logs(m: &Mgr) -> usize {
  m.logs().iter().filter(|l| l.contains("idle, agent process released")).count()
}

// A session nobody looks at ends its process after the wait (the one on screen never does); its list entry and lease
// stay consistent, and opening it resumes the same native session with its history and a working follow-up
#[tokio::test(flavor = "multi_thread")]
async fn an_unseen_idle_session_releases_its_process_and_resumes_when_opened() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native } })), "fake"));
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let a = m.active_id().unwrap();
  let acp_id = m.view_of(&a).unwrap()["acpSessionId"].clone();
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hello" })).await;
  let b = m.active_id().unwrap();
  assert_ne!(a, b);
  // Not unseen for long enough, and within the kept few: nothing goes
  assert!(m.m.release_idle(Duration::from_secs(3600), 4).await.is_empty());
  // Past the wait: the background one goes, the one on screen stays
  assert_eq!(m.m.release_idle(Duration::ZERO, 0).await, vec![a.clone()]);
  assert_eq!(released_logs(&m), 1);
  assert!(m.view_of(&a).is_none(), "released session is no longer live");
  assert!(m.view_of(&b).is_some());
  assert!(m.session_ids().contains(&a), "the list keeps the released session");
  assert!(!m.m.leased().contains(&a), "the released session's lease goes with its process");
  assert!(m.m.leased().contains(&b));
  // Opening it again: a resume of the same native session, history intact, and a follow-up still runs
  m.logs.lock().unwrap().clear();
  m.m.select_session_for(&m.v, &a).await;
  let view = m.active().unwrap();
  assert_eq!(view["acpSessionId"], acp_id);
  assert_eq!(view["status"], "ready", "{:#?}", m.logs());
  assert!(m.logs().iter().any(|l| l.contains("session/resume ok")), "{:#?}", m.logs());
  assert_eq!(turns_len(Some(view)), 2);
  m.handle(json!({ "type": "send", "text": "again" })).await;
  assert_eq!(turns_len(m.active()), 4);
  // b is now the unseen one
  assert_eq!(m.m.release_idle(Duration::ZERO, 0).await, vec![b.clone()]);
  m.dispose().await;
}

// A turn still running in the background keeps its process; once it ends, the session can go
#[tokio::test(flavor = "multi_thread")]
async fn a_running_background_session_keeps_its_process_until_the_turn_ends() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let agents = fake.setting(json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_SLOW_STEP_MS": "200" } }));
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(agents, "fake"));
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let a = m.active_id().unwrap();
  let sending = m.spawn_handle(json!({ "type": "send", "text": "slow" }));
  until(|| m.view_of(&a).is_some_and(|v| v["turns"].as_array().is_some_and(|t| t.len() == 4)), 5000).await;
  m.new_session(None).await;
  assert!(m.m.release_idle(Duration::ZERO, 0).await.is_empty(), "a running turn is never cut");
  m.handle(json!({ "type": "stop", "sessionId": a })).await;
  sending.await.unwrap();
  until(|| m.sessions().iter().any(|s| s["id"] == a.as_str() && s["state"].is_null()), 5000).await;
  assert_eq!(m.m.release_idle(Duration::ZERO, 0).await, vec![a.clone()]);
  m.dispose().await;
}
