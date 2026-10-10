//! Sessions on disk: a turn's history, controls and request records survive the agent process, and come back through
//! `session/list`, `session/resume` (no replay) and `session/load` (replayed as updates)

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::Harness;

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

fn events(h: &Harness, sid: &str) -> Vec<Value> {
  let file = h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"));
  std::fs::read_to_string(file).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

/// End the agent process and start another on the same data root and project
async fn reopen(h: Harness) -> Harness {
  let Harness { conn, home, cwd, .. } = h;
  drop(conn);
  Harness::start_in(home, cwd).await
}

#[tokio::test]
async fn a_session_comes_back_after_the_agent_restarts() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }, { "id": "m2" }]));
  std::fs::write(h.cwd().join("a.txt"), "alpha\n").unwrap();
  // A session that never got a prompt leaves no file and no list entry
  h.new_session().await;
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "approval", "value": "full" })).await.unwrap();
  server.push(mock::tools(&[("c1", "read", json!({ "path": "a.txt" }))]));
  server.push(mock::text("It says alpha."));
  assert_eq!(prompt(&h, &sid, "what is in a.txt").await["stopReason"], "end_turn");
  let first = server.requests()[1].body["messages"].clone();

  // One request record per model call, each with the prompt it was built from and the model's usage
  let evs = events(&h, &sid);
  assert_eq!(evs[0]["type"], "session");
  let reqs: Vec<&Value> = evs.iter().filter(|e| e["type"] == "request").collect();
  assert_eq!(reqs.len(), 2);
  assert_eq!(reqs[0]["model"], "mock/m1");
  assert_eq!(reqs[0]["usage"]["input"], 100);
  assert!(reqs[0]["prompt"]["digest"].as_str().unwrap().len() == 12);
  assert!(evs.iter().all(|e| e["type"] != "view"), "nothing changed between the two calls");

  let h = reopen(h).await;
  let other = tempfile::tempdir().unwrap();
  let listed = h.conn.request("session/list", json!({ "cwd": h.cwd() })).await.unwrap();
  let sessions = listed["sessions"].as_array().unwrap();
  assert_eq!(sessions.len(), 1, "{listed}");
  assert_eq!(sessions[0]["sessionId"], sid);
  assert_eq!(sessions[0]["title"], "what is in a.txt");
  assert!(listed.get("nextCursor").is_none());
  let elsewhere = h.conn.request("session/list", json!({ "cwd": other.path() })).await.unwrap();
  assert_eq!(elsewhere["sessions"], json!([]));

  // Resume brings the controls back without replaying anything
  let resumed = h.conn.request("session/resume", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  let approval = resumed["configOptions"].as_array().unwrap().iter().find(|o| o["id"] == "approval").unwrap().clone();
  assert_eq!(approval["currentValue"], "full");
  assert!(h.updates().is_empty());

  // The follow-up's request carries the earlier history byte for byte, and a model switch is a view change
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/m2" })).await.unwrap();
  server.push(mock::text("Still alpha."));
  assert_eq!(prompt(&h, &sid, "and now?").await["stopReason"], "end_turn");
  let third = server.requests()[2].body["messages"].clone();
  let (a, b) = (first.as_array().unwrap(), third.as_array().unwrap());
  assert!(b.len() > a.len());
  for (x, y) in a.iter().zip(b) {
    assert_eq!(x.to_string(), y.to_string(), "the reopened history differs from the original");
  }
  // The new process starts without a previous view, so the switch shows as a fresh request with the new model
  let reqs: Vec<Value> = events(&h, &sid).into_iter().filter(|e| e["type"] == "request").collect();
  assert_eq!(reqs.len(), 3);
  assert_eq!(reqs[2]["model"], "mock/m2");

  // Load replays the transcript: both prompts, the tool call with its result, the answers
  let h = reopen(h).await;
  h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  let ups = h.updates();
  let users: Vec<&str> = ups.iter().filter(|u| u["sessionUpdate"] == "user_message_chunk").map(|u| u["content"]["text"].as_str().unwrap()).collect();
  assert_eq!(users, ["what is in a.txt", "and now?"]);
  let said: String = ups.iter().filter(|u| u["sessionUpdate"] == "agent_message_chunk").map(|u| u["content"]["text"].as_str().unwrap()).collect();
  assert_eq!(said, "It says alpha.Still alpha.");
  assert!(ups.iter().any(|u| u["sessionUpdate"] == "tool_call"));
  assert!(ups.iter().any(|u| u["sessionUpdate"] == "tool_call_update" && u["status"] == "completed"));
  assert!(ups.iter().all(|u| u["sessionUpdate"] != "usage_update" || u.get("size").is_some()));

  let missing = h.conn.request("session/load", json!({ "sessionId": "nope", "cwd": h.cwd(), "mcpServers": [] })).await.unwrap_err();
  assert_eq!(missing.code, -32002);
}

#[tokio::test]
async fn a_view_change_is_recorded_when_the_model_switches_mid_session() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }, { "id": "m2" }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  server.push(mock::text("one"));
  prompt(&h, &sid, "first").await;
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/m2" })).await.unwrap();
  server.push(mock::text("two"));
  prompt(&h, &sid, "second").await;
  let views: Vec<Value> = events(&h, &sid).into_iter().filter(|e| e["type"] == "view").collect();
  assert_eq!(views.len(), 1, "{views:?}");
  assert_eq!(views[0]["changed"], json!(["model"]));
  assert_eq!((views[0]["from"]["model"].clone(), views[0]["to"]["model"].clone()), (json!("mock/m1"), json!("mock/m2")));
}

#[tokio::test]
async fn a_catalogued_model_gets_its_window_and_a_priced_request_record() {
  let h = Harness::start().await;
  let server = MockModel::start();
  // A hand-entered id with no limits: the catalogue knows it by its normalized name
  h.providers(&server.base_url(), json!([{ "id": "Claude-Opus-4.6" }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  server.push(mock::text("ok"));
  prompt(&h, &sid, "hi").await;
  assert!(h.updates().iter().any(|u| u["sessionUpdate"] == "usage_update" && u["size"] == 1_000_000));
  // The catalogue's output limit is an estimate: it is not sent as max_tokens
  assert!(server.requests()[0].body.get("max_tokens").is_none());
  let req = events(&h, &sid).into_iter().find(|e| e["type"] == "request").unwrap();
  assert!(req["cost"].as_f64().is_some_and(|c| c > 0.0), "{req}");
}
