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
async fn a_zero_prompt_count_is_estimated_from_the_request_and_marked() {
  // A gateway's streamed usage with every prompt count at 0 (seen on a MiniMax route, 2026-10-11)
  let (h, server, sid) = setup(json!([{ "id": "m1", "context": 64000 }])).await;
  server.push(Reply::Sse(vec![mock::delta(json!({ "role": "assistant", "content": "ok" })), mock::finish("stop"), mock::usage(0, 2, 0)]));
  let r = prompt(&h, &sid, "hi").await;
  let input = r["usage"]["inputTokens"].as_u64().unwrap();
  // The system prompt and the tool schemas alone are well over a thousand tokens
  assert!(input > 1000, "estimated input {input}");
  assert!(h.updates().iter().any(|u| u["sessionUpdate"] == "usage_update" && u["used"] == input + 2));
  let log = std::fs::read_to_string(h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"))).unwrap();
  let req: Value = log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|e| e["type"] == "request").unwrap();
  assert_eq!((req["usage"]["input"].as_u64(), &req["usage"]["inputEstimated"]), (Some(input), &json!(true)));

  // A real count is taken as it is, unmarked
  server.push(mock::text("again"));
  let r = prompt(&h, &sid, "hi").await;
  assert_eq!(r["usage"]["inputTokens"], 100, "usage is per turn");
  let log = std::fs::read_to_string(h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"))).unwrap();
  let last: Value = log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).rfind(|e| e["type"] == "request").unwrap();
  assert!(last["usage"].get("inputEstimated").is_none());
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

#[tokio::test]
async fn an_interrupted_stream_is_retried_and_only_the_retry_counts() {
  // A gateway ending a 200 stream with an error event after some output (seen on a Claude route, 2026-10-11)
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(Reply::Sse(vec![
    mock::delta(json!({ "role": "assistant", "content": "half an ans" })),
    json!({ "error": { "message": "response stream interrupted" } }),
  ]));
  server.push(mock::text("the whole answer"));
  let started = Instant::now();
  let r = prompt(&h, &sid, "hi").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert!(started.elapsed() >= Duration::from_millis(900), "the retry waits about a second");
  assert_eq!(server.requests().len(), 2);
  let ups = h.updates();
  let notice = ups.iter().find(|u| u["sessionUpdate"] == "session_info_update").expect("a retry notice");
  let failure = &notice["_meta"]["jetbrains"]["air"]["sessionFailure"];
  assert_eq!((failure["severity"].as_str(), failure["category"].as_str()), (Some("warning"), Some("service")));
  assert!(failure["title"].as_str().unwrap().contains("attempt 2 of 4"), "{failure}");

  // The next request carries the retried answer only, and the log keeps both attempts
  server.push(mock::text("ok"));
  prompt(&h, &sid, "next").await;
  let msgs = server.requests()[2].body["messages"].as_array().unwrap().clone();
  assert_eq!(msgs[2], json!({ "role": "assistant", "content": "the whole answer" }));
  let log = std::fs::read_to_string(h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"))).unwrap();
  let reqs: Vec<Value> = log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|e| e["type"] == "request").collect();
  assert!(reqs[0]["error"].as_str().unwrap().contains("interrupted") && reqs[0].get("attempt").is_none());
  assert_eq!((reqs[1]["attempt"].as_u64(), reqs[1].get("error")), (Some(2), None));
}

#[tokio::test]
async fn a_reply_cut_at_its_second_tool_call_is_retried_one_call_at_a_time() {
  // The gateway ends the stream as soon as a second parallel tool call starts (seen on a Claude route, 2026-10-11)
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(Reply::Sse(vec![
    mock::delta(json!({ "role": "assistant", "tool_calls": [{ "index": 0, "id": "c1", "type": "function", "function": { "name": "list", "arguments": "{}" } }] })),
    mock::delta(json!({ "tool_calls": [{ "index": 1, "id": "c2", "type": "function", "function": { "name": "list", "arguments": "" } }] })),
    json!({ "error": { "message": "response stream interrupted" } }),
  ]));
  server.push(mock::tools(&[("c3", "list", json!({}))]));
  server.push(mock::text("done"));
  assert_eq!(prompt(&h, &sid, "look around").await["stopReason"], "end_turn");
  let reqs = server.requests();
  assert_eq!(reqs.len(), 3);
  assert!(reqs[0].body.get("parallel_tool_calls").is_none());
  assert_eq!((reqs[1].body["parallel_tool_calls"].as_bool(), reqs[2].body["parallel_tool_calls"].as_bool()), (Some(false), Some(false)));

  // Kept for the model in a new session too
  let other = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  server.push(mock::text("hi"));
  prompt(&h, &other, "hello").await;
  assert_eq!(server.requests()[3].body["parallel_tool_calls"], false);
}

#[tokio::test]
async fn a_bad_request_is_not_retried() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(Reply::Status(400, r#"{"error":{"message":"unknown parameter"}}"#.into()));
  let e = prompt_err(&h, &sid).await;
  assert!(e.contains("unknown parameter"), "{e}");
  assert_eq!(server.requests().len(), 1);
}

async fn prompt_err(h: &Harness, sid: &str) -> String {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "hi" }] })).await.unwrap_err().message
}

