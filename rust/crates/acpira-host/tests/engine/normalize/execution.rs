//! Commands and turn timing: terminal output, observed execution time, background shells, ended, failed and replayed turns

use super::*;

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
fn a_call_that_skips_in_progress_runs_from_its_last_pending_input() {
  // claude-agent-acp: pending, input streamed while pending, a progress beat while it runs, then completed
  let mut s = state();
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1000 })));
  let clock = Clock::at(2000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "sh", "title": "Terminal", "kind": "execute", "status": "pending", "rawInput": {} }));
  clock.set(3000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "rawInput": { "command": "pnpm test" } }));
  clock.set(9000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "_meta": { "claudeCode": { "elapsedTimeSeconds": 6 } } }));
  // No running clock on a pending row
  expect_absent(block(&s, 0, 0), "startedAt");
  clock.set(15000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "status": "completed" }));
  expect_match(&s.turns[0], json!({ "blocks": [{ "startedAt": 3000, "endedAt": 15000 }] }));
}

#[test]
fn counts_the_wait_on_background_work_past_the_grace_period() {
  let mut s = state();
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1000 })));
  let clock = Clock::at(2000);
  let backgrounded = json!({ "jetbrains": { "air": { "asyncTasks": { "backgrounded": true } } } });
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "mon", "title": "Monitor", "status": "in_progress", "_meta": backgrounded }));
  // The model goes quiet for 100 s, then reports back
  clock.set(102_000);
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "done" } }));
  // A silence inside the grace period is the model working
  clock.set(112_000);
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "." } }));
  // A foreground call open beside it is tool time, not waiting
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "sh", "kind": "execute", "status": "in_progress" }));
  clock.set(200_000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "sh", "status": "completed" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "mon", "status": "completed" }));
  clock.set(260_000);
  end_turn(&mut s, TurnStop::EndTurn);
  expect_match(&s.turns[0], json!({ "idleMs": 80_000 }));
}

#[test]
fn counts_subagent_waits_compaction_pauses_and_detached_continuations() {
  let mut s = state();
  let clock = Clock::at(1000);
  s.turns.push(turn(json!({ "role": "agent", "blocks": [], "startedAt": 1000 })));
  // Pre-existing session background before the turn's first block belongs to TTFT, never idleMs
  sync_idle(&mut s, true);
  clock.set(30_000);
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "dispatching" } }));
  expect_absent(&s.turns[0], "idleMs");
  // Native subagent runs for 120 s; child updates call sync_idle without resetting the 20 s grace window
  clock.set(70_000);
  sync_idle(&mut s, true);
  clock.set(150_000);
  // Mid-turn compaction interrupts the background wait (120 s - 20 s = 100 s) and counts its own 12 s in full
  apply(&mut s, json!({ "sessionUpdate": "compaction_update", "compactionId": "c1", "status": "in_progress" }));
  clock.set(162_000);
  apply(&mut s, json!({ "sessionUpdate": "compaction_update", "compactionId": "c1", "status": "completed" }));
  // Subagent finishes 50 s later (50 s - 20 s = 30 s), then the root writes its closing reply over 3 s
  clock.set(212_000);
  sync_idle(&mut s, false);
  clock.set(215_000);
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "all merged" } }));
  end_turn(&mut s, TurnStop::EndTurn);
  // Active decode so far: (215_000 - 30_000) - (100_000 + 12_000 + 30_000) = 43_000 -> 142_000 idleMs
  expect_match(&s.turns[0], json!({ "respondedAt": 30_000, "endedAt": 215_000, "idleMs": 142_000 }));

  // 60 s later an autonomous cycle reopens the turn, runs a 5 s foreground tool and 4 s of uncounted prose
  clock.set(275_000);
  reopen_idle(&mut s);
  if let Some(Turn::Agent(t)) = s.turns.last_mut() {
    t.stop = None;
    t.ended_at = None;
  }
  clock.set(276_000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "chk", "kind": "execute", "status": "in_progress" }));
  clock.set(281_000);
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "chk", "status": "completed" }));
  clock.set(284_000);
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "followup" } }));
  end_turn(&mut s, TurnStop::EndTurn);
  // Detached span is 284_000 - 215_000 = 69_000, minus 5_000 foreground tool = 64_000 added -> 206_000 idleMs,
  // preserving the exact 43_000 ms active decode window: (284_000 - 30_000) - 5_000 - 206_000 = 43_000
  expect_match(&s.turns[0], json!({ "endedAt": 284_000, "idleMs": 206_000 }));
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

// The built-in agent: `bash` with background answers `rawOutput.job`; each `job` row opens under the bare tool name,
// then takes its "Check job 2 (wait 1s)" title with the arguments
#[test]
fn builtin_job_checks_name_the_background_command() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "bash", "kind": "execute", "status": "pending" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "title": "cargo test", "kind": "execute",
    "rawInput": { "command": "cargo test", "background": true } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed", "rawOutput": { "job": "2" } }));
  expect_absent(block(&s, 0, 0), "content");
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-2", "title": "job", "kind": "execute", "status": "pending" }));
  expect_match(block(&s, 0, 1), json!({ "verbKey": "verb.wait" }));
  expect_absent(block(&s, 0, 1), "target");
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-2", "title": "Check job 2 (wait 1s)", "kind": "execute",
    "rawInput": { "id": 2, "wait": 1 } }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-2", "status": "completed", "rawOutput": { "job": "2" } }));
  expect_match(block(&s, 0, 1), json!({ "verbKey": "verb.wait", "target": "cargo test", "targetMono": true, "status": "completed" }));
  expect_absent(block(&s, 0, 1), "content");
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-3", "title": "job", "kind": "execute", "status": "pending" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-3", "title": "Stop job 2", "rawInput": { "id": "2", "kill": true } }));
  expect_match(block(&s, 0, 2), json!({ "verbKey": "verb.kill", "target": "cargo test" }));
  // Typing into an interactive job keeps its title, the typed text being the point
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "call-4", "title": "job", "kind": "execute", "status": "pending" }));
  apply(&mut s, json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-4", "title": "Type into job 2: y\\n", "rawInput": { "id": "2", "input": "y\n" } }));
  expect_absent(block(&s, 0, 3), "verbKey");
  expect_match(block(&s, 0, 3), json!({ "verb": "Run", "target": "Type into job 2: y\\n" }));
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

#[test]
fn live_turn_clock_starts_at_first_streamed_block() {
  let mut s = NormalizeState::new(vec![turn(json!({ "role": "agent", "startedAt": 1, "blocks": [] }))]);
  // An update that adds no block (usage) keeps the clock waiting
  apply(&mut s, json!({ "sessionUpdate": "usage_update", "used": 10, "size": 100 }));
  assert_eq!(s.turns[0].as_agent().unwrap().responded_at, None);
  apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "hmm" } }));
  let first = s.turns[0].as_agent().unwrap().responded_at.expect("stamped by the first block");
  assert!(first > 1);
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "hi" } }));
  assert_eq!(s.turns[0].as_agent().unwrap().responded_at, Some(first));
}

#[test]
fn replayed_turn_is_never_stamped() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "hi" } }));
  assert_eq!(s.turns[0].as_agent().unwrap().responded_at, None);
}
