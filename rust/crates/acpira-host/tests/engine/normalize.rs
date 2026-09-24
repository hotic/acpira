//! test/normalize.test.ts

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::acp::diff::diff_lines;
use acpira_host::acp::normalize::*;
use acpira_host::acp::wire::{AsyncTaskEvent, TaskEventKind};
use acpira_shared::transcript::*;

use crate::support::{Clock, expect_absent, expect_eq, expect_match, v};

fn state() -> NormalizeState {
  NormalizeState::new(vec![])
}

fn apply(s: &mut NormalizeState, u: Value) -> bool {
  apply_update(s, &u)
}

fn turn(j: Value) -> Turn {
  serde_json::from_value(j).expect("turn")
}

fn block(s: &NormalizeState, turn: usize, i: usize) -> Value {
  v(&s.turns[turn])["blocks"][i].clone()
}

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn saver(f: impl Fn(&str, &str) -> Option<String> + Send + Sync + 'static) -> Option<ImageSaver> {
  Some(Arc::new(f))
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

// pi-acp 0.0.33 shape: the tool_call carries a terminal placeholder, output and exit arrive in update _meta
#[test]
fn terminal_output_in_meta_deltas_concatenates_and_exit_0_adds_no_suffix() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "t", "title": "bash", "kind": "execute", "status": "in_progress",
    "content": [{ "type": "terminal", "terminalId": "term-1" }], "_meta": { "terminal_info": { "terminal_id": "term-1", "cwd": "/repo" } } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "_meta": { "terminal_output": { "terminal_id": "term-1", "data": "hel" } } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "_meta": { "terminal_output": { "terminal_id": "term-1", "data": "lo\n" } } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "status": "completed",
    "_meta": { "terminal_exit": { "terminal_id": "term-1", "exit_code": 0, "signal": null } } }));
  expect_match(block(&s, 0, 0), json!({ "status": "completed", "content": { "type": "text", "text": "hello\n" } }));
}

#[test]
fn a_nonzero_terminal_exit_appends_the_exit_code_once() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "t", "title": "bash", "kind": "execute", "status": "in_progress",
    "content": [{ "type": "terminal", "terminalId": "term-1" }] }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "_meta": { "terminal_output": { "terminal_id": "term-1", "data": "hi\n" } } }));
  for _ in 0..2 {
    apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "status": "completed",
      "_meta": { "terminal_exit": { "terminal_id": "term-1", "exit_code": 2, "signal": null } } }));
  }
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "t2", "title": "bash", "kind": "execute", "status": "in_progress",
    "content": [{ "type": "terminal", "terminalId": "term-2" }], "_meta": { "terminal_info": { "terminal_id": "term-2", "cwd": "/repo" } } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t2", "status": "completed",
    "_meta": { "terminal_exit": { "terminal_id": "term-2", "exit_code": 2, "signal": null } } }));
  let text = block(&s, 0, 0)["content"]["text"].as_str().unwrap().to_owned();
  assert!(text.ends_with("exit code 2"));
  assert_eq!(text.matches("exit code 2").count(), 1);
  // No output deltas at all: the suffix stands alone instead of opening with a blank line
  expect_match(block(&s, 0, 1), json!({ "content": { "type": "text", "text": "exit code 2" } }));
}

#[test]
fn a_bare_terminal_item_with_no_meta_still_shows_the_not_wired_placeholder() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "t", "title": "bash", "kind": "execute", "status": "in_progress",
    "content": [{ "type": "terminal", "terminalId": "term-9" }] }));
  expect_match(block(&s, 0, 0), json!({ "content": { "type": "text", "text": "Terminal term-9 (output not wired up)" } }));
}

