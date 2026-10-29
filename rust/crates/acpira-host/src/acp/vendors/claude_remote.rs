//! Claude Code in a folder on an SSH host under local run (docs/dev/remote.md, "Claude Code"). The CLI runs here, on
//! this machine's login, in a stand-in folder under the data root. Its own file and shell tools are switched off with
//! `disallowedTools`, which also stops the harness's direct calls (a command's inline shell), and the model gets
//! Acpira's instead (`host_tools::REMOTE`, served by `acpira mcp` as `mcp__acpira__Read` …). Those run on the host
//! through its helper, with the built-in agent's tool code. `toolAliases` sends a native name the model still writes
//! (a skill's text says `Bash`) to the same tools. A path under the stand-in means the host folder; Claude's own files
//! on this machine (plans, memory and saved outputs under its config folder, its temp folder) stay here. Rows and
//! permission cards show the calls as the native tools (`view`)

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// How the adapter names a tool of Acpira's server
pub const PREFIX: &str = "mcp__acpira__";

/// Claude Code's tools that touch this machine's files or shell (CLI 2.1.300: the deferred Monitor runs a command, the
/// worktree tools a git checkout of the folder). Names an older CLI lacks are ignored by it
pub const DISALLOWED: &[&str] = &[
  "Read",
  "Write",
  "Edit",
  "MultiEdit",
  "NotebookEdit",
  "Bash",
  "BashOutput",
  "KillShell",
  "PowerShell",
  "Grep",
  "Glob",
  "LS",
  "LSP",
  "Monitor",
  "EnterWorktree",
  "ExitWorktree",
];

/// The remote tools allowed without a permission request, as the native read-only ones are
const READ_ONLY: &[&str] = &["Read", "Grep", "Glob"];

/// A session folder on an SSH host and its stand-in folder here
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stand {
  pub alias: String,
  /// The folder on the host, absolute
  pub host: String,
  /// Where the CLI runs here: `<data root>/remote/hosts/<alias>/<host path>`. Empty but for what the CLI keeps itself
  pub local: PathBuf,
}

impl Stand {
  pub fn new(alias: &str, host: &str, data_root: &Path) -> Stand {
    let host = if host.len() > 1 { host.trim_end_matches('/') } else { host };
    let mut local = data_root.join("remote").join("hosts").join(alias);
    let rel = host.trim_start_matches('/');
    if !rel.is_empty() {
      local.push(rel);
    }
    Stand { alias: alias.to_owned(), host: host.to_owned(), local }
  }

  /// A path the model wrote, as the host knows it: under the stand-in it moves to the host folder, a relative one is
  /// read against the host folder, anything else is the host's already
  pub fn host_path(&self, p: &str) -> String {
    let local = self.local.to_string_lossy();
    let base = self.host.trim_end_matches('/');
    match p.strip_prefix(local.as_ref()) {
      Some("") => self.host.clone(),
      Some(rest) if rest.starts_with('/') => format!("{base}{rest}"),
      _ if p.starts_with('/') => p.to_owned(),
      _ => format!("{base}/{}", p.trim_start_matches("./")),
    }
  }

  /// Whether `p` names the stand-in or something in it
  fn in_stand(&self, p: &str) -> bool {
    Path::new(p).starts_with(&self.local)
  }
}

/// The folders of this machine whose files Claude's tools reach for and must find here: its config folder (plans,
/// memory, saved tool outputs) and Acpira's data root. Its temp folder (`/tmp/claude-<uid>`) is matched by name
pub fn here_roots(home: &Path, data_root: &Path) -> Vec<PathBuf> {
  let mut out = vec![home.join(".claude"), data_root.to_path_buf()];
  if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
    out.push(PathBuf::from(dir));
  }
  out
}

fn claude_temp(p: &str) -> bool {
  ["/tmp/claude-", "/private/tmp/claude-"].iter().any(|t| p.starts_with(t))
}

/// The path a call of a remote tool names, if any
fn path_arg<'a>(name: &str, args: &'a Value) -> Option<&'a str> {
  let key = |k: &str| args.get(k).and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty());
  match name {
    "Read" | "Write" | "Edit" => key("file_path").or_else(|| key("path")),
    "Grep" | "Glob" => key("path"),
    _ => None,
  }
}

