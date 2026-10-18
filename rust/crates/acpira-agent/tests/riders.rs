//! Calls that do more than their own tool's work, through whole turns against the scripted model: a to-do list riding
//! on another call (`todos`), a command run after a change (`then_run`), one edit across several files. Each rider shows
//! as a row of its own and asks like its own tool would; the model gets one result for the call

mod support;

use std::time::Duration;

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

/// The content of a message `nth_from_end` places from the end of a request
fn message(server: &MockModel, request: usize, nth_from_end: usize) -> String {
  let msgs = server.requests()[request].body["messages"].as_array().unwrap().clone();
  msgs[msgs.len() - 1 - nth_from_end]["content"].as_str().unwrap().to_owned()
}

/// Every update of one ACP tool call, in order
fn row(h: &Harness, id: &str) -> Vec<Value> {
  h.updates().into_iter().filter(|u| u["toolCallId"] == id).collect()
}

fn spec<'a>(tools: &'a [Value], name: &str) -> &'a Value {
  tools.iter().find(|t| t["function"]["name"] == name).unwrap_or_else(|| panic!("no {name} tool"))
}

#[tokio::test]
async fn the_tools_offer_the_riders() {
  let (h, server, sid) = setup().await;
  server.push(mock::text("ok"));
  prompt(&h, &sid, "hi").await;
  let tools = server.requests()[0].body["tools"].as_array().unwrap().clone();
  let props = |name: &str| spec(&tools, name)["function"]["parameters"]["properties"].clone();
  for name in ["read", "write", "edit", "bash"] {
    assert_eq!(props(name)["todos"]["type"], "array", "{name} takes the list");
  }
  for name in ["write", "edit"] {
    assert_eq!(props(name)["then_run"]["type"], "string", "{name} takes a command");
  }
  assert!(props("grep")["todos"].is_null() && props("bash")["then_run"].is_null());
  assert_eq!(props("grep")["context"]["type"], "integer");
  assert_eq!(props("edit")["edits"]["items"]["properties"]["path"]["type"], "string");
  assert!(spec(&tools, "todo")["function"]["description"].as_str().unwrap().contains("pass it as todos on that read, edit, write or bash call"));
}

#[tokio::test]
async fn a_to_do_list_riding_on_an_edit_shows_as_a_todo_row_of_its_own() {
  let (h, server, sid) = setup().await;
  h.approval("full");
  std::fs::write(h.cwd().join("a.txt"), "hello world\n").unwrap();
  let todos = json!([{ "content": "fix the greeting", "status": "completed" }, { "content": "run the tests", "status": "in_progress" }]);
  server.push(mock::tools(&[("c1", "edit", json!({ "path": "a.txt", "old_string": "world", "new_string": "there", "todos": todos }))]));
  // An empty list, as models send for an optional array, is no update
  server.push(mock::tools(&[("c2", "read", json!({ "path": "a.txt", "todos": [] }))]));
  server.push(mock::text("done"));
  assert_eq!(prompt(&h, &sid, "change it").await["stopReason"], "end_turn");
  assert_eq!(std::fs::read_to_string(h.cwd().join("a.txt")).unwrap(), "hello there\n");
  // call-1 is the edit, call-2 the list it carried: titled like the todo tool's row, so the host's to-do bar takes it
  let todo = row(&h, "call-2");
  assert_eq!((todo[0]["sessionUpdate"].as_str(), todo[0]["title"].as_str()), (Some("tool_call"), Some("todo")));
  let done = todo.last().unwrap();
  assert_eq!(done["status"], "completed");
  assert_eq!(done["rawOutput"]["todos"][1], json!({ "content": "run the tests", "status": "in_progress" }));
  assert!(row(&h, "call-1").iter().any(|u| u["status"] == "completed" && u["content"][0]["type"] == "diff"));
  // One result for the call, the list's note in front
  assert_eq!(message(&server, 1, 0), "To-do list updated (0 pending, 1 in progress, 1 completed).\n\nEdited a.txt.");
  assert!(message(&server, 2, 0).starts_with("/"), "no note for an empty list: {}", message(&server, 2, 0));
  assert_eq!(h.updates().iter().filter(|u| u["title"] == "todo").count(), 1);
  // A list that does not parse leaves the bar alone, and the call still runs
  server.push(mock::tools(&[("c3", "bash", json!({ "command": "echo ran", "todos": [{ "content": "x", "status": "someday" }] }))]));
  server.push(mock::text("ok"));
  prompt(&h, &sid, "again").await;
  let result = message(&server, 4, 0);
  assert!(result.starts_with("The to-do list was not updated: todos[0] has an unknown status") && result.contains("\n\nran"), "{result}");
}

