//! Whole turns against the scripted model: streaming, tools with permission cards, rejection, cancellation, the output
//! budget, and a byte-stable request prefix between calls

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel, Reply};
use support::{Answer, Harness};

async fn setup(models: Value) -> (Harness, MockModel, String) {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), models);
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  (h, server, sid)
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

/// Every request's messages start with the previous request's messages, byte for byte
fn assert_prefix_stable(server: &MockModel) {
  let reqs = server.requests();
  for pair in reqs.windows(2) {
    let (a, b) = (pair[0].body["messages"].as_array().unwrap(), pair[1].body["messages"].as_array().unwrap());
    assert!(b.len() > a.len(), "a later request grows the history");
    for (x, y) in a.iter().zip(b) {
      assert_eq!(x.to_string(), y.to_string(), "the prefix changed between two requests");
    }
    assert_eq!(pair[0].body["tools"].to_string(), pair[1].body["tools"].to_string());
  }
}

#[tokio::test]
async fn a_plain_answer_streams_and_reports_usage() {
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000 }])).await;
  server.push(mock::text("Hello from mock"));
  let r = prompt(&h, &sid, "hi").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert_eq!(r["usage"]["inputTokens"], 100);
  assert_eq!(r["_meta"]["usage"]["modelCalls"], 1);
  let ups = h.updates();
  let text: String = ups.iter().filter(|u| u["sessionUpdate"] == "agent_message_chunk").map(|u| u["content"]["text"].as_str().unwrap()).collect();
  assert_eq!(text, "Hello from mock");
  assert!(ups.iter().any(|u| u["sessionUpdate"] == "usage_update" && u["size"] == 64000));
  let req = &server.requests()[0];
  assert_eq!(req.body["model"], "m1");
  assert!(req.body["messages"][0]["content"].as_str().unwrap().contains("Acpira"));
  assert_eq!(req.body["messages"][1], json!({ "role": "user", "content": "hi" }));
}

#[tokio::test]
async fn an_allowed_edit_changes_the_file_and_the_model_sees_the_result() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  std::fs::write(h.cwd().join("a.txt"), "hello world\n").unwrap();
  server.push(mock::tools(&[("call_1", "read", json!({ "path": "a.txt" }))]));
  server.push(mock::tools(&[("call_1", "edit", json!({ "path": "a.txt", "old_string": "world", "new_string": "there" }))]));
  server.push(mock::text("Done."));
  let r = prompt(&h, &sid, "change it").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert_eq!(std::fs::read_to_string(h.cwd().join("a.txt")).unwrap(), "hello there\n");
  let perms = h.client.permissions.lock().clone();
  assert_eq!(perms.len(), 1, "only the edit asks");
  assert_eq!(perms[0]["toolCall"]["kind"], "edit");
  assert_eq!(perms[0]["toolCall"]["content"][0]["newText"], "hello there\n");
  // The model's reused call id still maps to two distinct ACP tool calls
  let ups = h.updates();
  let calls: Vec<&str> = ups.iter().filter(|u| u["sessionUpdate"] == "tool_call").map(|u| u["toolCallId"].as_str().unwrap()).collect();
  assert_eq!(calls, ["call-1", "call-2"]);
  assert!(ups.iter().any(|u| u["toolCallId"] == "call-2" && u["status"] == "completed" && u["content"][0]["type"] == "diff"));
  let reqs = server.requests();
  assert_eq!(reqs.len(), 3);
  let read_result = &reqs[1].body["messages"].as_array().unwrap().last().unwrap()["content"];
  assert!(read_result.as_str().unwrap().contains("     1\thello world"), "{read_result}");
  assert_eq!(reqs[2].body["messages"].as_array().unwrap().last().unwrap()["content"], "Edited a.txt.");
  assert_prefix_stable(&server);
}

#[tokio::test]
async fn a_rejected_command_ends_the_turn_and_never_runs() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  *h.client.answer.lock() = Answer::Reject;
  let marker = h.cwd().join("ran");
  server.push(mock::tools(&[("c1", "bash", json!({ "command": format!("touch {}", marker.display()) }))]));
  let r = prompt(&h, &sid, "run it").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert!(!marker.exists());
  assert_eq!(server.requests().len(), 1, "no model call after a rejection");
  assert!(h.updates().iter().any(|u| u["toolCallId"] == "call-1" && u["status"] == "failed"));
  // The next turn's history answers the rejected call
  server.push(mock::text("ok"));
  prompt(&h, &sid, "never mind").await;
  let msgs = server.requests()[1].body["messages"].as_array().unwrap().clone();
  assert_eq!(msgs[3]["role"], "tool");
  assert!(msgs[3]["content"].as_str().unwrap().contains("rejected"));
  assert_prefix_stable(&server);
}