/// Whether a call runs on this machine: a file tool whose path is one of Claude's own files here (`here_roots`, its
/// temp folder). Commands always run on the host
pub fn runs_here(name: &str, args: &Value, stand: &Stand, roots: &[PathBuf]) -> bool {
  let Some(p) = path_arg(name, args).filter(|p| p.starts_with('/')) else { return false };
  if stand.in_stand(p) {
    return false;
  }
  claude_temp(p) || roots.iter().any(|r| Path::new(p).starts_with(r))
}

/// The built-in agent's tool and arguments (`acpira_agent::tools`) for a call of a remote tool. `here`: the call runs on
/// this machine, so its paths stay as written
pub fn builtin_call(name: &str, args: &Value, stand: &Stand, here: bool) -> Result<(&'static str, Value), String> {
  use acpira_agent::tools;
  let empty = Map::new();
  let a = args.as_object().unwrap_or(&empty);
  let path = |p: &str| if here { p.to_owned() } else { stand.host_path(p) };
  let mut out = Map::new();
  let copy = |out: &mut Map<String, Value>, from: &str, to: &str| {
    if let Some(v) = a.get(from).filter(|v| !v.is_null()) {
      out.insert(to.to_owned(), v.clone());
    }
  };
  let tool = match name {
    "Read" | "Write" | "Edit" => {
      let p = path_arg(name, args).ok_or("file_path is required")?;
      out.insert("path".into(), json!(path(p)));
      match name {
        "Read" => {
          for k in ["offset", "limit", "pages"] {
            copy(&mut out, k, k);
          }
          tools::READ
        }
        "Write" => {
          copy(&mut out, "content", "content");
          tools::WRITE
        }
        _ => {
          for k in ["old_string", "new_string", "replace_all"] {
            copy(&mut out, k, k);
          }
          tools::EDIT
        }
      }
    }
    "Bash" => {
      copy(&mut out, "command", "command");
      copy(&mut out, "timeout", "timeout");
      tools::BASH
    }
    "Grep" => {
      let pattern = a.get("pattern").and_then(Value::as_str).ok_or("pattern is required")?;
      let insensitive = a.get("-i").and_then(Value::as_bool).unwrap_or(false) && !pattern.starts_with("(?i)");
      out.insert("pattern".into(), json!(if insensitive { format!("(?i){pattern}") } else { pattern.to_owned() }));
      copy(&mut out, "glob", "include");
      copy(&mut out, "-C", "context");
      if let Some(p) = path_arg(name, args) {
        out.insert("path".into(), json!(path(p)));
      }
      tools::GREP
    }
    "Glob" => {
      copy(&mut out, "pattern", "pattern");
      if let Some(p) = path_arg(name, args) {
        out.insert("path".into(), json!(path(p)));
      }
      tools::GLOB
    }
    other => return Err(format!("{other} is not one of this server's project tools")),
  };
  Ok((tool, Value::Object(out)))
}

/// A run's output as an MCP tool result: the model's text, plus the pictures an image file was read as
pub fn mcp_result(out: &acpira_agent::tools::Output) -> Value {
  let mut content = vec![json!({ "type": "text", "text": out.model })];
  for c in &out.content {
    if let Some(img) = c.get("content").filter(|x| x.get("type").and_then(Value::as_str) == Some("image")) {
      content.push(json!({ "type": "image", "data": img.get("data"), "mimeType": img.get("mimeType") }));
    }
  }
  json!({ "content": content, "isError": out.is_error })
}

