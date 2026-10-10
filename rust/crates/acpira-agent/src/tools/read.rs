//! `read`: a text file with line numbers, a window at a time. The header always states the total line count, so the
//! model knows whether it saw everything

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Action, Ctx, Output, num_arg, resolve, str_arg};
use crate::budget::MAX_BYTES;
use crate::llm::ToolSpec;

pub const DEFAULT_LIMIT: usize = 2000;
/// A longer line is cut, with a marker
const MAX_LINE: usize = 2000;

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::READ.into(),
    description: "Read a text file. Returns numbered lines (up to 2000 per call) and the file's total line count; use offset / limit for \
                  the next window. Relative paths resolve against the session folder."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "File path, absolute or relative to the session folder" },
        "offset": { "type": "integer", "description": "First line to read, 1-based (default 1)" },
        "limit": { "type": "integer", "description": "How many lines to read (default 2000)" },
      },
      "required": ["path"],
    }),
  }
}

pub fn prepare(args: &Value, cwd: &Path) -> Result<Action, String> {
  let path = resolve(str_arg(args, "path").or_else(|_| str_arg(args, "file_path"))?, cwd);
  let offset = num_arg(args, "offset").unwrap_or(1).max(1) as usize;
  let limit = num_arg(args, "limit").filter(|l| *l > 0).unwrap_or(DEFAULT_LIMIT as u64) as usize;
  Ok(Action::Read { path, offset, limit })
}

pub fn run(path: &PathBuf, offset: usize, limit: usize, _ctx: &Ctx) -> Output {
  let meta = match std::fs::metadata(path) {
    Ok(m) => m,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Output::error(format!("File not found: {}", path.display())),
    Err(e) => return Output::error(format!("Cannot read {}: {e}", path.display())),
  };
  if meta.is_dir() {
    return Output::error(format!("{} is a directory; list it with bash (ls) instead", path.display()));
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
    if body.len() + row.len() > MAX_BYTES && last >= offset {
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
    Ctx { cwd: dir.to_owned(), outputs: dir.join("outputs"), call_id: "c".into(), progress: Box::new(|_| {}) }
  }

  #[test]
  fn windows_and_totals() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, (1..=10).map(|n| format!("l{n}\n")).collect::<String>()).unwrap();
    let all = run(&f, 1, 2000, &ctx(dir.path()));
    assert!(all.model.contains("(10 lines)") && all.model.contains("    10\tl10"));
    let part = run(&f, 3, 2, &ctx(dir.path()));
    assert!(part.model.contains("(lines 3-4 of 10; continue with offset 5)"), "{}", part.model);
    assert!(run(&f, 11, 5, &ctx(dir.path())).is_error);
    std::fs::write(&f, [0u8, 1, 2]).unwrap();
    assert!(run(&f, 1, 10, &ctx(dir.path())).model.contains("binary"));
  }
}
