//! Antigravity's background commands (`session/late_reply.rs`, fake `FAKE_LATE_REPLY`): the row outlives its turn, and
//! the reply the CLI stores once the command exits is read back from a replay

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_background_command_outlives_its_turn_and_its_late_reply_is_read_back() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "antigravity", json!({ "env": { "FAKE_LATE_REPLY": "1200" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "late-command").await;
  // The turn ended on its own with the command still running: its row stays open in the background, not failed
  let vw = view(&s);
  expect_match(&vw, json!({ "running": false }));
  expect_match(last_turn(&vw), json!({ "stop": "end_turn", "blocks": [{ "status": "in_progress", "background": true }, { "markdown": "Started it." }] }));
  // The command exits, then the model answers that: neither is sent, both are read back
  until(|| agent_text(&last_turn(&view(&s))).contains("Late reply"), 15000).await;
  // The turn ends once the session's time has stood still
  until(|| !s.is_running(), 20000).await;
  let turn = last_turn(&view(&s));
  expect_match(&turn, json!({ "stop": "end_turn" }));
  let blocks = turn["blocks"].as_array().unwrap();
  expect_match(&blocks[0], json!({ "status": "completed" }));
  // The receipt came with the replay: the row shows the command's output alone
  assert!(blocks[0].to_string().contains("late-done") && !blocks[0].to_string().contains("Command exited"));
  // The reply the turn ended on is not repeated
  let texts: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["markdown"].as_str()).collect();
  assert_eq!(texts, ["Started it.", "Late reply: late-done"]);
  // The session goes on as usual
  prompt(&s, "say:next").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  assert_eq!(agent_text(&last_turn(&view(&s))), "next");
}

fn calls(s: &AcpSession) -> Vec<Value> {
  last_turn(&view(s))["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "tool_call").cloned().collect()
}

// The model writes its late reply in steps, each moving the session's time: one turn, open until the reply is whole
#[tokio::test(flavor = "multi_thread")]
async fn a_late_reply_written_in_steps_is_one_turn() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "antigravity", json!({ "env": { "FAKE_LATE_REPLY": "1200" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "late-steps").await;
  // The reply was read while its second call ran: the turn is open, and stays so
  until(|| calls(&s).len() == 2, 15000).await;
  expect_match(&calls(&s)[1], json!({ "status": "in_progress" }));
  assert!(s.is_running());
  until(|| agent_text(&last_turn(&view(&s))).contains("Late reply"), 15000).await;
  assert!(s.is_running());
  // The call read while it ran is closed in place, not left as a background task of its own
  let second = calls(&s)[1].clone();
  expect_match(&second, json!({ "status": "completed" }));
  assert!(second.get("background").is_none_or(|b| b != true), "{second}");
  until(|| !s.is_running(), 20000).await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
}

// A stop ends the late reply's turn for good: what the CLI stores after it is not followed, and the session goes on
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_ends_a_late_reply_and_nothing_reopens_it() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "antigravity", json!({ "env": { "FAKE_LATE_REPLY": "1200" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "late-steps").await;
  until(|| calls(&s).len() == 2, 15000).await;
  assert!(s.is_running());
  s.cancel().await;
  until(|| !s.is_running(), 5000).await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "cancelled" }));
  // The reply is whole by now
  tokio::time::sleep(std::time::Duration::from_millis(5000)).await;
  assert!(!s.is_running());
  assert!(!agent_text(&last_turn(&view(&s))).contains("Late reply"));
  prompt(&s, "say:next").await;
  assert_eq!(agent_text(&last_turn(&view(&s))), "next");
}

// Any other agent's command still running at the end of a turn is a call that never finished
#[tokio::test(flavor = "multi_thread")]
async fn another_agents_unfinished_command_still_fails_with_the_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_LATE_REPLY": "1200" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "late-command").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn", "blocks": [{ "status": "failed" }, { "markdown": "Started it." }] }));
}
