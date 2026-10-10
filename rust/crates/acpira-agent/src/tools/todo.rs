//! `todo`: the model's to-do list, replaced whole on every call. The card is titled `todo` and its rawOutput carries
//! `todos`, which the host turns into the to-do bar (`todo_tools.rs`); the model gets the list back as plain lines

use serde_json::{Value, json};

use super::{Action, Output};
use crate::llm::ToolSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Todo {
  pub content: String,
  /// `pending` | `in_progress` | `completed`
  pub status: String,
}

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::TODO.into(),
    description: "Keep a to-do list for multi-step work. Send the whole list every time; mark one item in_progress while working on it and \
                  completed as soon as it is done."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "todos": {
          "type": "array",
          "items": {
            "type": "object",
            "properties": {
              "content": { "type": "string" },
              "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] },
            },
            "required": ["content", "status"],
          },
        },
      },
      "required": ["todos"],
    }),
  }
}

pub fn prepare(args: &Value) -> Result<Action, String> {
  let list = args.get("todos").and_then(Value::as_array).ok_or("Missing required array argument \"todos\"")?;
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
  Ok(Action::Todo { todos })
}

pub fn run(todos: &[Todo]) -> Output {
  let count = |s: &str| todos.iter().filter(|t| t.status == s).count();
  let mut model = format!("To-do list updated ({} pending, {} in progress, {} completed):\n", count("pending"), count("in_progress"), count("completed"));
  for t in todos {
    model.push_str(&format!("[{}] {}\n", t.status, t.content));
  }
  let raw = json!({ "todos": todos.iter().map(|t| json!({ "content": t.content, "status": t.status })).collect::<Vec<_>>() });
  Output { model, is_error: false, content: vec![], raw_output: Some(raw) }
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
}
