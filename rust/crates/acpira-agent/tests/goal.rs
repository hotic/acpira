//! Session goals (`goal.rs`) against the scripted model: the codex-acp goal extension on the wire, goal turns opened
//! by `/goal …` prompts, the client's continuations, the tools the model marks the goal with, and what stops a goal

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel, Reply};
use support::Harness;

async fn setup() -> (Harness, MockModel, String) {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }]));
  std::fs::write(h.cwd().join("a.txt"), "alpha\n").unwrap();
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  (h, server, sid)
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

async fn control(h: &Harness, sid: &str, action: &str, objective: Option<&str>) -> Result<Value, i64> {
  let mut params = json!({ "sessionId": sid, "action": action });
  if let Some(o) = objective {
    params["objective"] = json!(o);
  }
  h.conn.request("_session/goal", params).await.map_err(|e| e.code)
}

/// Every goal snapshot the client got, in order (`null` once cleared)
fn snapshots(h: &Harness) -> Vec<Value> {
  h.updates().iter().filter(|u| u["sessionUpdate"] == "session_info_update").filter_map(|u| u["_meta"].get("goal").cloned()).collect()
}

fn last_status(h: &Harness) -> Value {
  snapshots(h).last().map(|g| g["status"].clone()).unwrap_or(Value::Null)
}

fn messages(server: &MockModel, n: usize) -> Vec<Value> {
  server.requests()[n].body["messages"].as_array().unwrap().clone()
}

/// The text of the request's last user message
fn last_user(server: &MockModel, n: usize) -> String {
  let msgs = messages(server, n);
  let m = msgs.iter().rev().find(|m| m["role"] == "user").unwrap();
  m["content"].as_str().map(str::to_owned).unwrap_or_else(|| m["content"].to_string())
}

fn replies(h: &Harness) -> String {
  h.updates().iter().filter(|u| u["sessionUpdate"] == "agent_message_chunk").filter_map(|u| u["content"]["text"].as_str()).collect()
}

