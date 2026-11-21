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
  // The session list shows it as background work, the same mark as any other agent's live task
  assert_eq!(s.list_state(), Some(acpira_shared::transcript::SummaryState::Background));
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
  assert_eq!(s.list_state(), None);
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

pub(super) fn texts_of(turn: &Value, kind: &str, field: &str) -> Vec<String> {
  turn["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == kind).filter_map(|b| b[field].as_str()).map(str::to_owned).collect()
}

// agy 1.5.12 streams the exit and the reply as they happen: they land once, in one detached turn, and no replay is read
// (a load would restart the stream and bring the reply's steps a second time)
#[tokio::test(flavor = "multi_thread")]
async fn a_late_reply_that_streams_lands_once_and_reads_no_replay() {
  let fake = fake_or_skip!();
  let logs = tempfile::tempdir().unwrap();
  let meta = logs.path().join("meta.log");
  let h = Harness::for_agent(&fake, "antigravity", json!({ "env": { "FAKE_LATE_REPLY": "1200", "FAKE_LATE_LIVE": "1", "FAKE_META_LOG": meta } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "late-steps").await;
  // The streamed thought and call open the late reply's turn
  until(|| calls(&s).len() == 2, 15000).await;
  assert!(s.is_running());
  until(|| agent_text(&last_turn(&view(&s))).contains("Late reply"), 15000).await;
  until(|| !s.is_running(), 20000).await;
  let turn = last_turn(&view(&s));
  expect_match(&turn, json!({ "stop": "end_turn" }));
  assert_eq!(texts_of(&turn, "text", "markdown"), ["Started it.", "Late reply: late-done"]);
  assert_eq!(texts_of(&turn, "thought", "text"), ["The command exited."]);
  // The streamed exit settled the command's task and closed its row, receipt stripped
  let first = &calls(&s)[0];
  expect_match(first, json!({ "status": "completed" }));
  assert!(first.to_string().contains("late-done") && !first.to_string().contains("Command exited"), "{first}");
  expect_match(&calls(&s)[1], json!({ "status": "completed" }));
  let loads = std::fs::read_to_string(&meta).unwrap_or_default().lines().filter(|l| l.contains(r#""method":"load""#)).count();
  assert_eq!(loads, 0, "{:?}", h.logs());
  prompt(&s, "say:next").await;
  assert_eq!(agent_text(&last_turn(&view(&s))), "next");
}

// A stop ends a streaming late reply for good: what still streams of it is dropped, nothing reopens the turn
#[tokio::test(flavor = "multi_thread")]
async fn a_stop_ends_a_streaming_late_reply_and_drops_what_follows() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "antigravity", json!({ "env": { "FAKE_LATE_REPLY": "1200", "FAKE_LATE_LIVE": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "late-steps").await;
  until(|| calls(&s).len() == 2, 15000).await;
  assert!(s.is_running());
  s.cancel().await;
  until(|| !s.is_running(), 5000).await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "cancelled" }));
  // The fake streams the rest of the reply regardless
  tokio::time::sleep(std::time::Duration::from_millis(4000)).await;
  assert!(!s.is_running());
  assert!(!agent_text(&last_turn(&view(&s))).contains("Late reply"));
  prompt(&s, "say:next").await;
  assert_eq!(agent_text(&last_turn(&view(&s))), "next");
}

// A turn that leaves the context over budget while its command runs in the background: the auto /compact waits for that
// command's late reply and follows it, so the model wakes up to the history it had
#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_waits_for_a_background_commands_late_reply() {
  let fake = fake_or_skip!();
  let env = json!({ "env": { "FAKE_LATE_REPLY": "1200", "FAKE_LATE_LIVE": "1", "FAKE_COMPACTION": "antigravity" } });
  let mut h = Harness::for_agent(&fake, "antigravity", env);
  h.deps.policy = Arc::new(TestPolicy {
    compaction: Some(Arc::new(|| acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: true })),
    ..TestPolicy::default()
  });
  let s = started(&h, "/tmp").await;
  let users = || -> Vec<String> {
    view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").filter_map(|t| t["text"].as_str()).map(str::to_owned).collect()
  };
  prompt(&s, "late-big").await;
  until(|| h.logs().iter().any(|l| l.contains("auto compaction waits for background commands")), 5000).await;
  assert_eq!(users(), ["late-big"]);
  until(|| agent_text(&last_turn(&view(&s))).contains("Late reply"), 15000).await;
  assert_eq!(users(), ["late-big"]);
  // The late reply's turn ends, and the compaction it held back goes out
  until(|| users().len() == 2 && !s.is_running(), 30000).await;
  assert_eq!(users(), ["late-big", "/compact"]);
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn", "blocks": [{ "type": "compaction", "status": "completed" }] }));
}
