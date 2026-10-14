//! Steering (`_session/steering`) against the scripted model: a message sent while a turn runs reaches the model with
//! the turn's next call, behind the tool round in flight or behind a reply that would have ended the turn; one no call
//! carried stays in the history for the next turn, and a turn that takes no messages hands it back

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_agent::mock::{self, Gate, MockModel, Reply};
use support::{Answer, Harness};

async fn setup(format: &str, model: &str) -> (Harness, MockModel, String) {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers_in(format, &server.base_url(), json!([{ "id": model }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  (h, server, sid)
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

/// A prompt left running, for the test to steer into
fn start(h: &Harness, sid: &str, text: &str) -> tokio::task::JoinHandle<Value> {
  let conn = h.conn.clone();
  let params = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] });
  tokio::spawn(async move { conn.request("session/prompt", params).await.unwrap() })
}

/// A steered message as the engine sends it (`vendors/steering.rs`: idle messages are handed back)
async fn steer(h: &Harness, sid: &str, text: &str) -> Value {
  let params = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }], "_meta": { "steering": { "idleBehavior": "promptRequired" } } });
  h.conn.request("_session/steering", params).await.unwrap()
}

/// Wait until the turn reached the point the test steers at
async fn until(ready: impl Fn() -> bool) {
  let t = Instant::now();
  while !ready() {
    assert!(t.elapsed() < Duration::from_secs(10), "the turn never got there");
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
}

fn messages(server: &MockModel, n: usize) -> Vec<Value> {
  server.requests()[n].body["messages"].as_array().unwrap().clone()
}

fn user_texts(h: &Harness) -> Vec<String> {
  h.updates().iter().filter(|u| u["sessionUpdate"] == "user_message_chunk").map(|u| u["content"]["text"].as_str().unwrap().to_owned()).collect()
}

fn injected() -> Value {
  json!({ "outcome": "injected" })
}

#[tokio::test]
async fn steering_is_advertised_and_an_idle_session_hands_the_message_back() {
  let (h, server, sid) = setup("openai-chat", "m1").await;
  let init = h.conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": {} })).await.unwrap();
  assert_eq!(init["_meta"]["steering"]["supported"], true);
  assert_eq!(steer(&h, &sid, "hello").await, json!({ "outcome": "promptRequired", "reason": "noRunningTurn" }));
  // Without the opt-in it is refused: this agent starts no turn of its own
  let refused = h.conn.request("_session/steering", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "hello" }] })).await.unwrap_err();
  assert_eq!(refused.code, -32603);
  let empty = h.conn.request("_session/steering", json!({ "sessionId": sid, "prompt": [] })).await.unwrap_err();
  assert_eq!(empty.code, -32602);
  // Nothing handed back was kept
  server.push(mock::text("Hi."));
  prompt(&h, &sid, "go").await;
  assert!(!server.requests()[0].body.to_string().contains("steer"));
}

