//! Permission requests applied to the row they name

use super::*;

fn permission_req() -> Value {
  // OpenCode's embedded copy: kind 'other', the parent dir as title, file + dir locations, rawInput { filepath, parentDir }
  json!({ "toolCallId": "w1", "kind": "other", "status": "pending", "title": "/tmp/proj",
    "locations": [{ "path": "/tmp/proj/a.txt" }, { "path": "/tmp/proj" }],
    "rawInput": { "filepath": "/tmp/proj/a.txt", "parentDir": "/tmp/proj" } })
}

#[test]
fn permission_without_a_block_applies_the_request_whole() {
  let mut want = permission_req();
  want["sessionUpdate"] = json!("tool_call_update");
  expect_match(permission_tool_update(None, &permission_req()), want);
}

#[test]
fn permission_on_a_title_only_block_takes_raw_input_for_the_real_path() {
  let block: ToolCallBlock = serde_json::from_value(json!({ "type": "tool_call", "id": "w1", "kind": "edit", "verb": "Edit", "status": "pending", "locations": [], "target": "write" })).unwrap();
  let u = permission_tool_update(Some(&block), &permission_req());
  expect_match(&u, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "w1", "status": "pending", "title": "/tmp/proj", "rawInput": permission_req()["rawInput"] }));
  expect_absent(&u, "kind");
  expect_absent(&u, "locations");
  // Through mergeTool the forwarded rawInput becomes the single file location and the target
  let mut s = state();
  let mut b = v(&block);
  b["type"] = json!("tool_call");
  s.turns.push(turn(json!({ "role": "agent", "blocks": [b] })));
  apply_update(&mut s, &u);
  expect_match(block_of(&s), json!({ "kind": "edit", "locations": [{ "path": "/tmp/proj/a.txt" }], "target": "a.txt" }));
}

fn block_of(s: &NormalizeState) -> Value {
  block(s, 0, 0)
}

#[test]
fn permission_on_a_block_that_knows_its_file_only_takes_the_status() {
  let block: ToolCallBlock = serde_json::from_value(json!({ "type": "tool_call", "id": "w1", "kind": "edit", "verb": "Edit", "status": "in_progress",
    "locations": [{ "path": "/tmp/proj/a.txt" }], "target": "a.txt" })).unwrap();
  let u = permission_tool_update(Some(&block), &permission_req());
  expect_match(&u, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "w1", "status": "pending" }));
  for k in ["kind", "title", "locations", "rawInput"] {
    expect_absent(&u, k);
  }
}
