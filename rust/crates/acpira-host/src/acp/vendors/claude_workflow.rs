//! Claude Code dynamic workflows (the `Workflow` tool, which ultracode runs on every substantive task).
//!
//! claude-agent-acp 0.83.0 / 0.85.1 report a workflow as one AIR async task (`taskType: "workflow"`) and drop the per-agent
//! progress the CLI draws its own agent rows from: the SDK's `system` / `task_progress` frame carries `workflow_progress`
//! (`workflow_phase` and `workflow_agent` entries). The adapter does forward raw SDK frames as the `_claude/sdkMessage` ext
//! notification when the session asks through `_meta.claudeCode.emitRawSDKMessages`, so the host subscribes to
//! `task_progress` only and turns each frame into a synthetic `workflow_progress` session update (`wire.rs`), which
//! `SubagentTree::workflow_progress` keeps as one receipt node per agent.
//!
//! Observed 2026-10-02 with claude-agent-acp 0.83.0 / Claude Code 2.1.284 (`probe-subagents.ts claude --claude-raw`):
//! `workflow_agent { index, label, phaseIndex, phaseTitle, agentId, model, state: start | progress | done | error,
//! queuedAt, startedAt, attempt, promptPreview, tokens, toolCalls, durationMs, resultPreview }`, every frame the full list.
//!
//! Dynamic workflows are off for SDK hosts unless asked: the session then has no `Workflow` tool at all and the
//! `ultracode` keyword is ignored. `CLAUDE_CODE_WORKFLOWS=1` only moves the default the terminal CLI already has; an
//! explicit `enableWorkflows: false` (or managed `disableWorkflows`) still wins

use serde_json::{Value, json};

/// The ext notification claude-agent-acp relays raw SDK frames on
pub const SDK_MESSAGE: &str = "_claude/sdkMessage";

/// The variable that turns dynamic workflows on by default for the SDK host
pub const WORKFLOWS_ENV: &str = "CLAUDE_CODE_WORKFLOWS";

/// The synthetic session update kind (`wire.rs` decodes it)
pub const WORKFLOW_KIND: &str = "workflow_progress";

/// Ask for the raw `task_progress` frames only: everything else the adapter already maps
pub fn with_raw_progress(mut req: Value) -> Value {
  req["_meta"]["claudeCode"]["emitRawSDKMessages"] = json!([{ "type": "system", "subtype": "task_progress" }]);
  req
}

/// A `_claude/sdkMessage` notification → the `session/update` params of a `workflow_progress` update, or None for any
/// other frame (a plain subagent's `task_progress` has no `workflow_progress`)
pub fn workflow_update(params: &Value) -> Option<Value> {
  let session_id = params.get("sessionId").and_then(Value::as_str)?;
  let msg = params.get("message")?;
  if msg.get("type").and_then(Value::as_str) != Some("system") || msg.get("subtype").and_then(Value::as_str) != Some("task_progress") {
    return None;
  }
  let task_id = msg.get("task_id").and_then(Value::as_str).filter(|s| !s.is_empty())?;
  let agents: Vec<Value> = msg
    .get("workflow_progress")?
    .as_array()?
    .iter()
    .filter(|e| e.get("type").and_then(Value::as_str) == Some("workflow_agent"))
    .cloned()
    .collect();
  if agents.is_empty() {
    return None;
  }
  let mut update = json!({ "sessionUpdate": WORKFLOW_KIND, "asyncTaskId": task_id, "agents": agents });
  if let Some(tool) = msg.get("tool_use_id").and_then(Value::as_str) {
    update["toolCallId"] = json!(tool);
  }
  Some(json!({ "sessionId": session_id, "update": update }))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn frame(progress: Value) -> Value {
    json!({ "sessionId": "s1", "message": {
      "type": "system", "subtype": "task_progress", "task_id": "w8loyb2vg", "tool_use_id": "toolu_1",
      "workflow_progress": progress,
    } })
  }

  #[test]
  fn raw_progress_joins_the_thinking_meta() {
    let req = with_raw_progress(json!({ "cwd": "/w", "_meta": { "claudeCode": { "options": { "thinking": { "type": "adaptive" } } } } }));
    assert_eq!(req["_meta"]["claudeCode"]["options"]["thinking"]["type"], "adaptive");
    assert_eq!(req["_meta"]["claudeCode"]["emitRawSDKMessages"][0]["subtype"], "task_progress");
    let bare = with_raw_progress(json!({ "cwd": "/w" }));
    assert_eq!(bare["_meta"]["claudeCode"]["emitRawSDKMessages"][0]["type"], "system");
  }

  #[test]
  fn keeps_the_agent_entries_of_a_workflow_frame() {
    let n = workflow_update(&frame(json!([
      { "type": "workflow_phase", "index": 1, "title": "Reply" },
      { "type": "workflow_agent", "index": 1, "label": "alpha", "state": "done", "resultPreview": "alpha" },
      { "type": "workflow_log", "text": "x" },
    ])))
    .expect("a workflow frame");
    assert_eq!(n["sessionId"], "s1");
    assert_eq!(n["update"]["sessionUpdate"], WORKFLOW_KIND);
    assert_eq!(n["update"]["asyncTaskId"], "w8loyb2vg");
    assert_eq!(n["update"]["toolCallId"], "toolu_1");
    assert_eq!(n["update"]["agents"].as_array().map(Vec::len), Some(1));
  }

  #[test]
  fn ignores_frames_without_workflow_agents() {
    assert!(workflow_update(&frame(json!([]))).is_none());
    let mut plain = frame(json!([]));
    plain["message"].as_object_mut().unwrap().remove("workflow_progress");
    assert!(workflow_update(&plain).is_none());
    let mut other = frame(json!([{ "type": "workflow_agent", "index": 1 }]));
    other["message"]["subtype"] = json!("task_started");
    assert!(workflow_update(&other).is_none());
  }
}
