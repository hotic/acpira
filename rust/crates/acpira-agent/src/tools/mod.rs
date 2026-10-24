//! The tools the model can call. Each call goes through three steps: `prepare` validates the arguments and works out
//! exactly what will happen (an edit's resulting file, so the permission card shows the real diff), `describe` gives
//! its ACP presentation, `run` does it. Names stay short and unprefixed: weaker models copy them more reliably

pub mod bash;
pub mod edit;
pub mod files;
pub mod history;
pub mod html;
pub mod jobs;
pub mod mcp;
pub mod names;
pub mod pdf;
pub mod plan;
pub mod question;
pub mod read;
pub mod search;
pub mod skill;
pub mod todo;
pub mod web;

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
  /// Streams a partial `tool_call_update` (terminal output, an MCP server's progress) while the tool runs
  pub progress: std::sync::Arc<dyn Fn(Value) + Send + Sync>,
  /// The session's background commands
  pub jobs: std::sync::Arc<jobs::Jobs>,
  /// The search backend set on the settings page, read when the call is prepared
  pub search: web::SearchSetup,
  /// The session's MCP servers
  pub mcp: std::sync::Arc<crate::mcp::Hub>,
  /// The session's log, which the history tools read
  pub history: PathBuf,
}

/// A validated call, ready to describe and run. The variants an SSH host runs (`remote.rs`) cross the wire as JSON; the
/// others run here only and are never serialized
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Action {
  /// The windows of one call, the first as the call names it in `path`; `images` whether the turn's model takes the
  /// pictures an image file is read as
  Read { files: Vec<read::Slice>, images: bool },
  /// The whole file replaced (`write`) or computed edits (`edit`, one change per file)
  Write { files: Vec<edit::Change> },
  /// `background`: start it as a job and return at once; `interactive`: that job runs in a terminal and takes input
  Bash { command: String, workdir: PathBuf, timeout_ms: u64, background: bool, interactive: bool },
  /// Check (or stop) a background job, typing `input` into an interactive one first
  Job { id: String, wait_s: u64, kill: bool, input: Option<String> },
  /// `context`: lines shown around each match
  Grep { pattern: String, path: PathBuf, include: Option<String>, context: usize },
  Glob { pattern: String, path: PathBuf },
  List { path: PathBuf, ignore: Vec<String> },
  #[serde(skip)]
  Todo { todos: Vec<todo::Todo> },
  /// A skill's instructions (`path` is its SKILL.md, `body` its text read at prepare)
  Skill { name: String, path: PathBuf, body: String },
  /// Plan mode's approval request; the turn runs it itself
  #[serde(skip)]
  ExitPlan { plan: Option<String> },
  /// Questions on the host's question card; the turn runs it itself
  #[serde(skip)]
  Question { questions: Vec<question::Question> },
  /// `private`: the URL names this machine or a private network, so the call asks (`web.rs`)
  #[serde(skip)]
  Fetch { url: String, format: web::Format, timeout_s: u64, private: bool },
  #[serde(skip)]
  Search { query: String, count: usize },
  /// A tool of an MCP server: `server` is its key in the session's hub, `tool` the server's own name for it, `name`
  /// what the model called (`<server>_<tool>`). `read_only` is the server's `readOnlyHint`; `images` whether the
  /// turn's model takes the images a result may hold
  #[serde(skip)]
  Mcp { server: String, tool: String, name: String, args: Value, read_only: bool, images: bool },
  /// The folded history of chunk compaction (`history.rs`)
  #[serde(skip)]
  HistoryGrep { pattern: String, chunk: Option<String>, case_sensitive: bool, limit: usize },
  #[serde(skip)]
  HistoryList { chunk: String, offset: usize, limit: usize },
  /// A goal tool's call (`goal.rs`); the turn runs it itself, without a row or a permission
  #[serde(skip)]
  Goal(crate::goal::Call),
}

/// How a call reads on the tool card and in the permission request
pub struct Presentation {
  pub title: String,
  /// ACP ToolKind
  pub kind: &'static str,
  pub locations: Vec<Location>,
  /// Shown before the run (an edit's diff)
  pub content: Vec<Value>,
}