#[test]
fn times_observed_execution_across_sparse_updates_excluding_pending_approval_and_replay() {
  let mut s = state();
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1000 })));
  let clock = Clock::at(2000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "sh", "title": "bash", "status": "pending" }));
  expect_absent(block(&s, 0, 0), "startedAt");
  clock.set(5000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "status": "in_progress" }));
  clock.set(8000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "kind": "execute", "rawInput": { "command": "pnpm test" } }));
  expect_match(&s.turns[0], json!({ "blocks": [{ "startedAt": 5000 }] }));
  clock.set(14000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "status": "completed" }));
  clock.set(19000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "status": "completed" }));
  end_turn(&mut s, TurnStop::EndTurn);
  expect_match(&s.turns[0], json!({ "blocks": [{ "startedAt": 5000, "endedAt": 14000 }] }));

  let mut replay = state();
  apply(&mut replay, json!({ "sessionUpdate": "tool_call", "toolCallId": "old", "title": "bash", "status": "in_progress" }));
  apply(&mut replay, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "old", "status": "completed" }));
  expect_absent(block(&replay, 0, 0), "startedAt");
}

#[test]
fn freezes_outstanding_tool_timers_when_the_turn_ends() {
  for (stop, status) in [(TurnStop::Cancelled, "cancelled"), (TurnStop::EndTurn, "failed")] {
    let mut s = state();
    s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1000 })));
    let clock = Clock::at(2000);
    apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "kind": "execute", "status": "in_progress" }));
    clock.set(6000);
    end_turn(&mut s, stop);
    expect_match(&s.turns[0], json!({ "blocks": [{ "startedAt": 2000, "endedAt": 6000, "status": status }] }));
  }
}

#[test]
fn records_the_full_live_turn_duration_once_without_inventing_replay_timestamps() {
  let mut s = state();
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1000 })));
  let clock = Clock::at(287000);
  end_turn(&mut s, TurnStop::EndTurn);
  expect_match(&s.turns[0], json!({ "startedAt": 1000, "endedAt": 287000 }));
  clock.set(300000);
  end_turn(&mut s, TurnStop::EndTurn);
  expect_match(&s.turns[0], json!({ "endedAt": 287000 }));
  let mut replay = state();
  replay.turns.push(turn(json!({ "role": "agent", "blocks": [] })));
  end_turn(&mut replay, TurnStop::EndTurn);
  expect_absent(&replay.turns[0], "endedAt");
}

#[test]
fn switching_from_a_thought_block_to_a_text_block_finalizes_it_and_records_the_duration() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "a" } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "b" } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "x" } }));
  expect_match(block(&s, 0, 0), json!({ "type": "thought", "text": "ab", "streaming": false }));
  assert!(block(&s, 0, 0)["durationSec"].as_f64().unwrap() >= 1.0);
  expect_match(block(&s, 0, 1), json!({ "type": "text", "markdown": "x", "streaming": true }));
}

#[test]
fn tool_call_update_without_a_tool_call_inserts_one_and_execute_targets_the_command() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "x", "kind": "execute", "status": "in_progress", "rawInput": { "command": "ls -la" } }));
  expect_match(block(&s, 0, 0), json!({ "type": "tool_call", "id": "x", "verb": "Run", "target": "ls -la", "targetMono": true, "status": "in_progress" }));
}

