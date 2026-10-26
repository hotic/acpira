//! Todo-tool result parsing (mirror of packages/shared/todoTools.ts)

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use crate::transcript::{PlanEntry, PlanPriority, PlanStatus, ToolCallBlock};

static RECEIPT: LazyLock<Regex> =
  LazyLock::new(|| crate::re::compile(r"^Todo list updated\.\r?\nCurrent todo list:\r?\n((?s).*?)(?:\r?\n\r?\n|$)"));
static ITEM: LazyLock<Regex> = LazyLock::new(|| crate::re::compile(r"^  \[(pending|in_progress|completed|done)\] (.+)$"));

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
  let todos = todo_list(value)?;
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
      // OpenCode's todowrite has a fourth status; a cancelled item is closed, so it counts as done rather than hiding the list
      "completed" | "cancelled" => PlanStatus::Completed,
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

/// The list inside a result: Grok's `{ type: "Todo", TodosUpdated: { todos } }`, a plain `{ todos }`, OpenCode's
/// rawOutput `{ output, metadata: { todos } }` or its output text, which is the bare array
fn todo_list(value: &Value) -> Option<&Vec<Value>> {
  if let Some(list) = value.as_array() {
    return Some(list);
  }
  let object = value.as_object()?;
  let result = if object.get("type").and_then(Value::as_str) == Some("Todo") { object.get("TodosUpdated")?.as_object()? } else { object };
  result.get("todos").or_else(|| result.get("metadata").and_then(|m| m.get("todos")))?.as_array()
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::*;

  #[test]
  fn opencode_todowrite_output_parses_with_cancelled_items_closed() {
    let list = json!([{ "content": "Inspect", "status": "completed", "priority": "low" }, { "content": "Drop", "status": "cancelled" }]);
    // rawOutput as opencode 1.18.15 sends it, then the output text the row keeps
    let raw = json!({ "output": serde_json::to_string_pretty(&list).unwrap(), "metadata": { "todos": list } });
    let entries = todo_entries(&raw).expect("metadata.todos");
    assert_eq!(entries.iter().map(|e| e.status).collect::<Vec<_>>(), [PlanStatus::Completed, PlanStatus::Completed]);
    assert_eq!(entries[0].priority, Some(PlanPriority::Low));
    assert_eq!(todo_entries(&Value::String(serde_json::to_string_pretty(&list).unwrap())).map(|e| e.len()), Some(2));
    assert!(todo_entries(&json!({ "todos": [{ "content": "x", "status": "blocked" }] })).is_none());
  }
}
