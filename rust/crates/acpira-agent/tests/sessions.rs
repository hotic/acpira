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
  let model = resumed["configOptions"].as_array().unwrap().iter().find(|o| o["id"] == "model").unwrap().clone();
  assert_eq!(model["currentValue"], "mock/m1");
  // Only the session's commands are announced, as on session/new
  assert!(h.updates().iter().all(|u| u["sessionUpdate"] == "available_commands_update"), "{:?}", h.updates());

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

/// The main source `mock` and a second one, `fast`, each on its own scripted server; `fast_model` as the settings page
/// writes it
fn two_sources(h: &Harness, main: &MockModel, fast: &MockModel, fast_model: &str) {
  let source = |id: &str, url: String, model: &str| json!({ "id": id, "name": id, "baseUrl": url, "models": [{ "id": model }] });
  std::fs::write(
    h.home().join("providers.json"),
    json!({ "version": 1, "fastModel": fast_model, "providers": [source("mock", main.base_url(), "m1"), source("fast", fast.base_url(), "f1")] }).to_string(),
  )
  .unwrap();
  std::fs::write(h.home().join("secrets.json"), json!({ "acpira.provider.mock": "sk-main", "acpira.provider.fast": "sk-fast" }).to_string()).unwrap();
}

/// Wait for the background title call to be logged
async fn title_event(h: &Harness, sid: &str) -> Value {
  for _ in 0..200 {
    if let Some(e) = events(h, sid).into_iter().find(|e| e["type"] == "title") {
      return e;
    }
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
  }
  panic!("no title event");
}

#[tokio::test]
async fn the_fast_model_names_a_session_once_from_its_first_prompt() {
  let h = Harness::start().await;
  let (main, fast) = (MockModel::start(), MockModel::start());
  two_sources(&h, &main, &fast, "fast/f1");
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  fast.push(mock::text("Title: \"Fix the build.\""));
  main.push(mock::text("Done."));
  assert_eq!(prompt(&h, &sid, "the build fails on Windows, please fix it").await["stopReason"], "end_turn");
  let ev = title_event(&h, &sid).await;
  assert_eq!((ev["title"].as_str(), ev["model"].as_str(), ev["usage"]["output"].as_u64()), (Some("Fix the build"), Some("fast/f1"), Some(10)));
  assert!(h.updates().iter().any(|u| u == &json!({ "sessionUpdate": "session_info_update", "title": "Fix the build" })));

  // One small call of its own: the fast source's key, no tools, thinking off, only the prompt's text
  let req = &fast.requests()[0];
  assert_eq!(req.header("authorization"), Some("Bearer sk-fast"));
  assert_eq!(req.body["model"], "f1");
  assert!(req.body.get("tools").is_none_or(|t| t.as_array().is_some_and(|a| a.is_empty())), "{}", req.body);
  assert_eq!(req.body["messages"][1], json!({ "role": "user", "content": "the build fails on Windows, please fix it" }));
  assert_eq!(main.requests().len(), 1, "the turn's own requests are untouched");

  // A later prompt never renames the session
  main.push(mock::text("Again."));
  prompt(&h, &sid, "and the tests too").await;
  assert_eq!(fast.requests().len(), 1);

  // The title outlives the process: the listing shows it and a load replays it
  let h = reopen(h).await;
  let listed = h.conn.request("session/list", json!({ "cwd": h.cwd() })).await.unwrap();
  assert_eq!(listed["sessions"][0]["title"], "Fix the build");
  h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  assert!(h.updates().iter().any(|u| u["sessionUpdate"] == "session_info_update" && u["title"] == "Fix the build"));
}

#[tokio::test]
async fn without_a_usable_fast_model_the_session_keeps_its_prompt_title() {
  let h = Harness::start().await;
  let (main, fast) = (MockModel::start(), MockModel::start());
  two_sources(&h, &main, &fast, "fast/gone");
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  main.push(mock::text("Done."));
  prompt(&h, &sid, "the build fails").await;
  assert!(fast.requests().is_empty());
  assert!(h.updates().iter().all(|u| u["sessionUpdate"] != "session_info_update"));
  assert!(events(&h, &sid).iter().all(|e| e["type"] != "title"));
  let listed = h.conn.request("session/list", json!({ "cwd": h.cwd() })).await.unwrap();
  assert_eq!(listed["sessions"][0]["title"], "the build fails");
}

async fn named_prompt(h: &Harness, sid: &str, turn: &str, text: &str) -> Value {
  let params = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }], "_meta": { "acpira/turn": turn } });
  h.conn.request("session/prompt", params).await.unwrap()
}

#[tokio::test]
async fn a_rewind_takes_the_conversation_back_to_before_a_named_turn() {
  let h = Harness::start().await;
  let init = h.conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": {} })).await.unwrap();
  assert_eq!(init["_meta"]["acpira/rewind"]["supported"], true);
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }]));
  std::fs::write(h.cwd().join("a.txt"), "alpha\n").unwrap();
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  server.push(mock::text("one"));
  named_prompt(&h, &sid, "t1", "first message").await;
  server.push(mock::tools(&[("c1", "read", json!({ "path": "a.txt" }))]));
  server.push(mock::text("read it"));
  named_prompt(&h, &sid, "t2", "the wrong message").await;
  // A prompt the client did not name leaves no mark to rewind to
  server.push(mock::text("unnamed"));
  prompt(&h, &sid, "unnamed").await;

  let unknown = h.conn.request("_session/rewind", json!({ "sessionId": sid, "turnId": "t9" })).await.unwrap_err();
  assert_eq!(unknown.code, -32602);
  // A dry run changes nothing
  let before = events(&h, &sid).len();
  h.conn.request("_session/rewind", json!({ "sessionId": sid, "turnId": "t2", "dryRun": true })).await.unwrap();
  assert_eq!(events(&h, &sid).len(), before);

  h.conn.request("_session/rewind", json!({ "sessionId": sid, "turnId": "t2" })).await.unwrap();
  // The cut turns' request records stay; their messages, updates and marks go
  let evs = events(&h, &sid);
  assert_eq!(evs.iter().filter(|e| e["type"] == "request").count(), 4);
  assert_eq!(evs.iter().filter(|e| e["type"] == "mark").count(), 1);
  assert!(!evs.iter().any(|e| e.to_string().contains("the wrong message") && e["type"] != "request"));

  server.push(mock::text("better"));
  named_prompt(&h, &sid, "t3", "the right message").await;
  let sent = server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("first message") && sent.contains("one") && sent.contains("the right message"), "{sent}");
  assert!(!sent.contains("the wrong message") && !sent.contains("read it") && !sent.contains("unnamed"), "{sent}");

  // A reopened session sees the same shorter conversation, in its view and its replay
  let h = reopen(h).await;
  h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  let ups = h.updates();
  let users: Vec<&str> = ups.iter().filter(|u| u["sessionUpdate"] == "user_message_chunk").map(|u| u["content"]["text"].as_str().unwrap()).collect();
  assert_eq!(users, ["first message", "the right message"]);
  server.push(mock::text("still here"));
  prompt(&h, &sid, "again").await;
  let sent = server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("the right message") && !sent.contains("the wrong message"), "{sent}");
}