fn events(h: &Harness, sid: &str) -> Vec<Value> {
  let file = h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"));
  std::fs::read_to_string(file).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

async fn reopen(h: Harness) -> Harness {
  let Harness { conn, home, cwd, .. } = h;
  drop(conn);
  Harness::start_in(home, cwd).await
}

#[tokio::test]
async fn the_goal_extension_is_advertised_and_controlled_without_a_turn() {
  let (h, server, sid) = setup().await;
  let init = h.conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": {} })).await.unwrap();
  assert_eq!(
    init["_meta"]["goal"],
    json!({ "version": 1, "controlMethod": "_session/goal", "actions": ["set", "pause", "resume", "clear"], "acpira/continue": "/goal continue" })
  );
  let commands = h.updates().into_iter().find(|u| u["sessionUpdate"] == "available_commands_update").unwrap();
  assert_eq!(commands["availableCommands"][1]["name"], "goal");

  control(&h, &sid, "set", Some(" ship the parser ")).await.unwrap();
  let g = snapshots(&h).last().unwrap().clone();
  assert_eq!((g["objective"].as_str(), g["status"].as_str(), g["tokensUsed"].as_u64()), (Some("ship the parser"), Some("active"), Some(0)));
  control(&h, &sid, "pause", None).await.unwrap();
  assert_eq!(last_status(&h), "paused");
  control(&h, &sid, "resume", None).await.unwrap();
  assert_eq!(last_status(&h), "active");
  control(&h, &sid, "clear", None).await.unwrap();
  assert_eq!(snapshots(&h).last(), Some(&Value::Null));
  // Pausing nothing changes nothing, and says nothing
  let n = snapshots(&h).len();
  control(&h, &sid, "pause", None).await.unwrap();
  assert_eq!(snapshots(&h).len(), n);
  assert_eq!(control(&h, &sid, "set", None).await, Err(-32602));
  assert_eq!(control(&h, &sid, "launch", None).await, Err(-32602));
  // None of it called the model
  assert!(server.requests().is_empty());

  // The commands that only look at or change the goal answer at once
  prompt(&h, &sid, "/goal").await;
  prompt(&h, &sid, "/goal pause").await;
  assert_eq!(replies(&h), "No goal is set. Start one with /goal <objective>; /goal pause, /goal resume and /goal clear control it.There is no active goal to pause.");
  assert!(server.requests().is_empty());
}

#[tokio::test]
async fn a_goal_runs_across_turns_until_the_model_marks_it_complete() {
  let (h, server, sid) = setup().await;
  // The set turn: the note in front of the objective, a tool, a reply that leaves the goal active
  server.push(mock::tools(&[("c1", "read", json!({ "path": "a.txt" }))])).push(mock::text("Read it; the edit comes next."));
  assert_eq!(prompt(&h, &sid, "/goal make a.txt say beta").await["stopReason"], "end_turn");
  let first = last_user(&server, 0);
  assert!(first.starts_with("<goal>\nThe user set a goal for this session") && first.ends_with("</goal>\n\nmake a.txt say beta"), "{first}");
  let g = snapshots(&h).last().unwrap().clone();
  assert_eq!((g["status"].as_str(), g["tokensUsed"].as_u64()), (Some("active"), Some(220)), "two calls of 100 in, 10 out");

  // The client's continuation: the note quotes the objective and the budget; the model marks the goal complete
  server.push(mock::tools(&[("c2", "update_goal", json!({ "status": "complete" }))])).push(mock::text("Done: a.txt says beta."));
  assert_eq!(prompt(&h, &sid, "/goal continue").await["stopReason"], "end_turn");
  let cont = last_user(&server, 2);
  assert!(cont.starts_with("<goal>\nContinue working toward the active goal.") && cont.contains("<objective>\nmake a.txt say beta\n</objective>"), "{cont}");
  assert!(cont.contains("- Tokens used: 220 (no budget)") && cont.contains("No-progress check") && cont.contains("Completion audit"), "{cont}");
  let msgs = messages(&server, 3);
  let result: Value = serde_json::from_str(msgs[msgs.len() - 1]["content"].as_str().unwrap()).unwrap();
  assert_eq!((result["goal"]["status"].as_str(), result["remainingTokens"].clone()), (Some("complete"), Value::Null), "{result}");
  assert_eq!(last_status(&h), "complete");
  // The turns build on one another: every request extends the one before it
  for n in 1..4 {
    let (a, b) = (messages(&server, n - 1), messages(&server, n));
    assert!(b.len() > a.len());
    for (x, y) in a.iter().zip(&b) {
      assert_eq!(x.to_string(), y.to_string(), "request {n} changed the prefix");
    }
  }
  // The goal's tool shows no row: the goal's own rows stand for it
  let rows: Vec<Value> = h.updates().into_iter().filter(|u| u["sessionUpdate"] == "tool_call").collect();
  assert_eq!(rows.len(), 1, "{rows:?}");
  assert_eq!(rows[0]["kind"], "read");

  // A continuation after the goal was met calls no model, and tells the client there is nothing to continue
  prompt(&h, &sid, "/goal continue").await;
  assert_eq!(server.requests().len(), 4);
  assert_eq!(snapshots(&h).last(), Some(&Value::Null));

  // The list names the session after the objective; the replay shows the user's /goal, not the continuations
  let h = reopen(h).await;
  let listed = h.conn.request("session/list", json!({ "cwd": h.cwd() })).await.unwrap();
  assert_eq!(listed["sessions"][0]["title"], "make a.txt say beta");
  h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  let sent: Vec<Value> = h.updates().into_iter().filter(|u| u["sessionUpdate"] == "user_message_chunk").map(|u| u["content"]["text"].clone()).collect();
  assert_eq!(sent, [json!("/goal make a.txt say beta")]);
  // A met goal is not published again
  assert!(snapshots(&h).is_empty());
}

#[tokio::test]
async fn continuations_without_a_tool_call_block_the_goal() {
  let (h, server, sid) = setup().await;
  control(&h, &sid, "set", Some("find the bug")).await.unwrap();
  for n in 1..=3 {
    server.push(mock::text("Still thinking."));
    prompt(&h, &sid, "/goal continue").await;
    assert_eq!(last_status(&h), if n < 3 { "active" } else { "blocked" }, "after continuation {n}");
  }
  prompt(&h, &sid, "/goal continue").await;
  assert_eq!(server.requests().len(), 3, "a blocked goal is not continued");
  // A resume starts the count again
  server.push(mock::tools(&[("c1", "read", json!({ "path": "a.txt" }))])).push(mock::text("Found it."));
  prompt(&h, &sid, "/goal resume").await;
  let resumed = last_user(&server, 3);
  assert!(resumed.starts_with("<goal>\nThe user resumed the goal below") && resumed.contains("<objective>\nfind the bug\n</objective>"), "{resumed}");
  assert_eq!(last_status(&h), "active");
}

#[tokio::test]
async fn a_stop_pauses_the_goal_and_a_rejected_action_too() {
  let (h, server, sid) = setup().await;
  server.push(Reply::Hang);
  let conn = h.conn.clone();
  let params = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "/goal port the parser" }] });
  let turn = tokio::spawn(async move { conn.request("session/prompt", params).await.unwrap() });
  let t = Instant::now();
  while server.hanging() == 0 {
    assert!(t.elapsed() < Duration::from_secs(10), "the call never started");
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
  h.conn.notify("session/cancel", json!({ "sessionId": sid }));
  assert_eq!(turn.await.unwrap()["stopReason"], "cancelled");
  assert_eq!(last_status(&h), "paused");

  // Resumed, the turn ends on a rejected command: the goal waits for the user again
  *h.client.answer.lock() = support::Answer::Reject;
  server.push(mock::tools(&[("c1", "bash", json!({ "command": "touch x" }))]));
  prompt(&h, &sid, "/goal resume").await;
  assert_eq!(h.client.permissions.lock().len(), 1);
  assert_eq!(last_status(&h), "paused");
}

