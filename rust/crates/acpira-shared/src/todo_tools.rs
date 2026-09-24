//! Todo-tool result parsing (mirror of src/shared/todoTools.ts)

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::transcript::{PlanEntry, PlanPriority, PlanStatus, ToolCallBlock};

static RECEIPT: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^Todo list updated\.\r?\nCurrent todo list:\r?\n((?s).*?)(?:\r?\n\r?\n|$)").unwrap());
static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^  \[(pending|in_progress|completed|done)\] (.+)$").unwrap());

pub fn is_todo_tool(block: &ToolCallBlock) -> bool {
  block.verb_key.as_deref() == Some("verb.todo")
}

fn status(s: &str) -> Option<PlanStatus> {
  Some(match s {
    "pending" => PlanStatus::Pending,
    "in_progress" => PlanStatus::InProgress,
    "completed" | "done" => PlanStatus::Completed,
    _ => return None,
  })
}

/// Parse confirmed tool results only; input may be a partial merge request
pub fn todo_entries(value: &Value) -> Option<Vec<PlanEntry>> {
  let parsed;
  let value = if let Value::String(s) = value {
    if let Some(c) = RECEIPT.captures(s) {
      let mut entries = vec![];
      for line in c[1].split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).filter(|l| !l.is_empty()) {
        let item = ITEM.captures(line)?;
        entries.push(PlanEntry { title: item[2].to_owned(), status: status(&item[1])?, priority: None });
      }
      return Some(entries);
    }
    parsed = serde_json::from_str::<Value>(s).ok()?;
    &parsed
  } else {
    value
  };
  let object = value.as_object()?;
  let result = if object.get("type").and_then(Value::as_str) == Some("Todo") { object.get("TodosUpdated")?.as_object()? } else { object };
  let todos = result.get("todos")?.as_array()?;
  let mut entries = vec![];
  for item in todos {
    let item = item.as_object()?;
    let content = item.get("content")?.as_str()?;
    if content.trim().is_empty() {
      return None;
    }
    let st = match item.get("status")?.as_str()? {
      "pending" => PlanStatus::Pending,
      "in_progress" => PlanStatus::InProgress,
      "completed" => PlanStatus::Completed,
      _ => return None,
    };
    let priority = match item.get("priority") {
      None => None,
      Some(Value::String(p)) if p == "low" => Some(PlanPriority::Low),
      Some(Value::String(p)) if p == "medium" => Some(PlanPriority::Medium),
      Some(Value::String(p)) if p == "high" => Some(PlanPriority::High),
      Some(_) => return None,
    };
    entries.push(PlanEntry { title: content.to_owned(), status: st, priority });
  }
  Some(entries)
}
