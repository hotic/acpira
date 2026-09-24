//! Extension session updates (mirror of src/host/acp/subagents/wire.ts). The TS host parks these kinds inside
//! `session_info_update` so the SDK's closed union does not drop them; this host has no closed union, so the raw kind is
//! read directly. The parked form is still understood (recorded fixtures carry it)

use serde_json::Value;

use acpira_shared::subagents::SubagentState;
use acpira_shared::transcript::{AsyncTaskState, TaskUsage};

pub const EXT_META_KEY: &str = "acpira/extension";

const EXT_KINDS: [&str; 6] =
  ["subagent_update", "subagent_spawned", "subagent_state_update", "async_task_spawned", "async_task_progress", "async_task_state_update"];

pub fn is_extension_kind(kind: &str) -> bool {
  EXT_KINDS.contains(&kind)
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubagentLifecycle {
  pub peer_session_id: String,
  pub title: Option<String>,
  pub task: Option<String>,
  pub cancel: Option<bool>,
  pub state: Option<SubagentState>,
  pub meta: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskEventKind {
  Spawned,
  Progress,
  State,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AsyncTaskEvent {
  pub event: TaskEventKind,
  pub async_task_id: String,
  pub name: Option<String>,
  pub task_type: Option<String>,
  pub description: Option<String>,
  pub show_in_transcript: bool,
  pub can_stop: Option<bool>,
  pub output_file_path: Option<String>,
  pub tool_call_id: Option<String>,
  pub summary: Option<String>,
  pub last_tool_name: Option<String>,
  pub usage: Option<TaskUsage>,
  pub state: Option<AsyncTaskState>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExtensionUpdate {
  Lifecycle(SubagentLifecycle),
  AsyncTask(AsyncTaskEvent),
  Ignored(String),
}

fn s(v: &Value, k: &str) -> Option<String> {
  v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned)
}

/// The raw extension update of a `session/update` payload: either the kind itself or the TS-parked form
pub fn raw_extension(update: &Value) -> Option<&Value> {
  let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
  if is_extension_kind(kind) {
    return Some(update);
  }
  if kind == "session_info_update" {
    return update.get("_meta").and_then(|m| m.get(EXT_META_KEY)).filter(|r| r.is_object());
  }
  None
}

pub fn extension_of(update: &Value, log: &dyn Fn(&str)) -> Option<ExtensionUpdate> {
  let raw = raw_extension(update)?;
  let kind = raw.get("sessionUpdate").and_then(Value::as_str).unwrap_or("?");
  match kind {
    "async_task_spawned" | "async_task_progress" | "async_task_state_update" => Some(async_task_event(kind, raw, log)),
    "subagent_update" | "subagent_spawned" | "subagent_state_update" => {
      let Some(peer_session_id) = s(raw, "subagentSessionId") else {
        log(&format!("{kind} without subagentSessionId dropped"));
        return Some(ExtensionUpdate::Ignored(kind.into()));
      };
      let state = match raw.get("state") {
        None | Some(Value::Null) => None,
        Some(Value::String(v)) if matches!(v.as_str(), "running" | "completed" | "failed" | "cancelled" | "disconnected") => {
          serde_json::from_value(Value::String(v.clone())).ok()
        }
        Some(other) => {
          log(&format!("unknown subagent state {other} — treating as running"));
          Some(SubagentState::Running)
        }
      };
      let cancel = raw.get("capabilities").and_then(Value::as_object).map(|c| c.get("cancel") == Some(&Value::Bool(true)));
      Some(ExtensionUpdate::Lifecycle(SubagentLifecycle {
        peer_session_id,
        title: s(raw, "name"),
        task: s(raw, "task"),
        cancel,
        state,
        meta: raw.clone(),
      }))
    }
    other => Some(ExtensionUpdate::Ignored(other.into())),
  }
}

fn async_task_event(kind: &str, raw: &Value, log: &dyn Fn(&str)) -> ExtensionUpdate {
  let Some(async_task_id) = s(raw, "asyncTaskId") else {
    log(&format!("{kind} without asyncTaskId dropped"));
    return ExtensionUpdate::Ignored(kind.into());
  };
  let event = match kind {
    "async_task_spawned" => TaskEventKind::Spawned,
    "async_task_progress" => TaskEventKind::Progress,
    _ => TaskEventKind::State,
  };
  let usage = raw.get("usage").and_then(Value::as_object).map(|u| TaskUsage {
    total_tokens: u.get("totalTokens").and_then(Value::as_f64).map(Into::into),
    tool_uses: u.get("toolUses").and_then(Value::as_f64).map(Into::into),
    duration_ms: u.get("durationMs").and_then(Value::as_f64).map(Into::into),
  });
  let mut e = AsyncTaskEvent {
    event,
    async_task_id,
    name: s(raw, "name"),
    task_type: s(raw, "taskType"),
    description: s(raw, "description"),
    show_in_transcript: raw.get("showInTranscript") == Some(&Value::Bool(true)),
    can_stop: raw.get("canStop").and_then(Value::as_bool),
    output_file_path: s(raw, "outputFilePath"),
    tool_call_id: s(raw, "toolCallId"),
    summary: s(raw, "summary"),
    last_tool_name: s(raw, "lastToolName"),
    usage,
    state: None,
  };
  if event == TaskEventKind::State {
    let state = s(raw, "state").and_then(|v| serde_json::from_value::<AsyncTaskState>(Value::String(v)).ok());
    match state {
      Some(st) => e.state = Some(st),
      None => {
        log(&format!(
          "async_task_state_update with unknown state dropped: {}",
          raw.get("state").map(Value::to_string).unwrap_or_else(|| "undefined".into())
        ));
        return ExtensionUpdate::Ignored(kind.into());
      }
    }
  }
  ExtensionUpdate::AsyncTask(e)
}