// Devin 3000.6.14 wire shape: exec past its timeout is parked with cognition.ai/background, then get_output ("Read shell", no kind) blocks on it
#[test]
fn devin_background_shell_parks_the_exec_and_get_output_waits_on_that_command() {
  let mut s = state();
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1 })));
  let label = |s: &NormalizeState| activity_of(&s.turns).map(|a| a.label);
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "exec:0", "title": "Ran python3", "kind": "execute",
    "rawInput": { "command": "python3 snap.py save", "timeout": 10000 }, "_meta": { "cognition.ai/inferenceToolName": "exec" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "exec:0", "status": "in_progress", "_meta": { "cognition.ai/inferenceToolName": "exec" } }));
  assert_eq!(label(&s).as_deref(), Some("Run python3 snap.py save"));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "exec:0", "status": "in_progress", "_meta": {
    "cognition.ai/inferenceToolName": "exec", "cognition.ai/background": true, "cognition.ai/backgroundShellId": "0d95e3",
    "cognition.ai/backgroundCommand": "python3 snap.py save" } }));
  expect_match(block(&s, 0, 0), json!({ "kind": "execute", "status": "in_progress", "background": true, "target": "python3 snap.py save" }));
  // The agent has moved on: a streaming thought is the current action, not the parked command
  apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "checking progress" } }));
  assert_eq!(label(&s).as_deref(), Some("Working"));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "get_output:1", "title": "Read shell",
    "rawInput": { "shell_id": "0d95e3", "timeout": 60000 }, "_meta": { "cognition.ai/inferenceToolName": "get_output" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "get_output:1", "status": "in_progress" }));
  expect_match(block(&s, 0, 2), json!({ "kind": "other", "verbKey": "verb.wait", "verb": "Wait for background command",
    "target": "python3 snap.py save", "targetMono": true, "status": "in_progress" }));
  expect_absent(block(&s, 0, 2), "background");
  assert_eq!(label(&s).as_deref(), Some("Wait for background command python3 snap.py save"));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "get_output:1", "status": "completed", "_meta": { "cognition.ai/inferenceToolName": "get_output" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "exec:0", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "text", "text": "saved 33/33" } }],
    "_meta": { "cognition.ai/inferenceToolName": "exec", "terminal_exit": { "terminal_id": "0d95e3", "exit_code": 0, "signal": null } } }));
  expect_match(block(&s, 0, 0), json!({ "status": "completed", "content": { "type": "text", "text": "saved 33/33" } }));
  expect_match(block(&s, 0, 2), json!({ "status": "completed", "target": "python3 snap.py save" }));
}

#[test]
fn a_wait_on_an_unknown_shell_falls_back_to_the_shell_id() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "w", "title": "Read shell", "status": "in_progress", "rawInput": { "shell_id": "abc123", "timeout": 5000 } }));
  expect_match(block(&s, 0, 0), json!({ "kind": "other", "verbKey": "verb.wait", "target": "abc123", "targetMono": true }));
}

#[test]
fn kill_shell_stops_the_parked_command_by_name() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "exec_0", "title": "Ran sleep", "kind": "execute", "rawInput": { "command": "sleep 120", "timeout": 3000 } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "exec_0", "status": "in_progress",
    "_meta": { "cognition.ai/background": true, "cognition.ai/backgroundShellId": "481dbc", "cognition.ai/backgroundCommand": "sleep 120" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "kill_shell_3", "title": "Kill shell", "rawInput": { "shell_id": "481dbc" },
    "_meta": { "cognition.ai/inferenceToolName": "kill_shell" } }));
  expect_match(block(&s, 0, 1), json!({ "kind": "other", "verbKey": "verb.kill", "verb": "Stop background command", "target": "sleep 120", "targetMono": true }));
}

#[test]
fn end_turn_cancels_running_tools_and_records_the_stop_reason() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "a", "title": "t", "status": "in_progress" }));
  end_turn(&mut s, TurnStop::Cancelled);
  expect_match(block(&s, 0, 0), json!({ "status": "cancelled" }));
  assert_eq!(v(&s.turns[0])["stop"], "cancelled");
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "partial" } }));
  end_turn(&mut s, TurnStop::MaxTokens);
  assert_eq!(v(&s.turns[0])["stop"], "max_tokens");
  expect_match(block(&s, 0, 1), json!({ "type": "text", "streaming": false }));
}

#[test]
fn fail_turn_seals_the_turn_like_a_cancellation_and_keeps_the_error() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "hm" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "a", "title": "t", "status": "in_progress" }));
  fail_turn(&mut s, serde_json::from_value(json!({ "message": "Upstream error", "code": -32603, "kind": "upstream_error", "retryable": true })).unwrap());
  let t = v(&s.turns[0]);
  assert_eq!(t["stop"], "error");
  expect_eq(&t["error"], json!({ "message": "Upstream error", "code": -32603, "kind": "upstream_error", "retryable": true }));
  expect_absent(&t, "activity");
  expect_match(&t["blocks"][0], json!({ "type": "thought", "streaming": false }));
  expect_match(&t["blocks"][1], json!({ "status": "cancelled" }));
}