fn tool_result(server: &MockModel, request: usize, nth_from_end: usize) -> String {
  let msgs = server.requests()[request].body["messages"].as_array().unwrap().clone();
  msgs[msgs.len() - 1 - nth_from_end]["content"].as_str().unwrap().to_owned()
}

async fn approval(h: &Harness, sid: &str, level: &str) {
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "approval", "value": level })).await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn always_allow_covers_the_command_pattern_for_the_rest_of_the_session() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  *h.client.answer.lock() = Answer::Always;
  server.push(mock::tools(&[("c1", "bash", json!({ "command": "ls -a" }))]));
  server.push(mock::tools(&[("c2", "bash", json!({ "command": "ls -l" }))]));
  server.push(mock::tools(&[("c3", "bash", json!({ "command": "pwd" }))]));
  server.push(mock::text("done"));
  prompt(&h, &sid, "look").await;
  let perms = h.client.permissions.lock().clone();
  assert_eq!(perms.len(), 2, "ls -l rode on the first answer, pwd asked");
  assert_eq!(perms[0]["options"][1]["name"], "Always allow `ls *` in this session");
  assert_eq!(perms[1]["toolCall"]["title"], "pwd");
}

#[tokio::test]
async fn full_access_runs_commands_but_edits_to_agent_config_still_ask() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  approval(&h, &sid, "full").await;
  server.push(mock::tools(&[
    ("c1", "write", json!({ "path": "notes.txt", "content": "hi\n" })),
    ("c2", "write", json!({ "path": ".agents/hooks.json", "content": "{}" })),
  ]));
  server.push(mock::text("done"));
  prompt(&h, &sid, "write").await;
  let perms = h.client.permissions.lock().clone();
  assert_eq!(perms.len(), 1);
  assert_eq!(perms[0]["toolCall"]["title"], "Write .agents/hooks.json");
  assert!(h.cwd().join("notes.txt").exists() && h.cwd().join(".agents/hooks.json").exists());
}

#[tokio::test]
async fn auto_edit_writes_without_a_card_and_a_reject_still_stops_commands() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  approval(&h, &sid, "auto-edit").await;
  *h.client.answer.lock() = Answer::Reject;
  server.push(mock::tools(&[("c1", "write", json!({ "path": "a.txt", "content": "x" })), ("c2", "bash", json!({ "command": "echo hi" }))]));
  let r = prompt(&h, &sid, "go").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert!(h.cwd().join("a.txt").exists());
  let perms = h.client.permissions.lock().clone();
  assert_eq!((perms.len(), perms[0]["toolCall"]["kind"].as_str()), (1, Some("execute")));
}

#[tokio::test]
async fn misnamed_tools_are_recovered_twice_a_turn_then_refused() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  std::fs::write(h.cwd().join("a.txt"), "alpha\n").unwrap();
  server.push(mock::tools(&[
    ("c1", "read_file", json!({ "path": "a.txt" })),
    ("c2", "functions.Read", json!({ "path": "a.txt" })),
    ("c3", "ReadFile", json!({ "path": "a.txt" })),
  ]));
  server.push(mock::text("ok"));
  prompt(&h, &sid, "read").await;
  let first = tool_result(&server, 1, 2);
  assert!(first.starts_with("(\"read_file\" was taken as the read tool") && first.contains("     1\talpha"), "{first}");
  assert!(tool_result(&server, 1, 1).contains("alpha"));
  let third = tool_result(&server, 1, 0);
  assert!(third.starts_with("Unknown tool \"ReadFile\" (did you mean \"read\"?)"), "{third}");
  // The cards show the real tool, and the history keeps the name the model used
  assert!(h.updates().iter().any(|u| u["sessionUpdate"] == "tool_call" && u["title"] == "read"));
  assert_eq!(server.requests()[1].body["messages"][2]["tool_calls"][0]["function"]["name"], "read_file");
}

#[tokio::test]
async fn bad_arguments_come_back_with_what_was_received() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  server.push(mock::tools(&[("c1", "grep", json!({ "query": "x" }))]));
  server.push(mock::text("ok"));
  prompt(&h, &sid, "find").await;
  let r = tool_result(&server, 1, 0);
  assert!(r.contains("Missing required string argument \"pattern\"") && r.contains("Received arguments: {\"query\":\"x\"}"), "{r}");
}

