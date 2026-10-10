//! `grep`, `glob` and `list`: read-only views of the workspace over the file set of `files.rs`. Each caps its output
//! and says so, so the model narrows the search instead of reading a silent cut

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use regex::RegexBuilder;
use serde_json::{Value, json};

use super::files::{self, Glob};
use super::{Action, Ctx, Output, budgeted, resolve, shown, str_arg};
use crate::budget::Keep;
use crate::llm::ToolSpec;

/// Files the search tools consider at most
const MAX_FILES: usize = 50_000;
const MAX_MATCHES: usize = 100;
const MAX_GLOB: usize = 100;
const MAX_LIST: usize = 500;
/// A grep match line longer than this is cut
const MAX_LINE: usize = 400;
/// Larger files are not searched
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

pub fn grep_spec() -> ToolSpec {
  ToolSpec {
    name: super::GREP.into(),
    description: "Search file contents with a regular expression (Rust regex syntax). Returns `path:line: text` for up to 100 matches, \
                  skipping ignored and binary files."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "pattern": { "type": "string", "description": "Regular expression; prefix (?i) for case-insensitive" },
        "path": { "type": "string", "description": "File or folder to search (default: the session folder)" },
        "include": { "type": "string", "description": "Only files matching this glob, e.g. *.rs or src/**/*.{ts,tsx}" },
      },
      "required": ["pattern"],
    }),
  }
}

pub fn glob_spec() -> ToolSpec {
  ToolSpec {
    name: super::GLOB.into(),
    description: "Find files by name with a glob pattern (** crosses folders; a pattern without / matches the file name at any depth). \
                  Returns up to 100 paths, ignored files left out."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "pattern": { "type": "string", "description": "Glob, e.g. **/*.test.ts or Cargo.toml" },
        "path": { "type": "string", "description": "Folder to search (default: the session folder)" },
      },
      "required": ["pattern"],
    }),
  }
}

pub fn list_spec() -> ToolSpec {
  ToolSpec {
    name: super::LIST.into(),
    description: "List the files under a folder as an indented tree (up to 500 files, ignored files left out).".into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "Folder to list; \".\" for the session folder" },
        "ignore": { "type": "array", "items": { "type": "string" }, "description": "Globs to leave out" },
      },
      // Required although a call without it lists the session folder: a Claude gateway route ends the stream at a tool
      // call with empty input (2026-10-11), and an optional-only schema invites exactly that call
      "required": ["path"],
    }),
  }
}

fn root_arg(args: &Value, cwd: &Path) -> PathBuf {
  match args.get("path").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) {
    Some(p) => resolve(p, cwd),
    None => cwd.to_path_buf(),
  }
}

pub fn prepare_grep(args: &Value, cwd: &Path) -> Result<Action, String> {
  let pattern = str_arg(args, "pattern")?.to_owned();
  RegexBuilder::new(&pattern).size_limit(1 << 22).build().map_err(|e| format!("Invalid regular expression: {e}"))?;
  let include = args.get("include").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
  if let Some(g) = &include {
    Glob::new(g)?;
  }
  Ok(Action::Grep { pattern, path: root_arg(args, cwd), include })
}

pub fn prepare_glob(args: &Value, cwd: &Path) -> Result<Action, String> {
  let pattern = str_arg(args, "pattern")?.to_owned();
  Glob::new(&pattern)?;
  Ok(Action::Glob { pattern, path: root_arg(args, cwd) })
}

pub fn prepare_list(args: &Value, cwd: &Path) -> Result<Action, String> {
  let ignore: Vec<String> = match args.get("ignore") {
    Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_owned).collect(),
    Some(Value::String(s)) => vec![s.clone()],
    _ => vec![],
  };
  for g in &ignore {
    Glob::new(g)?;
  }
  Ok(Action::List { path: root_arg(args, cwd), ignore })
}

fn missing(path: &Path) -> Option<Output> {
  (!path.exists()).then(|| Output::error(format!("Not found: {}", path.display())))
}

pub fn grep(pattern: &str, path: &Path, include: Option<&str>, ctx: &Ctx) -> Output {
  if let Some(e) = missing(path) {
    return e;
  }
  let re = match RegexBuilder::new(pattern).size_limit(1 << 22).build() {
    Ok(r) => r,
    Err(e) => return Output::error(format!("Invalid regular expression: {e}")),
  };
  let include = include.map(|g| Glob::new(g).expect("checked in prepare"));
  // A single file is searched as itself; its shown path is relative to the session folder
  let (base, files, cut) = if path.is_file() {
    (path.parent().unwrap_or(path).to_path_buf(), vec![path.file_name().unwrap_or_default().to_string_lossy().into_owned()], false)
  } else {
    let (f, cut) = files::list(path, MAX_FILES);
    (path.to_path_buf(), f, cut)
  };
  let mut out = String::new();
  let (mut count, mut in_files, mut more) = (0usize, 0usize, false);
  'files: for rel in &files {
    if include.as_ref().is_some_and(|g| !g.matches(rel)) {
      continue;
    }
    let abs = base.join(rel);
    if std::fs::metadata(&abs).map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
      continue;
    }
    let Ok(bytes) = std::fs::read(&abs) else { continue };
    if bytes.iter().take(8192).any(|b| *b == 0) {
      continue;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut hit = false;
    for (n, line) in text.lines().enumerate() {
      if !re.is_match(line) {
        continue;
      }
      if count == MAX_MATCHES {
        more = true;
        break 'files;
      }
      hit = true;
      count += 1;
      let _ = writeln!(out, "{}:{}: {}", shown(&abs, &ctx.cwd), n + 1, crate::budget::cut(line.trim_end(), MAX_LINE));
    }
    in_files += usize::from(hit);
  }
  if count == 0 {
    return Output { model: "No matches.".into(), is_error: false, content: vec![], raw_output: Some(json!({ "matches": 0 })) };
  }
  let mut model = format!("{count} matches in {in_files} files");
  if more {
    model.push_str(&format!(" (stopped at {MAX_MATCHES}; narrow the pattern, path or include to see the rest)"));
  }
  if cut {
    model.push_str(&format!(" (only the first {MAX_FILES} files were searched)"));
  }
  model.push_str(":\n");
  model.push_str(&out);
  Output { model: budgeted(&model, Keep::Head, ctx), is_error: false, content: vec![], raw_output: Some(json!({ "matches": count })) }
}