#[test]
fn a_later_plan_replaces_the_whole_list_within_the_turn() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "plan", "entries": [{ "content": "a", "priority": "high", "status": "in_progress" }, { "content": "b", "priority": "low", "status": "pending" }] }));
  apply(&mut s, json!({ "sessionUpdate": "plan", "entries": [{ "content": "a", "priority": "high", "status": "completed" }, { "content": "b", "priority": "low", "status": "in_progress" }] }));
  let t = v(&s.turns[0]);
  assert_eq!(t["blocks"].as_array().unwrap().len(), 1);
  expect_eq(&t["blocks"][0], json!({ "type": "plan", "changed": true,
    "entries": [{ "title": "a", "status": "completed", "priority": "high" }, { "title": "b", "status": "in_progress", "priority": "low" }] }));
}

#[test]
fn available_commands_keep_the_input_hint_drop_meta_and_are_replaced_by_later_lists() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "available_commands_update", "availableCommands": [
    { "name": "compact", "description": "Compact", "_meta": { "x": 1 } },
    { "name": "review", "description": "Review", "input": { "hint": "files to review", "_meta": { "y": 2 } } },
    { "name": "plain", "description": "No hint", "input": null },
  ] }));
  expect_eq(&s.commands, json!([
    { "name": "compact", "description": "Compact" },
    { "name": "review", "description": "Review", "input": { "hint": "files to review" } },
    { "name": "plain", "description": "No hint" },
  ]));
  apply(&mut s, json!({ "sessionUpdate": "available_commands_update", "availableCommands": [{ "name": "review", "description": "Review" }] }));
  assert_eq!(s.commands.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["review"]);
  apply(&mut s, json!({ "sessionUpdate": "available_commands_update", "availableCommands": [] }));
  assert!(s.commands.is_empty());
}

#[test]
fn config_options_become_controls_sorted_by_category_with_mode_promoted() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "config_option_update", "configOptions": [
    { "id": "verbose", "name": "Verbose", "type": "boolean", "currentValue": true },
    { "id": "custom", "name": "Style", "type": "select", "currentValue": "x", "options": [{ "value": "x", "name": "X" }] },
    { "id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": "hi", "options": [{ "value": "lo", "name": "Lo" }, { "value": "hi", "name": "Hi" }] },
    { "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "plan", "options": [{ "value": "agent", "name": "Agent" }, { "value": "plan", "name": "Plan" }] },
    { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "b",
      "options": [{ "group": "g1", "name": "Group 1", "options": [{ "value": "a", "name": "A" }] }, { "group": "g2", "name": "Group 2", "options": [{ "value": "b", "name": "B" }] }] },
  ] }));
  assert_eq!(s.controls.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["model", "effort", "verbose", "custom"]);
  // A boolean option is a string-valued control everywhere but the wire (synthetic Off/On pair, value stringified)
  expect_eq(&s.controls.options[2], json!({ "id": "verbose", "name": "Verbose", "type": "boolean", "value": "true", "options": [{ "id": "false", "name": "Off" }, { "id": "true", "name": "On" }] }));
  expect_eq(&s.controls.options[0].options, json!([
    { "id": "a", "name": "A", "description": "Group 1", "group": { "id": "g1", "name": "Group 1" } },
    { "id": "b", "name": "B", "description": "Group 2", "group": { "id": "g2", "name": "Group 2" } },
  ]));
  assert_eq!(s.controls.options[0].value.as_deref(), Some("b"));
  assert_eq!(s.controls.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["agent", "plan"]);
  assert_eq!(s.controls.mode_id.as_deref(), Some("plan"));
  assert_eq!(s.controls.mode_config_id.as_deref(), Some("mode"));
}

