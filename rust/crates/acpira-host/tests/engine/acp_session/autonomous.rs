//! Claude's autonomous cycles: a task-notification followup streams after `end_turn` with no prompt on the wire
//! (`vendors::claude_autonomous`, fake "autonomous")

use super::*;

fn claude(fake: &FakeAgent) -> Harness {
  Harness::for_agent(fake, "claude", json!({}))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claude_followup_after_end_turn_runs_the_last_turn_until_its_result() {
  let fake = fake_or_skip!();
  let h = claude(&fake);
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous").await;
  // The followup reopens the finished turn: running, no stop, and a prompt sent meanwhile only queues
  until(|| agent_text(&view(&s)["turns"][1]).contains("followup"), 5000).await;
  let vw = view(&s);
  assert_eq!(vw["running"], true);
  expect_absent(&vw["turns"][1], "stop");
  // The reply the turn ended on stays its reply; the cycle's prose starts a stretch of its own
  expect_match(&vw["turns"][1]["blocks"], json!([{ "markdown": "waiting in background", "phase": "final" }, { "type": "text" }]));
  expect_absent(&vw["turns"][1]["blocks"][1], "phase");
  prompt(&s, "next").await;
  expect_match(&view(&s)["queued"], json!([{ "text": "next" }]));
  // Its result settles the turn and releases the queue
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn", "usage": { "context": { "used": 20 } } }));
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "next" }));
  until(|| view(&s)["turns"][3]["stop"] == "end_turn", 5000).await;
}

// A cycle a scheduled wakeup started: its result names no origin, only its cost. It spun until a stop
#[tokio::test(flavor = "multi_thread")]
async fn a_cycle_whose_result_names_no_origin_still_settles() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_AUTONOMOUS_NO_ORIGIN": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous").await;
  until(|| agent_text(&view(&s)["turns"][1]).contains("followup"), 5000).await;
  until(|| !s.is_running(), 5000).await;
  expect_match(&view(&s)["turns"][1], json!({ "stop": "end_turn", "usage": { "context": { "used": 20 } } }));
}

#[tokio::test(flavor = "multi_thread")]
async fn stopping_an_autonomous_cycle_settles_it_without_a_prompt_response() {
  let fake = fake_or_skip!();
  let h = claude(&fake);
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous-hang").await;
  until(|| s.is_running(), 5000).await;
  s.cancel().await;
  assert!(!s.is_running());
  expect_match(&view(&s)["turns"][1], json!({ "stop": "cancelled" }));
}

// claude-agent-acp 0.93.0 acknowledges a stopped background Bash after end_turn. As agent text it opened a cycle no result
// ever closed (the turn spun until a stop); as the `notice` the host now asks for, it is a row on the settled turn
#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_task_notice_after_end_turn_does_not_reopen_the_turn() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_NOTICES": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "task-stopped").await;
  until(|| view(&s)["turns"][1]["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["type"] == "notice")), 5000).await;
  tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  assert!(!s.is_running(), "the notice must not reopen the turn: {}", view(&s));
  let vw = view(&s);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
  expect_match(&vw["turns"][1]["blocks"][1], json!({ "type": "notice", "severity": "warning", "info": true, "title": "Task stopped by user" }));
  assert_eq!(agent_text(&vw["turns"][1]), "waiting in background");
}

#[tokio::test(flavor = "multi_thread")]
async fn another_agent_s_late_prose_does_not_reopen_the_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous").await;
  until(|| agent_text(&view(&s)["turns"][1]).contains("followup"), 5000).await;
  assert!(!s.is_running());
  expect_match(&view(&s)["turns"][1], json!({ "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn review_a_late_nested_child_tool_does_not_open_a_root_autonomous_cycle() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_AUTONOMOUS_CHILD": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous").await;
  tokio::time::sleep(std::time::Duration::from_millis(500)).await;
  assert!(!s.is_running(), "nested child traffic must not reopen the finished root: {}", view(&s));
}

#[tokio::test(flavor = "multi_thread")]
async fn review_an_autonomous_result_after_stop_does_not_reopen_the_cancelled_turn() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_AUTONOMOUS_AFTER_STOP": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous").await;
  until(|| s.is_running(), 5000).await;
  s.cancel().await;
  expect_match(&view(&s)["turns"][1], json!({ "stop": "cancelled" }));
  // This packet represents queued output that the adapter flushes after receiving session/cancel.
  until(|| agent_text(&view(&s)["turns"][1]).contains("queued tail"), 5000).await;
  until(|| !s.is_running(), 5000).await;
  expect_match(&view(&s)["turns"][1], json!({ "stop": "cancelled" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_background_task_s_cycle_opens_after_the_first_one_s_result() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_AUTONOMOUS_TWICE": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous").await;
  // The first cycle settles on its result; the second one reopens the turn until its own result
  until(|| s.is_running() && agent_text(&view(&s)["turns"][1]).contains("second"), 5000).await;
  until(|| !s.is_running(), 5000).await;
  expect_match(&view(&s)["turns"][1], json!({ "stop": "end_turn", "usage": { "context": { "used": 30 } } }));
}

// A cycle is no turn of the adapter's to steer into: from claude-agent-acp 0.95.0 a steer goes out as an ordinary prompt at
// once, and nothing is stopped
#[tokio::test(flavor = "multi_thread")]
async fn a_steer_during_an_autonomous_cycle_goes_out_as_a_prompt_without_a_stop() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_STEERING": "1", "FAKE_AGENT_VERSION": "0.95.0" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous-hang").await;
  until(|| s.is_running() && agent_text(&view(&s)["turns"][1]).contains("followup"), 5000).await;
  s.steer_prompt("next".into(), vec![]).await.unwrap();
  let vw = view(&s);
  expect_absent(&vw, "queued");
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "next" }));
  // The prompt owns the session from here: its own response settles it
  until(|| !s.is_running(), 5000).await;
  expect_match(&view(&s)["turns"][3], json!({ "stop": "end_turn" }));
}

// An older adapter never settles a prompt sent mid-cycle: the steer waits in the queue for the cycle's end
#[tokio::test(flavor = "multi_thread")]
async fn a_steer_during_an_autonomous_cycle_stays_queued_on_an_older_adapter() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_STEERING": "1", "FAKE_AGENT_VERSION": "0.94.0" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "autonomous").await;
  until(|| s.is_running() && agent_text(&view(&s)["turns"][1]).contains("followup"), 5000).await;
  s.steer_prompt("next".into(), vec![]).await.unwrap();
  expect_match(&view(&s)["queued"], json!([{ "text": "next" }]));
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  expect_match(&view(&s)["turns"][1], json!({ "stop": "end_turn" }));
}
