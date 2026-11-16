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
  until(|| !s.is_running(), 5000).await;
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

// Any other agent's command still running at the end of a turn is a call that never finished
#[tokio::test(flavor = "multi_thread")]
async fn another_agents_unfinished_command_still_fails_with_the_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_LATE_REPLY": "1200" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "late-command").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn", "blocks": [{ "status": "failed" }, { "markdown": "Started it." }] }));
}