#[test]
fn a_mode_config_option_duplicating_modes_stays_out_of_the_controls() {
  let mut s = state();
  s.controls = serde_json::from_value(json!({ "modes": [{ "id": "default", "name": "Default" }, { "id": "yolo", "name": "YOLO" }], "modeId": "default", "options": [] })).unwrap();
  apply(&mut s, json!({ "sessionUpdate": "config_option_update", "configOptions": [
    { "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "default", "options": [{ "value": "default", "name": "Default" }, { "value": "yolo", "name": "YOLO" }] },
    { "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": "k3", "options": [{ "value": "k3", "name": "K3" }] },
  ] }));
  assert_eq!(s.controls.options.iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), ["model"]);
  assert_eq!(s.controls.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["default", "yolo"]);
  assert_eq!(s.controls.mode_id.as_deref(), Some("default"));
  assert!(s.controls.mode_config_id.is_none());
}

#[test]
fn usage_update_stamps_the_context_snapshot_on_the_trailing_agent_turn_only() {
  let mut s = state();
  s.turns.push(turn(json!({ "role": "user", "text": "hi" })));
  s.turns.push(turn(json!({ "role": "agent", "blocks": [] })));
  apply(&mut s, json!({ "sessionUpdate": "usage_update", "used": 5000, "size": 100_000 }));
  expect_eq(s.usage, json!({ "used": 5000, "size": 100_000 }));
  expect_match(&s.turns[1], json!({ "usage": { "context": { "used": 5000, "size": 100_000 } } }));
  // Per-call updates keep overwriting: the last one before end_turn is the end-of-turn snapshot
  apply(&mut s, json!({ "sessionUpdate": "usage_update", "used": 7000, "size": 100_000 }));
  expect_match(&s.turns[1], json!({ "usage": { "context": { "used": 7000, "size": 100_000 } } }));

  // A trailing user turn is left alone: the snapshot never reaches back across it
  let mut t = state();
  t.turns.push(turn(json!({ "role": "agent", "blocks": [] })));
  t.turns.push(turn(json!({ "role": "user", "text": "next" })));
  apply(&mut t, json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2 }));
  expect_absent(&t.turns[0], "usage");
  expect_absent(&t.turns[1], "usage");
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

// codex-acp streams command output as _meta.terminal_output_delta and answers with a { formatted_output, exit_code } receipt
#[test]
fn terminal_output_delta_concatenates_and_marks_the_terminal_agent_wired() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "t", "title": "bash", "kind": "execute", "status": "in_progress",
    "content": [{ "type": "terminal", "terminalId": "term-1" }] }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "_meta": { "terminal_output_delta": { "terminal_id": "term-1", "data": "hel" } } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "_meta": { "terminal_output_delta": { "terminal_id": "term-1", "data": "lo\n" } } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "status": "completed",
    "_meta": { "terminal_exit": { "terminal_id": "term-1", "exit_code": 3, "signal": null } } }));
  expect_match(block(&s, 0, 0), json!({ "status": "completed", "content": { "type": "text", "text": "hello\nexit code 3" } }));
  // A delta alone already means the agent wired the terminal: no not-wired placeholder even mid-stream
  let mut s2 = state();
  apply(&mut s2, json!({ "sessionUpdate": "tool_call", "toolCallId": "t", "title": "bash", "kind": "execute", "status": "in_progress",
    "content": [{ "type": "terminal", "terminalId": "term-1" }] }));
  apply(&mut s2, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "t", "_meta": { "terminal_output_delta": { "terminal_id": "term-1", "data": "hi" } } }));
  assert_eq!(block(&s2, 0, 0)["content"]["text"], "hi");
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