#[tokio::test]
async fn the_todo_tool_feeds_the_to_do_bar_and_search_tools_run_without_cards() {
  let (h, server, sid) = setup(json!([{ "id": "m1" }])).await;
  std::fs::create_dir_all(h.cwd().join("src")).unwrap();
  std::fs::write(h.cwd().join("src/lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
  server.push(mock::tools(&[
    ("c1", "todo", json!({ "todos": [{ "content": "find it", "status": "in_progress" }, { "content": "fix it", "status": "pending" }] })),
    ("c2", "grep", json!({ "pattern": "fn answer" })),
    ("c3", "glob", json!({ "pattern": "*.rs" })),
    ("c4", "list", json!({})),
  ]));
  server.push(mock::text("ok"));
  prompt(&h, &sid, "plan").await;
  assert!(h.client.permissions.lock().is_empty());
  let ups = h.updates();
  let todo = ups.iter().find(|u| u["toolCallId"] == "call-1" && u["status"] == "completed").unwrap();
  assert_eq!(todo["rawOutput"]["todos"][0], json!({ "content": "find it", "status": "in_progress" }));
  assert!(ups.iter().any(|u| u["toolCallId"] == "call-1" && u["title"] == "todo"));
  assert!(tool_result(&server, 1, 2).contains("src/lib.rs:1: pub fn answer()"));
  assert_eq!(tool_result(&server, 1, 1), "src/lib.rs\n");
  assert!(tool_result(&server, 1, 0).contains("  src/\n    lib.rs"));
}

#[tokio::test]
async fn a_turn_over_the_anthropic_format_replays_its_blocks_and_marks_cache_breakpoints() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers_in("anthropic", &server.base_url(), json!([{ "id": "claude-mock", "output": 4096 }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  std::fs::write(h.cwd().join("a.txt"), "alpha\n").unwrap();
  server.push(mock::messages(&[json!({ "type": "text", "text": "Looking." }), json!({ "type": "tool_use", "id": "toolu_1", "name": "read", "input": { "path": "a.txt" } })], "tool_use", 0));
  server.push(mock::messages(&[json!({ "type": "text", "text": "It says alpha." })], "end_turn", 90));
  let r = prompt(&h, &sid, "what is in a.txt?").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert_eq!((r["usage"]["inputTokens"].as_u64(), r["usage"]["cachedReadTokens"].as_u64()), (Some(200), Some(90)));
  let reqs = server.requests();
  assert_eq!(reqs[0].body["max_tokens"], 4096);
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  assert_eq!(msgs[1]["content"], json!([{ "type": "text", "text": "Looking." }, { "type": "tool_use", "id": "toolu_1", "name": "read", "input": { "path": "a.txt" } }]));
  assert_eq!(msgs[2]["content"][0]["tool_use_id"], "toolu_1");
  assert!(msgs[2]["content"][0]["content"].as_str().unwrap().contains("alpha"));
  // The previous request's end and this one's carry the markers; the earlier marker moved with the prefix
  assert!(msgs[0]["content"][0].get("cache_control").is_some() && msgs[2]["content"][0].get("cache_control").is_some());
  assert_eq!(reqs[1].body["system"][0]["cache_control"], json!({ "type": "ephemeral" }));
  // Apart from the markers, the second request starts with the first one
  let strip = |v: &serde_json::Value| v.to_string().replace(",\"cache_control\":{\"type\":\"ephemeral\"}", "");
  assert_eq!(strip(&reqs[0].body["messages"][0]), strip(&msgs[0]));
  assert_eq!(reqs[0].body["tools"], reqs[1].body["tools"]);
}

#[tokio::test]
async fn the_prompt_variant_follows_the_model_and_is_recorded_per_turn() {
  let (h, server, sid) = setup(json!([{ "id": "deepseek-chat" }, { "id": "claude-x" }])).await;
  server.push(mock::text("one"));
  server.push(mock::text("two"));
  server.push(mock::text("three"));
  let r = prompt(&h, &sid, "hi").await;
  assert_eq!(r["_meta"]["acpira/prompt"]["variant"], "deepseek");
  assert_eq!(r["_meta"]["acpira/prompt"]["version"], "1");
  prompt(&h, &sid, "again").await;
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/claude-x" })).await.unwrap();
  let r = prompt(&h, &sid, "now you").await;
  assert_eq!(r["_meta"]["acpira/prompt"]["variant"], "claude");
  let system = |i: usize| server.requests()[i].body["messages"][0]["content"].as_str().unwrap().to_owned();
  assert_eq!(system(0), system(1), "the same variant keeps the same system prompt");
  assert!(system(0).contains("## Tool calls") && !system(0).contains("## Scope"));
  assert!(system(2).contains("## Scope") && !system(2).contains("## Tool calls"));
}
