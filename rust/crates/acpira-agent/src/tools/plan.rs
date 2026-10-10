//! `exit_plan`: Plan mode's way out. The turn runs it itself (`turn::exit_plan`): the plan file goes to the user on the
//! host's plan approval card, and an approval switches the session to Agent mode for the rest of the turn

use serde_json::{Value, json};

use super::Action;
use crate::llm::ToolSpec;

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::EXIT_PLAN.into(),
    description: "Ask the user to approve the plan and leave Plan mode. Call it when the plan file is complete. `plan` optionally \
                  replaces the plan file's content first, so writing and submitting can be one call."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "plan": { "type": "string", "description": "The whole plan in Markdown (optional when the plan file is already written)" },
      },
    }),
  }
}

pub fn prepare(args: &Value) -> Result<Action, String> {
  let plan = match args.get("plan") {
    None | Some(Value::Null) => None,
    Some(Value::String(s)) => Some(s.clone()).filter(|s| !s.trim().is_empty()),
    Some(_) => return Err("\"plan\" must be a string of Markdown".into()),
  };
  Ok(Action::ExitPlan { plan })
}