#[tokio::test]
async fn a_spent_budget_limits_the_goal_and_tells_the_turn_to_wrap_up() {
  let (h, server, sid) = setup().await;
  server.push(mock::tools(&[("c1", "create_goal", json!({ "objective": "make it work", "token_budget": 150 }))]));
  server.push(mock::tools(&[("c2", "read", json!({ "path": "a.txt" }))]));
  server.push(mock::tools(&[("c3", "read", json!({ "path": "a.txt" })), ("c4", "create_goal", json!({ "objective": "another" }))]));
  server.push(mock::text("Out of budget: the parser half works; the lexer is next."));
  assert_eq!(prompt(&h, &sid, "Keep going until it works, within 150 tokens").await["stopReason"], "end_turn");
  // The goal the model created counts from the call after it
  let created = messages(&server, 1);
  let created: Value = serde_json::from_str(created.last().unwrap()["content"].as_str().unwrap()).unwrap();
  assert_eq!((created["goal"]["status"].as_str(), created["remainingTokens"].as_u64()), (Some("active"), Some(150)), "{created}");
  // Two calls later the budget is spent: the next call is told to wrap up, behind the round's results
  let msgs = messages(&server, 3);
  let n = msgs.len();
  assert_eq!((msgs[n - 3]["role"].as_str(), msgs[n - 2]["role"].as_str(), msgs[n - 1]["role"].as_str()), (Some("tool"), Some("tool"), Some("user")));
  assert!(msgs[n - 2]["content"].as_str().unwrap().contains("Cannot create a goal: this session already has an unfinished one"), "{}", msgs[n - 2]);
  let note = msgs[n - 1]["content"].as_str().unwrap();
  assert!(note.starts_with("<steer>\nThe goal has reached its token budget") && note.contains("220 tokens used of 150 (0 left)"), "{note}");
  assert!(!last_user(&server, 2).contains("token budget"), "told once, in front of the call after the budget ran out");
  let g = snapshots(&h).last().unwrap().clone();
  assert_eq!((g["status"].as_str(), g["tokenBudget"].as_u64(), g["tokensUsed"].as_u64()), (Some("budgetLimited"), Some(150), Some(330)));
  // A budget-limited goal is not continued
  prompt(&h, &sid, "/goal continue").await;
  assert_eq!(server.requests().len(), 4);
  // Nor does a later, unrelated turn count toward it: only the turn that ran out of budget did, while it wrapped up
  let goal_events = |h: &Harness| events(h, &sid).into_iter().filter(|e| e["type"] == "goal").count();
  let (logged, published) = (goal_events(&h), snapshots(&h).len());
  server.push(mock::text("4."));
  prompt(&h, &sid, "What is 2 + 2?").await;
  assert_eq!(server.requests().len(), 5);
  assert_eq!((goal_events(&h), snapshots(&h).len()), (logged, published), "the goal did not move");
  assert_eq!(snapshots(&h).last().unwrap()["tokensUsed"].as_u64(), Some(330));
}

