//! Async task rows: the authoritative completed edge and stopped tasks

use super::*;

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