#[test]
fn an_image_chunk_closes_the_text_run_and_keeps_the_saved_blob_name() {
  let saved = Arc::new(Mutex::new(Vec::<String>::new()));
  let mut s = state();
  let log = saved.clone();
  s.ctx.save_image = saver(move |_data, mime| {
    let mut l = log.lock().unwrap();
    l.push(mime.to_owned());
    Some(format!("blob-{}.png", l.len()))
  });
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "here is " } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "the dot" } }));
  expect_eq(&v(&s.turns[0])["blocks"], json!([
    { "type": "text", "markdown": "here is ", "streaming": false },
    { "type": "image", "id": "img-1", "mimeType": "image/png", "blob": "blob-1.png" },
    { "type": "text", "markdown": "the dot", "streaming": true },
  ]));
  assert_eq!(*saved.lock().unwrap(), ["image/png"]);
}

#[test]
fn without_a_saver_an_image_degrades_to_a_placeholder() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } }));
  let mut s2 = state();
  s2.ctx.save_image = saver(|_, _| None);
  apply(&mut s2, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": PNG, "mimeType": "image/tiff" } }));
  expect_match(block(&s, 0, 0), json!({ "type": "text", "markdown": "[image]" }));
  expect_match(block(&s2, 0, 0), json!({ "type": "text", "markdown": "[image: image/tiff, not shown]" }));
}

#[test]
fn a_tool_image_item_keeps_wire_order_splitting_the_text_run() {
  let mut s = state();
  s.ctx.save_image = saver(|_, _| Some("img.png".into()));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "im", "title": "view", "kind": "other", "status": "completed", "content": [
    { "type": "content", "content": { "type": "text", "text": "Revised prompt" } },
    { "type": "content", "content": { "type": "image", "data": PNG, "mimeType": "image/png", "uri": "/repo/red.png" } },
    { "type": "content", "content": { "type": "text", "text": "saved to disk" } },
  ] }));
  expect_match(block(&s, 0, 0), json!({ "contents": [
    { "type": "text", "text": "Revised prompt" },
    { "type": "image", "mimeType": "image/png", "blob": "img.png", "uri": "/repo/red.png" },
    { "type": "text", "text": "saved to disk" },
  ] }));
  // A lone image is the primary content — no contents list needed
  let mut s2 = state();
  s2.ctx.save_image = saver(|_, _| Some("img.png".into()));
  apply(&mut s2, json!({ "sessionUpdate": "tool_call", "toolCallId": "im", "title": "view", "kind": "other", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "image", "data": PNG, "mimeType": "image/png" } }] }));
  expect_match(block(&s2, 0, 0), json!({ "content": { "type": "image", "blob": "img.png" } }));
  expect_absent(block(&s2, 0, 0), "contents");
}

#[test]
fn a_resource_link_to_a_local_image_becomes_an_image_and_unreadable_stays_a_link() {
  use base64::Engine;
  let dir = tempfile::tempdir().unwrap();
  let png_path = dir.path().join("shot.png");
  std::fs::write(&png_path, base64::engine::general_purpose::STANDARD.decode(PNG).unwrap()).unwrap();
  let png = png_path.to_string_lossy().into_owned();
  let seen = Arc::new(Mutex::new(Vec::<String>::new()));
  let mut s = state();
  let log = seen.clone();
  s.ctx.save_image_file = Some(Arc::new(move |p: &str| {
    log.lock().unwrap().push(p.to_owned());
    Some("file.png".into())
  }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "v", "title": "view_image", "kind": "read", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "resource_link", "name": "shot.png", "uri": png } }] }));
  assert_eq!(*seen.lock().unwrap(), [png.clone()]);
  expect_match(block(&s, 0, 0), json!({ "content": { "type": "image", "mimeType": "image/png", "blob": "file.png", "uri": png } }));
  // A file:// uri resolves the same way
  let mut s2 = state();
  s2.ctx.save_image_file = Some(Arc::new(|_: &str| Some("f2.png".into())));
  apply(&mut s2, json!({ "sessionUpdate": "tool_call", "toolCallId": "v", "title": "view_image", "kind": "read", "status": "completed",
    "content": [{ "type": "content", "content": { "type": "resource_link", "name": "shot.png", "uri": format!("file://{png}") } }] }));
  expect_match(block(&s2, 0, 0), json!({ "content": { "type": "image", "blob": "f2.png" } }));
  // Non-image links, remote links and a saver that refuses all keep the link's text rendering
  let mut s3 = state();
  s3.ctx.save_image_file = Some(Arc::new(|_: &str| None));
  apply(&mut s3, json!({ "sessionUpdate": "tool_call", "toolCallId": "v", "title": "view", "kind": "read", "status": "completed", "content": [
    { "type": "content", "content": { "type": "resource_link", "name": "nope.png", "uri": "/missing/nope.png" } },
    { "type": "content", "content": { "type": "resource_link", "name": "notes", "uri": "/repo/notes.md" } },
    { "type": "content", "content": { "type": "resource_link", "name": "remote", "uri": "https://x.io/pic.png" } },
  ] }));
  expect_match(block(&s3, 0, 0), json!({ "content": { "type": "text", "text": "/missing/nope.png\n/repo/notes.md\nhttps://x.io/pic.png" } }));
}

