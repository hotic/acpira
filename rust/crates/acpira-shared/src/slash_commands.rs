//! Slash command helpers (mirror of src/shared/slashCommands.ts)

use std::sync::LazyLock;

use regex::Regex;

use crate::transcript::{CommandOption, CommandReceipt, SessionControls, SlashCommand, Turn, TurnSettings, TurnStop};

static NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/(\$?[\p{L}\p{N}][\p{L}\p{N}_.:-]*)").unwrap());

/// A command candidate must be a leading token followed by whitespace or the end
pub fn command_name(text: &str) -> Option<&str> {
  let c = NAME.captures(text)?;
  let m = c.get(1)?;
  let rest = &text[m.end()..];
  if rest.is_empty() || rest.starts_with(char::is_whitespace) { Some(m.as_str()) } else { None }
}

pub fn named_command<'a>(commands: &'a [SlashCommand], text: &str) -> Option<&'a SlashCommand> {
  let name = command_name(text)?;
  commands.iter().find(|c| c.name == name)
}

/// Older records kept the native stop reason but no command receipt; recover the empty-request explanation
pub fn restore_command_receipts(mut turns: Vec<Turn>) -> Vec<Turn> {
  for i in 1..turns.len() {
    let name = match (&turns[i - 1], &turns[i]) {
      (Turn::User(u), Turn::Agent(a)) if a.command.is_none() && a.stop == Some(TurnStop::EndTurn) && a.blocks.is_empty() => {
        command_name(&u.text).map(str::to_owned)
      }
      _ => None,
    };
    if let (Some(name), Some(a)) = (name, turns[i].as_agent_mut()) {
      a.command = Some(CommandReceipt { name, mode: None, options: None });
    }
  }
  turns
}

/// Report only settings that actually changed on the ACP wire
pub fn command_changes(before: &TurnSettings, after: &SessionControls) -> (Option<String>, Option<Vec<CommandOption>>) {
  let mode = match &after.mode_id {
    Some(m) if before.mode_id.as_ref() != Some(m) => {
      Some(after.modes.iter().find(|x| &x.id == m).map(|x| x.name.clone()).unwrap_or_else(|| m.clone()))
    }
    _ => None,
  };
  let options: Vec<CommandOption> = after
    .options
    .iter()
    .filter(|c| c.value.as_ref() != before.config.get(&c.id))
    .filter_map(|c| {
      // JS renders an undefined value as absent; the receipt keeps only real values
      let v = c.value.as_ref()?;
      Some(CommandOption {
        name: c.name.clone(),
        value: c.options.iter().find(|o| &o.id == v).map(|o| o.name.clone()).unwrap_or_else(|| v.clone()),
      })
    })
    .collect();
  (mode, if options.is_empty() { None } else { Some(options) })
}

#[cfg(test)]
mod tests {
  use super::command_name;

  #[test]
  fn names() {
    assert_eq!(command_name("/compact"), Some("compact"));
    assert_eq!(command_name("/review now"), Some("review"));
    assert_eq!(command_name("/a/b"), None);
    assert_eq!(command_name("/计划 x"), Some("计划"));
    assert_eq!(command_name("hi /x"), None);
    assert_eq!(command_name("/$git-commit x"), Some("$git-commit"));
    assert_eq!(command_name("/skill:dig"), Some("skill:dig"));
    assert_eq!(command_name("/$ x"), None);
  }
}
