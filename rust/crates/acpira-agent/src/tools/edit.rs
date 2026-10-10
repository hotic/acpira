//! `write` (a whole file) and `edit` (replace exact snippets: one, or several to the same file applied in order, which
//! saves a model round trip per extra change). The edit is computed while preparing, so the permission card shows the
//! real result; at run time it is re-applied to whatever the file holds then. Matching tolerates the
//! two slips models make most: LF written for a CRLF file, and trailing whitespace

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Action, Ctx, Output, diff_content, resolve, shown, str_arg};
use crate::llm::ToolSpec;

#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
  pub old: String,
  pub new: String,
  pub all: bool,
}

pub fn write_spec() -> ToolSpec {
  ToolSpec {
    name: super::WRITE.into(),
    description: "Create a file or replace its whole content. Prefer edit for changes to an existing file.".into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "File path, absolute or relative to the session folder" },
        "content": { "type": "string", "description": "The complete new content" },
      },
      "required": ["path", "content"],
    }),
  }
}

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::EDIT.into(),
    description: "Replace an exact snippet of an existing file. old_string must match the file text exactly (copy it from read output, \
                  without the line-number prefix) and be unique, unless replace_all is set. For several changes to one file, give \
                  edits instead: they apply in order, each to the result of the previous one, and all or none are made."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "File path, absolute or relative to the session folder" },
        "old_string": { "type": "string", "description": "The exact text to replace" },
        "new_string": { "type": "string", "description": "The replacement text" },
        "replace_all": { "type": "boolean", "description": "Replace every occurrence (default false)" },
        "edits": {
          "type": "array",
          "description": "Several replacements in this file, instead of old_string / new_string",
          "items": {
            "type": "object",
            "properties": { "old_string": { "type": "string" }, "new_string": { "type": "string" }, "replace_all": { "type": "boolean" } },
            "required": ["old_string", "new_string"],
          },
        },
      },
      "required": ["path"],
    }),
  }
}

fn path_arg(args: &Value, cwd: &Path) -> Result<PathBuf, String> {
  Ok(resolve(str_arg(args, "path").or_else(|_| str_arg(args, "file_path"))?, cwd))
}

fn read_text(path: &Path) -> Result<Option<String>, String> {
  match std::fs::read(path) {
    Ok(b) => String::from_utf8(b).map(Some).map_err(|_| format!("{} is not a UTF-8 text file", path.display())),
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
    Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
  }
}

pub fn prepare_write(args: &Value, cwd: &Path) -> Result<Action, String> {
  let path = path_arg(args, cwd)?;
  let after = str_arg(args, "content")?.to_owned();
  if path.is_dir() {
    return Err(format!("{} is a directory", path.display()));
  }
  let before = read_text(&path)?;
  Ok(Action::Write { path, before, after, edits: vec![] })
}

fn edit_of(v: &Value) -> Result<Edit, String> {
  Ok(Edit {
    old: str_arg(v, "old_string")?.to_owned(),
    new: str_arg(v, "new_string")?.to_owned(),
    all: v.get("replace_all").and_then(Value::as_bool).unwrap_or(false),
  })
}

pub fn prepare(args: &Value, cwd: &Path) -> Result<Action, String> {
  let path = path_arg(args, cwd)?;
  let edits = match args.get("edits").and_then(Value::as_array).filter(|a| !a.is_empty()) {
    Some(list) => list.iter().map(edit_of).collect::<Result<Vec<_>, _>>()?,
    None => vec![edit_of(args)?],
  };
  let before = read_text(&path)?.ok_or_else(|| format!("File not found: {} (use write to create it)", path.display()))?;
  let after = apply_all(&before, &edits).map_err(|e| format!("{e} in {}", path.display()))?;
  Ok(Action::Write { path, before: Some(before), after, edits })
}

/// The file with every edit applied in order; the first that does not apply fails the whole call
pub fn apply_all(content: &str, edits: &[Edit]) -> Result<String, String> {
  let mut out = content.to_owned();
  for (i, e) in edits.iter().enumerate() {
    out = apply(&out, e).map_err(|err| if edits.len() > 1 { format!("edits[{i}]: {err}") } else { err })?;
  }
  Ok(out)
}

/// The file with the edit applied
pub fn apply(content: &str, e: &Edit) -> Result<String, String> {
  if e.old.is_empty() {
    return Err("old_string is empty (use write to create or replace a whole file)".into());
  }
  if e.old == e.new {
    return Err("old_string and new_string are identical".into());
  }
  let mut candidates = vec![(e.old.clone(), e.new.clone())];
  // The file uses CRLF, the model wrote LF
  if content.contains("\r\n") && !e.old.contains("\r\n") {
    candidates.push((e.old.replace('\n', "\r\n"), e.new.replace('\n', "\r\n")));
  }
  for (old, new) in &candidates {
    match content.matches(old.as_str()).count() {
      0 => continue,
      1 => return Ok(content.replacen(old.as_str(), new, 1)),
      _ if e.all => return Ok(content.replace(old.as_str(), new)),
      n => return Err(format!("old_string occurs {n} times; add surrounding lines to make it unique, or set replace_all")),
    }
  }
  // Trailing whitespace differs: match line by line ignoring it, and only a unique match
  if let Some((start, end)) = unique_trimmed_match(content, &e.old) {
    let new = if content.contains("\r\n") && !e.new.contains("\r\n") { e.new.replace('\n', "\r\n") } else { e.new.clone() };
    return Ok(format!("{}{}{}", &content[..start], new, &content[end..]));
  }
  Err("old_string not found; read the file again and copy the exact text".into())
}

