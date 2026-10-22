//! The goal extension (codex-acp / claude-agent-acp dialects of test/fake-agent.ts `FAKE_GOAL`)

use super::*;

fn goal_log(dir: &tempfile::TempDir) -> String {
  std::fs::read_to_string(dir.path().join("goal.log")).unwrap_or_default()
}

fn goal_rows(vw: &Value) -> Vec<Value> {
  vw["turns"]
    .as_array()
    .unwrap()
    .iter()
    .flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default())
    .filter(|b| b["type"] == "goal")
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn codex_goals_show_on_the_view_mark_each_change_and_split_controls_between_requests_and_prompts() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::for_agent(&fake, "codex", json!({ "env": { "FAKE_GOAL": "codex", "FAKE_GOAL_LOG": dir.path().join("goal.log") } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["goalActions"], json!(["set", "pause", "resume", "clear"]));
  expect_absent(view(&s), "goal");

  // set goes out as the `/goal` prompt: a host turn without a bubble, no request
  s.control_goal(GoalAction::Set, Some(" ship it ".into())).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  expect_match(&vw["goal"], json!({ "objective": "ship it", "status": "active", "tokensUsed": 1200 }));
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "/goal ship it", "auto": true, "autoReason": "goal" }));
  assert_eq!(goal_rows(&vw), [json!({ "type": "goal", "event": "set", "objective": "ship it", "tokensUsed": 1200, "timeUsedSeconds": 0 })]);
  assert_eq!(goal_log(&dir), "");

  // pause is a request; its row lands on the last (settled) agent turn
  s.control_goal(GoalAction::Pause, None).await.unwrap();
  until(|| view(&s)["goal"]["status"] == "paused", 5000).await;
  assert_eq!(goal_log(&dir), "pause\n");
  assert_eq!(turn_count(&s), 2);
  assert_eq!(goal_rows(&view(&s)).last().unwrap()["event"], "paused");

  // resume starts a turn on codex-acp, so it is the `/goal resume` prompt
  s.control_goal(GoalAction::Resume, None).await.unwrap();
  until(|| !s.is_running() && view(&s)["goal"]["status"] == "active", 5000).await;
  expect_match(&view(&s)["turns"][2], json!({ "role": "user", "text": "/goal resume", "auto": true, "autoReason": "goal" }));
  assert_eq!(goal_rows(&view(&s)).last().unwrap()["event"], "resumed");

  // A counter-only change marks nothing; meeting it does, and the met goal leaves the view when its turn ends
  prompt(&s, "goal-done").await;
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(goal_rows(&vw).last().unwrap()["event"], "complete");
  expect_absent(&vw, "goal");
  assert!(s.to_record().goal.is_none());

  // clear on codex-acp is a request again
  prompt(&s, "/goal again").await;
  until(|| !s.is_running(), 5000).await;
  assert_eq!(s.to_record().goal.map(|g| g.objective).as_deref(), Some("again"));
  s.control_goal(GoalAction::Clear, None).await.unwrap();
  until(|| view(&s)["goal"].is_null(), 5000).await;
  assert_eq!(goal_log(&dir), "pause\nclear\n");
  assert_eq!(goal_rows(&view(&s)).last().unwrap(), &json!({ "type": "goal", "event": "cleared", "tokensUsed": 1200, "timeUsedSeconds": 0 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_goals_are_met_when_their_turn_ends_and_unadvertised_controls_send_nothing() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_GOAL": "claude", "FAKE_GOAL_LOG": dir.path().join("goal.log") } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["goalActions"], json!(["set", "clear"]));
  // The Stop hook holds the turn until the goal is met, so a turn that ends on its own met it, snapshot or not
  prompt(&s, "/goal fix the build").await;
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(goal_rows(&vw).iter().map(|r| r["event"].clone()).collect::<Vec<_>>(), [json!("set"), json!("complete")]);
  expect_absent(&vw, "goal");
  assert!(s.to_record().goal.is_none());
  // Not advertised: neither a request nor a prompt
  s.control_goal(GoalAction::Pause, None).await.unwrap();
  assert_eq!(turn_count(&s), 2);
  assert_eq!(goal_log(&dir), "");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_claude_turn_keeps_its_goal_and_clear_is_sent_as_the_command() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_GOAL": "claude", "FAKE_SESSION_DIR": dir.path() } }));
  // A goal still active while idle: here a reopened record's, as after a cancel
  let mut record = ran_once(&h, "/tmp").await;
  record.goal = Some(serde_json::from_value(json!({ "objective": "fix the build", "status": "active" })).unwrap());
  let s = reopened(&h, record).await;
  let pending = spawn_prompt(&s, "slow");
  until(|| s.is_running(), 5000).await;
  s.cancel().await;
  pending.await.unwrap();
  expect_match(&view(&s)["goal"], json!({ "objective": "fix the build", "status": "active" }));
  // claude-agent-acp runs clear as the command, so the host sends it as its own turn
  s.control_goal(GoalAction::Clear, None).await.unwrap();
  until(|| !s.is_running() && view(&s)["goal"].is_null(), 5000).await;
  expect_match(view(&s)["turns"].as_array().unwrap().iter().rev().nth(1).unwrap(), json!({ "role": "user", "text": "/goal clear", "auto": true, "autoReason": "goal" }));
  assert_eq!(goal_rows(&view(&s)).last().unwrap()["event"], "cleared");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reopened_record_keeps_its_goal_until_the_agent_reports_another() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let mut record = ran_once(&h, "/tmp").await;
  record.goal = Some(serde_json::from_value(json!({ "objective": "keep going", "status": "paused", "timeUsedSeconds": 40 })).unwrap());
  let s = reopened(&h, record).await;
  let vw = view(&s);
  expect_match(&vw["goal"], json!({ "objective": "keep going", "status": "paused", "timeUsedSeconds": 40 }));
  // An agent without the extension advertises no controls
  expect_absent(&vw, "goalActions");
  assert!(goal_rows(&vw).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claude_clear_during_a_turn_is_steered_in_as_a_request() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let env = json!({ "FAKE_GOAL": "claude", "FAKE_SESSION_DIR": dir.path(), "FAKE_GOAL_LOG": dir.path().join("goal.log") });
  let h = Harness::for_agent(&fake, "claude", json!({ "env": env }));
  let mut record = ran_once(&h, "/tmp").await;
  record.goal = Some(serde_json::from_value(json!({ "objective": "fix the build", "status": "active" })).unwrap());
  let s = reopened(&h, record).await;
  let pending = spawn_prompt(&s, "slow");
  until(|| s.is_running(), 5000).await;
  // The goal holds the running turn until met: a queued clear would wait for good, so it goes over `_session/goal`
  s.control_goal(GoalAction::Clear, None).await.unwrap();
  until(|| view(&s)["goal"].is_null(), 5000).await;
  assert_eq!(goal_log(&dir), "clear
");
  assert!(view(&s)["queued"].as_array().is_none_or(|q| q.is_empty()));
  assert_eq!(goal_rows(&view(&s)).last().unwrap()["event"], "cleared");
  s.cancel().await;
  pending.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codex_goal_continuation_runs_as_a_turn_and_holds_the_queue() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "codex", json!({ "env": { "FAKE_GOAL": "codex" } }));
  let s = started(&h, "/tmp").await;
  // The prompt returns with the goal still active; Codex then goes on by itself with nothing on the wire
  prompt(&s, "/goal continue the port").await;
  until(|| s.is_running() && last_turn(&view(&s))["blocks"].to_string().contains("continued"), 5000).await;
  prompt(&s, "hi").await;
  assert_eq!(view(&s)["queued"].as_array().map(Vec::len), Some(1));
  // Its idle ends that turn: the met goal leaves the view and the queue moves on
  until(|| !s.is_running() && view(&s)["queued"].as_array().is_none_or(|q| q.is_empty()), 5000).await;
  let vw = view(&s);
  expect_absent(&vw, "goal");
  assert_eq!(goal_rows(&vw).iter().map(|r| r["event"].clone()).collect::<Vec<_>>(), [json!("set"), json!("complete")]);
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "hi" }));
}