/// A file a call touches, with the line a read window starts at (ACP `ToolCallLocation`): the host shows it after the
/// file name and opens the file there
pub struct Location {
  pub path: PathBuf,
  pub line: Option<usize>,
}

impl From<&PathBuf> for Location {
  fn from(path: &PathBuf) -> Location {
    Location { path: path.clone(), line: None }
  }
}

impl Location {
  pub fn json(&self) -> Value {
    match self.line {
      Some(line) => json!({ "path": self.path, "line": line }),
      None => json!({ "path": self.path }),
    }
  }
}

/// What a finished call gives back
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
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
pub const JOB: &str = "job";
pub const GREP: &str = "grep";
pub const GLOB: &str = "glob";
pub const LIST: &str = "list";
pub const TODO: &str = "todo";
pub const SKILL: &str = "skill";
pub const EXIT_PLAN: &str = "exit_plan";
pub const WEBFETCH: &str = "webfetch";
pub const WEBSEARCH: &str = "websearch";
pub const QUESTION: &str = "question";
pub const HISTORY_GREP: &str = "history_grep";
pub const HISTORY_LIST: &str = "history_list";

/// The tools whose calls take the to-do list as `todos` (`todo::rider`): the ones the model mostly calls right after
/// moving the list on (edits, writes and commands, then reads, in the 2026-10-18 count)
pub const TODO_RIDERS: &[&str] = &[READ, WRITE, EDIT, BASH];

/// The specs of these tools, in the given order (a mode's tool set). `skill` is not among them: its spec lists the
/// session's skills (`skill::spec`), so the turn adds it when there are any
pub fn specs(names: &[&str]) -> Vec<ToolSpec> {
  let mut specs: Vec<ToolSpec> = names.iter().filter_map(|n| spec(n)).collect();
  for s in specs.iter_mut().filter(|s| TODO_RIDERS.contains(&s.name.as_str())) {
    s.parameters["properties"]["todos"] = todo::rider_property();
  }
  specs
}

fn spec(name: &str) -> Option<ToolSpec> {
  match name {
    READ => Some(read::spec()),
    WRITE => Some(edit::write_spec()),
    EDIT => Some(edit::spec()),
    BASH => Some(bash::spec()),
    JOB => Some(jobs::spec()),
    GREP => Some(search::grep_spec()),
    GLOB => Some(search::glob_spec()),
    LIST => Some(search::list_spec()),
    TODO => Some(todo::spec()),
    EXIT_PLAN => Some(plan::spec()),
    WEBFETCH => Some(web::fetch_spec()),
    WEBSEARCH => Some(web::search_spec()),
    QUESTION => Some(question::spec()),
    HISTORY_GREP => Some(history::grep_spec()),
    HISTORY_LIST => Some(history::list_spec()),
    crate::goal::GET | crate::goal::CREATE | crate::goal::UPDATE => crate::goal::spec(name),
    _ => None,
  }
}

/// Validate a call of a tool by its real name (`names::resolve` first); the error goes back to the model as the result.
/// `images`: the turn's model takes images
pub fn prepare(name: &str, args: &Value, cwd: &Path, images: bool) -> Result<Action, String> {
  match name {
    READ => read::prepare(args, cwd, images),
    WRITE => edit::prepare_write(args, cwd),
    EDIT => edit::prepare(args, cwd),
    BASH => bash::prepare(args, cwd),
    JOB => jobs::prepare(args),
    GREP => search::prepare_grep(args, cwd),
    GLOB => search::prepare_glob(args, cwd),
    LIST => search::prepare_list(args, cwd),
    TODO => todo::prepare(args),
    SKILL => skill::prepare(args, &crate::prompt::Places::new(cwd, crate::turn::user_home().as_deref())),
    EXIT_PLAN => plan::prepare(args),
    WEBFETCH => web::prepare_fetch(args),
    WEBSEARCH => web::prepare_search(args),
    QUESTION => question::prepare(args),
    HISTORY_GREP => history::prepare_grep(args),
    HISTORY_LIST => history::prepare_list(args),
    crate::goal::GET | crate::goal::CREATE | crate::goal::UPDATE => crate::goal::prepare(name, args),
    other => Err(format!("Unknown tool \"{other}\". Available tools: {}.", names::ALL.join(", "))),
  }
}

