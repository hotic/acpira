//! `write` (a whole file) and `edit` (replace exact snippets: one, or several applied in order, also across files,
//! which saves a model round trip per extra change). The edit is computed while preparing, so the permission card shows
//! the real result; at run time it is re-applied to whatever the file holds then. Matching tolerates the two slips models
//! make most: LF written for a CRLF file, and trailing whitespace.
//!
//! Both take `then_run`, a command run once the change is made, on a command row of its own (`turn.rs`): the check
//! after an edit was a request of its own in 14% of the steps of the 2026-10-15 eval runs (SoL-Pi's Action Fusion)

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{Action, Ctx, Output, diff_content, resolve, shown, str_arg};
use crate::llm::ToolSpec;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edit {
  pub old: String,
  pub new: String,
  pub all: bool,
}

/// One file a call changes: `before` None for a new file, `edits` empty for a whole-file write
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Change {
  pub path: PathBuf,
  pub before: Option<String>,
  pub after: String,
  pub edits: Vec<Edit>,
  /// Set where the change was prepared on an SSH host: the host's own guard (`permission::Guard`) found the file
  /// protected, which a guard here cannot see through the host's symlinks
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub protected: bool,
}

impl Change {
  /// The file ends up as it was
  pub fn unchanged(&self) -> bool {
    self.before.as_deref() == Some(self.after.as_str())
  }
}

fn then_run_property() -> Value {
  json!({
    "type": "string",
    "description": "A command to run right after the change, such as the tests, a build or a linter; its output comes back with this result. Not run when the change fails or leaves the file as it was",
  })
}

/// The command a write or an edit asks to run after the change
pub fn then_run(args: &Value) -> Option<String> {
  args.get("then_run").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty()).map(str::to_owned)
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
        "then_run": then_run_property(),
      },
      "required": ["path", "content"],
    }),
  }
}

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::EDIT.into(),
    description: "Replace an exact snippet of an existing file. old_string must match the file text exactly (copy it from read output, \
                  without the line-number prefix) and be unique, unless replace_all is set. For several changes, give edits instead: \
                  they apply in order, each to the result of the previous one in its file, and all or none are made. An item may name \
                  its own path, so one call can change several files."
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
          "description": "Several replacements, instead of old_string / new_string",
          "items": {
            "type": "object",
            "properties": {
              "path": { "type": "string", "description": "The file of this replacement, when it is not path" },
              "old_string": { "type": "string" },
              "new_string": { "type": "string" },
              "replace_all": { "type": "boolean" },
            },
            "required": ["old_string", "new_string"],
          },
        },
        "then_run": then_run_property(),
      },
      "required": ["path"],
    }),
  }
}

fn path_arg(args: &Value, cwd: &Path) -> Result<PathBuf, String> {
  Ok(resolve(str_arg(args, "path").or_else(|_| str_arg(args, "file_path"))?, cwd))
}

/// An edits item's own file, when it names one
fn item_path(v: &Value, cwd: &Path) -> Option<PathBuf> {
  v.get("path").or_else(|| v.get("file_path")).and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()).map(|p| resolve(p, cwd))
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
  Ok(Action::Write { files: vec![Change { path, before, after, edits: vec![], protected: false }] })
}

fn edit_of(v: &Value) -> Result<Edit, String> {
  Ok(Edit {
    old: str_arg(v, "old_string")?.to_owned(),
    new: str_arg(v, "new_string")?.to_owned(),
    all: v.get("replace_all").and_then(Value::as_bool).unwrap_or(false),
  })
}

/// The files an edit call's arguments name: `path`, then the items' own paths
pub fn targets(args: &Value, cwd: &Path) -> Vec<PathBuf> {
  let mut out: Vec<PathBuf> = path_arg(args, cwd).into_iter().collect();
  for p in args.get("edits").and_then(Value::as_array).into_iter().flatten().filter_map(|v| item_path(v, cwd)) {
    if !out.contains(&p) {
      out.push(p);
    }
  }
  out
}

