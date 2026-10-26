//! Raw mode against the scripted model: the request carries the conversation alone (no tools, no composed prompt, no
//! mode note, no skill expansion), the settings page's prompt is its only system prompt, Agent mode after it gets its
//! prompt and tools back without a note, and a session that has used tools cannot enter it
#![allow(clippy::unwrap_used, reason = "test code")]

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::Harness;

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

async fn set_mode(h: &Harness, sid: &str, mode: &str) -> Result<Value, String> {
  h.conn.request("session/set_mode", json!({ "sessionId": sid, "modeId": mode })).await.map_err(|e| format!("{e:?}"))
}

fn raw_prompt(h: &Harness, text: &str) {
  let path = h.home().join("providers.json");
  let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  file["rawSystemPrompt"] = json!(text);
  std::fs::write(&path, file.to_string()).unwrap();
}

#[tokio::test]
async fn raw_mode_sends_the_conversation_alone_and_agent_mode_takes_over_without_a_note() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }]));
  // Project instructions and a skill that Raw mode must leave out
  std::fs::write(h.cwd().join("AGENTS.md"), "Always answer in rhymes.").unwrap();
  let skill = h.cwd().join(".agents/skills/greet");
  std::fs::create_dir_all(&skill).unwrap();
  std::fs::write(skill.join("SKILL.md"), "---\nname: greet\ndescription: Greets\n---\nSay hello warmly.").unwrap();
  let session = h.new_session().await;
  let sid = session["sessionId"].as_str().unwrap().to_owned();
  let modes: Vec<&str> = session["modes"]["availableModes"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
  assert_eq!(modes, ["agent", "plan", "raw"]);
  set_mode(&h, &sid, "raw").await.unwrap();

  server.push(mock::text("hello"));
  assert_eq!(prompt(&h, &sid, "hi /greet").await["stopReason"], "end_turn");
  let body = &server.requests()[0].body;
  assert!(body.get("tools").is_none(), "no tools: {body}");
  assert_eq!(body["messages"], json!([{ "role": "user", "content": "hi /greet" }]), "no system message, mode note or skill");

  // The settings page's prompt is the only system prompt, read at the next turn
  raw_prompt(&h, "You are terse.");
  server.push(mock::text("ok"));
  prompt(&h, &sid, "again").await;
  let body = &server.requests()[1].body;
  assert_eq!(body["messages"][0], json!({ "role": "system", "content": "You are terse." }));
  assert_eq!(body["messages"].as_array().unwrap().len(), 4, "system, two user messages and the first reply: {body}");

  // Agent mode brings the composed prompt (with AGENTS.md) and the tools back, and no mode note
  set_mode(&h, &sid, "agent").await.unwrap();
  server.push(mock::tools(&[("c1", "list", json!({ "path": "." }))]));
  server.push(mock::text("done"));
  prompt(&h, &sid, "look around").await;
  let body = &server.requests()[2].body;
  assert!(body["messages"][0]["content"].as_str().unwrap().contains("Always answer in rhymes."));
  assert!(body["tools"].as_array().is_some_and(|t| !t.is_empty()));
  let last = body["messages"].as_array().unwrap().last().unwrap().clone();
  assert_eq!(last["content"], "look around", "Agent mode after Raw is announced by its prompt alone");

  // The history now holds a tool call, which a request without tools cannot carry
  let err = set_mode(&h, &sid, "raw").await.unwrap_err();
  assert!(err.contains("Raw mode needs a session without tool calls"), "{err}");
  set_mode(&h, &sid, "plan").await.unwrap();
}

#[tokio::test]
async fn raw_mode_without_a_prompt_sends_no_system_block_on_anthropic() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers_in("anthropic", &server.base_url(), json!([{ "id": "claude-mock", "output": 4096 }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  set_mode(&h, &sid, "raw").await.unwrap();
  server.push(mock::messages(&[json!({ "type": "text", "text": "hello" })], "end_turn", 0));
  assert_eq!(prompt(&h, &sid, "hi").await["stopReason"], "end_turn");
  let body = &server.requests()[0].body;
  assert!(body.get("system").is_none() && body.get("tools").is_none(), "{body}");
  assert_eq!(body["messages"][0]["role"], "user");
}
