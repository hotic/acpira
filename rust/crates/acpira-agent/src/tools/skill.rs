//! `skill`: the model loads a skill's instructions by name. The tool is offered only while the session has skills
//! (`skills::scan`), and its description lists them with their descriptions, so the model knows what exists and reads
//! the full text only when it needs it (OpenCode's progressive loading)

use std::path::Path;

use serde_json::{Value, json};

use super::{Action, Output, str_arg, text_content};
use crate::llm::ToolSpec;
use crate::prompt::Places;
use crate::skills::{self, Skill};

pub fn spec(skills: &[Skill]) -> ToolSpec {
  let list: Vec<String> = skills
    .iter()
    .map(|s| if s.description.is_empty() { format!("- {}", s.name) } else { format!("- {}: {}", s.name, s.description) })
    .collect();
  ToolSpec {
    name: super::SKILL.to_owned(),
    description: format!(
      "Load the instructions of a skill: steps and conventions for a kind of task. Call it when the task matches one of the skills below, before starting on it, and follow what it says.\n\nAvailable skills:\n{}",
      list.join("\n")
    ),
    parameters: json!({
      "type": "object",
      "properties": { "name": { "type": "string", "description": "The skill's name, exactly as listed" } },
      "required": ["name"],
    }),
  }
}

/// `places`: where the session's skills are (a folder on an SSH host: what the host sent)
pub fn prepare(args: &Value, places: &Places) -> Result<Action, String> {
  let name = str_arg(args, "name")?.trim();
  let skills = skills::scan(places);
  match skills.iter().find(|s| s.name == name) {
    Some(s) => Ok(Action::Skill { name: s.name.clone(), path: s.path.clone(), body: s.body.clone() }),
    None => Err(format!("Unknown skill \"{name}\". Available skills: {}.", skills.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))),
  }
}

/// The instructions were read when the call was prepared
pub fn run(path: &Path, body: &str) -> Output {
  let dir = path.parent().unwrap_or(path);
  let model = format!("{body}\n\n(Files the skill refers to are relative to {})", dir.display());
  Output { content: vec![text_content(body)], model, is_error: false, raw_output: None }
}