pub fn prepare(args: &Value, cwd: &Path) -> Result<Action, String> {
  let default = path_arg(args, cwd)?;
  let items: Vec<(PathBuf, Edit)> = match args.get("edits").and_then(Value::as_array).filter(|a| !a.is_empty()) {
    Some(list) => list
      .iter()
      .enumerate()
      .map(|(i, v)| edit_of(v).map(|e| (item_path(v, cwd).unwrap_or_else(|| default.clone()), e)).map_err(|err| format!("edits[{i}]: {err}")))
      .collect::<Result<_, _>>()?,
    None => vec![(default, edit_of(args)?)],
  };
  let several = items.len() > 1;
  // One change per file, in the order the files first come up; a file's edits apply in the order given. Files are told
  // apart by what they resolve to, so `a.txt` and `nested/../a.txt` (or a symlink) share one change: two changes to one
  // file would each be computed from the original and the later write would drop the earlier one's edits
  let mut files: Vec<Change> = vec![];
  let mut ids: Vec<PathBuf> = vec![];
  for (i, (path, e)) in items.into_iter().enumerate() {
    let id = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
    let k = match ids.iter().position(|x| *x == id) {
      Some(k) => k,
      None => {
        let before = read_text(&path)?.ok_or_else(|| format!("File not found: {} (use write to create it)", path.display()))?;
        files.push(Change { path, after: before.clone(), before: Some(before), edits: vec![], protected: false });
        ids.push(id);
        files.len() - 1
      }
    };
    let c = &mut files[k];
    c.after = apply(&c.after, &e).map_err(|err| if several { format!("edits[{i}]: {err} in {}", c.path.display()) } else { format!("{err} in {}", c.path.display()) })?;
    c.edits.push(e);
  }
  Ok(Action::Write { files })
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

/// Write every change. A file that changed while the card waited gets its edits again, applied to what it holds now;
/// nothing is written unless every file still takes them, and a write that fails puts back the files written before it
pub fn run(files: Vec<Change>, ctx: &Ctx) -> Output {
  let mut ready: Vec<Change> = vec![];
  for c in files {
    let current = match read_text(&c.path) {
      Ok(x) => x,
      Err(e) => return Output::error(e),
    };
    let edited = !c.edits.is_empty();
    let (before, after) = match current {
      Some(now) if edited && Some(&now) != c.before.as_ref() => match apply_all(&now, &c.edits) {
        Ok(a) => (Some(now), a),
        Err(err) => return Output::error(format!("The file {} changed since the edit was prepared and the edit no longer applies: {err}", shown(&c.path, &ctx.cwd))),
      },
      None if edited => return Output::error(format!("{} was deleted before the edit ran", c.path.display())),
      now => (now.or(c.before), c.after),
    };
    ready.push(Change { path: c.path, before, after, edits: c.edits, protected: false });
  }
  for (k, c) in ready.iter().enumerate() {
    let failed = match c.path.parent().map(std::fs::create_dir_all) {
      Some(Err(e)) => Some(format!("Cannot create {}: {e}", c.path.parent().unwrap_or(&c.path).display())),
      _ => std::fs::write(&c.path, &c.after).err().map(|e| format!("Cannot write {}: {e}", c.path.display())),
    };
    if let Some(message) = failed {
      return undo(&ready[..k], message, ctx);
    }
  }
  let model = match &ready[..] {
    [c] => {
      let name = shown(&c.path, &ctx.cwd);
      match (c.edits.len(), &c.before) {
        (0, Some(_)) => format!("Replaced {name} ({} lines).", c.after.lines().count()),
        (0, None) => format!("Created {name} ({} lines).", c.after.lines().count()),
        (1, _) => format!("Edited {name}."),
        (n, _) => format!("Edited {name} ({n} changes)."),
      }
    }
    several => {
      let names: Vec<String> = several
        .iter()
        .map(|c| match c.edits.len() {
          n if n > 1 => format!("{} ({n} changes)", shown(&c.path, &ctx.cwd)),
          _ => shown(&c.path, &ctx.cwd),
        })
        .collect();
      format!("Edited {} files: {}.", several.len(), names.join(", "))
    }
  };
  let content = ready.iter().map(|c| diff_content(&c.path, c.before.as_deref(), &c.after)).collect();
  Output { model, is_error: false, content, raw_output: None }
}

/// Put back files a failed call already wrote, newest first. Any that cannot be restored are named, and their diffs
/// stay on the card, so the failure never hides a change that is still on disk
fn undo(written: &[Change], mut message: String, ctx: &Ctx) -> Output {
  let stuck: Vec<&Change> = written
    .iter()
    .rev()
    .filter(|c| match &c.before {
      Some(b) => std::fs::write(&c.path, b).is_err(),
      None => std::fs::remove_file(&c.path).is_err(),
    })
    .collect();
  if stuck.is_empty() {
    if !written.is_empty() {
      message.push_str("; no file was changed (the ones already written were restored)");
    }
    return Output::error(message);
  }
  let names: Vec<String> = stuck.iter().rev().map(|c| shown(&c.path, &ctx.cwd)).collect();
  message.push_str(&format!("; could not restore, still changed: {}", names.join(", ")));
  let mut out = Output::error(message);
  out.content.extend(stuck.iter().rev().map(|c| diff_content(&c.path, c.before.as_deref(), &c.after)));
  out
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

  fn changes(args: Value, cwd: &Path) -> Vec<Change> {
    let Action::Write { files } = prepare(&args, cwd).unwrap() else { panic!() };
    files
  }

  fn ctx(dir: &Path) -> Ctx {
    Ctx { cwd: dir.to_owned(), outputs: dir.join("o"), call_id: "c".into(), progress: std::sync::Arc::new(|_| {}), jobs: Default::default(), search: Default::default(), mcp: Default::default(), history: Default::default() }
  }

  #[test]
  fn several_edits_apply_in_order_or_not_at_all() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("x.txt"), "a b c\n").unwrap();
    let args = json!({ "path": "x.txt", "edits": [{ "old_string": "a", "new_string": "x" }, { "old_string": "x b", "new_string": "y" }] });
    let [c] = &changes(args, dir.path())[..] else { panic!() };
    assert_eq!((c.after.as_str(), c.edits.len()), ("y c\n", 2));
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
    let files = changes(json!({ "path": "x.txt", "old_string": "beta", "new_string": "gamma" }), dir.path());
    std::fs::write(&f, "first line\nalpha beta\n").unwrap();
    let out = run(files, &ctx(dir.path()));
    assert!(!out.is_error, "{}", out.model);
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "first line\nalpha gamma\n");
  }

  #[test]
  fn one_call_edits_several_files_all_or_none() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.rs"), dir.path().join("src/b.rs"));
    std::fs::create_dir_all(b.parent().unwrap()).unwrap();
    std::fs::write(&a, "fn old() {}\nold();\n").unwrap();
    std::fs::write(&b, "use old;\n").unwrap();
    let args = json!({ "path": "a.rs", "edits": [
      { "old_string": "fn old()", "new_string": "fn new()" },
      { "path": "src/b.rs", "old_string": "use old;", "new_string": "use new;" },
      { "path": "./a.rs", "old_string": "old();", "new_string": "new();" },
    ] });
    let files = changes(args.clone(), dir.path());
    assert_eq!(files.iter().map(|c| (c.path.clone(), c.edits.len())).collect::<Vec<_>>(), [(a.clone(), 2), (b.clone(), 1)], "one change per file, the same file under another spelling included");
    assert_eq!(targets(&args, dir.path()), [a.clone(), b.clone()]);
    let out = run(files, &ctx(dir.path()));
    assert_eq!(out.model, "Edited 2 files: a.rs (2 changes), src/b.rs.");
    assert_eq!(out.content.len(), 2, "a diff per file");
    assert_eq!((std::fs::read_to_string(&a).unwrap(), std::fs::read_to_string(&b).unwrap()), ("fn new() {}\nnew();\n".into(), "use new;\n".into()));
    // One item that does not apply leaves every file alone
    let bad = json!({ "path": "a.rs", "edits": [{ "old_string": "fn new()", "new_string": "fn x()" }, { "path": "src/b.rs", "old_string": "nope", "new_string": "y" }] });
    let e = prepare(&bad, dir.path()).unwrap_err();
    assert!(e.starts_with("edits[1]: old_string not found") && e.ends_with("b.rs"), "{e}");
    assert!(prepare(&json!({ "path": "a.rs", "edits": [{ "path": "gone.rs", "old_string": "a", "new_string": "b" }] }), dir.path()).unwrap_err().starts_with("File not found"));
    // At run time too: the second file changed meanwhile and no longer takes its edit
    let files = changes(json!({ "path": "a.rs", "edits": [{ "old_string": "fn new()", "new_string": "fn x()" }, { "path": "src/b.rs", "old_string": "use new;", "new_string": "use x;" }] }), dir.path());
    std::fs::write(&b, "rewritten\n").unwrap();
    let out = run(files, &ctx(dir.path()));
    assert!(out.is_error && out.model.contains("src/b.rs changed since the edit was prepared"), "{}", out.model);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "fn new() {}\nnew();\n", "nothing was written");
    assert_eq!(then_run(&json!({ "then_run": "  cargo test " })).as_deref(), Some("cargo test"));
    assert_eq!(then_run(&json!({ "then_run": " " })), None);
  }

  #[test]
  fn one_file_under_two_spellings_takes_both_edits() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("nested")).unwrap();
    let a = dir.path().join("a.txt");
    std::fs::write(&a, "alpha beta\n").unwrap();
    let args = json!({ "path": "a.txt", "edits": [
      { "old_string": "alpha", "new_string": "ALPHA" },
      { "path": "nested/../a.txt", "old_string": "beta", "new_string": "BETA" },
    ] });
    let files = changes(args, dir.path());
    assert_eq!(files.len(), 1, "one change for the file");
    assert!(!run(files, &ctx(dir.path())).is_error);
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "ALPHA BETA\n");
  }

  #[cfg(unix)]
  #[test]
  fn a_failed_write_restores_the_files_written_before_it() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    std::fs::write(&a, "one\n").unwrap();
    std::fs::write(&b, "two\n").unwrap();
    let files = changes(json!({ "path": "a.txt", "edits": [
      { "old_string": "one", "new_string": "ONE" },
      { "path": "b.txt", "old_string": "two", "new_string": "TWO" },
    ] }), dir.path());
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o444)).unwrap();
    let out = run(files, &ctx(dir.path()));
    std::fs::set_permissions(&b, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(out.is_error && out.model.contains("Cannot write") && out.model.contains("no file was changed"), "{}", out.model);
    assert_eq!((std::fs::read_to_string(&a).unwrap(), std::fs::read_to_string(&b).unwrap()), ("one\n".into(), "two\n".into()));
  }
}
