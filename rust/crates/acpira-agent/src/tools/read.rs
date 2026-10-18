//! `read`: a text file with line numbers, a window at a time. The header always states the total line count, so the
//! model knows whether it saw everything. `paths` adds more files, or more windows, to the same call: a model that
//! makes one call per reply (GPT-6.1 on every harness, 2026-10) otherwise spends a request per file. A path may carry a
//! line range the way grep prints a hit (`src/a.rs:120-180`, or `src/a.rs:120` from that line on), and the windows of
//! one call share the output budget evenly, so a large first file no longer crowds out the rest (before 2026-10-18 the
//! first file took what it needed and the others what was left: 17 of 63 multi-file reads in the local logs ran out)

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Action, Ctx, Output, num_arg, resolve, str_arg};
use crate::budget::MAX_BYTES;
use crate::llm::ToolSpec;

pub const DEFAULT_LIMIT: usize = 2000;
/// Windows one call may read
pub const MAX_PATHS: usize = 12;
/// A longer line is cut, with a marker
const MAX_LINE: usize = 2000;

/// One window of a file
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
  pub path: PathBuf,
  /// First line, 1-based
  pub offset: usize,
  pub limit: usize,
}

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::READ.into(),
    description: "Read a text file. Returns numbered lines (up to 2000 per call) and the file's total line count; use offset / limit for \
                  the next window. Relative paths resolve against the session folder. To look at several files or places, list them in \
                  paths, each whole or with a line range as grep prints it (src/a.rs:120-180, or src/a.rs:120 from that line on): one \
                  call reads them all and shares its output budget evenly between them."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "File path, absolute or relative to the session folder" },
        "offset": { "type": "integer", "description": "First line to read, 1-based (default 1)" },
        "limit": { "type": "integer", "description": "How many lines to read (default 2000)" },
        "paths": { "type": "array", "items": { "type": "string" }, "description": "More files to read in the same call: path, path:start-end or path:start" },
      },
      "required": ["path"],
    }),
  }
}

/// The first line of a range and its last, None for the rest of the file
type Range = (usize, Option<usize>);

/// A path as the model gave it, with the line range it carries: `file:N-M` is lines N to M, `file:N` from line N on
/// (also `file:N:`, copied from a grep hit). A file whose name really ends that way is taken as named
fn split_range(raw: &str, cwd: &Path) -> Result<(PathBuf, Option<Range>), String> {
  let whole = resolve(raw, cwd);
  let Some((file, range)) = raw.strip_suffix(':').unwrap_or(raw).rsplit_once(':').filter(|_| !whole.exists()) else { return Ok((whole, None)) };
  let (start, end) = match range.split_once('-') {
    Some((a, b)) => (a, Some(b)),
    None => (range, None),
  };
  let Ok(start) = start.trim().parse::<usize>() else { return Ok((whole, None)) };
  let end = match end.map(|e| e.trim().parse::<usize>()) {
    None => None,
    Some(Ok(e)) => Some(e),
    Some(Err(_)) => return Ok((whole, None)),
  };
  let start = start.max(1);
  if end.is_some_and(|e| e < start) {
    return Err(format!("{raw}: the line range ends before it starts"));
  }
  Ok((resolve(file, cwd), Some((start, end))))
}

/// A path argument as a window: `offset` / `limit` when the call gives either, else the range the path carries, else
/// the whole file. The bool says whether the window came from the path
fn window(raw: &str, cwd: &Path, offset: Option<usize>, limit: Option<usize>) -> Result<(Slice, bool), String> {
  let (path, range) = split_range(raw, cwd)?;
  if offset.is_some() || limit.is_some() {
    return Ok((Slice { path, offset: offset.unwrap_or(1).max(1), limit: limit.unwrap_or(DEFAULT_LIMIT) }, false));
  }
  Ok(match range {
    Some((start, end)) => (Slice { path, offset: start, limit: end.map_or(DEFAULT_LIMIT, |e| e + 1 - start) }, true),
    None => (Slice { path, offset: 1, limit: DEFAULT_LIMIT }, false),
  })
}