#[tokio::test]
async fn messages_sent_during_a_tool_round_reach_the_next_call_as_one_message() {
  let (h, server, sid) = setup("openai-chat", "m1").await;
  *h.client.answer.lock() = Answer::Wait { allow: true };
  server.push(mock::tools(&[("c1", "bash", json!({ "command": "echo hi" }))])).push(mock::text("Done with printf."));
  let turn = start(&h, &sid, "run it");
  until(|| h.client.permissions.lock().len() == 1).await;
  assert_eq!(steer(&h, &sid, "use printf instead").await, injected());
  assert_eq!(steer(&h, &sid, "and keep it short").await, injected());
  h.release();
  assert_eq!(turn.await.unwrap()["stopReason"], "end_turn");
  assert_eq!(server.requests().len(), 2, "one call after the round, none after the answer");
  // System, prompt, the call, its result, then both messages in one user message
  let msgs = messages(&server, 1);
  assert_eq!(msgs.len(), 5, "{msgs:?}");
  assert_eq!((msgs[3]["role"].as_str(), msgs[4]["role"].as_str()), (Some("tool"), Some("user")));
  let steered = msgs[4]["content"].as_str().unwrap();
  assert!(steered.starts_with("<steer>\nThe user wrote this while you were working.") && steered.ends_with("</steer>\n\nuse printf instead\n\nand keep it short"), "{steered}");
  for (x, y) in messages(&server, 0).iter().zip(&msgs) {
    assert_eq!(x.to_string(), y.to_string(), "the prefix changed");
  }
  // Live, the client shows the message itself (it sent it); the log keeps it for the replay, without the note
  assert_eq!(user_texts(&h), Vec::<String>::new());
  let h = Harness::start_in(h.home, h.cwd).await;
  h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  assert_eq!(user_texts(&h), ["run it", "use printf instead", "\n\n", "and keep it short"]);
  let ups = h.updates();
  let at = |f: &dyn Fn(&Value) -> bool| ups.iter().position(f).unwrap();
  let steered_at = at(&|u| u["content"]["text"] == "use printf instead");
  assert!(at(&|u| u["sessionUpdate"] == "tool_call") < steered_at && steered_at < at(&|u| u["sessionUpdate"] == "agent_message_chunk"));
  // The reopened history goes out byte for byte
  server.push(mock::text("ok"));
  prompt(&h, &sid, "next").await;
  let next = messages(&server, 2);
  assert_eq!(next.len(), 7);
  for (x, y) in msgs.iter().zip(&next) {
    assert_eq!(x.to_string(), y.to_string(), "the reopened history differs");
  }
}

#[tokio::test]
async fn a_message_sent_while_the_reply_streams_keeps_the_turn_going() {
  let (h, server, sid) = setup("openai-chat", "m1").await;
  let gate = Gate::default();
  server.push(Reply::Held(gate.clone(), Box::new(mock::text("Here is the plan.")))).push(mock::text("README checked too."));
  let turn = start(&h, &sid, "make a plan");
  until(|| server.requests().len() == 1).await;
  assert_eq!(steer(&h, &sid, "also check the README").await, injected());
  gate.open();
  assert_eq!(turn.await.unwrap()["stopReason"], "end_turn");
  // The reply would have ended the turn; the model answers the message in the same turn instead
  assert_eq!(server.requests().len(), 2);
  let msgs = messages(&server, 1);
  assert_eq!(msgs.len(), 4, "{msgs:?}");
  assert_eq!((msgs[2]["role"].as_str(), msgs[2]["content"].as_str()), (Some("assistant"), Some("Here is the plan.")));
  assert!(msgs[3]["content"].as_str().unwrap().ends_with("</steer>\n\nalso check the README"));
  let said: String = h.updates().iter().filter(|u| u["sessionUpdate"] == "agent_message_chunk").map(|u| u["content"]["text"].as_str().unwrap().to_owned()).collect();
  assert!(said.starts_with("Here is the plan.") && said.ends_with("README checked too."), "{said}");
}

#[tokio::test]
async fn a_rejected_card_with_a_message_waiting_goes_on_to_the_message() {
  let (h, server, sid) = setup("openai-chat", "m1").await;
  *h.client.answer.lock() = Answer::Wait { allow: false };
  let marker = h.cwd().join("ran");
  server.push(mock::tools(&[("c1", "bash", json!({ "command": format!("touch {}", marker.display()) }))])).push(mock::text("Leaving the files alone."));
  let turn = start(&h, &sid, "run it");
  until(|| h.client.permissions.lock().len() == 1).await;
  assert_eq!(steer(&h, &sid, "do not touch any files").await, injected());
  h.release();
  assert_eq!(turn.await.unwrap()["stopReason"], "end_turn");
  assert!(!marker.exists());
  // A rejection alone ends the turn; with a message waiting, the message is the direction the model was told to wait for
  let msgs = messages(&server, 1);
  assert!(msgs[3]["content"].as_str().unwrap().contains("The user rejected this action"));
  assert!(msgs[4]["content"].as_str().unwrap().ends_with("</steer>\n\ndo not touch any files"));
}