impl Action {
  /// Changes nothing on disk: such calls run side by side
  pub fn read_only(&self) -> bool {
    !matches!(
      self,
      Action::Write { .. }
        | Action::Bash { .. }
        | Action::Job { .. }
        | Action::ExitPlan { .. }
        | Action::Question { .. }
        | Action::Mcp { read_only: false, .. }
        | Action::Goal(_)
    )
  }

  /// The permission key and the target the rules match (`permission.rs`)
  pub fn permission(&self, cwd: &Path) -> (&'static str, String) {
    use crate::permission::{self as perm, path_target};
    match self {
      Action::Read { files, .. } => (perm::READ, files.first().map_or_else(String::new, |f| path_target(&f.path, cwd))),
      Action::Grep { path, .. } | Action::Glob { path, .. } | Action::List { path, .. } | Action::Skill { path, .. } => (perm::READ, path_target(path, cwd)),
      Action::Write { files } => (perm::EDIT, files.first().map_or_else(String::new, |f| path_target(&f.path, cwd))),
      Action::Bash { command, .. } => (perm::BASH, command.clone()),
      Action::Todo { .. } => (perm::TODO, "*".to_owned()),
      // Starting the job was the permission
      Action::Job { id, .. } => (perm::JOB, id.clone()),
      // Never evaluated: the approval card is the permission
      Action::ExitPlan { .. } => ("plan", "*".to_owned()),
      // Never evaluated: asking the user needs no permission
      Action::Question { .. } => ("question", "*".to_owned()),
      Action::Fetch { url, private: false, .. } => (perm::WEB, url.clone()),
      Action::Fetch { url, private: true, .. } => (perm::WEB_LOCAL, url.clone()),
      Action::Search { query, .. } => (perm::WEB, query.clone()),
      Action::Mcp { server, tool, read_only, .. } => (if *read_only { perm::MCP_READ } else { perm::MCP }, format!("{server}/{tool}")),
      // The session's own log, allowed by default
      Action::HistoryGrep { pattern, .. } => (perm::HISTORY, pattern.clone()),
      Action::HistoryList { chunk, .. } => (perm::HISTORY, chunk.clone()),
      // Never evaluated: the goal is the session's own
      Action::Goal(_) => ("goal", "*".to_owned()),
    }
  }

  /// Every (permission, target) the call touches: a read or an edit of several files is checked for each of them
  pub fn permissions(&self, cwd: &Path) -> Vec<(&'static str, String)> {
    use crate::permission::{self as perm, path_target};
    match self {
      Action::Read { files, .. } => files.iter().map(|f| (perm::READ, path_target(&f.path, cwd))).collect(),
      Action::Write { files } => files.iter().map(|f| (perm::EDIT, path_target(&f.path, cwd))).collect(),
      _ => vec![self.permission(cwd)],
    }
  }

  /// The files a write or an edit changes
  pub fn written(&self) -> Vec<&Path> {
    match self {
      Action::Write { files } => files.iter().map(|f| f.path.as_path()).collect(),
      _ => vec![],
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
      // A local address is allowed per origin: the dev server's other pages need no second card
      Action::Fetch { url, private: true, .. } => web::origin(url).map(|o| (format!("{o}/*"), format!("Always allow {o} in this session"))),
      Action::Mcp { server, tool, name, .. } => Some((format!("{server}/{tool}"), format!("Always allow {name} in this session"))),
      _ => None,
    }
  }