#[cfg(unix)]
#[tokio::test]
async fn then_run_runs_after_the_change_on_a_command_row_with_its_own_card() {
  let (h, server, sid) = setup().await;
  std::fs::write(h.cwd().join("a.txt"), "hello world\n").unwrap();
  server.push(mock::tools(&[("c1", "edit", json!({ "path": "a.txt", "old_string": "world", "new_string": "there", "then_run": "cat a.txt" }))]));
  server.push(mock::text("done"));
  prompt(&h, &sid, "change and check").await;
  // Two cards, as an edit and a command would have: the second is the command's own
  let perms = h.client.permissions.lock().clone();
  assert_eq!(perms.len(), 2);
  assert_eq!((perms[0]["toolCall"]["kind"].as_str(), perms[1]["toolCall"]["kind"].as_str()), (Some("edit"), Some("execute")));
  assert_eq!(perms[1]["toolCall"]["title"], "cat a.txt");
  assert_eq!(perms[1]["options"][1]["name"], "Always allow `cat *` in this session");
  // The edit's row completes before the command's row opens; the command streams its output to its own row
  let ups = h.updates();
  let edit_done = ups.iter().position(|u| u["toolCallId"] == "call-1" && u["status"] == "completed").unwrap();
  let command_opened = ups.iter().position(|u| u["toolCallId"] == "call-2" && u["sessionUpdate"] == "tool_call").unwrap();
  assert!(edit_done < command_opened);
  assert_eq!(ups[command_opened]["kind"], "execute");
  assert!(row(&h, "call-2").iter().any(|u| u["_meta"]["terminal_output_delta"]["data"].as_str().is_some_and(|d| d.contains("hello there"))));
  assert_eq!(row(&h, "call-2").last().unwrap()["status"], "completed");
  assert_eq!(message(&server, 1, 0), "Edited a.txt.\n\n$ cat a.txt\nhello there");
  assert_eq!(server.requests().len(), 2, "the check took no request of its own");
}

#[cfg(unix)]
#[tokio::test]
async fn then_run_is_skipped_when_the_change_fails_or_changes_nothing() {
  let (h, server, sid) = setup().await;
  h.approval("full");
  std::fs::write(h.cwd().join("a.txt"), "same\n").unwrap();
  server.push(mock::tools(&[
    ("c1", "edit", json!({ "path": "a.txt", "old_string": "missing", "new_string": "x", "then_run": "touch ran1" })),
    ("c2", "write", json!({ "path": "a.txt", "content": "same\n", "then_run": "touch ran2" })),
    ("c3", "bash", json!({ "command": "echo hi", "then_run": "touch ran3" })),
  ]));
  server.push(mock::text("ok"));
  prompt(&h, &sid, "go").await;
  for marker in ["ran1", "ran2", "ran3"] {
    assert!(!h.cwd().join(marker).exists(), "{marker}");
  }
  assert!(message(&server, 1, 2).ends_with("\n\nthen_run `touch ran1` was not run: the change failed."), "{}", message(&server, 1, 2));
  assert!(message(&server, 1, 1).ends_with("\n\nthen_run `touch ran2` was not run: the file already held this content."), "{}", message(&server, 1, 1));
  assert!(message(&server, 1, 0).ends_with("hi\n\nthen_run is taken only by edit and write, so `touch ran3` was not run."), "{}", message(&server, 1, 0));
  assert!(!h.updates().iter().any(|u| u["kind"] == "execute" && u["title"].as_str().is_some_and(|t| t.starts_with("touch"))), "no command row");
}

#[cfg(unix)]
#[tokio::test]
async fn a_rejected_then_run_ends_the_turn_and_the_change_stays_made() {
  let (h, server, sid) = setup().await;
  // Edits run without a card, commands ask: the only card is the command's
  h.approval("auto-edit");
  *h.client.answer.lock() = Answer::Reject;
  std::fs::write(h.cwd().join("a.txt"), "v1\n").unwrap();
  server.push(mock::tools(&[
    ("c1", "edit", json!({ "path": "a.txt", "old_string": "v1", "new_string": "v2", "then_run": "touch ran" })),
    ("c2", "write", json!({ "path": "b.txt", "content": "later\n" })),
  ]));
  let r = prompt(&h, &sid, "go").await;
  assert_eq!(r["stopReason"], "end_turn");
  assert_eq!(std::fs::read_to_string(h.cwd().join("a.txt")).unwrap(), "v2\n");
  assert!(!h.cwd().join("ran").exists() && !h.cwd().join("b.txt").exists(), "the command and the rest of the round did not run");
  assert_eq!(h.client.permissions.lock().len(), 1);
  assert_eq!(server.requests().len(), 1, "no model call after a rejection");
  server.push(mock::text("ok"));
  prompt(&h, &sid, "never mind").await;
  let msgs = server.requests()[1].body["messages"].as_array().unwrap().clone();
  let edit = msgs.iter().find(|m| m["role"] == "tool" && m["tool_call_id"] == "c1").unwrap()["content"].as_str().unwrap().to_owned();
  assert_eq!(edit, "Edited a.txt.\n\n$ touch ran\nNot run: the user rejected it. Do not retry it; wait for the user's direction.");
}