/// The `paths` strings; a lone string, or an array sent as a JSON string, is taken too
fn more_paths(args: &Value) -> Vec<String> {
  let parsed;
  let list = match args.get("paths") {
    Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
      Ok(v @ Value::Array(_)) => {
        parsed = v;
        &parsed
      }
      _ => return vec![s.clone()].into_iter().filter(|p| !p.trim().is_empty()).collect(),
    },
    Some(v) => v,
    None => return vec![],
  };
  list.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

pub fn prepare(args: &Value, cwd: &Path) -> Result<Action, String> {
  let first = str_arg(args, "path").or_else(|_| str_arg(args, "file_path"))?;
  let offset = num_arg(args, "offset").map(|o| (o as usize).max(1));
  let limit = num_arg(args, "limit").filter(|l| *l > 0).map(|l| l as usize);
  let mut files = vec![window(first, cwd, offset, limit)?.0];
  for (i, raw) in more_paths(args).iter().enumerate() {
    let (s, ranged) = window(raw, cwd, None, None).map_err(|e| format!("paths[{i}]: {e}"))?;
    // Models repeat the first path in the list; a range of the same file is another window
    if (!ranged && s.path == files[0].path) || files.contains(&s) {
      continue;
    }
    files.push(s);
  }
  if files.len() > MAX_PATHS {
    return Err(format!("One read takes at most {MAX_PATHS} files; split the rest into another call"));
  }
  Ok(Action::Read { files })
}

/// The files a read call's arguments name, ranges left out (compaction's file restore and the chunk index)
pub fn targets(args: &Value, cwd: &Path) -> Vec<PathBuf> {
  let first = args.get("path").or_else(|| args.get("file_path")).and_then(Value::as_str);
  first.map(str::to_owned).into_iter().chain(more_paths(args)).filter_map(|raw| split_range(&raw, cwd).ok().map(|(p, _)| p)).collect()
}

/// A file's window as numbered rows, before any byte budget; Err is what the model gets instead (an error, or the
/// note that the file is empty)
struct View {
  path: PathBuf,
  offset: usize,
  total: usize,
  rows: Vec<String>,
}

impl View {
  fn size(&self) -> usize {
    self.rows.iter().map(String::len).sum()
  }

  /// The header and the rows that fit in `max_bytes`, at least one
  fn render(&self, max_bytes: usize) -> String {
    let mut body = String::new();
    let mut last = self.offset - 1;
    for row in &self.rows {
      if body.len() + row.len() > max_bytes && last >= self.offset {
        break;
      }
      body.push_str(row);
      last += 1;
    }
    let (path, total, offset) = (self.path.display(), self.total, self.offset);
    let header = match (offset, last == total) {
      (1, true) => format!("{path} ({total} lines)\n"),
      (_, true) => format!("{path} (lines {offset}-{last} of {total})\n"),
      _ => format!("{path} (lines {offset}-{last} of {total}; continue with offset {})\n", last + 1),
    };
    header + &body
  }
}

/// The lines of a window, or what the call says in their place (boxed: an Output is large next to a View)
fn view(s: &Slice) -> Result<View, Box<Output>> {
  let path = &s.path;
  let meta = match std::fs::metadata(path) {
    Ok(m) => m,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Box::new(Output::error(format!("File not found: {}", path.display())))),
    Err(e) => return Err(Box::new(Output::error(format!("Cannot read {}: {e}", path.display())))),
  };
  if meta.is_dir() {
    return Err(Box::new(Output::error(format!("{} is a directory; list it with the list tool instead", path.display()))));
  }
  let bytes = match std::fs::read(path) {
    Ok(b) => b,
    Err(e) => return Err(Box::new(Output::error(format!("Cannot read {}: {e}", path.display())))),
  };
  if bytes.iter().take(8192).any(|b| *b == 0) {
    return Err(Box::new(Output::error(format!("{} is a binary file ({} bytes); it cannot be read as text", path.display(), bytes.len()))));
  }
  let text = String::from_utf8_lossy(&bytes);
  let total = text.lines().count();
  if total == 0 {
    return Err(Box::new(Output { model: format!("{} is empty.", path.display()), is_error: false, content: vec![], raw_output: None }));
  }
  if s.offset > total {
    return Err(Box::new(Output::error(format!("offset {} is past the end: {} has {total} lines", s.offset, path.display()))));
  }
  let rows = text
    .lines()
    .enumerate()
    .skip(s.offset - 1)
    .take(s.limit)
    .map(|(i, line)| {
      let line = if line.len() > MAX_LINE { format!("{}… [line cut at {MAX_LINE} bytes]", crate::budget::cut(line, MAX_LINE)) } else { line.to_owned() };
      format!("{:>6}\t{line}\n", i + 1)
    })
    .collect();
  Ok(View { path: path.clone(), offset: s.offset, total, rows })
}