  pub fn describe(&self, cwd: &Path) -> Presentation {
    match self {
      Action::Read { files, .. } => {
        let paths = distinct(files.iter().map(|f| &f.path));
        // One location per window, so two ranges of one file each show where they start; line 1 is left out, as the
        // host takes it for a whole-file read
        let locations = files.iter().map(|f| Location { path: f.path.clone(), line: (f.offset > 1).then_some(f.offset) }).collect();
        Presentation { title: format!("Read {}", names(&paths, cwd)), kind: "read", locations, content: vec![] }
      }
      Action::Write { files } => {
        let verb = if files.iter().all(|f| f.before.is_none()) { "Write" } else { "Edit" };
        let paths = distinct(files.iter().map(|f| &f.path));
        Presentation {
          title: format!("{verb} {}", names(&paths, cwd)),
          kind: "edit",
          locations: paths.iter().map(Location::from).collect(),
          content: files.iter().map(|f| diff_content(&f.path, f.before.as_deref(), &f.after)).collect(),
        }
      }
      Action::Bash { command, .. } => Presentation { title: command.clone(), kind: "execute", locations: vec![], content: vec![] },
      Action::Job { id, wait_s, kill, input } => {
        let title = match input {
          // What is typed, escapes visible (`\n` is Enter), cut to a title's length
          Some(text) if !*kill => format!("Type into job {id}: {}", crate::budget::cut(text.escape_debug().to_string().as_str(), 80)),
          _ if *kill => format!("Stop job {id}"),
          _ if *wait_s > 0 => format!("Check job {id} (wait {wait_s}s)"),
          _ => format!("Check job {id}"),
        };
        Presentation { title, kind: "execute", locations: vec![], content: vec![] }
      }
      Action::Grep { pattern, path, include, .. } => {
        let scope = [(path != cwd).then(|| shown(path, cwd)), include.clone()].into_iter().flatten().collect::<Vec<_>>().join(" ");
        let title = if scope.is_empty() { format!("grep \"{pattern}\"") } else { format!("grep \"{pattern}\" in {scope}") };
        Presentation { title, kind: "search", locations: vec![path.into()], content: vec![] }
      }
      Action::Glob { pattern, path } => Presentation { title: format!("glob {pattern}"), kind: "search", locations: vec![path.into()], content: vec![] },
      Action::List { path, .. } => Presentation { title: format!("List {}", shown(path, cwd)), kind: "read", locations: vec![path.into()], content: vec![] },
      // The host knows a to-do update by this exact title
      Action::Todo { .. } => Presentation { title: TODO.to_owned(), kind: "other", locations: vec![], content: vec![] },
      Action::Skill { name, path, .. } => Presentation { title: format!("Skill {name}"), kind: "read", locations: vec![path.into()], content: vec![] },
      // The webview localizes this exact heading
      Action::ExitPlan { .. } => Presentation { title: "Exit plan mode".to_owned(), kind: "switch_mode", locations: vec![], content: vec![] },
      Action::Question { questions } => Presentation { title: question::title(questions.len()), kind: "other", locations: vec![], content: vec![] },
      // `fetch` for both, as claude-agent-acp sends WebFetch and WebSearch: the webview renders their markdown
      Action::Fetch { url, .. } => Presentation { title: format!("Fetch {url}"), kind: "fetch", locations: vec![], content: vec![] },
      Action::Search { query, .. } => Presentation { title: format!("Search \"{query}\""), kind: "fetch", locations: vec![], content: vec![] },
      // The name as the model called it: the engine knows `acpira_show_image` and `acpira_ask_agent` by it
      Action::Mcp { name, .. } => Presentation { title: name.clone(), kind: "other", locations: vec![], content: vec![] },
      Action::HistoryGrep { pattern, chunk, .. } => {
        let title = match chunk {
          Some(c) => format!("Search history \"{pattern}\" in {c}"),
          None => format!("Search history \"{pattern}\""),
        };
        Presentation { title, kind: "search", locations: vec![], content: vec![] }
      }
      Action::HistoryList { chunk, offset, limit } => Presentation { title: format!("Read history {chunk} lines {offset}..{}", offset + limit), kind: "read", locations: vec![], content: vec![] },
      // Shown only on a row the stream opened for a misnamed call; a goal call has none of its own
      Action::Goal(_) => Presentation { title: "Goal".to_owned(), kind: "other", locations: vec![], content: vec![] },
    }
  }