pub fn glob(pattern: &str, path: &Path, ctx: &Ctx) -> Output {
  if let Some(e) = missing(path) {
    return e;
  }
  let g = Glob::new(pattern).expect("checked in prepare");
  let (files, _) = files::list(path, MAX_FILES);
  let mut hits: Vec<(std::time::SystemTime, String)> = files
    .iter()
    .filter(|f| g.matches(f))
    .map(|f| {
      let abs = path.join(f);
      (std::fs::metadata(&abs).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), shown(&abs, &ctx.cwd))
    })
    .collect();
  if hits.is_empty() {
    return Output { model: "No files found.".into(), is_error: false, content: vec![], raw_output: Some(json!({ "files": 0 })) };
  }
  // Recently changed first: that is usually the file being worked on
  hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
  let total = hits.len();
  let mut model: String = hits.iter().take(MAX_GLOB).map(|(_, p)| format!("{p}\n")).collect();
  if total > MAX_GLOB {
    model.push_str(&format!("({} more not shown; use a narrower pattern)\n", total - MAX_GLOB));
  }
  Output { model, is_error: false, content: vec![], raw_output: Some(json!({ "files": total })) }
}

pub fn list(path: &Path, ignore: &[String], ctx: &Ctx) -> Output {
  if let Some(e) = missing(path) {
    return e;
  }
  if !path.is_dir() {
    return Output::error(format!("{} is a file; read it with the read tool", path.display()));
  }
  let ignore: Vec<Glob> = ignore.iter().filter_map(|g| Glob::new(g).ok()).collect();
  let (files, _) = files::list(path, MAX_FILES);
  let files: Vec<&String> = files.iter().filter(|f| !ignore.iter().any(|g| g.matches(f))).collect();
  let mut model = format!("{}/\n", shown(path, &ctx.cwd).trim_end_matches('/'));
  // Print each folder the first time a file under it comes up; `files` is sorted, so siblings stay together
  let mut open: Vec<&str> = vec![];
  for f in files.iter().take(MAX_LIST) {
    let parts: Vec<&str> = f.split('/').collect();
    let dirs = &parts[..parts.len() - 1];
    let same = open.iter().zip(dirs).take_while(|(a, b)| a == b).count();
    open.truncate(same);
    for d in &dirs[same..] {
      let _ = writeln!(model, "{}{d}/", "  ".repeat(open.len() + 1));
      open.push(d);
    }
    let _ = writeln!(model, "{}{}", "  ".repeat(dirs.len() + 1), parts[parts.len() - 1]);
  }
  if files.len() > MAX_LIST {
    let _ = writeln!(model, "({} more files not shown; list a subfolder)", files.len() - MAX_LIST);
  }
  if files.is_empty() {
    model.push_str("  (empty)\n");
  }
  Output { model, is_error: false, content: vec![], raw_output: Some(json!({ "files": files.len() })) }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ctx(cwd: &Path) -> Ctx {
    Ctx { cwd: cwd.to_path_buf(), outputs: cwd.join(".out"), call_id: "c".into(), progress: Box::new(|_| {}) }
  }

  fn tree(root: &Path) {
    for (p, body) in [("src/a.rs", "fn main() {\n    let todo = 1;\n}\n"), ("src/util/b.rs", "// TODO later\n"), ("README.md", "todo list\n"), ("bin.dat", "\0\0todo")] {
      std::fs::create_dir_all(root.join(p).parent().unwrap()).unwrap();
      std::fs::write(root.join(p), body).unwrap();
    }
  }

  #[test]
  fn grep_finds_lines_with_paths_and_skips_binaries() {
    let dir = tempfile::tempdir().unwrap();
    tree(dir.path());
    let c = ctx(dir.path());
    let out = grep("(?i)todo", dir.path(), None, &c);
    assert!(out.model.starts_with("3 matches in 3 files"), "{}", out.model);
    assert!(out.model.contains("src/a.rs:2:     let todo = 1;") && !out.model.contains("bin.dat"), "{}", out.model);
    let only_rs = grep("todo", dir.path(), Some("*.rs"), &c);
    assert!(only_rs.model.starts_with("1 matches"), "{}", only_rs.model);
    assert_eq!(grep("nothing-here", dir.path(), None, &c).model, "No matches.");
    let file = grep("TODO", &dir.path().join("src/util/b.rs"), None, &c);
    assert!(file.model.contains("src/util/b.rs:1:"), "{}", file.model);
    assert!(prepare_grep(&json!({ "pattern": "(" }), dir.path()).is_err());
  }

  #[test]
  fn glob_and_list() {
    let dir = tempfile::tempdir().unwrap();
    tree(dir.path());
    let c = ctx(dir.path());
    let g = glob("**/*.rs", dir.path(), &c).model;
    assert_eq!(g.lines().count(), 2, "{g}");
    assert!(g.contains("src/util/b.rs"));
    let l = list(dir.path(), &["*.dat".into()], &c).model;
    assert!(l.contains("  src/\n    a.rs\n    util/\n      b.rs\n") && l.contains("  README.md") && !l.contains("bin.dat"), "{l}");
  }
}
