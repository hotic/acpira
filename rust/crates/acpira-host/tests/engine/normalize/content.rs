//! Tool content: diffs and patch titles, thoughts, content items in wire order, writes and receipts

use super::*;

#[test]
fn generic_patch_titles_take_the_actual_file_names_from_diffs() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "patch", "kind": "edit", "title": "Edit files", "status": "pending" }));
  assert_ne!(block(&s, 0, 0)["target"], "s");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "patch", "status": "completed", "locations": [], "content": [
    { "type": "diff", "path": "/repo/docs/TASKS.md", "oldText": "old", "newText": "new" },
    { "type": "diff", "path": "/repo/src/open.rs", "oldText": "before", "newText": "after" }
  ] }));
  expect_match(block(&s, 0, 0), json!({ "target": "TASKS.md, open.rs" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "patch", "status": "completed" }));
  assert_eq!(block(&s, 0, 0)["target"], "TASKS.md, open.rs");
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "read-file", "kind": "read", "title": "Read file.ts" }));
  assert_eq!(block(&s, 0, 1)["target"], "file.ts");
}

#[test]
fn diff_lines_keeps_only_context_near_changes() {
  let old = (0..20).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
  let neu = old.replace("line 10", "LINE 10");
  let lines = diff_lines(&old, &neu);
  let of = |k: DiffKind| lines.iter().filter(|l| l.kind == k).collect::<Vec<_>>();
  assert_eq!(of(DiffKind::Del).iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["-line 10"]);
  assert_eq!(of(DiffKind::Add).iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), ["+LINE 10"]);
  // Leading omission stays (line offset), the trailing one is dropped: nothing follows it
  assert_eq!(of(DiffKind::Hunk).len(), 1);
  assert_eq!(lines[0].kind, DiffKind::Hunk);
  assert_eq!(lines.last().unwrap().kind, DiffKind::Ctx);
  assert_eq!(of(DiffKind::Ctx).len(), 6);
}

#[test]
fn diff_lines_new_file_is_all_add() {
  assert_eq!(diff_lines("", "a\nb").iter().map(|l| l.kind).collect::<Vec<_>>(), [DiffKind::Add, DiffKind::Add]);
}

#[test]
fn ignores_empty_thought_deltas_while_retaining_whitespace_inside_real_reasoning() {
  let mut s = state();
  assert!(!apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "" } })));
  assert!(s.turns.is_empty());
  for text in ["Hello", " ", "world", ""] {
    apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": text } }));
  }
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "read", "title": "Read", "kind": "read" }));
  for text in ["", "\n  "] {
    assert!(!apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": text } })));
  }
  expect_match(&s.turns[0], json!({ "blocks": [{ "type": "thought", "text": "Hello world", "streaming": false }, { "type": "tool_call" }] }));
}

// anvil over an OpenAI-compatible API: `content: ""` rides every reasoning delta while thoughts arrive in batches
#[test]
fn empty_or_blank_message_chunks_between_thought_batches_keep_one_thought() {
  let mut s = state();
  for (kind, text) in [
    ("agent_thought_chunk", "verify the current"),
    ("agent_message_chunk", ""),
    ("agent_message_chunk", "\n\n"),
    ("agent_thought_chunk", " file content"),
    ("agent_message_chunk", "Done"),
    ("agent_message_chunk", "\n"),
  ] {
    apply(&mut s, json!({ "sessionUpdate": kind, "content": { "type": "text", "text": text } }));
  }
  expect_match(
    &s.turns[0],
    json!({ "blocks": [{ "type": "thought", "text": "verify the current file content", "streaming": false }, { "type": "text", "markdown": "Done\n" }] }),
  );
}

#[test]
fn kimi_edit_diff_followed_by_a_text_result() {
  for status in ["completed", "failed"] {
    let mut s = state();
    apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "edit", "title": "Edit", "kind": "edit", "status": "pending" }));
    apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "edit", "status": "in_progress",
      "content": [{ "type": "diff", "path": "sample.ts", "oldText": "const n = 1;", "newText": "const n = 2;" }] }));
    let mut before = block(&s, 0, 0);
    let text = if status == "completed" { "Replaced 1 occurrence in sample.ts" } else { "File changed before edit" };
    apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "edit", "status": status,
      "content": [{ "type": "content", "content": { "type": "text", "text": text } }] }));
    if status == "completed" {
      before["status"] = json!(status);
      expect_match(block(&s, 0, 0), before);
    } else {
      expect_match(block(&s, 0, 0), json!({ "status": status, "content": { "type": "text", "text": text }, "diffStat": null }));
    }
  }
}

