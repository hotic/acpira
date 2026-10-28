//! A region's tool calls with their results: their state, whether they failed, the files they changed and how

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_json::Value;

use super::size::one_line;
use crate::llm::Item;

/// The calls of a region with their results, in order
pub(super) struct Call<'a> {
  pub(super) name: &'a str,
  pub(super) args: Value,
  pub(super) raw_args: &'a str,
  pub(super) result: Option<(&'a str, bool)>,
  /// The assistant item that made it (an index into the region)
  pub(super) at: usize,
}

pub(super) fn calls(region: &[Item]) -> Vec<Call<'_>> {
  // Providers reuse ids across replies: a result answers the latest open call with its id
  let mut out: Vec<Call> = vec![];
  let mut open: HashMap<&str, usize> = HashMap::new();
  for (i, item) in region.iter().enumerate() {
    match item {
      Item::Assistant { tool_calls, .. } => {
        for c in tool_calls {
          open.insert(c.id.as_str(), out.len());
          out.push(Call { name: &c.name, args: serde_json::from_str(&c.arguments).unwrap_or(Value::Null), raw_args: &c.arguments, result: None, at: i });
        }
      }
      Item::ToolResult { call_id, content, is_error, .. } => {
        if let Some(k) = open.remove(call_id.as_str()) {
          out[k].result = Some((content.as_str(), *is_error));
        }
      }
      Item::User(_) => {}
    }
  }
  out
}

pub(super) fn arg<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
  keys.iter().find_map(|k| args.get(*k).and_then(Value::as_str))
}

/// A path as the index shows it: relative to the session folder
pub(super) fn rel(path: &str, cwd: &Path) -> String {
  crate::tools::shown(&crate::tools::resolve(path, cwd), cwd)
}

/// A result without the to-do note a call that carried the list gets in front (`tools::todo::note`)
pub(super) fn own_result(output: &str) -> &str {
  match output.strip_prefix(crate::tools::todo::NOTE_PREFIX).and_then(|rest| rest.split_once("\n\n")) {
    Some((_, rest)) => rest,
    None => output,
  }
}

/// What the command an edit or a write ran after its change (`then_run`) printed: the section after its `$ command`
/// line, when it ran
pub(super) fn then_run_output<'a>(output: &'a str, command: &str) -> Option<&'a str> {
  let marker = format!("\n\n$ {command}\n");
  let at = output.rfind(&marker)?;
  let rest = output.get(at + marker.len()..)?;
  (!rest.starts_with("Not run: ")).then_some(rest)
}

/// The exit code a `bash` result ends with (`[exit code N]`)
pub(super) fn exit_code(output: &str) -> Option<i64> {
  output.trim_end().rsplit('\n').next()?.strip_prefix("[exit code ")?.strip_suffix(']')?.parse().ok()
}

/// A call's state as the history shows it
pub(super) fn state(call: &Call) -> &'static str {
  match call.result {
    Some((_, true)) => "error",
    Some(_) => "completed",
    None => "no result",
  }
}

/// A failed call: an error result, or a command that exited non-zero
pub(super) fn failed(call: &Call) -> bool {
  match call.result {
    Some((_, true)) => true,
    Some((out, false)) => call.name == crate::tools::BASH && exit_code(out).is_some_and(|c| c != 0),
    None => false,
  }
}

/// The files a call wrote, with the changed lines (`-` / `+`, a few of them); an edit's items may name their own files
pub(super) fn file_changes(call: &Call, cwd: &Path) -> Vec<(String, Vec<String>)> {
  let Some(path) = arg(&call.args, &["path", "file_path"]) else { return vec![] };
  let file = rel(path, cwd);
  match call.name {
    crate::tools::WRITE => vec![(file, edit_lines("", arg(&call.args, &["content"]).unwrap_or("")))],
    crate::tools::EDIT => match call.args.get("edits").and_then(Value::as_array).filter(|a| !a.is_empty()) {
      Some(edits) => {
        let mut out: Vec<(String, Vec<String>)> = vec![];
        for e in edits {
          let f = arg(e, &["path", "file_path"]).filter(|p| !p.trim().is_empty()).map_or_else(|| file.clone(), |p| rel(p.trim(), cwd));
          let lines = edit_lines(arg(e, &["old_string"]).unwrap_or(""), arg(e, &["new_string"]).unwrap_or(""));
          match out.iter_mut().find(|(name, _)| *name == f) {
            Some((_, l)) => l.extend(lines),
            None => out.push((f, lines)),
          }
        }
        out
      }
      None => vec![(file, edit_lines(arg(&call.args, &["old_string"]).unwrap_or(""), arg(&call.args, &["new_string"]).unwrap_or("")))],
    },
    _ => vec![],
  }
}

/// The lines an edit took out and put in, six at most, with a line of context for a pure insertion
fn edit_lines(before: &str, after: &str) -> Vec<String> {
  let old: Vec<&str> = before.split('\n').collect();
  let new: Vec<&str> = after.split('\n').collect();
  let mut lines: Vec<String> = old.iter().filter(|l| !l.trim().is_empty() && !new.contains(l)).map(|l| format!("- {}", one_line(l, 40))).collect();
  lines.extend(new.iter().filter(|l| !l.trim().is_empty() && !old.contains(l)).map(|l| format!("+ {}", one_line(l, 40))));
  let anchor = old.iter().find(|l| !l.trim().is_empty() && new.contains(l));
  let mut out: Vec<String> = lines.iter().take(6).cloned().collect();
  if lines.len() > 6 {
    out.push(format!("… {} more changed lines", lines.len() - 6));
  }
  if let Some(a) = anchor
    && lines.iter().all(|l| l.starts_with('+'))
  {
    out.push(format!("@ {}", one_line(a, 40)));
  }
  out
}

/// The files the region's calls changed, each once
pub(super) fn changed_files(region: &[Item], cwd: &Path) -> Vec<String> {
  let mut seen = HashSet::new();
  calls(region)
    .iter()
    .filter(|c| matches!(c.result, Some((_, false))))
    .flat_map(|c| file_changes(c, cwd))
    .map(|(f, _)| f)
    .filter(|f| seen.insert(f.clone()))
    .collect()
}
