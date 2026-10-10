//! `exit_plan`: Plan mode's way out. The turn runs it itself (`turn::exit_plan`): the plan file goes to the user on the
//! host's plan approval card, and an approval switches the session to Agent mode for the rest of the turn

use serde_json::{Value, json};

use super::Action;
use crate::llm::ToolSpec;

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::EXIT_PLAN.into(),
    description: "Plan mode only: ask the user to approve the plan and leave Plan mode. Write the plan file first, then call \
                  this with no arguments."
      .into(),
    // No `plan` parameter: the plan goes through the plan file once instead of being repeated in the call. With an
    // optional `plan` string, one gateway's Anthropic endpoint cut Claude's stream in 9 of 9 replays of a request where
    // the call followed a write, and in 0 of 5 without it (2026-10-11). `prepare` still takes a `plan` from models that
    // pass one anyway (Claude Code's ExitPlanMode habit)
    parameters: json!({ "type": "object", "properties": {} }),
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
