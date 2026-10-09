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

  // set goes out as the `/goal` prompt: a host turn, no request
  s.control_goal(GoalAction::Set, Some(" ship it ".into())).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  expect_match(&vw["goal"], json!({ "objective": "ship it", "status": "active", "tokensUsed": 1200 }));
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "/goal ship it" }));
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
  expect_match(&view(&s)["turns"][2], json!({ "role": "user", "text": "/goal resume" }));
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
async fn claude_goals_clear_through_the_command_and_unadvertised_controls_send_nothing() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_GOAL": "claude", "FAKE_GOAL_LOG": dir.path().join("goal.log") } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["goalActions"], json!(["set", "clear"]));
  prompt(&s, "/goal fix the build").await;
  until(|| !s.is_running(), 5000).await;
  expect_match(&view(&s)["goal"], json!({ "objective": "fix the build", "status": "active" }));
  // Not advertised: neither a request nor a prompt
  s.control_goal(GoalAction::Pause, None).await.unwrap();
  assert_eq!(turn_count(&s), 2);
  // claude-agent-acp runs clear as the command, so the host sends it as its own turn
  s.control_goal(GoalAction::Clear, None).await.unwrap();
  until(|| !s.is_running() && view(&s)["goal"].is_null(), 5000).await;
  expect_match(&view(&s)["turns"][2], json!({ "role": "user", "text": "/goal clear" }));
  assert_eq!(goal_log(&dir), "");
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