/// The MCP definitions of the remote tools, in the native tools' argument shapes. `alwaysLoad` keeps them out of
/// Claude Code's deferred tool search, since they replace tools the model always has
pub fn tool_defs(alias: &str) -> Vec<Value> {
  let def = |name: &str, description: String, properties: Value, required: &[&str], read_only: bool| {
    json!({
      "name": name,
      "description": description,
      "inputSchema": { "type": "object", "properties": properties, "required": required },
      "annotations": { "readOnlyHint": read_only, "destructiveHint": !read_only, "openWorldHint": false },
      "_meta": { "anthropic/alwaysLoad": true },
    })
  };
  let path = "Absolute path on the host, or relative to the project folder";
  vec![
    def(
      "Read",
      format!(
        "Read a file of the project on the SSH host {alias}. Returns numbered lines (up to 2000 per call) and the file's total line \
         count; use offset / limit for the next window. A PNG, JPEG, GIF or WebP file comes back as the image itself; a PDF as its \
         text, page by page (pages, e.g. 3-5). Your own files on this machine (plans, memory, saved outputs) are read here."
      ),
      json!({
        "file_path": { "type": "string", "description": path },
        "offset": { "type": "integer", "description": "First line to read, 1-based" },
        "limit": { "type": "integer", "description": "How many lines to read" },
        "pages": { "type": "string", "description": "For a PDF: the pages to read, e.g. 3-5" },
      }),
      &["file_path"],
      true,
    ),
    def(
      "Write",
      format!("Write a whole file on the SSH host {alias}, creating or replacing it. Prefer Edit for a change to an existing file."),
      json!({
        "file_path": { "type": "string", "description": path },
        "content": { "type": "string", "description": "The complete new content" },
      }),
      &["file_path", "content"],
      false,
    ),
    def(
      "Edit",
      format!(
        "Replace text in a file on the SSH host {alias}. old_string must occur exactly once (whitespace included) unless \
         replace_all is true; include enough surrounding lines to make it unique. Returns the change as a diff."
      ),
      json!({
        "file_path": { "type": "string", "description": path },
        "old_string": { "type": "string", "description": "The exact text to replace" },
        "new_string": { "type": "string", "description": "The replacement text" },
        "replace_all": { "type": "boolean", "description": "Replace every occurrence (default false)" },
      }),
      &["file_path", "old_string", "new_string"],
      false,
    ),
    def(
      "Bash",
      format!(
        "Run a shell command on the SSH host {alias} in the project folder and return its output (stdout and stderr together) and \
         exit code. Each call starts a fresh shell, so a cd does not carry over. Commands get no input. Default timeout 120000 ms, \
         at most 600000; for a dev server or watcher, start it detached (nohup … > log 2>&1 &) and read the log."
      ),
      json!({
        "command": { "type": "string", "description": "The command line" },
        "timeout": { "type": "integer", "description": "Timeout in milliseconds" },
        "description": { "type": "string", "description": "What the command does, in a few words" },
      }),
      &["command"],
      false,
    ),
    def(
      "Grep",
      format!(
        "Search file contents on the SSH host {alias} with a regular expression (Rust regex syntax). Returns `path:line: text` \
         for up to 100 matches, skipping ignored and binary files."
      ),
      json!({
        "pattern": { "type": "string", "description": "Regular expression" },
        "path": { "type": "string", "description": "File or folder to search (default: the project folder)" },
        "glob": { "type": "string", "description": "Only files matching this glob, e.g. *.rs or src/**/*.{ts,tsx}" },
        "-i": { "type": "boolean", "description": "Case-insensitive" },
        "-C": { "type": "integer", "description": "Lines to show before and after each match (at most 50)" },
      }),
      &["pattern"],
      true,
    ),
    def(
      "Glob",
      format!("Find files by name on the SSH host {alias} with a glob pattern (** crosses folders). Returns up to 100 paths, ignored files left out."),
      json!({
        "pattern": { "type": "string", "description": "Glob, e.g. **/*.test.ts" },
        "path": { "type": "string", "description": "Folder to search (default: the project folder)" },
      }),
      &["pattern"],
      true,
    ),
  ]
}

