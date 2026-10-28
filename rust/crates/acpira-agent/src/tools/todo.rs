//! `todo`: the model's to-do list, replaced whole on every call. The card is titled `todo` and its rawOutput carries
//! `todos`, which the host turns into the to-do bar (`todo_tools.rs`); the model gets the list back as plain lines.
//!
//! The list also rides on other calls: `read`, `write`, `edit` and `bash` take it as `todos` (`rider_property`), and
//! the turn shows it as a `todo` row of its own before the call runs (`turn/riders.rs`). A model that makes one call per
//! reply otherwise spends a whole request on moving the list on: 14% of the steps of the 2026-10-15 eval runs did
//! nothing else.

use serde_json::{Value, json};

use super::{Action, Output};
use crate::llm::ToolSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
  pub content: String,
  /// `pending` | `in_progress` | `completed`
  pub status: String,
}

/// One entry of the list, as the tool and the riders take it
fn item_schema() -> Value {
  json!({
    "type": "object",
    "properties": {
      "content": { "type": "string" },
      "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
    },
    "required": ["content", "status"],
  })
}

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::TODO.into(),
    // "Completed as soon as it is done" used to invite a reply that only moved the list on; the list now rides on the
    // step's own call instead
    description: "Keep a to-do list for multi-step work. Send the whole list every time; mark one item in_progress while working on it and \
                  completed once it is done. When the list changes together with an action, pass it as todos on that read, edit, write or \
                  bash call instead of calling this tool."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "todos": { "type": "array", "items": item_schema() },
      },
      "required": ["todos"],
    }),
  }
}

/// The `todos` property of the tools the list rides on
pub fn rider_property() -> Value {
  json!({
    "type": "array",
    "description": "The whole to-do list, only when this step moves it on (an item done, the next one started): it is updated before the call runs, so no separate todo call is needed",
    "items": item_schema(),
  })
}

/// The list another tool's call carries in `todos`. None when it carries none: an empty list or null is taken as no
/// update (models fill optional arrays with empty ones), so only the todo tool clears the list
pub fn rider(args: &Value) -> Option<Result<Vec<Todo>, String>> {
  let list = args.get("todos")?;
  if list.is_null() || list.as_str().is_some_and(|s| s.trim().is_empty()) {
    return None;
  }
  match parse(list) {
    Ok(todos) if todos.is_empty() => None,
    other => Some(other),
  }
}

pub fn prepare(args: &Value) -> Result<Action, String> {
  let list = args.get("todos").ok_or("Missing required array argument \"todos\"")?;
  Ok(Action::Todo { todos: parse(list)? })
}

/// The entries of a list; some models send the array as a JSON string
fn parse(list: &Value) -> Result<Vec<Todo>, String> {
  let parsed;
  let list = match list {
    Value::String(s) => {
      parsed = serde_json::from_str::<Value>(s).map_err(|_| "\"todos\" must be an array of { content, status }".to_owned())?;
      &parsed
    }
    other => other,
  };
  let list = list.as_array().ok_or("\"todos\" must be an array of { content, status }")?;
  let mut todos = vec![];
  for (i, item) in list.iter().enumerate() {
    let content = item.get("content").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty());
    let content = content.ok_or_else(|| format!("todos[{i}] needs a non-empty \"content\""))?;
    // Models write the status a few ways; the host only knows these three
    let status = match item.get("status").and_then(Value::as_str).unwrap_or("pending").to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
      "pending" | "todo" | "open" | "not_started" => "pending",
      "in_progress" | "active" | "doing" | "started" => "in_progress",
      "completed" | "done" | "complete" | "finished" => "completed",
      other => return Err(format!("todos[{i}] has an unknown status \"{other}\"; use pending, in_progress or completed")),
    };
    todos.push(Todo { content: content.to_owned(), status: status.to_owned() });
  }
  Ok(todos)
}

fn counts(todos: &[Todo]) -> String {
  let count = |s: &str| todos.iter().filter(|t| t.status == s).count();
  format!("{} pending, {} in progress, {} completed", count("pending"), count("in_progress"), count("completed"))
}

/// The list as the todo tool answers with it (compaction carries the last one in this form)
pub fn listing(todos: &[Todo]) -> String {
  let mut model = format!("To-do list updated ({}):\n", counts(todos));
  for t in todos {
    model.push_str(&format!("[{}] {}\n", t.status, t.content));
  }
  model
}

/// What a call that carried the list is told in front of its own result: the model sent the list itself, so the
/// counts are enough
pub fn note(todos: &[Todo]) -> String {
  format!("{NOTE_PREFIX}{}).", counts(todos))
}

/// How a rider's note starts; the chunk index skips such a line when it reads a result's first line
pub const NOTE_PREFIX: &str = "To-do list updated (";

/// The rawInput and rawOutput a `todo` row carries
pub fn raw(todos: &[Todo]) -> Value {
  json!({ "todos": todos.iter().map(|t| json!({ "content": t.content, "status": t.status })).collect::<Vec<_>>() })
}

pub fn run(todos: &[Todo]) -> Output {
  Output { model: listing(todos), is_error: false, content: vec![], raw_output: Some(raw(todos)) }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn statuses_are_normalised_and_the_output_feeds_the_bar() {
    let Ok(Action::Todo { todos }) = prepare(&json!({ "todos": [{ "content": "a", "status": "Done" }, { "content": "b", "status": "in-progress" }, { "content": "c" }] }))
    else {
      panic!()
    };
    assert_eq!(todos.iter().map(|t| t.status.as_str()).collect::<Vec<_>>(), ["completed", "in_progress", "pending"]);
    let out = run(&todos);
    assert_eq!(out.raw_output.unwrap()["todos"][1], json!({ "content": "b", "status": "in_progress" }));
    assert!(out.model.starts_with("To-do list updated (1 pending, 1 in progress, 1 completed)"));
    assert!(prepare(&json!({ "todos": [{ "content": "x", "status": "blocked" }] })).is_err());
  }

  #[test]
  fn a_rider_list_is_taken_only_when_it_holds_entries() {
    assert_eq!(rider(&json!({ "path": "a.rs" })), None);
    assert_eq!(rider(&json!({ "todos": [] })), None, "an empty list is no update");
    assert_eq!(rider(&json!({ "todos": null })), None);
    assert_eq!(rider(&json!({ "todos": "[]" })), None);
    assert_eq!(rider(&json!({ "todos": "" })), None);
    let Some(Ok(todos)) = rider(&json!({ "todos": "[{\"content\": \"a\", \"status\": \"done\"}]" })) else { panic!("a list sent as a string") };
    assert_eq!(todos, [Todo { content: "a".into(), status: "completed".into() }]);
    assert!(matches!(rider(&json!({ "todos": [{ "content": "a", "status": "later" }] })), Some(Err(e)) if e.contains("unknown status")));
    assert!(matches!(rider(&json!({ "todos": 3 })), Some(Err(_))));
    assert_eq!(note(&todos), "To-do list updated (0 pending, 0 in progress, 1 completed).");
    assert!(note(&todos).starts_with(NOTE_PREFIX));
  }
}
