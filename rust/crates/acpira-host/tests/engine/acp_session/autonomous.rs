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
  prompt(&s, "next").await;
  expect_match(&view(&s)["queued"], json!([{ "text": "next" }]));
  // Its result settles the turn and releases the queue
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn", "usage": { "context": { "used": 20 } } }));
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "next" }));
  until(|| view(&s)["turns"][3]["stop"] == "end_turn", 5000).await;
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