/// The windows within one output budget, split evenly: a window under its share is read whole and leaves the rest to
/// the others. A file that cannot be read is reported in its place; only a single file that fails makes the call an
/// error
pub fn run(files: &[Slice], _ctx: &Ctx) -> Output {
  let mut views: Vec<Result<View, Box<Output>>> = files.iter().map(view).collect();
  if views.len() == 1 {
    return match views.remove(0) {
      Ok(v) => Output { model: v.render(MAX_BYTES), is_error: false, content: vec![], raw_output: None },
      Err(out) => *out,
    };
  }
  let sizes: Vec<usize> = views.iter().map(|v| v.as_ref().map_or_else(|o| o.model.len(), View::size)).collect();
  let parts: Vec<String> =
    views.iter().zip(shares(&sizes, MAX_BYTES)).map(|(v, share)| v.as_ref().map_or_else(|o| o.model.clone(), |v| v.render(share))).collect();
  Output { model: parts.join("\n\n"), is_error: false, content: vec![], raw_output: None }
}

/// `budget` split over requests of these sizes: from the smallest up, each gets what it asks while that is under an even
/// split of what is left, and the rest share the remainder
fn shares(sizes: &[usize], budget: usize) -> Vec<usize> {
  let mut order: Vec<usize> = (0..sizes.len()).collect();
  order.sort_by_key(|&i| sizes[i]);
  let mut out = vec![0; sizes.len()];
  let mut left = budget;
  for (k, &i) in order.iter().enumerate() {
    out[i] = sizes[i].min(left / (sizes.len() - k));
    left -= out[i];
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ctx(dir: &Path) -> Ctx {
    Ctx { cwd: dir.to_owned(), outputs: dir.join("outputs"), call_id: "c".into(), progress: std::sync::Arc::new(|_| {}), jobs: Default::default(), search: Default::default(), mcp: Default::default(), history: Default::default() }
  }

  #[test]
  fn windows_and_totals() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, (1..=10).map(|n| format!("l{n}\n")).collect::<String>()).unwrap();
    let read = |offset, limit| run(&[Slice { path: f.clone(), offset, limit }], &ctx(dir.path()));
    let all = read(1, 2000);
    assert!(all.model.contains("(10 lines)") && all.model.contains("    10\tl10"));
    let part = read(3, 2);
    assert!(part.model.contains("(lines 3-4 of 10; continue with offset 5)"), "{}", part.model);
    assert!(read(9, 5).model.contains("(lines 9-10 of 10)\n"), "a window that reaches the end offers no next one");
    assert!(read(11, 5).is_error);
    std::fs::write(&f, [0u8, 1, 2]).unwrap();
    assert!(read(1, 10).model.contains("binary"));
  }

  fn slices(args: Value, cwd: &Path) -> Vec<Slice> {
    let Ok(Action::Read { files }) = prepare(&args, cwd) else { panic!("{args}") };
    files
  }

  #[test]
  fn several_files_in_one_call() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    std::fs::write(&a, "a1\na2\n").unwrap();
    std::fs::write(&b, "b1\n").unwrap();
    let files = slices(json!({ "path": "a.txt", "paths": ["a.txt", "b.txt", "gone.txt"] }), dir.path());
    let paths: Vec<&Path> = files.iter().map(|s| s.path.as_path()).collect();
    assert_eq!(paths, [a.as_path(), b.as_path(), dir.path().join("gone.txt").as_path()], "the first path is not read twice");
    let out = run(&files, &ctx(dir.path()));
    assert!(!out.is_error, "one missing file does not fail the others");
    assert!(out.model.contains("     2\ta2") && out.model.contains("     1\tb1") && out.model.contains("File not found"), "{}", out.model);
    let many: Vec<String> = (0..MAX_PATHS).map(|n| format!("f{n}")).collect();
    assert!(prepare(&json!({ "path": "a.txt", "paths": many }), dir.path()).is_err());
    // A lone string, and an array sent as a JSON string
    assert_eq!(slices(json!({ "path": "a.txt", "paths": "b.txt" }), dir.path()).len(), 2);
    assert_eq!(slices(json!({ "path": "a.txt", "paths": "[\"b.txt\"]" }), dir.path()).len(), 2);
  }

  #[test]
  fn paths_take_line_ranges_as_grep_prints_them() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.rs");
    std::fs::write(&f, (1..=300).map(|n| format!("line {n}\n")).collect::<String>()).unwrap();
    assert_eq!(slices(json!({ "path": "a.rs:7:" }), dir.path()), [Slice { path: f.clone(), offset: 7, limit: DEFAULT_LIMIT }]);
    let files = slices(json!({ "path": "a.rs:120-122", "paths": ["a.rs:200", "a.rs:120-122", "a.rs"] }), dir.path());
    assert_eq!(
      files,
      [Slice { path: f.clone(), offset: 120, limit: 3 }, Slice { path: f.clone(), offset: 200, limit: DEFAULT_LIMIT }],
      "a repeated window and a bare repeat of the first file are dropped"
    );
    let out = run(&files, &ctx(dir.path()));
    assert!(out.model.contains("(lines 120-122 of 300; continue with offset 123)\n   120\tline 120\n   121\tline 121\n   122\tline 122\n"), "{}", out.model);
    assert!(out.model.contains("(lines 200-300 of 300)\n   200\tline 200"), "{}", out.model);
    // offset / limit given with the call win over the range in the path
    assert_eq!(slices(json!({ "path": "a.rs:120-122", "offset": 5, "limit": 2 }), dir.path()), [Slice { path: f.clone(), offset: 5, limit: 2 }]);
    // A file really named that way is read as named; a backwards range is refused
    std::fs::write(dir.path().join("odd:7"), "x\n").unwrap();
    assert_eq!(slices(json!({ "path": "odd:7" }), dir.path())[0].path, dir.path().join("odd:7"));
    assert!(prepare(&json!({ "path": "a.rs", "paths": ["a.rs:20-10"] }), dir.path()).unwrap_err().contains("paths[0]: a.rs:20-10"));
    assert_eq!(targets(&json!({ "path": "a.rs:1-2", "paths": ["b.rs:9", "c.rs"] }), dir.path()), [f, dir.path().join("b.rs"), dir.path().join("c.rs")]);
  }

  #[test]
  fn the_budget_is_split_evenly_between_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let (big, other, small) = (dir.path().join("big.txt"), dir.path().join("other.txt"), dir.path().join("small.txt"));
    // Each about 60 KB, more than the whole budget
    std::fs::write(&big, format!("{}\n", "x".repeat(1999)).repeat(30)).unwrap();
    std::fs::write(&other, format!("{}\n", "y".repeat(1999)).repeat(30)).unwrap();
    std::fs::write(&small, "tiny\n").unwrap();
    let files = slices(json!({ "path": "big.txt", "paths": ["other.txt", "small.txt"] }), dir.path());
    let out = run(&files, &ctx(dir.path()));
    assert!(out.model.contains("     1\ttiny"), "the small file is read whole");
    let rows = |c: &str| out.model.lines().filter(|l| l.contains(&format!("\t{c}{c}{c}"))).count();
    assert_eq!((rows("x"), rows("y")), (12, 12), "the two large files get the same share");
    assert!(out.model.len() < MAX_BYTES + 1024, "{}", out.model.len());
    assert_eq!(shares(&[100, 100_000, 100_000], 51_200), [100, 25_550, 25_550]);
    assert_eq!(shares(&[10, 20], 100), [10, 20]);
  }
}
