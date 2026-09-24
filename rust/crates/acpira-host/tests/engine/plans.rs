//! test/plans.test.ts, test/planSnapshots.test.ts and test/restoreTurns.test.ts

use serde_json::{Value, json};

use acpira_host::acp::grok::parse_exit_plan;
use acpira_host::acp::normalize::{NormalizeState, apply_update, end_turn};
use acpira_host::acp::plan_snapshots::restore_plan_snapshots;
use acpira_host::acp::plans::{capture_plan, plan_documents, plan_documents_mut};
use acpira_host::acp::restore_turns::restore_interrupted_turns;
use acpira_host::util::iso_of_ms;
use acpira_shared::transcript::{Turn, TurnStop};

use crate::support::{expect_absent, expect_eq, expect_match, v};

fn turns(j: Value) -> Vec<Turn> {
  serde_json::from_value(j).unwrap()
}

fn agent_turn() -> Vec<Turn> {
  turns(json!([{ "role": "agent", "blocks": [] }]))
}

fn plan(t: &[Turn]) -> Value {
  v(plan_documents(t)[0])
}

#[test]
fn devin_a_late_plan_write_fills_the_inline_exit_preview_and_keeps_its_status() {
  for status in ["ready", "approved", "rejected"] {
    let mut t = agent_turn();
    let id = capture_plan(&mut t, &json!({ "toolCallId": "exit", "title": "Exit plan mode",
      "_meta": { "cognition.ai/isExitPlan": true }, "rawInput": { "plan": "Create hello.txt." } })).unwrap();
    assert_eq!(plan(&t)["markdown"], "Create hello.txt.");
    assert_eq!(capture_plan(&mut t, &json!({ "toolCallId": "exit" })).as_deref(), Some(id.as_str()));
    plan_documents_mut(&mut t)[0].status = serde_json::from_value::<acpira_shared::transcript::PlanDocStatus>(json!(status)).unwrap();
    let update = json!({ "toolCallId": "write", "_meta": { "cognition.ai/isPlanFileEdit": true },
      "rawInput": { "file_path": "/plans/demo.md", "content": "# Demo\n\nCreate hello.txt." } });
    assert_eq!(capture_plan(&mut t, &update).as_deref(), Some(id.as_str()));
    let mut completed = update.clone();
    completed["status"] = json!("completed");
    capture_plan(&mut t, &completed);
    expect_match(plan(&t), json!({ "status": status, "toolCallId": "write", "approvalToolCallId": "exit", "path": "/plans/demo.md", "markdown": "# Demo\n\nCreate hello.txt." }));
    assert_eq!(plan_documents(&t).len(), 1);
    // A later exit summary must not replace the complete saved document
    capture_plan(&mut t, &json!({ "toolCallId": "exit", "rawInput": { "plan": "Summary only." } }));
    assert_eq!(plan(&t)["markdown"], "# Demo\n\nCreate hello.txt.");
  }
}

#[test]
fn devin_keeps_plan_content_and_path_strips_frontmatter_and_attaches_title_only_permissions() {
  let mut t = agent_turn();
  let id = capture_plan(&mut t, &json!({ "toolCallId": "write", "title": "Updated plan: Demo", "_meta": { "cognition.ai/isPlanFileEdit": true },
    "content": [{ "type": "diff", "path": "/plans/demo.md", "newText": "---\nagent: devin\n---\n# Demo\n\nFull plan." }] })).unwrap();
  capture_plan(&mut t, &json!({ "toolCallId": "write", "status": "completed" }));
  capture_plan(&mut t, &json!({ "toolCallId": "exit", "title": "Exit plan mode", "_meta": { "cognition.ai/isExitPlan": true, "cognition.ai/planFilePath": "/plans/demo.md" } }));
  assert_eq!(capture_plan(&mut t, &json!({ "toolCallId": "exit" })).as_deref(), Some(id.as_str()));
  expect_match(plan(&t), json!({ "title": "Demo", "markdown": "# Demo\n\nFull plan.", "path": "/plans/demo.md", "status": "ready", "approvalToolCallId": "exit" }));
  assert_eq!(plan_documents(&t).len(), 1);
}

#[test]
fn kimi_uses_the_full_permission_body_rather_than_the_generic_label() {
  let mut t = agent_turn();
  capture_plan(&mut t, &json!({ "toolCallId": "exit", "title": "ExitPlanMode", "content": [{ "type": "content", "content": { "type": "text", "text": "Plan saved to: /plans/kimi.md\n\n# Kimi plan\n\nSteps." } }] }));
  expect_match(plan(&t), json!({ "title": "Kimi plan", "markdown": "# Kimi plan\n\nSteps.", "path": "/plans/kimi.md", "status": "ready" }));
}

