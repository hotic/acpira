//! What a tool row names as its target: the path and read range, the command, an image prompt, or the file(s) a
//! show_image / show_widget call shows

use serde_json::{Map, Value};

use acpira_shared::transcript::*;

use crate::json::{basename, text_of};

use super::ToolCtx;
use super::tool_kind::strip_verb;

pub(super) struct Target {
  pub(super) text: String,
  pub(super) mono: bool,
  pub(super) from_title: bool,
}

pub fn path_from_raw(raw: Option<&Map<String, Value>>) -> Option<String> {
  let raw = raw?;
  ["path", "file_path", "filePath", "filepath"]
    .iter()
    .find_map(|k| raw.get(*k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned))
}

fn positive(v: Option<&Value>) -> Option<u64> {
  let n = v?.as_f64()?;
  (n.fract() == 0.0 && n > 0.0 && n <= 9_007_199_254_740_991.0).then_some(n as u64)
}

// JS `a ?? b`: the first operand that is neither absent nor null
pub(super) fn first<'a>(raw: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
  keys.iter().find_map(|k| raw.get(*k).filter(|v| !v.is_null()))
}

pub(super) fn read_range_from_raw(raw: &Map<String, Value>, locations: Option<&[Location]>) -> Option<ReadRange> {
  let path = match path_from_raw(Some(raw)) {
    // The built-in agent's input names the file relative to the session folder while its location is absolute: the
    // range takes the location's spelling, which the webview matches it against
    Some(p) => locations
      .and_then(|l| l.iter().find(|loc| loc.path != p && std::path::Path::new(&loc.path).ends_with(&p)))
      .map_or(p, |loc| loc.path.clone()),
    None => locations.filter(|l| l.len() == 1).map(|l| l[0].path.clone())?,
  };
  let start = positive(first(raw, &["line_offset", "start_line", "startLine", "offset"]))?;
  let count = positive(first(raw, &["n_lines", "limit", "line_count"]));
  let end = positive(first(raw, &["end_line", "endLine"])).or_else(|| count.map(|c| start + c - 1));
  Some(ReadRange { path, start, end: end.filter(|e| *e >= start) })
}

pub fn command_from_raw(raw: Option<&Map<String, Value>>) -> Option<String> {
  let raw = raw?;
  raw.get("command").and_then(Value::as_str).or_else(|| raw.get("cmd").and_then(Value::as_str)).map(str::to_owned)
}

/// Display label for a file path on a tool row: normally its basename, except `agy`'s per-step spilled files
/// (`.../.system_generated/steps/<N>/media_0.png` or `output.txt`), which keep `steps/<N>/<file>` so rows from
/// different steps do not all read as the same file
pub(super) fn display_file_name(path: &str) -> String {
  let parts: Vec<&str> = path.split(['/', '\\']).filter(|s| !s.is_empty()).collect();
  if let [.., ".system_generated", "steps", step, file] = parts.as_slice()
    && !step.is_empty()
    && step.bytes().all(|b| b.is_ascii_digit())
  {
    return format!("steps/{step}/{file}");
  }
  basename(path).to_owned()
}

pub(super) fn pick_target(u: &Value, kind: ToolKind) -> Option<Target> {
  let raw = u.get("rawInput").and_then(Value::as_object);
  let mono = |text: String| Some(Target { text, mono: true, from_title: false });
  if kind == ToolKind::Execute
    && let Some(cmd) = command_from_raw(raw).filter(|c| !c.is_empty())
  {
    return mono(cmd);
  }
  if kind == ToolKind::Search {
    let q = raw.and_then(|r| r.get("pattern").and_then(Value::as_str).or_else(|| r.get("query").and_then(Value::as_str)));
    if let Some(q) = q.filter(|q| !q.is_empty()) {
      return mono(q.to_owned());
    }
  }
  // codex-acp's opened page keeps its URL in `rawInput.action.url`
  if kind == ToolKind::Fetch
    && let Some(url) = raw
      .and_then(|r| r.get("url").or_else(|| r.get("action").and_then(|a| a.get("url"))))
      .and_then(Value::as_str)
      .filter(|x| !x.is_empty())
  {
    return mono(url.to_owned());
  }
  if let Some(loc) = u.get("locations").and_then(Value::as_array).and_then(|l| l.first()).and_then(|l| text_of(l, "path")) {
    return Some(Target { text: display_file_name(loc), mono: false, from_title: false });
  }
  if matches!(kind, ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
    && let Some(path) = path_from_raw(raw)
  {
    return Some(Target { text: display_file_name(&path), mono: false, from_title: false });
  }
  text_of(u, "title").map(|title| Target { text: strip_verb(title), mono: false, from_title: true })
}

pub(super) fn shell_target(raw: Option<&Map<String, Value>>, ctx: Option<&ToolCtx>) -> Option<Target> {
  let raw = raw?;
  // Remote `BashOutput` passes `bash_id`, `KillShell` / Devin pass `shell_id`, and the built-in agent's `job` may write `id` as a number
  let shell_id = ["shell_id", "shellId", "bash_id", "bashId", "id"].iter().find_map(|k| match raw.get(*k)? {
    Value::String(v) if !v.is_empty() => Some(v.clone()),
    Value::Number(n) => Some(n.to_string()),
    _ => None,
  })?;
  let text = ctx.and_then(|c| c.shells.get(&shell_id)).cloned().unwrap_or(shell_id);
  Some(Target { text, mono: true, from_title: false })
}

/// An image generation call's prompt, when the agent puts it in rawInput (codex-acp only sends the item id)
pub(super) fn prompt_target(raw: Option<&Map<String, Value>>) -> Option<Target> {
  let raw = raw?;
  let prompt = ["prompt", "description"].iter().find_map(|k| raw.get(*k).and_then(Value::as_str).map(str::trim).filter(|v| !v.is_empty()))?;
  Some(Target { text: prompt.to_owned(), mono: false, from_title: false })
}

/// A show_image row names its caption, else the file(s) it shows
pub(super) fn show_image_target(args: Option<&Map<String, Value>>, locs: Option<&[Location]>) -> Option<Target> {
  if let Some(caption) = args.and_then(|a| a.get("caption")).and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty()) {
    return Some(Target { text: caption.to_owned(), mono: false, from_title: false });
  }
  let names: Vec<String> = locs?.iter().map(|l| basename(&l.path).to_owned()).collect();
  (!names.is_empty()).then(|| Target { text: names.join(", "), mono: true, from_title: false })
}

