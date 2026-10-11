//! Idle release: background sessions with nothing in flight give their agent process back and resume when opened again

use std::time::Duration;

use super::*;

// The native session id lives on the list entry (the session view does not carry it)
fn acp_id_of(m: &Mgr, id: &str) -> String {
  m.sessions().iter().find(|s| s["id"] == id).unwrap()["acpSessionId"].as_str().unwrap().to_owned()
}

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
  let acp_id = acp_id_of(&m, &a);
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
  until(|| !m.m.leased().contains(&a), 5000).await;
  assert!(m.m.leased().contains(&b));
  // Opening it again: a resume of the same native session, history intact, and a follow-up still runs
  m.logs.lock().unwrap().clear();
  m.m.select_session_for(&m.v, &a).await;
  let view = m.active().unwrap();
  assert_eq!(acp_id_of(&m, &a), acp_id);
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
  until(|| m.sessions().iter().any(|s| s["id"] == a.as_str() && s["state"] == "unread"), 5000).await;
  assert_eq!(m.m.release_idle(Duration::ZERO, 0).await, vec![a.clone()]);
  m.dispose().await;
}

// The old process closes the native session before anyone resumes it: a reopen right after the release waits for the
// slow session/close, and the lease stays with this engine until then
#[tokio::test(flavor = "multi_thread")]
async fn reopening_a_released_session_waits_for_the_old_process_to_close() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let close_log = dir.path().join("close.log");
  let env = json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_CLOSE_LOG": close_log, "FAKE_CLOSE_DELAY_MS": "1500" } });
  let m = Mgr::new(&dir.path().join("sessions"), Opts::with_agents(fake.setting(env), "fake"));
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let a = m.active_id().unwrap();
  let acp_id = acp_id_of(&m, &a);
  m.new_session(None).await;
  assert_eq!(m.m.release_idle(Duration::ZERO, 0).await, vec![a.clone()]);
  assert!(m.m.leased().contains(&a), "the lease is held while the old process closes");
  m.m.select_session_for(&m.v, &a).await;
  let closed = std::fs::read_to_string(&close_log).unwrap_or_default();
  assert!(closed.contains(&acp_id), "the reopen ran before the old process closed the native session");
  let view = m.active().unwrap();
  assert_eq!(acp_id_of(&m, &a), acp_id);
  assert_eq!(view["status"], "ready", "{:#?}", m.logs());
  assert!(m.m.leased().contains(&a));
  m.dispose().await;
}

// A reopened session gets its own choices back, not the agent's resume defaults (fake: m1 / high / agent), even when no
// idle release in this engine remembered them: a restarted engine reads them from the record. Reported 2026-10-11 on
// Claude, whose session/resume answers Fable 5.1 Medium / Manual and used to overwrite the session's Opus / bypass
#[tokio::test(flavor = "multi_thread")]
async fn a_session_reopened_by_a_new_engine_gets_its_own_controls_back() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let native = dir.path().join("native");
  std::fs::create_dir(&native).unwrap();
  let env = json!({ "env": { "FAKE_SESSION_DIR": native, "FAKE_RESTORE_DEFAULTS": "1" } });
  let opts = || Opts::with_agents(fake.setting(env.clone()), "fake");
  let m = Mgr::new(&dir.path().join("sessions"), opts());
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "low" })).await;
  m.handle(json!({ "type": "setMode", "id": "plan" })).await;
  m.handle(json!({ "type": "send", "text": "hi" })).await;
  let a = m.active_id().unwrap();
  // The agent's memory now holds other choices, so only the record can bring a's back
  m.new_session(None).await;
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "high" })).await;
  m.handle(json!({ "type": "setMode", "id": "agent" })).await;
  m.handle(json!({ "type": "send", "text": "hello" })).await;
  m.dispose().await;

  let m2 = Mgr::new(&dir.path().join("sessions"), opts());
  m2.init().await;
  m2.m.select_session_for(&m2.v, &a).await;
  let view = m2.active().unwrap();
  assert_eq!(view["status"], "ready", "{:#?}", m2.logs());
  assert!(m2.logs().iter().any(|l| l.contains("session/resume ok")), "{:#?}", m2.logs());
  let values: Vec<Value> = view["controls"]["options"].as_array().unwrap().iter().map(|o| o["value"].clone()).collect();
  assert_eq!(values, [json!("m2"), json!("low")]);
  assert_eq!(view["controls"]["modeId"], "plan");
  // The record keeps them too: the resume defaults never reach the disk
  m2.dispose().await;
  let store = TranscriptStore::new(dir.path().join("sessions"), Arc::new(|_: &str| {}), None);
  let record = v(store.load(&a).await.unwrap());
  assert_eq!(record["controls"]["modeId"], "plan");
  assert_eq!(record["controls"]["options"][0]["value"], "m2");
}
