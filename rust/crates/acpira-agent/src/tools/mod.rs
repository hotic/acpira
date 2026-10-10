//! The tools the model can call. Each call goes through three steps: `prepare` validates the arguments and works out
//! exactly what will happen (an edit's resulting file, so the permission card shows the real diff), `describe` gives
//! its ACP presentation, `run` does it. Names stay short and unprefixed: weaker models copy them more reliably

pub mod bash;
pub mod edit;
pub mod files;
pub mod names;
pub mod read;
pub mod search;
pub mod todo;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::budget::{self, Keep};
use crate::llm::ToolSpec;

/// Everything a running tool may use
pub struct Ctx {
  pub cwd: PathBuf,
  /// Where outputs over the budget are saved
  pub outputs: PathBuf,
  /// The ACP tool call id (names spill files)
  pub call_id: String,
  /// Streams a partial `tool_call_update` (terminal output) while the tool runs
  pub progress: Box<dyn Fn(Value) + Send + Sync>,
}

/// A validated call, ready to describe and run
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
  Read { path: PathBuf, offset: usize, limit: usize },
  /// The whole file replaced (`write`) or a computed edit (`edit`): `before` None for a new file
  Write { path: PathBuf, before: Option<String>, after: String, edit: Option<edit::Edit> },
  Bash { command: String, workdir: PathBuf, timeout_ms: u64 },
  Grep { pattern: String, path: PathBuf, include: Option<String> },
  Glob { pattern: String, path: PathBuf },
  List { path: PathBuf, ignore: Vec<String> },
  Todo { todos: Vec<todo::Todo> },
}

/// How a call reads on the tool card and in the permission request
pub struct Presentation {
  pub title: String,
  /// ACP ToolKind
  pub kind: &'static str,
  pub locations: Vec<PathBuf>,
  /// Shown before the run (an edit's diff)
  pub content: Vec<Value>,
}

/// What a finished call gives back
pub struct Output {
  /// The model's copy, already within the budget
  pub model: String,
  pub is_error: bool,
  /// ACP content for the tool card
  pub content: Vec<Value>,
  pub raw_output: Option<Value>,
}

impl Output {
  pub fn error(message: impl Into<String>) -> Output {
    let message = message.into();
    Output { content: vec![text_content(&message)], model: message, is_error: true, raw_output: None }
  }
}

pub fn text_content(text: &str) -> Value {
  json!({ "type": "content", "content": { "type": "text", "text": text } })
}

pub fn diff_content(path: &Path, before: Option<&str>, after: &str) -> Value {
  json!({ "type": "diff", "path": path, "oldText": before, "newText": after })
}

pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const EDIT: &str = "edit";
pub const BASH: &str = "bash";
pub const GREP: &str = "grep";
pub const GLOB: &str = "glob";
pub const LIST: &str = "list";
pub const TODO: &str = "todo";

/// The tool set of a turn, in `names::ALL` order
pub fn specs() -> Vec<ToolSpec> {
  vec![read::spec(), edit::write_spec(), edit::spec(), bash::spec(), search::grep_spec(), search::glob_spec(), search::list_spec(), todo::spec()]
}

/// Validate a call of a tool by its real name (`names::resolve` first); the error goes back to the model as the result
pub fn prepare(name: &str, args: &Value, cwd: &Path) -> Result<Action, String> {
  match name {
    READ => read::prepare(args, cwd),
    WRITE => edit::prepare_write(args, cwd),
    EDIT => edit::prepare(args, cwd),
    BASH => bash::prepare(args, cwd),
    GREP => search::prepare_grep(args, cwd),
    GLOB => search::prepare_glob(args, cwd),
    LIST => search::prepare_list(args, cwd),
    TODO => todo::prepare(args),
    other => Err(format!("Unknown tool \"{other}\". Available tools: {}.", names::ALL.join(", "))),
  }
}

impl Action {
  /// Changes nothing on disk: such calls run side by side
  pub fn read_only(&self) -> bool {
    !matches!(self, Action::Write { .. } | Action::Bash { .. })
  }

  /// The permission key and the target the rules match (`permission.rs`)
  pub fn permission(&self, cwd: &Path) -> (&'static str, String) {
    use crate::permission::{self as perm, path_target};
    match self {
      Action::Read { path, .. } | Action::Grep { path, .. } | Action::Glob { path, .. } | Action::List { path, .. } => (perm::READ, path_target(path, cwd)),
      Action::Write { path, .. } => (perm::EDIT, path_target(path, cwd)),
      Action::Bash { command, .. } => (perm::BASH, command.clone()),
      Action::Todo { .. } => (perm::TODO, "*".to_owned()),
    }
  }