#[test]
fn several_content_items_keep_wire_order_with_the_diff_as_primary() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "edit", "title": "Edit", "kind": "edit", "status": "in_progress" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "edit", "status": "completed", "content": [
    { "type": "diff", "path": "a.ts", "oldText": "const n = 1;", "newText": "const n = 2;" },
    { "type": "content", "content": { "type": "text", "text": "ok" } },
    { "type": "diff", "path": "b.ts", "oldText": "x\ny", "newText": "x\nY\nz" },
  ] }));
  let b = block(&s, 0, 0);
  let kinds: Vec<&str> = b["contents"].as_array().unwrap().iter().map(|c| c["type"].as_str().unwrap()).collect();
  assert_eq!(kinds, ["diff", "text", "diff"]);
  assert_eq!(b["content"]["source"]["path"], "a.ts");
  // The stat sums every diff, not just the primary
  expect_eq(&b["diffStat"], json!({ "add": 3, "del": 2 }));
}

#[test]
fn a_single_content_item_leaves_contents_unset() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "edit", "title": "Edit", "kind": "edit", "status": "in_progress" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "edit", "status": "completed",
    "content": [{ "type": "diff", "path": "a.ts", "oldText": "const n = 1;", "newText": "const n = 2;" }] }));
  expect_match(block(&s, 0, 0), json!({ "content": { "type": "diff" } }));
  expect_absent(block(&s, 0, 0), "contents");
}

#[test]
fn consecutive_text_content_items_merge_into_one() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "sh", "title": "bash", "kind": "execute", "status": "completed", "content": [
    { "type": "content", "content": { "type": "text", "text": "a" } },
    { "type": "content", "content": { "type": "text", "text": "b" } },
    { "type": "content", "content": { "type": "text", "text": "c" } },
  ] }));
  expect_match(block(&s, 0, 0), json!({ "content": { "type": "text", "text": "a\nb\nc" } }));
  expect_absent(block(&s, 0, 0), "contents");
}

// OpenCode's write reports the file contents in the in_progress update's rawInput.content and completes with a bare
// receipt; metadata.exists === false proves the file is new, so the write renders as an all-add diff
#[test]
fn a_new_file_write_renders_as_an_all_add_diff_and_an_overwrite_keeps_the_receipt() {
  let play = |exists: bool| {
    let mut s = state();
    apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "w", "title": "write", "kind": "edit", "status": "pending", "locations": [], "rawInput": {} }));
    apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "w", "kind": "edit", "status": "in_progress",
      "locations": [{ "path": "/tmp/proj/a.txt" }], "rawInput": { "filePath": "/tmp/proj/a.txt", "content": "alpha\n" } }));
    apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "w", "status": "completed",
      "rawOutput": { "output": "Wrote file successfully.", "metadata": { "exists": exists, "filepath": "/tmp/proj/a.txt" } },
      "content": [{ "type": "content", "content": { "type": "text", "text": "Wrote file successfully." } }] }));
    block(&s, 0, 0)
  };
  let fresh = play(false);
  expect_match(&fresh["content"], json!({ "type": "diff", "source": { "path": "/tmp/proj/a.txt", "oldText": "", "newText": "alpha\n" } }));
  expect_eq(&fresh["diffStat"], json!({ "add": 1, "del": 0 }));
  let kinds: Vec<&str> = fresh["contents"].as_array().unwrap().iter().map(|c| c["type"].as_str().unwrap()).collect();
  assert_eq!(kinds, ["diff", "text"]);
  let overwritten = play(true);
  expect_eq(&overwritten["content"], json!({ "type": "text", "text": "Wrote file successfully." }));
  expect_absent(&overwritten, "contents");
}

#[test]
fn a_formatted_output_receipt_renders_as_plain_text_and_other_objects_stay_pretty_json() {
  let mut s = state();
  let run = |s: &mut NormalizeState, id: &str, raw: Value| {
    apply(s, json!({ "sessionUpdate": "tool_call", "toolCallId": id, "title": "bash", "kind": "execute", "status": "in_progress" }));
    apply(s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": "completed", "rawOutput": raw }));
  };
  run(&mut s, "sh", json!({ "formatted_output": "tests passed\n", "exit_code": 0 }));
  run(&mut s, "sh2", json!({ "formatted_output": "boom", "exit_code": 2 }));
  run(&mut s, "sh3", json!({ "output": "done", "metadata": { "exists": true } }));
  expect_match(block(&s, 0, 0), json!({ "content": { "type": "text", "text": "tests passed\n" } }));
  expect_match(block(&s, 0, 1), json!({ "content": { "type": "text", "text": "boom\nexit code 2" } }));
  expect_match(block(&s, 0, 2), json!({ "content": { "type": "text", "text": "{\n  \"output\": \"done\",\n  \"metadata\": {\n    \"exists\": true\n  }\n}" } }));
}