#[tokio::test]
async fn a_goal_met_in_the_call_that_spent_its_budget_is_not_told_to_wrap_up() {
  let (h, server, sid) = setup().await;
  server.push(mock::tools(&[("c1", "create_goal", json!({ "objective": "make it work", "token_budget": 100 }))]));
  // This call's own tokens reach the budget, and it marks the goal complete
  server.push(mock::tools(&[("c2", "update_goal", json!({ "status": "complete" }))]));
  server.push(mock::text("Done: it works."));
  prompt(&h, &sid, "Keep going until it works, within 100 tokens").await;
  assert_eq!(server.requests().len(), 3);
  // The report call gets update_goal's result, not a wrap-up note contradicting it
  let msgs = messages(&server, 2);
  let last = msgs.last().unwrap();
  assert_eq!(last["role"], "tool", "{last}");
  assert!(last["content"].as_str().unwrap().contains("\"status\":\"complete\""), "{last}");
  assert!(!msgs.iter().any(|m| m["content"].as_str().is_some_and(|c| c.contains("reached its token budget"))));
  assert_eq!(last_status(&h), "complete");
}

#[tokio::test]
async fn a_reopened_active_goal_comes_back_paused() {
  let (h, server, sid) = setup().await;
  server.push(mock::tools(&[("c1", "read", json!({ "path": "a.txt" }))])).push(mock::text("Started."));
  prompt(&h, &sid, "/goal port the parser").await;
  assert_eq!(last_status(&h), "active");
  let h = reopen(h).await;
  h.conn.request("session/resume", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  let g = snapshots(&h).last().unwrap().clone();
  assert_eq!((g["objective"].as_str(), g["status"].as_str(), g["tokensUsed"].as_u64()), (Some("port the parser"), Some("paused"), Some(220)));
  let logged: Vec<Value> = events(&h, &sid).into_iter().filter(|e| e["type"] == "goal").collect();
  assert_eq!(logged.last().unwrap()["goal"]["status"], "paused");
  // One event for the set and one for the turn's counters, then the pause: not one per model call
  assert_eq!(logged.len(), 3, "{logged:?}");
}

#[tokio::test]
async fn plan_mode_pauses_a_goal_and_raw_mode_takes_none() {
  let (h, server, sid) = setup().await;
  h.conn.request("session/set_mode", json!({ "sessionId": sid, "modeId": "plan" })).await.unwrap();
  server.push(mock::text("Here is how I would do it."));
  prompt(&h, &sid, "/goal port the parser").await;
  let first = last_user(&server, 0);
  assert!(first.starts_with("<mode>\nPlan mode is on.") && first.contains("</mode>\n\n<goal>\nThe user set a goal"), "{first}");
  // The turn ended waiting for the plan's approval
  assert_eq!(last_status(&h), "paused");

  let raw = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  h.conn.request("session/set_mode", json!({ "sessionId": raw, "modeId": "raw" })).await.unwrap();
  prompt(&h, &raw, "/goal port the parser").await;
  assert!(replies(&h).ends_with("Goals need Agent or Plan mode: Raw mode gives the model no tools to work with."), "{}", replies(&h));
  assert_eq!(server.requests().len(), 1);
}
