//! The project tools a stand-in CLI gets in place of its own (`host_tools::REMOTE`, served by `acpira mcp`): their MCP
//! definitions in Claude Code's argument shapes, the built-in agent's call each becomes on the host, and how their rows
//! and permission cards read as the native tools

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::StandProfile;
use super::stand::{Stand, path_arg, runs_here};

/// Words a CLI puts around the server and the tool when it names a call (`Run mcp_acpira_Read`, Devin's
/// `Calling Bash from acpira`)
const NAME_FILLERS: [&str; 6] = ["mcp", "run", "calling", "called", "call", "from"];

/// The remote tools allowed without a permission request, as the native ones are: the read-only ones, and the two that
/// only reach a background command the session started itself
pub const READ_ONLY: &[&str] = &["Read", "Grep", "Glob", "LS", "BashOutput", "KillShell"];

/// Strips extra metadata fields (`toolSummary`) that `agy acp` injects into ACP `args` / `rawInput` before sending the
/// tool's real arguments over MCP
pub fn clean_input(input: &Value) -> Value {
  let mut out = input.clone();
  if let Some(o) = out.as_object_mut() {
    o.remove("toolSummary");
  }
  out
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
    "MultiEdit" => {
      let p = path_arg(name, args).ok_or("file_path is required")?;
      out.insert("path".into(), json!(path(p)));
      // Only the replacement itself travels: the built-in edit lets an item name a file of its own, which would
      // change one the call's path (what the row shows, the permission asks for and `runs_here` decides by) never named
      let items = args.get("edits").and_then(Value::as_array).ok_or("edits is required")?;
      let edits: Vec<Value> = items
        .iter()
        .map(|item| {
          let mut edit = Map::new();
          for k in ["old_string", "new_string", "replace_all"] {
            if let Some(v) = item.get(k) {
              edit.insert(k.into(), v.clone());
            }
          }
          Value::Object(edit)
        })
        .collect();
      out.insert("edits".into(), json!(edits));
      tools::EDIT
    }
    "Bash" => {
      copy(&mut out, "command", "command");
      copy(&mut out, "timeout", "timeout");
      copy(&mut out, "run_in_background", "background");
      tools::BASH
    }
    "BashOutput" | "KillShell" => {
      let id = ["bash_id", "shell_id", "id"].iter().find_map(|k| a.get(*k).filter(|v| v.is_string() || v.is_number())).ok_or("bash_id is required")?;
      out.insert("id".into(), id.clone());
      if name == "KillShell" {
        out.insert("kill".into(), json!(true));
      } else {
        copy(&mut out, "wait", "wait");
      }
      tools::JOB
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
    "LS" => {
      if let Some(p) = path_arg(name, args) {
        out.insert("path".into(), json!(path(p)));
      }
      copy(&mut out, "ignore", "ignore");
      tools::LIST
    }
    other => return Err(format!("{other} is not one of this server's project tools")),
  };
  Ok((tool, Value::Object(out)))
}