#[tokio::test]
async fn a_message_no_call_carried_is_kept_for_the_next_turn() {
  let (h, server, sid) = setup("openai-chat", "m1").await;
  *h.client.answer.lock() = Answer::Hold;
  server.push(mock::tools(&[("c1", "bash", json!({ "command": "echo hi" }))]));
  let turn = start(&h, &sid, "run it");
  until(|| h.client.permissions.lock().len() == 1).await;
  assert_eq!(steer(&h, &sid, "list the files first").await, injected());
  h.conn.notify("session/cancel", json!({ "sessionId": sid }));
  assert_eq!(turn.await.unwrap()["stopReason"], "cancelled");
  // The turn is over: a message now is handed back
  assert_eq!(steer(&h, &sid, "late").await["outcome"], "promptRequired");
  server.push(mock::text("ok"));
  prompt(&h, &sid, "go on").await;
  // System, prompt, the call, its cancelled result, the message the cancel left, the new prompt
  let msgs = messages(&server, 1);
  assert_eq!(msgs.len(), 6, "{msgs:?}");
  assert!(msgs[3]["content"].as_str().unwrap().contains("Cancelled"));
  assert!(msgs[4]["content"].as_str().unwrap().ends_with("</steer>\n\nlist the files first"));
  assert_eq!(msgs[5]["content"], "go on");
  assert!(!server.requests()[1].body.to_string().contains("late"));
}

#[tokio::test]
async fn over_anthropic_messages_the_message_rides_behind_the_tool_results() {
  let (h, server, sid) = setup("anthropic", "claude-mock").await;
  *h.client.answer.lock() = Answer::Wait { allow: true };
  server.push(mock::messages(&[json!({ "type": "tool_use", "id": "toolu_1", "name": "bash", "input": { "command": "echo hi" } })], "tool_use", 0));
  server.push(mock::messages(&[json!({ "type": "text", "text": "Done." })], "end_turn", 0));
  let turn = start(&h, &sid, "run it");
  until(|| h.client.permissions.lock().len() == 1).await;
  assert_eq!(steer(&h, &sid, "use printf instead").await, injected());
  h.release();
  assert_eq!(turn.await.unwrap()["stopReason"], "end_turn");
  let msgs = messages(&server, 1);
  assert_eq!(msgs.len(), 3, "the results' message carries it: {msgs:?}");
  let content = msgs[2]["content"].as_array().unwrap();
  assert_eq!((content[0]["type"].as_str(), content[0]["tool_use_id"].as_str()), (Some("tool_result"), Some("toolu_1")));
  assert_eq!(content[1]["type"], "text");
  assert!(content[1]["text"].as_str().unwrap().ends_with("</steer>\n\nuse printf instead"));
  // The next turn sends that message unchanged (cache markers aside)
  server.push(mock::messages(&[json!({ "type": "text", "text": "ok" })], "end_turn", 0));
  prompt(&h, &sid, "next").await;
  let strip = |v: &Value| v.to_string().replace(",\"cache_control\":{\"type\":\"ephemeral\"}", "");
  assert_eq!(strip(&msgs[2]), strip(&messages(&server, 2)[2]));
}

#[tokio::test]
async fn a_compaction_takes_no_messages() {
  let (h, server, sid) = setup("openai-chat", "m1").await;
  server.push(mock::text("A done"));
  prompt(&h, &sid, "first task").await;
  let gate = Gate::default();
  server.push(Reply::Held(gate.clone(), Box::new(mock::text("## Objective\n- first task"))));
  let turn = start(&h, &sid, "/compact");
  until(|| server.requests().len() == 2).await;
  // It calls no model a message could reach: handed back, the client sends it as the next prompt
  assert_eq!(steer(&h, &sid, "and then?").await, json!({ "outcome": "promptRequired", "reason": "noRunningTurn" }));
  gate.open();
  assert_eq!(turn.await.unwrap()["stopReason"], "end_turn");
  assert_eq!(server.requests().len(), 2);
}