#[test]
fn a_data_url_image_saves_the_decoded_payload_and_a_uri_only_image_keeps_the_reference() {
  let saved = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
  let mut s = state();
  let log = saved.clone();
  s.ctx.save_image = saver(move |data, mime| {
    log.lock().unwrap().push((data.to_owned(), mime.to_owned()));
    Some("i.png".into())
  });
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": "", "mimeType": "", "uri": format!("data:image/png;base64,{PNG}") } }));
  let mut s2 = state();
  s2.ctx.save_image = saver(|_, _| Some("x.png".into()));
  apply(&mut s2, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "image", "data": "", "mimeType": "image/png", "uri": "/tmp/red.png" } }));
  assert_eq!(*saved.lock().unwrap(), [(PNG.to_owned(), "image/png".to_owned())]);
  expect_match(block(&s, 0, 0), json!({ "type": "image", "blob": "i.png", "uri": null }));
  expect_match(block(&s2, 0, 0), json!({ "type": "image", "blob": null, "uri": "/tmp/red.png" }));
}

#[test]
fn boolean_controls_send_a_real_boolean_on_the_wire() {
  let bool_: ConfigControl = serde_json::from_value(json!({ "id": "fast", "name": "Fast", "type": "boolean", "value": "false", "options": [{ "id": "false", "name": "Off" }, { "id": "true", "name": "On" }] })).unwrap();
  expect_eq(config_option_set_value(Some(&bool_), "true"), json!({ "type": "boolean", "value": true }));
  expect_eq(config_option_set_value(Some(&bool_), "false"), json!({ "type": "boolean", "value": false }));
  let sel: ConfigControl = serde_json::from_value(json!({ "id": "model", "name": "Model", "value": "a", "options": [{ "id": "a", "name": "A" }] })).unwrap();
  expect_eq(config_option_set_value(Some(&sel), "b"), json!({ "value": "b" }));
  expect_eq(config_option_set_value(None, "x"), json!({ "value": "x" }));
}

#[test]
fn mode_meta_kind_lands_on_the_session_option() {
  let mut s = state();
  init_controls(&mut s.controls, Some(&json!({ "currentModeId": "default", "availableModes": [
    { "id": "default", "name": "Default" },
    { "id": "full", "name": "Full access", "_meta": { "kind": "full_access" } },
  ] })), None);
  assert_eq!(v(&s.controls.modes).as_array().unwrap().iter().map(|m| m["kind"].as_str()).collect::<Vec<_>>(), [None, Some("full_access")]);
}