#[test]
fn grok_captures_content_without_a_write_and_takes_the_later_plan_path() {
  let mut t = agent_turn();
  capture_plan(&mut t, &json!({ "toolCallId": "exit", "title": "exit_plan_mode", "rawInput": { "planContent": "# Grok plan\n\nSteps." } })).unwrap();
  capture_plan(&mut t, &json!({ "toolCallId": "exit", "status": "completed", "rawOutput": { "PlanReady": { "plan_file_path": "/plans/plan.md", "plan_content": "# Grok plan\n\nSteps." } } }));
  expect_match(plan(&t), json!({ "markdown": "# Grok plan\n\nSteps.", "path": "/plans/plan.md", "status": "ready" }));
}

#[test]
fn a_codex_switch_mode_carrying_a_plan_is_the_plan_review_approval() {
  let mut t = agent_turn();
  capture_plan(&mut t, &json!({ "toolCallId": "plan-review:1", "title": "Implement this plan?", "kind": "switch_mode", "rawInput": { "plan": "# Codex plan\n\nSteps." } })).unwrap();
  expect_match(plan(&t), json!({ "markdown": "# Codex plan\n\nSteps.", "status": "ready", "approvalToolCallId": "plan-review:1" }));
  // A switch_mode without a plan body stays an ordinary tool call
  assert!(capture_plan(&mut t, &json!({ "toolCallId": "mode", "kind": "switch_mode", "rawInput": { "mode": "default" } })).is_none());
}

#[test]
fn arbitrary_markdown_edits_are_not_implementation_plans() {
  let mut t = agent_turn();
  assert!(capture_plan(&mut t, &json!({ "toolCallId": "write", "title": "Write", "rawInput": { "path": "/repo/plan.md", "content": "# Notes" } })).is_none());
  assert!(plan_documents(&t).is_empty());
}

#[test]
fn malformed_private_plan_requests_are_rejected() {
  assert!(parse_exit_plan(&json!({ "sessionId": "s", "planContent": "text" })).is_err());
  assert!(parse_exit_plan(&json!({ "sessionId": "s", "toolCallId": "t", "planContent": [] })).is_err());
  let ok = parse_exit_plan(&json!({ "sessionId": "s", "toolCallId": "t", "planContent": null })).unwrap();
  expect_eq(ok, json!({ "sessionId": "s", "toolCallId": "t" }));
}

fn snapshot(status: &str) -> Value {
  json!({ "sessionUpdate": "plan", "entries": [{ "content": "Implement feature", "priority": "medium", "status": status }] })
}

fn user(text: &str) -> Turn {
  serde_json::from_value(json!({ "role": "user", "text": text })).unwrap()
}