  /// A read-only call, on a blocking thread so several run side by side
  pub fn run_sync(self, ctx: Ctx) -> Output {
    match self {
      Action::Read { files, images } => read::run(&files, images, &ctx),
      Action::Grep { pattern, path, include, context } => search::grep(&pattern, &path, include.as_deref(), context, &ctx),
      Action::Glob { pattern, path } => search::glob(&pattern, &path, &ctx),
      Action::List { path, ignore } => search::list(&path, &ignore, &ctx),
      Action::Todo { todos } => todo::run(&todos),
      Action::Skill { path, body, .. } => skill::run(&path, &body),
      Action::Fetch { url, format, timeout_s, private } => web::fetch(&url, format, timeout_s, private, &ctx),
      Action::Search { query, count } => web::search(&query, count, &ctx.search, &ctx),
      Action::HistoryGrep { pattern, chunk, case_sensitive, limit } => history::grep(&pattern, chunk.as_deref(), case_sensitive, limit, &ctx),
      Action::HistoryList { chunk, offset, limit } => history::list(&chunk, offset, limit, &ctx),
      other => Output::error(format!("{other:?} cannot run synchronously")),
    }
  }

  pub async fn run(self, ctx: Ctx) -> Output {
    match self {
      // Off the runtime's workers: a PDF can take seconds to parse
      Action::Read { files, images } => tokio::task::spawn_blocking(move || read::run(&files, images, &ctx)).await.unwrap_or_else(|e| Output::error(format!("The tool crashed: {e}"))),
      Action::Write { files } => edit::run(files, &ctx),
      Action::Bash { command, workdir, background: true, interactive, .. } => ctx.jobs.clone().start(&command, &workdir, interactive, &ctx).await,
      Action::Bash { command, workdir, timeout_ms, .. } => bash::run(&command, &workdir, timeout_ms, &ctx).await,
      Action::Job { id, wait_s, kill, input } => ctx.jobs.clone().check(&id, wait_s, kill, input.as_deref(), &ctx).await,
      // Network calls block for up to two minutes: off the runtime's workers (an approved local fetch comes this way)
      Action::Mcp { server, tool, args, images, .. } => mcp::run(&server, &tool, args, images, &ctx).await,
      web @ (Action::Fetch { .. } | Action::Search { .. }) => {
        tokio::task::spawn_blocking(move || web.run_sync(ctx)).await.unwrap_or_else(|e| Output::error(format!("The tool crashed: {e}")))
      }
      read_only => read_only.run_sync(ctx),
    }
  }
}

/// Each path once, in order
fn distinct<'a>(paths: impl Iterator<Item = &'a PathBuf>) -> Vec<PathBuf> {
  let mut out: Vec<PathBuf> = vec![];
  for p in paths {
    if !out.contains(p) {
      out.push(p.clone());
    }
  }
  out
}

/// A title's file list: one or two names, else the first two and how many more
fn names(paths: &[PathBuf], cwd: &Path) -> String {
  match paths {
    [] => String::new(),
    [a] => shown(a, cwd),
    [a, b] => format!("{}, {}", shown(a, cwd), shown(b, cwd)),
    [a, b, rest @ ..] => format!("{}, {} and {} more", shown(a, cwd), shown(b, cwd), rest.len()),
  }
}

/// The files a call of read, write or edit names (compaction's file restore and the chunk index read them off the
/// session log): read's ranges left out, edit's items with their own paths included
pub fn files_of(name: &str, args: &Value, cwd: &Path) -> Vec<PathBuf> {
  match name {
    READ => read::targets(args, cwd),
    EDIT => edit::targets(args, cwd),
    WRITE => args.get("path").or_else(|| args.get("file_path")).and_then(Value::as_str).map(|p| resolve(p, cwd)).into_iter().collect(),
    _ => vec![],
  }
}

/// A path as titles and tool results show it: relative inside the session folder, with forward slashes on Windows
/// too, where a joined path otherwise mixes them (`src/util\b.rs`) and Windows accepts `/` when the model sends it back
pub fn shown(path: &Path, cwd: &Path) -> String {
  let s = path.strip_prefix(cwd).ok().filter(|p| !p.as_os_str().is_empty()).unwrap_or(path).to_string_lossy().into_owned();
  if cfg!(windows) { s.replace('\\', "/") } else { s }
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
