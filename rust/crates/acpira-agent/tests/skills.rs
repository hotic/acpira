//! Skills as slash commands: advertised with /compact on session/new and on a reopen, expanded behind a prompt that
//! names one anywhere in its text, and loaded by the model through the `skill` tool
#![allow(clippy::unwrap_used, reason = "test code")]

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::Harness;

fn write_skill(h: &Harness, folder: &str, text: &str) {
  let dir = h.cwd().join(".agents").join("skills").join(folder);
  std::fs::create_dir_all(&dir).unwrap();
  std::fs::write(dir.join("SKILL.md"), text).unwrap();
}

fn commands(h: &Harness) -> Vec<Value> {
  h.updates().into_iter().filter(|u| u["sessionUpdate"] == "available_commands_update").map(|u| u["availableCommands"].clone()).collect()
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

#[tokio::test]
async fn skills_are_advertised_expanded_and_loadable() {
  let h = Harness::start().await;
  std::fs::create_dir_all(h.cwd().join(".git")).unwrap();
  write_skill(&h, "release", "---\nname: release\ndescription: Cut a release\nargument-hint: version\n---\nTag $ARGUMENTS and push.\n");
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1", "context": 64000 }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();

  // Advertised inside session/new: /compact first, then the skills
  let listed = commands(&h);
  assert_eq!(listed.len(), 1, "{listed:?}");
  let names: Vec<&str> = listed[0].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
  assert_eq!(names[0], "compact");
  assert!(names.contains(&"release"), "{names:?}");
  let release = listed[0].as_array().unwrap().iter().find(|c| c["name"] == "release").unwrap();
  assert_eq!(release, &json!({ "name": "release", "description": "Cut a release", "input": { "hint": "version" } }));

  // A mention mid-sentence: the message goes out as typed, the skill's text behind it, the rest of the line as arguments
  server.push(mock::text("ok"));
  prompt(&h, &sid, "please /release 1.2 now").await;
  let req = &server.requests()[0];
  let user = req.body["messages"][1]["content"].as_str().unwrap();
  assert!(user.starts_with("please /release 1.2 now\n\n<skill name=\"release\" path="), "{user}");
  assert!(user.ends_with("\nTag 1.2 now and push.\n</skill>"), "{user}");
  // The replay (recorded in the session log, not sent) shows the prompt as typed
  let log = std::fs::read_to_string(h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"))).unwrap();
  assert!(log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).any(|e| e["type"] == "update" && e["update"]["sessionUpdate"] == "user_message_chunk" && e["update"]["content"]["text"] == "please /release 1.2 now"));
  // The skill tool is offered, listing the skills
  let tools = req.body["tools"].as_array().unwrap();
  let skill = tools.iter().find(|t| t["function"]["name"] == "skill").unwrap();
  assert!(skill["function"]["description"].as_str().unwrap().contains("- release: Cut a release"));
  assert_eq!(tools.last().unwrap()["function"]["name"], "skill", "appended behind the fixed set");

  // The model loads a skill itself, and an unknown name is an error listing the real ones
  server.push(mock::tools(&[("c1", "skill", json!({ "name": "release" })), ("c2", "skill", json!({ "name": "nope" }))])).push(mock::text("done"));
  prompt(&h, &sid, "go").await;
  let ups = h.updates();
  let row = ups.iter().find(|u| u["sessionUpdate"] == "tool_call_update" && u["title"] == "Skill release").unwrap();
  assert_eq!(row["kind"], "read");
  assert!(ups.iter().any(|u| u["sessionUpdate"] == "tool_call_update" && u["toolCallId"] == row["toolCallId"] && u["status"] == "completed"));
  let next = &server.requests()[2];
  let results: Vec<&str> = next.body["messages"].as_array().unwrap().iter().filter(|m| m["role"] == "tool").map(|m| m["content"].as_str().unwrap()).collect();
  assert!(results[0].starts_with("Tag $ARGUMENTS and push.\n\n(Files the skill refers to are relative to"), "{}", results[0]);
  assert!(results[1].starts_with("Unknown skill \"nope\". Available skills:") && results[1].contains("release"), "{}", results[1]);

  // A reopen advertises the current list again
  let h = Harness::start_in(h.home, h.cwd).await;
  write_skill(&h, "deploy", "---\ndescription: Ship it\n---\nShip.\n");
  h.conn.request("session/load", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  let listed = commands(&h);
  let names: Vec<&str> = listed.last().unwrap().as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
  assert!(names.contains(&"deploy") && names.contains(&"release"), "{names:?}");
}

#[tokio::test]
async fn without_skills_the_tool_set_is_unchanged() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1", "context": 64000 }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  let listed = commands(&h);
  assert_eq!(listed[0][0]["name"], "compact");
  server.push(mock::text("ok"));
  prompt(&h, &sid, "/nothing here").await;
  let req = &server.requests()[0];
  assert_eq!(req.body["messages"][1]["content"], "/nothing here");
  // Only when the user has no skills of their own in ~/.agents/skills, which the test cannot control
  let has_user_skills = acpira_agent::turn::user_home().is_some_and(|h| h.join(".agents").join("skills").is_dir());
  if !has_user_skills {
    assert!(!req.body["tools"].as_array().unwrap().iter().any(|t| t["function"]["name"] == "skill"));
  }
}
