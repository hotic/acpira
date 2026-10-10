//! Plan mode against the scripted model: only the plan file is writable whatever the approval level, `exit_plan` puts
//! the plan on the host's approval card, an approval switches to Agent mode inside the same turn, a refusal ends it

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::{Answer, Harness};

async fn setup() -> (Harness, MockModel, String) {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "m1" }]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  (h, server, sid)
}

async fn prompt(h: &Harness, sid: &str, text: &str) -> Value {
  h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": text }] })).await.unwrap()
}

async fn set_mode(h: &Harness, sid: &str, mode: &str) {
  h.conn.request("session/set_mode", json!({ "sessionId": sid, "modeId": mode })).await.unwrap();
}

fn tool_names(server: &MockModel, request: usize) -> Vec<String> {
  server.requests()[request].body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_owned()).collect()
}

/// The text of the request's last user message
fn user_text(server: &MockModel, request: usize) -> String {
  let msgs = server.requests()[request].body["messages"].as_array().unwrap().clone();
  msgs.iter().rev().find(|m| m["role"] == "user").unwrap()["content"].as_str().unwrap().to_owned()
}

fn tool_result(server: &MockModel, request: usize, nth_from_end: usize) -> String {
  let msgs = server.requests()[request].body["messages"].as_array().unwrap().clone();
  msgs[msgs.len() - 1 - nth_from_end]["content"].as_str().unwrap().to_owned()
}

#[cfg(unix)]
#[tokio::test]
async fn plan_mode_writes_only_the_plan_and_an_approval_builds_in_the_same_turn() {
  let (h, server, sid) = setup().await;
  let modes = h.new_session().await["modes"].clone();
  assert_eq!(modes["availableModes"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect::<Vec<_>>(), ["agent", "plan"]);
  set_mode(&h, &sid, "plan").await;
  // Full access does not lift Plan mode's restrictions
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "approval", "value": "full" })).await.unwrap();
  let plan = h.home().join("agent/sessions").join(&sid).join("plan.md");
  server.push(mock::tools(&[
    ("c1", "write", json!({ "path": "src.txt", "content": "x" })),
    ("c2", "write", json!({ "path": plan.to_string_lossy(), "content": "# Add src.txt\n\n1. Write it.\n" })),
    ("c3", "bash", json!({ "command": "ls" })),
  ]));
  server.push(mock::tools(&[("c4", "exit_plan", json!({}))]));
  server.push(mock::tools(&[("c5", "write", json!({ "path": "src.txt", "content": "x" }))]));
  server.push(mock::text("built"));
  let r = prompt(&h, &sid, "add src.txt").await;
  assert_eq!(r["stopReason"], "end_turn");

  assert!(user_text(&server, 0).starts_with("<mode>\nPlan mode is on.") && user_text(&server, 0).contains(&*plan.to_string_lossy()));
  assert!(tool_names(&server, 0).contains(&"exit_plan".to_owned()));
  assert!(tool_result(&server, 1, 2).starts_with("Not allowed: the permission rules deny edit on src.txt"), "{}", tool_result(&server, 1, 2));
  assert_eq!(std::fs::read_to_string(&plan).unwrap(), "# Add src.txt\n\n1. Write it.\n");

  let perms = h.client.permissions.lock().clone();
  assert_eq!(perms.len(), 2, "the command and the plan; the plan file and the built file needed no card");
  // An "always" answer could not hold over the mode's rule, so it is not offered
  let kinds: Vec<&str> = perms[0]["options"].as_array().unwrap().iter().map(|o| o["kind"].as_str().unwrap()).collect();
  assert_eq!((perms[0]["toolCall"]["title"].as_str(), kinds), (Some("ls"), vec!["allow_once", "reject_once"]));
  let card = &perms[1]["toolCall"];
  assert_eq!((card["kind"].as_str(), card["title"].as_str()), (Some("switch_mode"), Some("Exit plan mode")));
  assert_eq!(card["_meta"]["acpira/planApproval"], true);
  assert_eq!(card["rawInput"]["plan"], "# Add src.txt\n\n1. Write it.\n");
  assert_eq!(card["rawInput"]["planFilePath"], plan.to_string_lossy().as_ref());
  assert_eq!(perms[1]["options"], json!([
    { "optionId": "approved", "name": "Build", "kind": "allow_once" },
    { "optionId": "rejected", "name": "Revise", "kind": "reject_once" },
  ]));

  assert!(h.updates().iter().any(|u| u["sessionUpdate"] == "current_mode_update" && u["currentModeId"] == "agent"));
  assert!(tool_result(&server, 2, 0).starts_with("The user approved the plan. Plan mode is off."));
  // From the approval on, the turn runs with Agent mode's rules; the tools stay as they were, so the prefix holds
  assert_eq!(tool_names(&server, 2), tool_names(&server, 0));
  assert_eq!(std::fs::read_to_string(h.cwd().join("src.txt")).unwrap(), "x");
}

#[tokio::test]
async fn an_unapproved_plan_ends_the_turn_and_leaving_plan_mode_is_announced() {
  let (h, server, sid) = setup().await;
  set_mode(&h, &sid, "plan").await;
  *h.client.answer.lock() = Answer::Reject;
  server.push(mock::tools(&[("c1", "exit_plan", json!({}))]));
  server.push(mock::tools(&[("c2", "ExitPlanMode", json!({ "plan": "# Plan\n\nDo it.\n" }))]));
  let r = prompt(&h, &sid, "plan it").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert!(tool_result(&server, 1, 0).starts_with("There is no plan yet. Write it to"), "{}", tool_result(&server, 1, 0));
  assert_eq!(server.requests().len(), 2, "the refusal ended the turn");
  let perms = h.client.permissions.lock().clone();
  assert_eq!((perms.len(), perms[0]["toolCall"]["rawInput"]["plan"].as_str()), (1, Some("# Plan\n\nDo it.\n")));
  assert!(!h.updates().iter().any(|u| u["sessionUpdate"] == "current_mode_update"));

  // Still in Plan mode, already announced: the next message goes as typed
  server.push(mock::text("revising"));
  prompt(&h, &sid, "shorter").await;
  assert_eq!(user_text(&server, 2), "shorter");
  let last = server.requests()[2].body["messages"].as_array().unwrap().clone();
  assert!(last.iter().any(|m| m["role"] == "tool" && m["content"].as_str().unwrap().contains("The user did not approve the plan yet")));
  // The user leaves Plan mode from the picker: the model hears it with the next message
  set_mode(&h, &sid, "agent").await;
  server.push(mock::text("ok"));
  prompt(&h, &sid, "just do it").await;
  assert!(user_text(&server, 3).starts_with("<mode>\nPlan mode is off.") && user_text(&server, 3).ends_with("just do it"));
  // Within one mode the request prefix stays byte for byte
  let (a, b) = (server.requests()[1].body["messages"].as_array().unwrap().clone(), server.requests()[2].body["messages"].as_array().unwrap().clone());
  assert!(a.iter().zip(&b).all(|(x, y)| x == y) && b.len() > a.len());
  assert_eq!(tool_names(&server, 1), tool_names(&server, 2));
}