fn goal_events(vw: &Value) -> Vec<Value> {
  goal_rows(vw).iter().map(|r| r["event"].clone()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_goal_the_agent_leaves_to_the_client_is_continued_in_hidden_turns_until_it_is_met() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_GOAL": "acpira", "FAKE_GOAL_LOG": dir.path().join("goal.log") } }));
  let s = started(&h, "/tmp").await;
  // The set turn ends with the goal active: the host asks for the next goal turn, and again, until the agent meets it
  s.control_goal(GoalAction::Set, Some("port the parser".into())).await.unwrap();
  until(|| goal_log(&dir) == "continue\ncontinue\n" && !s.is_running() && view(&s)["goal"].is_null(), 5000).await;
  let vw = view(&s);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "/goal port the parser", "auto": true, "autoReason": "goal" }));
  for (i, n) in [(2, 1), (4, 2)] {
    expect_match(&vw["turns"][i], json!({ "role": "user", "text": "/goal continue", "auto": true, "autoReason": "goal", "goalContinue": true }));
    assert!(vw["turns"][i + 1]["blocks"].to_string().contains(&format!("continued {n}")), "{}", vw["turns"][i + 1]);
  }
  assert_eq!(turns_in(&vw), 6);
  assert_eq!(goal_events(&vw), [json!("set"), json!("complete")]);
  // Nothing more goes out once it is met
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  assert_eq!((goal_log(&dir).as_str(), turn_count(&s)), ("continue\ncontinue\n", 6));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_queue_goes_before_the_goals_next_turn_and_a_stop_ends_the_run() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let env = json!({ "FAKE_GOAL": "acpira", "FAKE_GOAL_ROUNDS": "5", "FAKE_GOAL_LOG": dir.path().join("goal.log") });
  let h = Harness::new(&fake, json!({ "env": env }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "/goal slow port").await;
  until(|| s.is_running() && goal_log(&dir) == "continue\n", 5000).await;
  // A message sent meanwhile waits for the goal turn, then goes before the next one
  prompt(&s, "hi").await;
  assert_eq!(view(&s)["queued"].as_array().map(Vec::len), Some(1));
  until(|| goal_log(&dir) == "continue\ncontinue\n" && s.is_running(), 5000).await;
  let vw = view(&s);
  expect_match(&vw["turns"][4], json!({ "role": "user", "text": "hi" }));
  expect_match(&vw["turns"][6], json!({ "role": "user", "text": "/goal continue", "autoReason": "goal", "goalContinue": true }));
  // The stop pauses the goal, and the host sends nothing after it
  s.cancel().await;
  until(|| !s.is_running() && view(&s)["goal"]["status"] == "paused", 5000).await;
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  assert_eq!(goal_log(&dir), "continue\ncontinue\n");
  assert_eq!(goal_events(&view(&s)), [json!("set"), json!("paused")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_docks_pause_ends_the_run_and_clear_is_a_request_for_an_agent_that_leaves_continuing_to_the_client() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let env = json!({ "FAKE_GOAL": "acpira", "FAKE_GOAL_ROUNDS": "5", "FAKE_GOAL_LOG": dir.path().join("goal.log") });
  let h = Harness::new(&fake, json!({ "env": env }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "/goal slow port").await;
  until(|| s.is_running() && goal_log(&dir) == "continue\n", 5000).await;
  // Paused during a goal turn: that turn finishes, and no other follows
  s.control_goal(GoalAction::Pause, None).await.unwrap();
  until(|| !s.is_running() && view(&s)["goal"]["status"] == "paused", 5000).await;
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  assert_eq!(goal_log(&dir), "continue\npause\n");
  // The agent starts no turn of its own on any action, so its clear needs none either
  let turns = turn_count(&s);
  s.control_goal(GoalAction::Clear, None).await.unwrap();
  until(|| view(&s)["goal"].is_null(), 5000).await;
  assert_eq!((goal_log(&dir).as_str(), turn_count(&s)), ("continue\npause\nclear\n", turns));
  // A goal set again from the dock runs again
  s.control_goal(GoalAction::Set, Some("port it".into())).await.unwrap();
  until(|| !s.is_running() && view(&s)["goal"].is_null() && goal_log(&dir).ends_with("continue\ncontinue\n"), 10000).await;
}
