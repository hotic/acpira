//! A session's events for the plugins' resident processes (docs/dev/plugins.md, Events). Every event carries the
//! session, its agent and its folder; nothing is built unless a process subscribed to it, and nothing waits on one.
//! Replayed history is never sent: only what happens live in this engine

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::transcript::{AgentBlock, ToolCallBlock, ToolKind, ToolStatus, Turn, TurnError, TurnStop};

use super::{AcpSession, Core};
use crate::acp::transcript::normalize::find_tool;

/// The last phase sent for each tool row, so a row that reports `completed` twice is told once
#[derive(Debug, Default)]
pub(crate) struct EventState {
  tools: HashMap<String, &'static str>,
}

fn phase_of(status: ToolStatus) -> &'static str {
  match status {
    ToolStatus::Pending | ToolStatus::InProgress => "started",
    ToolStatus::Completed => "completed",
    ToolStatus::Failed | ToolStatus::Cancelled => "failed",
  }
}

/// A tool row as an event shows it: what it is and where it points, not its output
fn tool_view(tc: &ToolCallBlock) -> Value {
  json!({
    "toolCallId": tc.id,
    "kind": tc.kind,
    "verb": tc.verb,
    "target": tc.target,
    "mcpServer": tc.mcp_server,
    "status": tc.status,
    "locations": tc.locations,
  })
}

impl AcpSession {
  /// Send `event` with the session's identity added to `data`
  pub(crate) fn emit(&self, event: &str, data: impl FnOnce() -> Value) {
    let Some(rt) = &self.deps.plugin_runtime else { return };
    rt.emit(event, || {
      let mut d = data();
      if let Some(o) = d.as_object_mut() {
        o.insert("session".into(), json!(self.id));
        o.insert("agent".into(), json!(self.agent));
        o.insert("cwd".into(), json!(self.cwd));
      }
      d
    });
  }

  /// After a live update was applied to the transcript: tool rows, streamed text, usage and mode changes
  pub(crate) fn emit_update(&self, c: &mut Core, kind: &str, u: &Value) {
    let Some(rt) = self.deps.plugin_runtime.clone() else { return };
    match kind {
      "agent_message_chunk" | "agent_thought_chunk" => {
        let event = if kind == "agent_message_chunk" { "text.delta" } else { "thought.delta" };
        if rt.hears(event)
          && let Some(text) = u.get("content").and_then(|x| x.get("text")).and_then(Value::as_str)
        {
          let text = text.to_owned();
          self.emit(event, || json!({ "text": text }));
        }
      }
      "tool_call" | "tool_call_update" => {
        let Some(id) = u.get("toolCallId").and_then(Value::as_str) else { return };
        let Some((ti, bi)) = find_tool(&c.state.turns, id) else { return };
        let Some(Turn::Agent(t)) = c.state.turns.get(ti) else { return };
        let Some(AgentBlock::ToolCall(tc)) = t.blocks.get(bi) else { return };
        let phase = phase_of(tc.status);
        let prev = c.plugin_events.tools.insert(id.to_owned(), phase);
        if prev == Some(phase) {
          return;
        }
        let row = tool_view(tc);
        // A row first seen already settled still starts, so every completion has its start
        if prev.is_none() && phase != "started" {
          let start = row.clone();
          self.emit("tool.started", || start);
        }
        let files = (phase == "completed" && matches!(tc.kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move))
          .then(|| tc.locations.iter().flatten().map(|l| l.path.clone()).collect::<Vec<_>>())
          .filter(|p| !p.is_empty());
        self.emit(&format!("tool.{phase}"), || row);
        if let Some(paths) = files {
          let id = id.to_owned();
          self.emit("files.changed", || json!({ "paths": paths, "toolCallId": id }));
        }
      }
      "usage_update" => {
        let mut usage = u.clone();
        if let Some(o) = usage.as_object_mut() {
          o.remove("sessionUpdate");
        }
        self.emit("usage.updated", || json!({ "usage": usage }));
      }
      "current_mode_update" => {
        let mode = u.get("currentModeId").cloned().unwrap_or(Value::Null);
        self.emit("model.mode", || json!({ "mode": mode }));
      }
      "config_option_update" => {
        let options = u.get("configOptions").cloned().unwrap_or(Value::Null);
        self.emit("model.config", || json!({ "configOptions": options }));
      }
      _ => {}
    }
  }

  /// A turn went out to the agent
  pub(crate) fn emit_turn_started(&self, origin: super::prompt::Origin, text: &str) {
    let text = text.to_owned();
    self.emit("turn.started", || json!({ "origin": format!("{origin:?}").to_lowercase(), "text": text }));
  }

  /// A turn settled: how it ended and the session's usage at that point
  pub(crate) fn emit_turn_ended(&self, c: &Core, stop: TurnStop, error: Option<&TurnError>) {
    if self.deps.plugin_runtime.is_none() {
      return;
    }
    let usage = c.state.usage.as_ref().map(|u| serde_json::to_value(u).unwrap_or(Value::Null));
    let error = error.map(|e| json!({ "message": e.message, "code": e.code, "kind": e.kind }));
    self.emit("turn.ended", || json!({ "stop": stop, "error": error, "usage": usage }));
  }

  /// A permission request reached the session (before any gate, yolo or card decided it)
  pub(crate) fn emit_permission_asked(&self, req: &Value) {
    if self.deps.plugin_runtime.is_none() {
      return;
    }
    let tool = req.get("toolCall").cloned().unwrap_or(Value::Null);
    let options = req.get("options").cloned().unwrap_or(Value::Null);
    self.emit("permission.asked", || json!({ "toolCall": tool, "options": options }));
  }

  /// How a permission request was answered: the option picked, or cancelled
  pub(crate) fn emit_permission_replied(self: &Arc<Self>, req: &Value, answer: &Value) {
    if self.deps.plugin_runtime.is_none() {
      return;
    }
    let id = req.get("toolCall").and_then(|t| t.get("toolCallId")).cloned().unwrap_or(Value::Null);
    let outcome = answer.get("outcome").cloned().unwrap_or(Value::Null);
    self.emit("permission.replied", || json!({ "toolCallId": id, "outcome": outcome }));
  }
}