/// The session request of a Claude Code session in a folder on an SSH host: the stand-in as its cwd, the native tools
/// switched off and aliased to the remote ones, and a system prompt section that says where the project is, with the
/// host's instructions file (`instructions`: path and text) since the CLI cannot find it from here
pub fn with_stand(mut req: Value, stand: &Stand, places: Option<&Value>, instructions: &[(String, String)]) -> Value {
  req["cwd"] = json!(stand.local);
  if let Some(meta) = req.get_mut("_meta").and_then(Value::as_object_mut) {
    meta.remove(crate::acp::session::remote::META);
  }
  let options = &mut req["_meta"]["claudeCode"]["options"];
  let mut disallowed: Vec<Value> = options.get("disallowedTools").and_then(Value::as_array).cloned().unwrap_or_default();
  disallowed.extend(DISALLOWED.iter().map(|t| json!(t)));
  options["disallowedTools"] = json!(disallowed);
  let aliases: Map<String, Value> = acpira_shared::host_tools::REMOTE.iter().map(|t| ((*t).to_owned(), json!(format!("{PREFIX}{t}")))).collect();
  options["toolAliases"] = Value::Object(aliases);
  let mut allowed: Vec<Value> = options.get("allowedTools").and_then(Value::as_array).cloned().unwrap_or_default();
  allowed.extend(READ_ONLY.iter().map(|t| json!(format!("{PREFIX}{t}"))));
  options["allowedTools"] = json!(allowed);
  let text = system_note(stand, places, instructions);
  req["systemPrompt"] = json!({ "mode": "append", "content": [{ "type": "text", "text": text }] });
  req
}

fn system_note(stand: &Stand, places: Option<&Value>, instructions: &[(String, String)]) -> String {
  let s = |k: &str| places.and_then(|p| p.get(k)).and_then(Value::as_str).filter(|v| !v.is_empty());
  let platform = match (s("os"), s("arch"), s("shell")) {
    (Some(os), Some(arch), Some(shell)) => format!(" ({os} {arch}, shell {shell})"),
    _ => String::new(),
  };
  let mut out = format!(
    "# Remote project\n\nThis session's project is on the SSH host {alias}{platform}, in {host}. Your working directory {local} is an \
     empty stand-in on this machine for that folder: it holds none of the project's files, and it is not a git repository even when \
     the project is one. The project tools {PREFIX}Read, Write, Edit, Bash, Grep and Glob run on {alias}: a relative path resolves \
     against {host}, a path under the stand-in means the same file there, and commands (git, builds, tests) run there. Write \
     paths as {host}/… Your own files on this machine (plans, memory, saved outputs) stay here, and the same tools reach them.",
    alias = stand.alias,
    host = stand.host,
    local = stand.local.display(),
  );
  for (path, text) in instructions {
    out.push_str(&format!("\n\nThe project's instructions, from {path} on {}:\n\n<instructions>\n{}\n</instructions>", stand.alias, text.trim()));
  }
  out
}

/// The native name of a remote tool the update (or a permission request's toolCall) names: the adapter's
/// `_meta.claudeCode.toolName`, its top-level `name`, else its title, which the generic reporter sets to the tool's name
pub fn remote_name(u: &Value) -> Option<&'static str> {
  let named = |s: Option<&str>| {
    let n = s?.strip_prefix(PREFIX)?;
    acpira_shared::host_tools::REMOTE.iter().find(|t| **t == n).copied()
  };
  ["/_meta/claudeCode/toolName", "/name", "/title"].iter().find_map(|at| named(u.pointer(at).and_then(Value::as_str)))
}

/// How the adapter reports the native tool for this input: kind, title, locations, and a write's diff from the input
pub struct NativeView {
  pub kind: &'static str,
  pub title: String,
  pub locations: Vec<Value>,
  pub diff: Option<Value>,
  /// The input with its path as the host knows it, for the row's target and the edit gate
  pub input: Value,
}