fn plan_blocks(t: &[Turn]) -> usize {
  v(t).as_array().unwrap().iter().filter(|x| x["role"] == "agent").map(|x| x["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "plan").count()).sum()
}

#[test]
fn identical_snapshots_across_follow_ups_neither_interrupt_prose_nor_add_plan_rows() {
  let mut s = NormalizeState::new(vec![]);
  apply_update(&mut s, &snapshot("completed"));
  end_turn(&mut s, TurnStop::EndTurn);
  for _ in 0..3 {
    s.turns.push(user("A follow-up question"));
    // A snapshot arriving before the reply must not create an empty agent turn
    assert!(!apply_update(&mut s, &snapshot("completed")));
    assert_eq!(v(s.turns.last().unwrap())["role"], "user");
    apply_update(&mut s, &json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Reply" } }));
    assert!(!apply_update(&mut s, &snapshot("completed")));
    apply_update(&mut s, &json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": " continued." } }));
    expect_eq(&v(s.turns.last().unwrap())["blocks"], json!([{ "type": "text", "markdown": "Reply continued.", "streaming": true }]));
    end_turn(&mut s, TurnStop::EndTurn);
  }
  // Only the first turn carries a plan row, so nothing is left for the dock to show
  assert_eq!(plan_blocks(&s.turns), 1);
}

#[test]
fn real_progress_is_kept_even_when_the_turn_ends_at_the_previous_completed_state() {
  let mut s = NormalizeState::new(vec![]);
  apply_update(&mut s, &snapshot("completed"));
  end_turn(&mut s, TurnStop::EndTurn);
  s.turns.push(user("Run the same work again"));
  apply_update(&mut s, &snapshot("in_progress"));
  assert_eq!(v(s.turns.last().unwrap())["blocks"][0]["entries"][0]["status"], "in_progress");
  apply_update(&mut s, &snapshot("completed"));
  end_turn(&mut s, TurnStop::EndTurn);
  let restored = restore_plan_snapshots(serde_json::from_value(v(&s.turns)).unwrap());
  assert_eq!(plan_blocks(&restored), 2);
}

#[test]
fn changes_to_content_priority_order_and_clearing_are_kept() {
  let mut s = NormalizeState::new(vec![]);
  apply_update(&mut s, &snapshot("completed"));
  let done = json!({ "content": "Implement feature", "priority": "medium", "status": "completed" });
  let with = |k: &str, x: &str| {
    let mut e = done.clone();
    e[k] = json!(x);
    e
  };
  for entries in [json!([with("content", "Another feature")]), json!([with("priority", "high")]), json!([done, with("content", "Second")]), json!([with("content", "Second"), done]), json!([])] {
    assert!(apply_update(&mut s, &json!({ "sessionUpdate": "plan", "entries": entries })));
  }
  assert!(!apply_update(&mut s, &json!({ "sessionUpdate": "plan", "entries": [] })));
}

#[test]
fn legacy_completed_echoes_are_removed_without_touching_the_record_or_turn_indices() {
  let done = json!({ "type": "plan", "entries": [{ "title": "Implement feature", "priority": "medium", "status": "completed" }] });
  let original = json!([
    { "role": "agent", "blocks": [done], "stop": "end_turn" },
    { "role": "user", "text": "Follow-up" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "Answer." }, done], "stop": "end_turn" },
  ]);
  let restored = v(restore_plan_snapshots(turns(original.clone())));
  assert_eq!(restored.as_array().unwrap().len(), 3);
  assert_eq!(restored[0], original[0]);
  assert_eq!(restored[1], original[1]);
  expect_eq(&restored[2]["blocks"], json!([{ "type": "text", "markdown": "Answer." }]));
}

#[test]
fn legacy_todo_edits_and_updates_after_an_empty_snapshot_are_kept() {
  let done = json!({ "type": "plan", "entries": [{ "title": "Implement feature", "priority": "medium", "status": "completed" }] });
  let t = json!([
    { "role": "agent", "blocks": [done] },
    { "role": "agent", "blocks": [{ "type": "tool_call", "id": "todo", "kind": "other", "verb": "Todo", "verbKey": "verb.todo", "status": "completed" }, done] },
    { "role": "agent", "blocks": [{ "type": "plan", "entries": [] }] },
    { "role": "agent", "blocks": [done] },
  ]);
  assert_eq!(v(restore_plan_snapshots(turns(t.clone()))), v(turns(t)));
}

#[test]
fn orphaned_approvals_and_questions_are_withdrawn_while_the_task_plan_stays() {
  let t = turns(json!([{ "role": "agent", "blocks": [
    { "type": "permission", "id": "p", "title": "Run command", "options": [] },
    { "type": "question", "id": "q", "questions": [] },
    { "type": "plan", "entries": [{ "title": "Continue project", "status": "in_progress" }] },
  ] }]));
  let restored = restore_interrupted_turns(t.clone(), &iso_of_ms(5000));
  expect_match(&restored[0], json!({ "stop": "cancelled", "blocks": [
    { "type": "question", "outcome": "cancelled" },
    { "type": "plan", "entries": [{ "status": "in_progress" }] },
  ] }));
  expect_absent(&t[0], "stop");
}

#[test]
fn residual_background_commands_in_completed_turns_stop_without_changing_the_result() {
  let t = turns(json!([{ "role": "agent", "startedAt": 1000, "endedAt": 3000, "stop": "end_turn", "blocks": [
    { "type": "tool_call", "id": "server", "kind": "execute", "verb": "Run", "background": true, "status": "in_progress", "startedAt": 2000 },
  ] }]));
  expect_match(&restore_interrupted_turns(t, &iso_of_ms(9000))[0], json!({ "stop": "end_turn", "endedAt": 3000, "blocks": [{ "status": "cancelled", "endedAt": 3000 }] }));
}

#[test]
fn healthy_history_is_kept_and_the_latest_recorded_activity_dates_an_interruption() {
  let complete = json!({ "role": "agent", "blocks": [{ "type": "text", "markdown": "Done" }], "stop": "end_turn" });
  let active = json!({ "role": "agent", "startedAt": 2000, "blocks": [
    { "type": "tool_call", "id": "run", "kind": "execute", "verb": "Run", "status": "in_progress", "startedAt": 7000 },
  ] });
  let restored = v(restore_interrupted_turns(turns(json!([complete, active])), &iso_of_ms(1000)));
  assert_eq!(restored[0], v(turns(json!([complete])))[0]);
  expect_match(&restored[1], json!({ "stop": "cancelled", "endedAt": 7000, "blocks": [{ "status": "cancelled", "endedAt": 7000 }] }));
}
