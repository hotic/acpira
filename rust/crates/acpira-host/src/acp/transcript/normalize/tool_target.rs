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
    return Some(Target { text: basename(loc), mono: false, from_title: false });
  }
  if matches!(kind, ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
    && let Some(path) = path_from_raw(raw)
  {
    return Some(Target { text: basename(&path), mono: false, from_title: false });
  }
  text_of(u, "title").map(|title| Target { text: strip_verb(title), mono: false, from_title: true })
}

pub(super) fn shell_target(raw: Option<&Map<String, Value>>, ctx: Option<&ToolCtx>) -> Option<Target> {
  let raw = raw?;
  // The built-in agent's model may write a job id as a number
  let shell_id = ["shell_id", "shellId", "id"].iter().find_map(|k| match raw.get(*k)? {
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