#[tokio::test]
async fn a_bad_tool_call_goes_back_to_the_model_with_its_arguments() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(Reply::Sse(vec![
    mock::delta(json!({ "tool_calls": [{ "index": 0, "id": "x", "function": { "name": "read", "arguments": "{\"path\": " } }] })),
    mock::finish("tool_calls"),
  ]));
  server.push(mock::tools(&[("y", "teleport", json!({}))]));
  server.push(mock::text("sorry"));
  prompt(&h, &sid, "go").await;
  let reqs = server.requests();
  let bad_json = reqs[1].body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
  assert!(bad_json.contains("not valid JSON") && bad_json.contains("{\"path\": "), "{bad_json}");
  let unknown = reqs[2].body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
  assert!(unknown.contains("Unknown tool \"teleport\"") && unknown.contains("read, write, edit, bash"), "{unknown}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_huge_command_output_reaches_the_model_as_a_preview_and_a_file() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(mock::tools(&[("c1", "bash", json!({ "command": "seq 1 6000" }))]));
  server.push(mock::text("counted"));
  prompt(&h, &sid, "count").await;
  let msgs = server.requests()[1].body["messages"].as_array().unwrap().clone();
  let result = msgs.last().unwrap()["content"].as_str().unwrap().to_owned();
  assert!(result.starts_with("1\n2\n") && result.contains("6000") && result.contains("lines omitted"), "{}", &result[..200]);
  let path = result.split("The full output is in ").nth(1).unwrap().split(';').next().unwrap();
  let saved = std::fs::read_to_string(path).unwrap();
  assert_eq!(saved.lines().count(), 6000);
  assert!(path.starts_with(&h.home().display().to_string()));
  // The reader's card got the whole stream
  let streamed: String = h
    .updates()
    .iter()
    .filter_map(|u| u.pointer("/_meta/terminal_output_delta/data").and_then(Value::as_str).map(str::to_owned))
    .collect();
  assert_eq!(streamed.lines().count(), 6000);
}

#[tokio::test]
async fn cancel_answers_at_once_and_nothing_follows() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(Reply::Hang);
  let conn = h.conn.clone();
  let sid2 = sid.clone();
  let turn = tokio::spawn(async move { conn.request("session/prompt", json!({ "sessionId": sid2, "prompt": [{ "type": "text", "text": "slow" }] })).await });
  while server.hanging() == 0 {
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
  let before = h.updates().len();
  let t = Instant::now();
  h.conn.notify("session/cancel", json!({ "sessionId": sid }));
  let r = tokio::time::timeout(Duration::from_secs(2), turn).await.expect("the turn answers promptly").unwrap().unwrap();
  assert!(t.elapsed() < Duration::from_secs(1));
  assert_eq!(r["stopReason"], "cancelled");
  tokio::time::sleep(Duration::from_millis(200)).await;
  assert_eq!(h.updates().len(), before, "no update after a cancelled turn");
  // The session takes the next prompt with a well-formed history
  server.push(mock::text("fast"));
  assert_eq!(prompt(&h, &sid, "again").await["stopReason"], "end_turn");
}

#[cfg(unix)]
#[tokio::test]
async fn cancel_during_a_command_kills_it_and_answers_the_call() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  let marker = h.cwd().join("survived");
  server.push(mock::tools(&[("c1", "bash", json!({ "command": format!("sleep 1; touch {}", marker.display()) }))]));
  let conn = h.conn.clone();
  let sid2 = sid.clone();
  let turn = tokio::spawn(async move { conn.request("session/prompt", json!({ "sessionId": sid2, "prompt": [{ "type": "text", "text": "run" }] })).await });
  while !h.updates().iter().any(|u| u["status"] == "in_progress") {
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
  h.conn.notify("session/cancel", json!({ "sessionId": sid }));
  assert_eq!(turn.await.unwrap().unwrap()["stopReason"], "cancelled");
  tokio::time::sleep(Duration::from_millis(1500)).await;
  assert!(!marker.exists(), "the command was killed");
  server.push(mock::text("ok"));
  prompt(&h, &sid, "next").await;
  let msgs = server.requests()[1].body["messages"].as_array().unwrap().clone();
  assert_eq!(msgs[3]["role"], "tool");
  assert!(msgs[3]["content"].as_str().unwrap().contains("Cancelled"));
}

#[tokio::test]
async fn no_model_is_a_readable_error() {
  let h = Harness::start().await;
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  let e = h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "hi" }] })).await.unwrap_err();
  assert!(e.message.contains("No model is configured"), "{}", e.message);
}

#[tokio::test]
async fn a_rejected_key_says_so() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(Reply::Status(401, r#"{"error":{"message":"bad key"}}"#.into()));
  let e = h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "hi" }] })).await.unwrap_err();
  assert!(e.message.contains("rejected the API key") && e.message.contains("bad key"), "{}", e.message);
}