  /// The target an "always allow" answer covers for the rest of the session, with its label; None offers no such answer
  pub fn always(&self) -> Option<(String, String)> {
    match self {
      Action::Write { .. } => Some(("*".to_owned(), "Allow all edits in this session".to_owned())),
      Action::Bash { command, .. } => {
        let pattern = crate::permission::command_pattern(command);
        let label = format!("Always allow `{pattern}` in this session");
        Some((pattern, label))
      }
      _ => None,
    }
  }

  pub fn describe(&self, cwd: &Path) -> Presentation {
    match self {
      Action::Read { path, .. } => Presentation { title: format!("Read {}", shown(path, cwd)), kind: "read", locations: vec![path.clone()], content: vec![] },
      Action::Write { path, before, after, .. } => Presentation {
        title: format!("{} {}", if before.is_some() { "Edit" } else { "Write" }, shown(path, cwd)),
        kind: "edit",
        locations: vec![path.clone()],
        content: vec![diff_content(path, before.as_deref(), after)],
      },
      Action::Bash { command, .. } => Presentation { title: command.clone(), kind: "execute", locations: vec![], content: vec![] },
      Action::Grep { pattern, path, include } => {
        let scope = [Some(shown(path, cwd)).filter(|s| s != &cwd.to_string_lossy()), include.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        let title = if scope.is_empty() { format!("grep \"{pattern}\"") } else { format!("grep \"{pattern}\" in {scope}") };
        Presentation { title, kind: "search", locations: vec![path.clone()], content: vec![] }
      }
      Action::Glob { pattern, path } => Presentation { title: format!("glob {pattern}"), kind: "search", locations: vec![path.clone()], content: vec![] },
      Action::List { path, .. } => Presentation { title: format!("List {}", shown(path, cwd)), kind: "read", locations: vec![path.clone()], content: vec![] },
      // The host knows a to-do update by this exact title
      Action::Todo { .. } => Presentation { title: TODO.to_owned(), kind: "other", locations: vec![], content: vec![] },
    }
  }

  /// A read-only call, on a blocking thread so several run side by side
  pub fn run_sync(self, ctx: Ctx) -> Output {
    match self {
      Action::Read { path, offset, limit } => read::run(&path, offset, limit, &ctx),
      Action::Grep { pattern, path, include } => search::grep(&pattern, &path, include.as_deref(), &ctx),
      Action::Glob { pattern, path } => search::glob(&pattern, &path, &ctx),
      Action::List { path, ignore } => search::list(&path, &ignore, &ctx),
      Action::Todo { todos } => todo::run(&todos),
      other => Output::error(format!("{other:?} cannot run synchronously")),
    }
  }

  pub async fn run(self, ctx: Ctx) -> Output {
    match self {
      Action::Read { path, offset, limit } => read::run(&path, offset, limit, &ctx),
      Action::Write { path, before, after, edit } => edit::run(&path, before, after, edit, &ctx),
      Action::Bash { command, workdir, timeout_ms } => bash::run(&command, &workdir, timeout_ms, &ctx).await,
      read_only => read_only.run_sync(ctx),
    }
  }
}

/// A path as the title shows it: relative inside the session folder
pub fn shown(path: &Path, cwd: &Path) -> String {
  path.strip_prefix(cwd).ok().filter(|p| !p.as_os_str().is_empty()).unwrap_or(path).to_string_lossy().into_owned()
}

/// A model-given path made absolute against the session folder
pub fn resolve(raw: &str, cwd: &Path) -> PathBuf {
  let expanded = match raw.strip_prefix("~/") {
    Some(rest) => std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).map(|h| h.join(rest)),
    None => None,
  };
  let p = expanded.unwrap_or_else(|| PathBuf::from(raw));
  if p.is_absolute() { p } else { cwd.join(p) }
}

/// A required string argument
pub fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
  args.get(key).and_then(Value::as_str).ok_or_else(|| format!("Missing required string argument \"{key}\""))
}

/// An optional count; models send numbers as strings too
pub fn num_arg(args: &Value, key: &str) -> Option<u64> {
  match args.get(key)? {
    Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
    Value::String(s) => s.trim().parse().ok(),
    _ => None,
  }
}

/// Fit a tool's model copy to the budget
pub fn budgeted(text: &str, keep: Keep, ctx: &Ctx) -> String {
  budget::fit(text, keep, &ctx.outputs, &ctx.call_id)
}