/// A background command's start notice names the built-in agent's `job` tool (`acpira_agent::tools::jobs`): its first
/// line is said again in this session's tools. The job id is the run's `raw_output.job`
pub fn name_job_tools(out: &mut acpira_agent::tools::Output, prefix: &str) {
  let Some(id) = out.raw_output.as_ref().and_then(|r| r.get("job")).map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())) else { return };
  let rest = out.model.split_once('\n').map(|(_, rest)| rest).unwrap_or("");
  out.model = format!(
    "Started background job {id}. Read its output with {prefix}BashOutput (bash_id \"{id}\", with wait to wait for more), stop it with {prefix}KillShell (shell_id \"{id}\").\n{rest}"
  );
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
         at most 600000; for a dev server, a watcher or a long build, set run_in_background: the call returns at once with a job \
         id, BashOutput reads what the command prints and KillShell stops it."
      ),
      json!({
        "command": { "type": "string", "description": "The command line" },
        "timeout": { "type": "integer", "description": "Timeout in milliseconds" },
        "description": { "type": "string", "description": "What the command does, in a few words" },
        "run_in_background": { "type": "boolean", "description": "Start it in the background on the host and return a job id" },
      }),
      &["command"],
      false,
    ),
    def(
      "BashOutput",
      format!(
        "Read what a background command on the SSH host {alias} (Bash with run_in_background) printed since the last check, and \
         whether it is still running."
      ),
      json!({
        "bash_id": { "type": "string", "description": "The job id Bash returned" },
        "wait": { "type": "integer", "description": "Seconds to wait for new output or for the command to end (default 0, at most 120)" },
      }),
      &["bash_id"],
      true,
    ),
    def(
      "KillShell",
      format!("Stop a background command on the SSH host {alias} (Bash with run_in_background)."),
      json!({ "shell_id": { "type": "string", "description": "The job id Bash returned" } }),
      &["shell_id"],
      false,
    ),
    def(
      "MultiEdit",
      format!(
        "Make several replacements in one file on the SSH host {alias} in a single call. The edits apply in order, each to the \
         result of the one before, and all or none are made; each old_string must occur exactly once at its turn unless \
         replace_all is true. Returns the change as a diff."
      ),
      json!({
        "file_path": { "type": "string", "description": path },
        "edits": {
          "type": "array",
          "description": "The replacements, in order",
          "items": {
            "type": "object",
            "properties": {
              "old_string": { "type": "string", "description": "The exact text to replace" },
              "new_string": { "type": "string", "description": "The replacement text" },
              "replace_all": { "type": "boolean", "description": "Replace every occurrence (default false)" },
            },
            "required": ["old_string", "new_string"],
          },
        },
      }),
      &["file_path", "edits"],
      false,
    ),
    def(
      "LS",
      format!("List the files under a folder on the SSH host {alias} as an indented tree (up to 500 files, ignored files left out)."),
      json!({
        "path": { "type": "string", "description": "Folder to list; \".\" for the project folder" },
        "ignore": { "type": "array", "items": { "type": "string" }, "description": "Globs of entries to leave out" },
      }),
      // Required as the built-in agent's `list` requires it: an optional-only schema invites a call with empty input
      &["path"],
      true,
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

/// The remote tool a CLI's name for a call means. Every CLI spells a server's tool its own way (`mcp__acpira__Edit`,
/// `mcp_acpira_Edit`, `acpira___Edit`, `acpira_Edit`, `acpira: Edit`, `Calling Edit from acpira`), so the name is read
/// by its words: Acpira's server, exactly one remote tool, and nothing else but the fillers. A search for tools
/// (`Search tools: acpira Edit`) or another server's `Edit` is none, and a hyphen keeps a word whole, so the server
/// `acpira-tool` is not Acpira's. `agy` registers a server whose name its process already holds under another
/// configuration as `acpira_<8 hex>` (`mcp_acpira_9791ed97_Read`): that word right after the server's is skipped
fn tool_named(profile: &StandProfile, s: &str) -> Option<&'static str> {
  let remote = |n: &str| acpira_shared::host_tools::REMOTE.iter().find(|t| **t == n).copied();
  if let Some(n) = Some(profile.prefix.as_str()).filter(|p| !p.is_empty()).and_then(|p| s.strip_prefix(p)).and_then(remote) {
    return Some(n);
  }
  let alias = |w: &str| w.len() == 8 && w.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
  let (mut server, mut tool, mut after_server) = (false, None, false);
  for w in s.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').filter(|w| !w.is_empty()) {
    let aliased = std::mem::take(&mut after_server) && alias(w);
    if w.eq_ignore_ascii_case(crate::host_mcp::SERVER_NAME) {
      (server, after_server) = (true, true);
    } else if !aliased && !NAME_FILLERS.iter().any(|f| w.eq_ignore_ascii_case(f)) {
      if tool.is_some() {
        return None;
      }
      tool = Some(remote(w)?);
    }
  }
  tool.filter(|_| server)
}

/// Where a CLI names the tool of a call: claude-agent-acp's and Devin's `_meta`, the top-level `name` (Antigravity,
/// Droid), the model's own argument when the CLI calls MCP tools through one of its own (Grok's `use_tool`), the title
const NAME_AT: [&str; 5] = ["/_meta/claudeCode/toolName", "/_meta/cognition.ai~1toolName", "/name", "/rawInput/tool_name", "/title"];

/// The native name of a remote tool the update (or a permission request's toolCall) names (`tool_named`)
pub fn remote_name(profile: &StandProfile, u: &Value) -> Option<&'static str> {
  // An argument called `tool_name` names the call only where the model's arguments sit beside it (`call_input`)
  let wrapped = u.pointer("/rawInput/tool_input").is_some_and(Value::is_object);
  NAME_AT.iter().filter(|at| wrapped || **at != "/rawInput/tool_name").find_map(|at| tool_named(profile, u.pointer(at)?.as_str()?))
}