#[cfg(unix)]
#[tokio::test]
async fn a_cancel_during_then_run_tells_the_model_the_change_was_made() {
  let (h, server, sid) = setup().await;
  h.approval("full");
  std::fs::write(h.cwd().join("a.txt"), "v1\n").unwrap();
  server.push(mock::tools(&[("c1", "edit", json!({ "path": "a.txt", "old_string": "v1", "new_string": "v2", "then_run": "sleep 30" }))]));
  let conn = h.conn.clone();
  let sid2 = sid.clone();
  let turn = tokio::spawn(async move { conn.request("session/prompt", json!({ "sessionId": sid2, "prompt": [{ "type": "text", "text": "go" }] })).await });
  while !h.updates().iter().any(|u| u["toolCallId"] == "call-2" && u["status"] == "in_progress") {
    tokio::time::sleep(Duration::from_millis(10)).await;
  }
  h.conn.notify("session/cancel", json!({ "sessionId": sid }));
  assert_eq!(turn.await.unwrap().unwrap()["stopReason"], "cancelled");
  assert_eq!(std::fs::read_to_string(h.cwd().join("a.txt")).unwrap(), "v2\n");
  server.push(mock::text("ok"));
  prompt(&h, &sid, "next").await;
  let msgs = server.requests()[1].body["messages"].as_array().unwrap().clone();
  let edit = msgs.iter().find(|m| m["role"] == "tool" && m["tool_call_id"] == "c1").unwrap()["content"].as_str().unwrap().to_owned();
  assert_eq!(edit, "Edited a.txt.\n\nthen_run `sleep 30` was cancelled by the user before it finished.");
}

#[tokio::test]
async fn one_edit_changes_several_files_behind_one_card() {
  let (h, server, sid) = setup().await;
  std::fs::write(h.cwd().join("a.txt"), "fn old() {}\n").unwrap();
  std::fs::write(h.cwd().join("b.txt"), "old();\n").unwrap();
  server.push(mock::tools(&[(
    "c1",
    "edit",
    json!({ "path": "a.txt", "edits": [{ "old_string": "fn old", "new_string": "fn new" }, { "path": "b.txt", "old_string": "old()", "new_string": "new()" }] }),
  )]));
  server.push(mock::text("done"));
  prompt(&h, &sid, "rename").await;
  let perms = h.client.permissions.lock().clone();
  assert_eq!(perms.len(), 1, "one card for both files");
  assert_eq!(perms[0]["toolCall"]["title"], "Edit a.txt, b.txt");
  assert_eq!(perms[0]["toolCall"]["content"].as_array().unwrap().len(), 2, "a diff per file");
  assert_eq!(perms[0]["toolCall"]["locations"].as_array().unwrap().len(), 2);
  assert_eq!(std::fs::read_to_string(h.cwd().join("b.txt")).unwrap(), "new();\n");
  assert_eq!(message(&server, 1, 0), "Edited 2 files: a.txt, b.txt.");
}

#[tokio::test]
async fn plan_mode_names_the_file_an_edit_across_files_may_not_touch() {
  let (h, server, sid) = setup().await;
  h.conn.request("session/set_mode", json!({ "sessionId": sid, "modeId": "plan" })).await.unwrap();
  std::fs::write(h.cwd().join("a.txt"), "x\n").unwrap();
  let plan = h.home().join("agent/sessions").join(&sid).join("plan.md");
  std::fs::create_dir_all(plan.parent().unwrap()).unwrap();
  std::fs::write(&plan, "# Plan\n").unwrap();
  server.push(mock::tools(&[(
    "c1",
    "edit",
    json!({ "path": plan, "edits": [{ "old_string": "# Plan", "new_string": "# Plan v2" }, { "path": "a.txt", "old_string": "x", "new_string": "y" }] }),
  )]));
  server.push(mock::text("ok"));
  prompt(&h, &sid, "plan it").await;
  assert_eq!(message(&server, 1, 0), "Not allowed: the permission rules deny edit on a.txt. Do not retry it.");
  assert_eq!(std::fs::read_to_string(&plan).unwrap(), "# Plan\n", "all or none");
}