/// A show_widget row names its title, else the file it shows
pub(super) fn show_widget_target(args: Option<&Map<String, Value>>, locs: Option<&[Location]>) -> Option<Target> {
  if let Some(title) = args.and_then(|a| a.get("title")).and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty()) {
    return Some(Target { text: title.to_owned(), mono: false, from_title: false });
  }
  let path = locs?.first()?;
  Some(Target { text: basename(&path.path).to_owned(), mono: true, from_title: false })
}

/// Every string inside a JSON value, depth first
pub(super) fn json_strings<'a>(v: &'a Value, out: &mut Vec<&'a str>) {
  match v {
    Value::String(s) => out.push(s),
    Value::Array(a) => a.iter().for_each(|x| json_strings(x, out)),
    Value::Object(o) => o.values().for_each(|x| json_strings(x, out)),
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::super::{NormalizeState, apply_update};
  use super::*;

  #[test]
  fn step_artifacts_keep_their_step_number() {
    assert_eq!(
      display_file_name("/C:/Users/me/.gemini/antigravity-cli/brain/b1/.system_generated/steps/114/media_0.png"),
      "steps/114/media_0.png"
    );
    assert_eq!(
      display_file_name(r"C:\Users\me\.gemini\antigravity-cli\brain\b1\.system_generated\steps\59\output.txt"),
      "steps/59/output.txt"
    );
    assert_eq!(display_file_name("/repo/assets/media_0.png"), "media_0.png");
  }

  #[test]
  fn bash_output_resolves_background_command_by_bash_id() {
    let mut s = NormalizeState::new(vec![]);
    apply_update(
      &mut s,
      &json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-1",
        "title": "cargo test",
        "kind": "execute",
        "status": "in_progress",
        "rawInput": { "command": "cargo test", "run_in_background": true },
      }),
    );
    apply_update(
      &mut s,
      &json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "call-1",
        "status": "completed",
        "rawOutput": { "job": "1" },
      }),
    );
    apply_update(
      &mut s,
      &json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-2",
        "title": "Check job 1",
        "kind": "execute",
        "status": "in_progress",
        "rawInput": { "bash_id": "1", "wait": 120 },
      }),
    );
    apply_update(
      &mut s,
      &json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "call-3",
        "title": "Check job 9",
        "kind": "execute",
        "status": "in_progress",
        "rawInput": { "bash_id": "9" },
      }),
    );
    let blocks = match s.turns.first() {
      Some(Turn::Agent(t)) => &t.blocks,
      _ => &[][..],
    };
    let check_known = match blocks.get(1) {
      Some(AgentBlock::ToolCall(b)) => Some((b.verb_key.as_deref(), b.target.as_deref(), b.target_mono)),
      _ => None,
    };
    let check_unknown = match blocks.get(2) {
      Some(AgentBlock::ToolCall(b)) => Some((b.verb_key.as_deref(), b.target.as_deref(), b.target_mono)),
      _ => None,
    };
    assert_eq!(check_known, Some((Some("verb.wait"), Some("cargo test"), Some(true))));
    assert_eq!(check_unknown, Some((Some("verb.wait"), Some("9"), Some(true))));
  }

  #[test]
  fn computer_act_names_the_app_then_the_steps_and_the_reply_names_the_elements() {
    let mut s = NormalizeState::new(vec![]);
    apply_update(
      &mut s,
      &json!({
        "sessionUpdate": "tool_call",
        "toolCallId": "cu-1",
        "title": "mcp__acpira__computer_act",
        "kind": "other",
        "status": "in_progress",
        "rawInput": { "app": "C:\\Windows\\notepad.exe", "steps": [["click", "b1"], ["type", "hi"], { "do": "key", "keys": "ctrl+a" }] },
      }),
    );
    let row = |s: &NormalizeState| match s.turns.first() {
      Some(Turn::Agent(t)) => match t.blocks.first() {
        Some(AgentBlock::ToolCall(b)) => Some((b.verb_key.clone(), b.target.clone(), b.mcp_server.clone())),
        _ => None,
      },
      _ => None,
    };
    assert_eq!(row(&s), Some((Some("verb.computerAct".into()), Some("notepad\nclick, type \"hi\", key \"ctrl+a\"".into()), None)));
    apply_update(
      &mut s,
      &json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": "cu-1",
        "status": "completed",
        "content": [{ "type": "content", "content": { "type": "text", "text": "# 3/3 steps, 80 ms\n> click \"Select\", type \"hi\", key \"ctrl+a\"\n~ t1 textfield" } }],
      }),
    );
    assert_eq!(row(&s), Some((Some("verb.computerAct".into()), Some("notepad\nclick \"Select\", type \"hi\", key \"ctrl+a\"".into()), None)));
  }
}
