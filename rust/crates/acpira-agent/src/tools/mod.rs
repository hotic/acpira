//! The tools the model can call. Each call goes through three steps: `prepare` validates the arguments and works out
//! exactly what will happen (an edit's resulting file, so the permission card shows the real diff), `describe` gives
//! its ACP presentation, `run` does it. Names stay short and unprefixed: weaker models copy them more reliably

pub mod bash;
pub mod edit;
pub mod read;

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
}

/// How a call reads on the tool card and in the permission request
pub struct Presentation {
  pub title: String,
  /// ACP ToolKind
  pub kind: &'static str,
  pub locations: Vec<PathBuf>,
  /// Shown before the run (an edit's diff)
  pub content: Vec<Value>,
  /// Whether the call needs the user's approval
  pub ask: bool,
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

/// The tool set of a turn
pub fn specs() -> Vec<ToolSpec> {
  vec![read::spec(), edit::write_spec(), edit::spec(), bash::spec()]
}

/// Validate a call; the error goes back to the model as the tool result
pub fn prepare(name: &str, args: &Value, cwd: &Path) -> Result<Action, String> {
  match name {
    READ => read::prepare(args, cwd),
    WRITE => edit::prepare_write(args, cwd),
    EDIT => edit::prepare(args, cwd),
    BASH => bash::prepare(args, cwd),
    other => Err(format!("Unknown tool \"{other}\". Available tools: {}.", specs().iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "))),
  }
}

impl Action {
  pub fn read_only(&self) -> bool {
    matches!(self, Action::Read { .. })
  }

  pub fn describe(&self, cwd: &Path) -> Presentation {
    match self {
      Action::Read { path, .. } => {
        Presentation { title: format!("Read {}", shown(path, cwd)), kind: "read", locations: vec![path.clone()], content: vec![], ask: false }
      }
      Action::Write { path, before, after, .. } => Presentation {
        title: format!("{} {}", if before.is_some() { "Edit" } else { "Write" }, shown(path, cwd)),
        kind: "edit",
        locations: vec![path.clone()],
        content: vec![diff_content(path, before.as_deref(), after)],
        ask: true,
      },
      Action::Bash { command, .. } => Presentation { title: command.clone(), kind: "execute", locations: vec![], content: vec![], ask: true },
    }
  }

  /// A read-only call, on a blocking thread so several run side by side
  pub fn run_sync(self, ctx: Ctx) -> Output {
    match self {
      Action::Read { path, offset, limit } => read::run(&path, offset, limit, &ctx),
      other => Output::error(format!("{other:?} cannot run synchronously")),
    }
  }

  pub async fn run(self, ctx: Ctx) -> Output {
    match self {
      Action::Read { path, offset, limit } => read::run(&path, offset, limit, &ctx),
      Action::Write { path, before, after, edit } => edit::run(&path, before, after, edit, &ctx),
      Action::Bash { command, workdir, timeout_ms } => bash::run(&command, &workdir, timeout_ms, &ctx).await,
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