/// Byte span of the only run of lines equal to `old`'s lines up to trailing whitespace
fn unique_trimmed_match(content: &str, old: &str) -> Option<(usize, usize)> {
  let want: Vec<&str> = old.lines().map(str::trim_end).collect();
  if want.is_empty() || want.iter().all(|l| l.is_empty()) {
    return None;
  }
  // (start byte, line without its terminator, end byte after the terminator)
  let mut lines = vec![];
  let mut pos = 0;
  for raw in content.split_inclusive('\n') {
    let body = raw.trim_end_matches(['\n', '\r']);
    lines.push((pos, body, pos + raw.len()));
    pos += raw.len();
  }
  let mut found = None;
  for i in 0..lines.len() {
    if i + want.len() > lines.len() {
      break;
    }
    if (0..want.len()).all(|k| lines[i + k].1.trim_end() == want[k]) {
      if found.is_some() {
        return None;
      }
      let last = &lines[i + want.len() - 1];
      // Keep the last line's terminator unless old_string itself ended with one
      let end = if old.ends_with('\n') { last.2 } else { last.0 + last.1.len() };
      found = Some((lines[i].0, end));
    }
  }
  found
}

pub fn run(path: &PathBuf, before: Option<String>, after: String, edits: Vec<Edit>, ctx: &Ctx) -> Output {
  // The file may have changed while the card waited: an edit is re-applied to the current text
  let current = match read_text(path) {
    Ok(c) => c,
    Err(e) => return Output::error(e),
  };
  let edited = !edits.is_empty();
  let (before, after) = match current {
    Some(now) if edited && Some(&now) != before.as_ref() => match apply_all(&now, &edits) {
      Ok(a) => (Some(now), a),
      Err(err) => return Output::error(format!("The file changed since the edit was prepared and the edit no longer applies: {err}")),
    },
    None if edited => return Output::error(format!("{} was deleted before the edit ran", path.display())),
    now => (now.or(before), after),
  };
  if let Some(dir) = path.parent()
    && let Err(e) = std::fs::create_dir_all(dir)
  {
    return Output::error(format!("Cannot create {}: {e}", dir.display()));
  }
  if let Err(e) = std::fs::write(path, &after) {
    return Output::error(format!("Cannot write {}: {e}", path.display()));
  }
  let name = shown(path, &ctx.cwd);
  let model = match (edited, &before) {
    (true, _) if edits.len() > 1 => format!("Edited {name} ({} changes).", edits.len()),
    (true, _) => format!("Edited {name}."),
    (false, Some(_)) => format!("Replaced {name} ({} lines).", after.lines().count()),
    (false, None) => format!("Created {name} ({} lines).", after.lines().count()),
  };
  Output { model, is_error: false, content: vec![diff_content(path, before.as_deref(), &after)], raw_output: None }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn e(old: &str, new: &str) -> Edit {
    Edit { old: old.into(), new: new.into(), all: false }
  }

  #[test]
  fn exact_unique_and_ambiguous() {
    assert_eq!(apply("a b c", &e("b", "x")).unwrap(), "a x c");
    assert!(apply("b b", &e("b", "x")).unwrap_err().contains("2 times"));
    assert_eq!(apply("b b", &Edit { all: true, ..e("b", "x") }).unwrap(), "x x");
    assert!(apply("abc", &e("zzz", "y")).unwrap_err().contains("not found"));
  }

  #[test]
  fn several_edits_apply_in_order_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.txt"), "a b c\n").unwrap();
    let args = json!({ "path": "x.txt", "edits": [{ "old_string": "a", "new_string": "x" }, { "old_string": "x b", "new_string": "y" }] });
    let Action::Write { after, edits, .. } = prepare(&args, dir.path()).unwrap() else { panic!() };
    assert_eq!((after.as_str(), edits.len()), ("y c\n", 2));
    let bad = json!({ "path": "x.txt", "edits": [{ "old_string": "a", "new_string": "x" }, { "old_string": "zzz", "new_string": "y" }] });
    let Err(e) = prepare(&bad, dir.path()) else { panic!() };
    assert!(e.starts_with("edits[1]: old_string not found"), "{e}");
  }

  #[test]
  fn crlf_files_take_lf_snippets() {
    assert_eq!(apply("one\r\ntwo\r\nthree\r\n", &e("one\ntwo", "1\n2")).unwrap(), "1\r\n2\r\nthree\r\n");
  }

  #[test]
  fn trailing_whitespace_is_forgiven_when_unique() {
    let file = "fn a() {   \n  x();\n}\nrest\n";
    assert_eq!(apply(file, &e("fn a() {\n  x();\n}", "fn a() {}")).unwrap(), "fn a() {}\nrest\n");
  }

  #[test]
  fn run_reapplies_to_a_file_changed_meanwhile() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("x.txt");
    std::fs::write(&f, "alpha beta\n").unwrap();
    let Action::Write { path, before, after, edits } =
      prepare(&json!({ "path": "x.txt", "old_string": "beta", "new_string": "gamma" }), dir.path()).unwrap()
    else {
      panic!()
    };
    std::fs::write(&f, "first line\nalpha beta\n").unwrap();
    let ctx = Ctx { cwd: dir.path().to_owned(), outputs: dir.path().join("o"), call_id: "c".into(), progress: Box::new(|_| {}) };
    let out = run(&path, before, after, edits, &ctx);
    assert!(!out.is_error, "{}", out.model);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "first line\nalpha gamma\n");
  }
}