/// The model's arguments of a call: a CLI that calls MCP tools through a tool of its own (Grok's `use_tool`) nests them
pub fn call_input(raw: &Value) -> &Value {
  match (raw.get("tool_name"), raw.get("tool_input")) {
    (Some(Value::String(_)), Some(input)) if input.is_object() => input,
    _ => raw,
  }
}

/// How the adapter reports the native tool for this input: kind, title, locations, and a write's diffs from the input
/// (one per replacement of a MultiEdit)
pub struct NativeView {
  pub kind: &'static str,
  pub title: String,
  pub locations: Vec<Value>,
  pub diffs: Vec<Value>,
  /// The input with its path as the host knows it, for the row's target and the edit gate
  pub input: Value,
}

pub fn view(name: &str, input: &Value, stand: &Stand, data_root: &Path, roots: &[PathBuf]) -> NativeView {
  let mut input = clean_input(input);
  let here = runs_here(name, &input, stand, data_root, roots);
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
    "Edit" | "MultiEdit" => ("edit", path.as_deref().map(|p| format!("Edit {}", shown(p))).unwrap_or_else(|| "Edit".into())),
    "Bash" => ("execute", Some(s("command")).filter(|c| !c.is_empty()).unwrap_or_else(|| "Terminal".into())),
    // As the built-in agent's `job` rows read (`acpira_agent::tools::Action::presentation`)
    "BashOutput" | "KillShell" => {
      let id = ["bash_id", "shell_id", "id"].iter().find_map(|k| input.get(*k)).map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_default();
      ("execute", format!("{} job {id}", if name == "KillShell" { "Stop" } else { "Check" }))
    }
    "LS" => ("read", format!("List {}", path.as_deref().map(shown).unwrap_or_else(|| ".".into()))),
    "Grep" => ("search", format!("grep {}{}", s("pattern"), path.as_deref().map(|p| format!(" {}", shown(p))).unwrap_or_default())),
    _ => ("search", format!("Find {}{}", path.as_deref().map(|p| format!("{} ", shown(p))).unwrap_or_default(), s("pattern"))),
  };
  let locations = match (&path, name) {
    (Some(p), "Read") => vec![json!({ "path": p, "line": input.get("offset").and_then(Value::as_u64).unwrap_or(1) })],
    (Some(p), _) => vec![json!({ "path": p })],
    _ => vec![],
  };
  let diffs = match (&path, name) {
    (Some(p), "Edit") => vec![json!({ "type": "diff", "path": p, "oldText": input.get("old_string"), "newText": input.get("new_string") })],
    (Some(p), "MultiEdit") => {
      let edits = input.get("edits").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
      edits.iter().map(|e| json!({ "type": "diff", "path": p, "oldText": e.get("old_string"), "newText": e.get("new_string") })).collect()
    }
    (Some(p), "Write") => vec![json!({ "type": "diff", "path": p, "oldText": null, "newText": input.get("content") })],
    _ => vec![],
  };
  NativeView { kind, title, locations, diffs, input }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::acp::local_run::{builtin, generic};
  use std::sync::Arc;

  fn profile(id: &str) -> Arc<StandProfile> {
    builtin(id).and_then(|l| l.stand().cloned()).unwrap()
  }

  fn stand() -> Stand {
    Stand::new(profile("claude"), "gen1", "/home/u/proj/", Path::new("/Users/me/.acpira"))
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
    // The tools the first set left out: several edits at once, a folder's tree, and background commands
    let edits = json!([{ "old_string": "a", "new_string": "b" }, { "old_string": "c", "new_string": "d", "replace_all": true }]);
    let (tool, a) = builtin_call("MultiEdit", &json!({ "file_path": "src/a.rs", "edits": edits }), &s, false).unwrap();
    assert_eq!((tool, a), ("edit", json!({ "path": "/home/u/proj/src/a.rs", "edits": edits })));
    // An item naming a file of its own cannot move the edit off the call's path
    let stray = json!([{ "old_string": "a", "new_string": "b", "path": "/etc/hosts", "file_path": "/Users/me/.ssh/config" }]);
    let (_, a) = builtin_call("MultiEdit", &json!({ "file_path": "/Users/me/.claude/plans/p.md", "edits": stray }), &s, true).unwrap();
    assert_eq!(a, json!({ "path": "/Users/me/.claude/plans/p.md", "edits": [{ "old_string": "a", "new_string": "b" }] }));
    assert!(builtin_call("MultiEdit", &json!({ "file_path": "src/a.rs" }), &s, false).is_err());
    let (tool, a) = builtin_call("LS", &json!({ "path": ".", "ignore": ["target"] }), &s, false).unwrap();
    assert_eq!((tool, a), ("list", json!({ "path": "/home/u/proj/.", "ignore": ["target"] })));
    let (tool, a) = builtin_call("Bash", &json!({ "command": "pnpm dev", "run_in_background": true }), &s, false).unwrap();
    assert_eq!((tool, a), ("bash", json!({ "command": "pnpm dev", "background": true })));
    let (tool, a) = builtin_call("BashOutput", &json!({ "bash_id": "2", "wait": 5 }), &s, false).unwrap();
    assert_eq!((tool, a), ("job", json!({ "id": "2", "wait": 5 })));
    let (tool, a) = builtin_call("KillShell", &json!({ "shell_id": "2" }), &s, false).unwrap();
    assert_eq!((tool, a), ("job", json!({ "id": "2", "kill": true })));
    assert!(builtin_call("BashOutput", &json!({}), &s, false).is_err());
    // Every offered tool has a definition and a mapping, and the built-in agent takes the mapped call
    let defs = tool_defs("gen1");
    for name in acpira_shared::host_tools::REMOTE {
      assert!(defs.iter().any(|d| d["name"] == *name), "{name} has no definition");
    }
    assert_eq!(defs.len(), acpira_shared::host_tools::REMOTE.len());
  }

  #[test]
  fn a_background_command_is_read_with_this_sessions_tools() {
    let mut out = acpira_agent::tools::Output::error("x");
    out.is_error = false;
    out.model = "Started background job 3. Check it with job (id \"3\"), with wait to wait for more output.\nlistening on :3000".into();
    out.raw_output = Some(json!({ "job": "3" }));
    name_job_tools(&mut out, "mcp__acpira__");
    assert!(out.model.starts_with("Started background job 3. Read its output with mcp__acpira__BashOutput (bash_id \"3\""), "{}", out.model);
    assert!(out.model.ends_with("\nlistening on :3000") && !out.model.contains("with job ("), "{}", out.model);
    // A foreground command's output is left as it is
    let mut plain = acpira_agent::tools::Output::error("boom");
    name_job_tools(&mut plain, "mcp__acpira__");
    assert_eq!(plain.model, "boom");
  }

  #[test]
  fn rows_read_as_the_native_tools() {
    let data = Path::new("/Users/me/.acpira");
    let s = stand();
    let claude = profile("claude");
    let u = json!({ "sessionUpdate": "tool_call", "title": "mcp__acpira__Edit", "_meta": { "claudeCode": { "toolName": "mcp__acpira__Edit" } } });
    assert_eq!(remote_name(&claude, &u), Some("Edit"));
    assert_eq!(remote_name(&claude, &json!({ "name": "mcp_acpira_Read", "title": "Run mcp_acpira_Read" })), Some("Read"));
    assert_eq!(remote_name(&claude, &json!({ "title": "Run mcp_acpira_Bash" })), Some("Bash"));
    // agy's name for the server of a second session request in one process
    let agy = profile("antigravity");
    assert_eq!(remote_name(&agy, &json!({ "name": "mcp_acpira_9791ed97_Glob", "title": "Run mcp_acpira_9791ed97_Glob" })), Some("Glob"));
    assert_eq!(remote_name(&agy, &json!({ "title": "Run mcp_acpira_9791ed97_Bash" })), Some("Bash"));
    assert_eq!(remote_name(&agy, &json!({ "name": "mcp_acpira_9791ed97_show_image" })), None);
    assert_eq!(remote_name(&agy, &json!({ "name": "mcp_blockbench_9defe6f0_Read" })), None);
    assert_eq!(remote_name(&agy, &json!({ "name": "mcp_acpira_Read_9791ed97" })), None, "only right after the server");
    assert_eq!(remote_name(&claude, &json!({ "title": "mcp__acpira__show_image" })), None);
    assert_eq!(remote_name(&claude, &json!({ "name": "mcp_acpira_show_image", "title": "Run mcp_acpira_show_image" })), None);
    assert_eq!(remote_name(&claude, &json!({ "title": "Edit" })), None);
    // A custom agent's own prefix is read too, and an empty one never makes a bare native name a remote tool
    let mut mine = generic("mine", "Mine");
    mine.prefix = "acpira_".into();
    assert_eq!(remote_name(&mine, &json!({ "title": "acpira_Grep" })), Some("Grep"));
    mine.prefix = String::new();
    assert_eq!(remote_name(&mine, &json!({ "title": "Grep" })), None);
    // Each CLI's own spelling: Droid, OpenCode, Devin, Grok's wrapper
    assert_eq!(remote_name(&mine, &json!({ "title": "acpira: Bash", "name": "acpira___Bash" })), Some("Bash"));
    assert_eq!(remote_name(&mine, &json!({ "title": "acpira_Write" })), Some("Write"));
    assert_eq!(remote_name(&mine, &json!({ "title": "Calling Edit from acpira" })), Some("Edit"));
    assert_eq!(remote_name(&mine, &json!({ "title": "Called Edit from acpira" })), Some("Edit"));
    let wrapped = json!({ "title": "use_tool", "rawInput": { "tool_name": "acpira__MultiEdit", "tool_input": { "file_path": "a" } } });
    assert_eq!(remote_name(&mine, &wrapped), Some("MultiEdit"));
    assert_eq!(call_input(&wrapped["rawInput"]), &json!({ "file_path": "a" }));
    assert_eq!(call_input(&json!({ "file_path": "a" })), &json!({ "file_path": "a" }));
    // A search for the tools, another server's tool of the same name and a native tool are none
    assert_eq!(remote_name(&mine, &json!({ "title": "Search tools: \"acpira Edit\"" })), None);
    assert_eq!(remote_name(&mine, &json!({ "title": "Search tools: acpira___Bash, acpira___Write" })), None);
    assert_eq!(remote_name(&mine, &json!({ "title": "github__Read" })), None);
    assert_eq!(remote_name(&mine, &json!({ "title": "mcp__acpira-tool__Read" })), None);
    assert_eq!(remote_name(&mine, &json!({ "name": "lookup", "rawInput": { "tool_name": "acpira__Bash" } })), None);
    assert_eq!(remote_name(&mine, &json!({ "title": "Listed MCP tools for acpira" })), None);
    assert_eq!(remote_name(&mine, &json!({ "title": "mcp__acpira__read_session" })), None);
    let v = view("Edit", &json!({ "file_path": "src/a.rs", "old_string": "a", "new_string": "b", "toolSummary": "Edit a.rs" }), &s, data, &[]);
    assert_eq!((v.kind, v.title.as_str()), ("edit", "Edit src/a.rs"));
    assert_eq!(v.locations, vec![json!({ "path": "/home/u/proj/src/a.rs" })]);
    assert_eq!(v.diffs[0]["oldText"], "a");
    assert_eq!(v.input["file_path"], "/home/u/proj/src/a.rs");
    assert!(v.input.get("toolSummary").is_none());
    let v = view("Bash", &json!({ "command": "cargo test" }), &s, data, &[]);
    assert_eq!((v.kind, v.title.as_str(), v.diffs.len()), ("execute", "cargo test", 0));
    let v = view("MultiEdit", &json!({ "file_path": "src/a.rs", "edits": [{ "old_string": "a", "new_string": "b" }, { "old_string": "c", "new_string": "d" }] }), &s, data, &[]);
    assert_eq!((v.kind, v.title.as_str(), v.diffs.len()), ("edit", "Edit src/a.rs", 2));
    assert_eq!(v.diffs[1], json!({ "type": "diff", "path": "/home/u/proj/src/a.rs", "oldText": "c", "newText": "d" }));
    let v = view("LS", &json!({ "path": "src" }), &s, data, &[]);
    assert_eq!((v.kind, v.title.as_str()), ("read", "List src"));
    assert_eq!(view("BashOutput", &json!({ "bash_id": "2" }), &s, data, &[]).title, "Check job 2");
    assert_eq!(view("KillShell", &json!({ "shell_id": "2" }), &s, data, &[]).title, "Stop job 2");
    let v = view("Read", &json!({ "file_path": "/Users/me/.acpira/remote/hosts/gen1/home/u/proj/README.md", "offset": 5 }), &s, data, &[]);
    assert_eq!(v.locations, vec![json!({ "path": "/home/u/proj/README.md", "line": 5 })]);
  }
}