pub fn view(name: &str, input: &Value, stand: &Stand, roots: &[PathBuf]) -> NativeView {
  let here = runs_here(name, input, stand, roots);
  let mut input = input.clone();
  let path = path_arg(name, &input).map(|p| if here { p.to_owned() } else { stand.host_path(p) });
  if let (Some(p), Some(o)) = (&path, input.as_object_mut()) {
    let key = if o.contains_key("file_path") { "file_path" } else { "path" };
    o.insert(key.to_owned(), json!(p));
  }
  let shown = |p: &str| p.strip_prefix(&format!("{}/", stand.host.trim_end_matches('/'))).unwrap_or(p).to_owned();
  let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
  let (kind, title) = match name {
    "Read" => ("read", path.as_deref().map(|p| format!("Read {}", shown(p))).unwrap_or_else(|| "Read File".into())),
    "Write" => ("edit", path.as_deref().map(|p| format!("Write {}", shown(p))).unwrap_or_else(|| "Write".into())),
    "Edit" => ("edit", path.as_deref().map(|p| format!("Edit {}", shown(p))).unwrap_or_else(|| "Edit".into())),
    "Bash" => ("execute", Some(s("command")).filter(|c| !c.is_empty()).unwrap_or_else(|| "Terminal".into())),
    "Grep" => ("search", format!("grep {}{}", s("pattern"), path.as_deref().map(|p| format!(" {}", shown(p))).unwrap_or_default())),
    _ => ("search", format!("Find {}{}", path.as_deref().map(|p| format!("{} ", shown(p))).unwrap_or_default(), s("pattern"))),
  };
  let locations = match (&path, name) {
    (Some(p), "Read") => vec![json!({ "path": p, "line": input.get("offset").and_then(Value::as_u64).unwrap_or(1) })],
    (Some(p), _) => vec![json!({ "path": p })],
    _ => vec![],
  };
  let diff = match (&path, name) {
    (Some(p), "Edit") => Some(json!({ "type": "diff", "path": p, "oldText": input.get("old_string"), "newText": input.get("new_string") })),
    (Some(p), "Write") => Some(json!({ "type": "diff", "path": p, "oldText": null, "newText": input.get("content") })),
    _ => None,
  };
  NativeView { kind, title, locations, diff, input }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn stand() -> Stand {
    Stand::new("gen1", "/home/u/proj/", Path::new("/Users/me/.acpira"))
  }

  #[test]
  fn the_stand_in_maps_to_the_host_folder() {
    let s = stand();
    assert_eq!(s.local, PathBuf::from("/Users/me/.acpira/remote/hosts/gen1/home/u/proj"));
    assert_eq!(s.host_path("/Users/me/.acpira/remote/hosts/gen1/home/u/proj"), "/home/u/proj");
    assert_eq!(s.host_path("/Users/me/.acpira/remote/hosts/gen1/home/u/proj/src/a.rs"), "/home/u/proj/src/a.rs");
    // A sibling whose name only starts the same is not inside
    assert_eq!(s.host_path("/Users/me/.acpira/remote/hosts/gen1/home/u/project2/a"), "/Users/me/.acpira/remote/hosts/gen1/home/u/project2/a");
    assert_eq!(s.host_path("src/a.rs"), "/home/u/proj/src/a.rs");
    assert_eq!(s.host_path("./a.rs"), "/home/u/proj/a.rs");
    assert_eq!(s.host_path("/etc/hosts"), "/etc/hosts");
    assert_eq!(Stand::new("h", "/", Path::new("/d")).host_path("/d/remote/hosts/h/x"), "/x");
  }

  #[test]
  fn claudes_own_files_stay_here_and_the_rest_goes_to_the_host() {
    let s = stand();
    let roots = here_roots(Path::new("/Users/me"), Path::new("/Users/me/.acpira"));
    let here = |name: &str, args: Value| runs_here(name, &args, &s, &roots);
    assert!(here("Write", json!({ "file_path": "/Users/me/.claude/plans/p.md", "content": "x" })));
    assert!(here("Read", json!({ "file_path": "/private/tmp/claude-501/out.txt" })));
    assert!(!here("Read", json!({ "file_path": "/Users/me/.acpira/remote/hosts/gen1/home/u/proj/a.rs" })));
    assert!(!here("Read", json!({ "file_path": "src/a.rs" })));
    assert!(!here("Bash", json!({ "command": "cat /Users/me/.claude/x" })));
    assert!(!here("Grep", json!({ "pattern": "x" })));
  }

  #[test]
  fn calls_become_the_builtin_agents() {
    let s = stand();
    let (tool, a) = builtin_call("Edit", &json!({ "file_path": "src/a.rs", "old_string": "a", "new_string": "b", "replace_all": true }), &s, false).unwrap();
    assert_eq!((tool, a), ("edit", json!({ "path": "/home/u/proj/src/a.rs", "old_string": "a", "new_string": "b", "replace_all": true })));
    let (tool, a) = builtin_call("Grep", &json!({ "pattern": "fn x", "-i": true, "glob": "*.rs", "-C": 2, "path": "src" }), &s, false).unwrap();
    assert_eq!((tool, a), ("grep", json!({ "pattern": "(?i)fn x", "include": "*.rs", "context": 2, "path": "/home/u/proj/src" })));
    let (tool, a) = builtin_call("Bash", &json!({ "command": "ls", "timeout": 5000, "description": "list" }), &s, false).unwrap();
    assert_eq!((tool, a), ("bash", json!({ "command": "ls", "timeout": 5000 })));
    let (_, a) = builtin_call("Write", &json!({ "file_path": "/Users/me/.claude/plans/p.md", "content": "x" }), &s, true).unwrap();
    assert_eq!(a["path"], "/Users/me/.claude/plans/p.md");
    assert!(builtin_call("Read", &json!({}), &s, false).is_err());
    assert!(builtin_call("Task", &json!({}), &s, false).is_err());
  }

  #[test]
  fn the_request_runs_in_the_stand_in_with_the_native_tools_off() {
    let s = stand();
    let req = json!({ "cwd": "/home/u/proj", "_meta": { "acpira/remote": { "alias": "gen1" }, "claudeCode": { "options": { "thinking": { "type": "adaptive" } } } } });
    let places = json!({ "os": "linux", "arch": "x86_64", "shell": "/bin/bash" });
    let out = with_stand(req, &s, Some(&places), &[("/home/u/proj/CLAUDE.md".into(), "Use pnpm.".into())]);
    assert_eq!(out["cwd"], json!(s.local));
    assert!(out["_meta"].get("acpira/remote").is_none());
    let o = &out["_meta"]["claudeCode"]["options"];
    assert_eq!(o["thinking"]["type"], "adaptive");
    assert!(o["disallowedTools"].as_array().unwrap().iter().any(|t| t == "Bash"));
    assert!(o["disallowedTools"].as_array().unwrap().iter().any(|t| t == "Monitor"));
    assert_eq!(o["toolAliases"]["Edit"], "mcp__acpira__Edit");
    assert_eq!(o["allowedTools"], json!(["mcp__acpira__Read", "mcp__acpira__Grep", "mcp__acpira__Glob"]));
    let text = out["systemPrompt"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(out["systemPrompt"]["mode"], "append");
    assert!(text.contains("SSH host gen1 (linux x86_64, shell /bin/bash), in /home/u/proj"));
    assert!(text.contains("Use pnpm."));
  }

  #[test]
  fn rows_read_as_the_native_tools() {
    let s = stand();
    let u = json!({ "sessionUpdate": "tool_call", "title": "mcp__acpira__Edit", "_meta": { "claudeCode": { "toolName": "mcp__acpira__Edit" } } });
    assert_eq!(remote_name(&u), Some("Edit"));
    assert_eq!(remote_name(&json!({ "title": "mcp__acpira__show_image" })), None);
    assert_eq!(remote_name(&json!({ "title": "Edit" })), None);
    let v = view("Edit", &json!({ "file_path": "src/a.rs", "old_string": "a", "new_string": "b" }), &s, &[]);
    assert_eq!((v.kind, v.title.as_str()), ("edit", "Edit src/a.rs"));
    assert_eq!(v.locations, vec![json!({ "path": "/home/u/proj/src/a.rs" })]);
    assert_eq!(v.diff.unwrap()["oldText"], "a");
    assert_eq!(v.input["file_path"], "/home/u/proj/src/a.rs");
    let v = view("Bash", &json!({ "command": "cargo test" }), &s, &[]);
    assert_eq!((v.kind, v.title.as_str(), v.diff), ("execute", "cargo test", None));
    let v = view("Read", &json!({ "file_path": "/Users/me/.acpira/remote/hosts/gen1/home/u/proj/README.md", "offset": 5 }), &s, &[]);
    assert_eq!(v.locations, vec![json!({ "path": "/home/u/proj/README.md", "line": 5 })]);
  }
}