#[test]
fn a_mode_config_select_keeps_meta_kind_on_its_options() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "config_option_update", "configOptions": [
    { "id": "mode", "name": "Mode", "category": "mode", "type": "select", "currentValue": "default", "options": [
      { "value": "default", "name": "Default" },
      { "value": "full", "name": "Full access", "_meta": { "kind": "full_access" } },
    ] },
  ] }));
  assert_eq!(v(&s.controls.modes).as_array().unwrap().iter().map(|m| m["kind"].as_str()).collect::<Vec<_>>(), [None, Some("full_access")]);
}

// OpenCode's session/load streams the whole history as live-looking chunks — no endTurn, no usage_update
#[test]
fn seal_replay_closes_a_replayed_history() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "first" } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "thinking" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "run", "title": "Run", "kind": "execute", "status": "in_progress" }));
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "one" } }));
  apply(&mut s, json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "second" } }));
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "two" } }));
  if let Some(Turn::Agent(a)) = s.turns.last_mut() {
    a.activity = Some(serde_json::from_value(json!({ "kind": "think", "label": "Working" })).unwrap());
  }
  seal_replay(&mut s);
  let turns = v(&s.turns);
  let roles: Vec<&str> = turns.as_array().unwrap().iter().map(|t| t["role"].as_str().unwrap()).collect();
  assert_eq!(roles, ["user", "agent", "user", "agent"]);
  assert!(s.open_user.is_none());
  for a in [&turns[1], &turns[3]] {
    assert_eq!(a["stop"], "end_turn");
    expect_absent(a, "activity");
    for b in a["blocks"].as_array().unwrap() {
      expect_absent(b, "streaming");
    }
  }
  expect_match(&turns[1]["blocks"], json!([{ "type": "thought" }, { "type": "tool_call", "status": "cancelled" }, { "type": "text", "markdown": "one" }]));
}

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

fn task(event: TaskEventKind, state: Option<&str>) -> AsyncTaskEvent {
  AsyncTaskEvent {
    event,
    async_task_id: "t1".into(),
    name: None,
    task_type: Some("shell".into()),
    description: None,
    show_in_transcript: false,
    can_stop: Some(true),
    output_file_path: None,
    tool_call_id: matches!(event, TaskEventKind::Spawned).then(|| "call-1".into()),
    summary: None,
    last_tool_name: None,
    usage: None,
    state: state.map(|x| serde_json::from_value(json!(x)).unwrap()),
  }
}

fn shell_task() -> NormalizeState {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "user_message_chunk", "content": { "type": "text", "text": "go" } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "sleep 15", "kind": "execute", "status": "in_progress" }));
  apply_async_task(&mut s, &task(TaskEventKind::Spawned, None));
  s
}

fn task_row(s: &NormalizeState) -> Value {
  v(s.turns.last().unwrap())["blocks"].as_array().unwrap().iter().find(|b| b["id"] == "call-1").cloned().unwrap()
}

// claude-agent-acp 0.81.0 on the wire: a best-effort 'stopped' and the authoritative 'completed' in the same millisecond
#[test]
fn the_authoritative_completed_edge_corrects_a_best_effort_stopped() {
  let mut s = shell_task();
  apply_async_task(&mut s, &task(TaskEventKind::State, Some("stopped")));
  apply_async_task(&mut s, &task(TaskEventKind::State, Some("completed")));
  expect_match(task_row(&s), json!({ "status": "completed", "asyncTask": { "state": "completed" } }));
}

#[test]
fn a_completed_task_never_moves_and_a_stopped_one_is_never_revived() {
  let mut s = shell_task();
  apply_async_task(&mut s, &task(TaskEventKind::State, Some("completed")));
  apply_async_task(&mut s, &task(TaskEventKind::State, Some("stopped")));
  assert_eq!(task_row(&s)["asyncTask"]["state"], "completed");
  let mut s2 = shell_task();
  apply_async_task(&mut s2, &task(TaskEventKind::State, Some("stopped")));
  apply_async_task(&mut s2, &task(TaskEventKind::State, Some("running")));
  expect_match(task_row(&s2), json!({ "status": "cancelled", "asyncTask": { "state": "stopped" } }));
}
