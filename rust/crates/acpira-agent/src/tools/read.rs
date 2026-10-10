//! `read`: a text file with line numbers, a window at a time. The header always states the total line count, so the
//! model knows whether it saw everything. `paths` adds more files to the same call (read from their first line, within
//! one shared output budget): a model that makes one call per reply (GPT-6.1 on every harness, 2026-10) otherwise
//! spends a request per file

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Action, Ctx, Output, num_arg, resolve, str_arg};
use crate::budget::MAX_BYTES;
use crate::llm::ToolSpec;

pub const DEFAULT_LIMIT: usize = 2000;
/// Files one call may read
pub const MAX_PATHS: usize = 12;
/// A longer line is cut, with a marker
const MAX_LINE: usize = 2000;

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::READ.into(),
    description: "Read a text file. Returns numbered lines (up to 2000 per call) and the file's total line count; use offset / limit for \
                  the next window. Relative paths resolve against the session folder. To look at several files, list the others in \
                  paths: one call reads them all."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "File path, absolute or relative to the session folder" },
        "offset": { "type": "integer", "description": "First line to read, 1-based (default 1)" },
        "limit": { "type": "integer", "description": "How many lines to read (default 2000)" },
        "paths": { "type": "array", "items": { "type": "string" }, "description": "More files to read in the same call, each from its first line" },
      },
      "required": ["path"],
    }),
  }
}

pub fn prepare(args: &Value, cwd: &Path) -> Result<Action, String> {
  let path = resolve(str_arg(args, "path").or_else(|_| str_arg(args, "file_path"))?, cwd);
  let offset = num_arg(args, "offset").unwrap_or(1).max(1) as usize;
  let limit = num_arg(args, "limit").filter(|l| *l > 0).unwrap_or(DEFAULT_LIMIT as u64) as usize;
  let mut more: Vec<PathBuf> = vec![];
  for p in args.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
    let p = resolve(p, cwd);
    // Models repeat the first path in the list
    if p != path && !more.contains(&p) {
      more.push(p);
    }
  }
  if more.len() + 1 > MAX_PATHS {
    return Err(format!("One read takes at most {MAX_PATHS} files; split the rest into another call"));
  }
  Ok(Action::Read { path, offset, limit, more })
}

/// The first file with its window, then the others whole, within one output budget; a file that cannot be read is
/// reported in its place. Only a single file that fails makes the call an error
pub fn run(path: &Path, offset: usize, limit: usize, more: &[PathBuf], _ctx: &Ctx) -> Output {
  let first = one(path, offset, limit, MAX_BYTES);
  if more.is_empty() {
    return first;
  }
  let mut text = first.model;
  let mut skipped: Vec<String> = vec![];
  for p in more {
    let room = MAX_BYTES.saturating_sub(text.len());
    // Less than a screenful left: name the rest instead of cutting each to nothing
    if room < 2048 {
      skipped.push(p.display().to_string());
      continue;
    }
    text.push_str("\n\n");
    text.push_str(&one(p, 1, DEFAULT_LIMIT, room).model);
  }
  if !skipped.is_empty() {
    text.push_str(&format!("\n\nNot read, the output budget is used up: {}. Read them in another call.", skipped.join(", ")));
  }
  Output { model: text, is_error: false, content: vec![], raw_output: None }
}

fn one(path: &Path, offset: usize, limit: usize, max_bytes: usize) -> Output {
  let meta = match std::fs::metadata(path) {
    Ok(m) => m,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Output::error(format!("File not found: {}", path.display())),
    Err(e) => return Output::error(format!("Cannot read {}: {e}", path.display())),
  };
  if meta.is_dir() {
    return Output::error(format!("{} is a directory; list it with the list tool instead", path.display()));
  }
  let bytes = match std::fs::read(path) {
    Ok(b) => b,
    Err(e) => return Output::error(format!("Cannot read {}: {e}", path.display())),
  };
  if bytes.iter().take(8192).any(|b| *b == 0) {
    return Output::error(format!("{} is a binary file ({} bytes); it cannot be read as text", path.display(), bytes.len()));
  }
  let text = String::from_utf8_lossy(&bytes);
  let total = text.lines().count();
  if total == 0 {
    return Output { model: format!("{} is empty.", path.display()), is_error: false, content: vec![], raw_output: None };
  }
  if offset > total {
    return Output::error(format!("offset {offset} is past the end: {} has {total} lines", path.display()));
  }
  let mut body = String::new();
  let mut last = offset - 1;
  for (i, line) in text.lines().enumerate().skip(offset - 1).take(limit) {
    let line = if line.len() > MAX_LINE { format!("{}… [line cut at {MAX_LINE} bytes]", crate::budget::cut(line, MAX_LINE)) } else { line.to_owned() };
    let row = format!("{:>6}\t{line}\n", i + 1);
    if body.len() + row.len() > max_bytes && last >= offset {
      break;
    }
    body.push_str(&row);
    last = i + 1;
  }
  let header = if offset == 1 && last == total {
    format!("{} ({total} lines)\n", path.display())
  } else {
    format!("{} (lines {offset}-{last} of {total}; continue with offset {})\n", path.display(), last + 1)
  };
  Output { model: header + &body, is_error: false, content: vec![], raw_output: None }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ctx(dir: &Path) -> Ctx {
    Ctx { cwd: dir.to_owned(), outputs: dir.join("outputs"), call_id: "c".into(), progress: Box::new(|_| {}), jobs: Default::default() }
  }

  #[test]
  fn windows_and_totals() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, (1..=10).map(|n| format!("l{n}\n")).collect::<String>()).unwrap();
    let all = run(&f, 1, 2000, &[], &ctx(dir.path()));
    assert!(all.model.contains("(10 lines)") && all.model.contains("    10\tl10"));
    let part = run(&f, 3, 2, &[], &ctx(dir.path()));
    assert!(part.model.contains("(lines 3-4 of 10; continue with offset 5)"), "{}", part.model);
    assert!(run(&f, 11, 5, &[], &ctx(dir.path())).is_error);
    std::fs::write(&f, [0u8, 1, 2]).unwrap();
    assert!(run(&f, 1, 10, &[], &ctx(dir.path())).model.contains("binary"));
  }

  #[test]
  fn several_files_in_one_call() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    std::fs::write(&a, "a1\na2\n").unwrap();
    std::fs::write(&b, "b1\n").unwrap();
    let args = json!({ "path": "a.txt", "paths": ["a.txt", "b.txt", "gone.txt"] });
    let Ok(Action::Read { path, offset, limit, more }) = prepare(&args, dir.path()) else { panic!() };
    assert_eq!(more, vec![b.clone(), dir.path().join("gone.txt")], "the first path is not read twice");
    let out = run(&path, offset, limit, &more, &ctx(dir.path()));
    assert!(!out.is_error, "one missing file does not fail the others");
    assert!(out.model.contains("     2\ta2") && out.model.contains("     1\tb1") && out.model.contains("File not found"), "{}", out.model);
    // The budget is shared: past it, the rest is named instead of read
    std::fs::write(&a, format!("{}\n", "x".repeat(1999)).repeat(30)).unwrap();
    let out = run(&a, 1, 2000, std::slice::from_ref(&b), &ctx(dir.path()));
    assert!(out.model.contains("Not read, the output budget is used up") && out.model.contains("b.txt"), "{}", &out.model[out.model.len() - 300..]);
    let many: Vec<String> = (0..MAX_PATHS).map(|n| format!("f{n}")).collect();
    assert!(prepare(&json!({ "path": "a.txt", "paths": many }), dir.path()).is_err());
  }
}
