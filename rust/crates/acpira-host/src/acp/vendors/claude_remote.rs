//! Claude Code and Antigravity in a folder on an SSH host under local run (docs/dev/remote.md). The CLI runs here, on
//! this machine's login, in a stand-in folder under the data root, and the model gets Acpira's project tools
//! (`host_tools::REMOTE`, served by `acpira mcp`), which run on the host through its helper with the built-in agent's
//! tool code. Claude switches its native tools off via `_meta.claudeCode.options.disallowedTools` and aliases them onto
//! `mcp__acpira__<Name>`; Antigravity denies its built-in workspace and shell tools via `<stand>/.agents/hooks.json`
//! `PreToolUse` (except its own `~/.gemini` files) and calls `mcp_acpira_<Name>`. A path under the stand-in means the
//! host folder; the CLI's own files on this machine stay here. Rows and permission cards show the calls as native tools

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// How claude-agent-acp names a tool of Acpira's server
pub const PREFIX: &str = "mcp__acpira__";
/// How Antigravity (`agy acp`) names a tool of Acpira's server
pub const ANTIGRAVITY_PREFIX: &str = "mcp_acpira_";

/// Which external CLI is running in a local stand-in for an SSH host folder
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
  Claude,
  Antigravity,
}

impl Dialect {
  pub fn label(self) -> &'static str {
    match self {
      Dialect::Claude => "Claude Code",
      Dialect::Antigravity => "Antigravity",
    }
  }

  pub fn prefix(self) -> &'static str {
    match self {
      Dialect::Claude => PREFIX,
      Dialect::Antigravity => ANTIGRAVITY_PREFIX,
    }
  }

  pub fn instruction_files(self) -> &'static [&'static str] {
    match self {
      Dialect::Claude => &["CLAUDE.md", "AGENTS.md"],
      Dialect::Antigravity => &["GEMINI.md", "AGENTS.md"],
    }
  }

  fn dir_name(self) -> &'static str {
    match self {
      // Kept empty (`docs/dev/remote.md`)
      Dialect::Claude => "hosts",
      Dialect::Antigravity => "antigravity",
    }
  }
}

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

/// Antigravity's built-in shell, search and directory tools that must never run on this machine for an SSH folder
const AGY_SHELL_SEARCH_TOOLS: &[&str] = &[
  "run_command",
  "command_status",
  "send_command_input",
  "grep_search",
  "find",
  "find_by_name",
  "list_dir",
  "list_directory",
  "delete_directory",
  "move",
];

/// Antigravity's built-in file tools: blocked except on its own `~/.gemini` skills, artifacts and scratch files
const AGY_FILE_TOOLS: &[&str] = &[
  "read_file",
  "view_file",
  "view_file_outline",
  "view_code_item",
  "write_file",
  "write_to_file",
  "edit_file",
  "replace_file_content",
  "multi_replace_file_content",
  "code_action",
];

/// The remote tools allowed without a permission request, as the native read-only ones are
pub const READ_ONLY: &[&str] = &["Read", "Grep", "Glob"];

/// A session folder on an SSH host and its stand-in folder here
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stand {
  pub dialect: Dialect,
  pub alias: String,
  /// The folder on the host, absolute
  pub host: String,
  /// Where the CLI runs here: `<data root>/remote/<hosts|antigravity>/<alias>/<host path>`
  pub local: PathBuf,
}

impl Stand {
  pub fn new(alias: &str, host: &str, data_root: &Path) -> Stand {
    Self::for_dialect(Dialect::Claude, alias, host, data_root)
  }

  pub fn for_dialect(dialect: Dialect, alias: &str, host: &str, data_root: &Path) -> Stand {
    let host = if host.len() > 1 { host.trim_end_matches('/') } else { host };
    let mut local = data_root.join("remote").join(dialect.dir_name()).join(alias);
    let rel = host.trim_start_matches('/');
    if !rel.is_empty() {
      local.push(rel);
    }
    Stand { dialect, alias: alias.to_owned(), host: host.to_owned(), local }
  }

  /// Makes the stand-in. A folder on the way that its owner cannot enter or write (a stray `chmod` on `<data root>/remote`
  /// left every session there failing with `spawn EACCES`) is opened to its owner and the stand-in made again:
  /// everything from `<data root>/remote` down is the engine's own. Antigravity's stand-in also carries `.agents/hooks.json`
  /// so `agy acp` denies built-in local workspace and shell tools before running them
  pub fn make(&self, data_root: &Path) -> std::io::Result<()> {
    match std::fs::create_dir_all(&self.local) {
      Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
        self.open_to_owner(data_root);
        std::fs::create_dir_all(&self.local)?;
      }
      Err(e) => return Err(e),
      Ok(()) => {}
    }
    if self.dialect == Dialect::Antigravity {
      write_antigravity_hooks(&self.local)?;
    }
    Ok(())
  }

  /// Top down, so a folder closed inside a closed one is reached once its parent is open
  #[cfg(unix)]
  fn open_to_owner(&self, data_root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(rest) = self.local.strip_prefix(data_root) else { return };
    let mut dir = data_root.to_path_buf();
    for part in rest.components() {
      dir.push(part);
      let Ok(meta) = std::fs::symlink_metadata(&dir) else { return };
      let mode = meta.permissions().mode();
      if meta.is_dir() && mode & 0o700 != 0o700 {
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode | 0o700));
      }
    }
  }

  #[cfg(not(unix))]
  fn open_to_owner(&self, _data_root: &Path) {}

  /// A path the model wrote, as the host knows it: under the stand-in it moves to the host folder, a relative one is
  /// read against the host folder, anything else is the host's already
  pub fn host_path(&self, p: &str) -> String {
    let base = self.host.trim_end_matches('/');
    // By components, not by string: on Windows the stand-in and the paths the CLI writes use either separator
    if let Ok(rest) = Path::new(p).strip_prefix(&self.local) {
      let parts: Vec<_> = rest.components().map(|c| c.as_os_str().to_string_lossy()).collect();
      return if parts.is_empty() { self.host.clone() } else { format!("{base}/{}", parts.join("/")) };
    }
    if p.starts_with('/') || Path::new(p).is_absolute() {
      return p.to_owned();
    }
    format!("{base}/{}", p.trim_start_matches("./"))
  }

  /// Whether `p` names the stand-in or something in it
  pub fn in_stand(&self, p: &str) -> bool {
    Path::new(p).starts_with(&self.local)
  }
}

/// Resolves a path from any stand-in of `(alias, host)` (Claude or Antigravity) or relative to `host`, for `openFile`
/// and `stageFile` where only the session's folder is known
pub fn resolve_host_path(alias: &str, host: &str, data_root: &Path, p: &str) -> String {
  for dialect in [Dialect::Claude, Dialect::Antigravity] {
    let stand = Stand::for_dialect(dialect, alias, host, data_root);
    if stand.in_stand(p) {
      return stand.host_path(p);
    }
  }
  Stand::new(alias, host, data_root).host_path(p)
}

fn write_antigravity_hooks(local: &Path) -> std::io::Result<()> {
  let dir = local.join(".agents");
  std::fs::create_dir_all(&dir)?;
  let file = dir.join("hooks.json");
  let shell_matcher = AGY_SHELL_SEARCH_TOOLS.join("|");
  let file_matcher = AGY_FILE_TOOLS.join("|");
  #[cfg(unix)]
  let (shell_cmd, file_cmd) = (
    r#"printf '%s\n' '{"decision":"deny","reason":"This workspace is on the remote SSH host. Use mcp_acpira_Bash, mcp_acpira_Grep, or mcp_acpira_Glob instead."}'"#,
    r#"in=$(cat); case "$in" in */.gemini/*|*\\.gemini\\*) exit 0 ;; esac; printf '%s\n' '{"decision":"deny","reason":"This workspace is on the remote SSH host. Use mcp_acpira_Read, mcp_acpira_Write, or mcp_acpira_Edit instead."}'"#,
  );
  #[cfg(not(unix))]
  let (shell_cmd, file_cmd) = (
    r#"powershell -NoProfile -Command "[Console]::Out.WriteLine('{\"decision\":\"deny\",\"reason\":\"This workspace is on the remote SSH host. Use mcp_acpira_Bash, mcp_acpira_Grep, or mcp_acpira_Glob instead.\"}')""#,
    r#"powershell -NoProfile -Command "$s=[Console]::In.ReadToEnd(); if ($s -match '[\\/]\.gemini[\\/]') { exit 0 }; [Console]::Out.WriteLine('{\"decision\":\"deny\",\"reason\":\"This workspace is on the remote SSH host. Use mcp_acpira_Read, mcp_acpira_Write, or mcp_acpira_Edit instead.\"}')""#,
  );
  let doc = json!({
    "hooks": {
      "PreToolUse": [
        { "matcher": shell_matcher, "hooks": [{ "type": "command", "command": shell_cmd }] },
        { "matcher": file_matcher, "hooks": [{ "type": "command", "command": file_cmd }] },
      ]
    }
  });
  let bytes = serde_json::to_vec_pretty(&doc).unwrap_or_default();
  if std::fs::read(&file).ok().as_deref() == Some(bytes.as_slice()) {
    return Ok(());
  }
  std::fs::write(&file, bytes)
}

/// The folders of this machine whose files the CLI's tools reach for and must find here: its config folder (plans,
/// memory, artifacts, saved tool outputs) and Acpira's data root. Claude's temp folder (`/tmp/claude-<uid>`) is matched
/// by name
pub fn here_roots(dialect: Dialect, home: &Path, data_root: &Path) -> Vec<PathBuf> {
  match dialect {
    Dialect::Claude => {
      let mut out = vec![home.join(".claude"), data_root.to_path_buf()];
      if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        out.push(PathBuf::from(dir));
      }
      out
    }
    Dialect::Antigravity => vec![home.join(".gemini"), data_root.to_path_buf()],
  }
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

/// Whether a call runs on this machine: a file tool whose path is one of the CLI's own files here (`here_roots`, or
/// Claude's temp folder). Anything under `<data_root>/remote` is stand-in space, never a local file; commands always
/// run on the host
pub fn runs_here(name: &str, args: &Value, stand: &Stand, data_root: &Path, roots: &[PathBuf]) -> bool {
  let Some(p) = path_arg(name, args).filter(|p| p.starts_with('/') || Path::new(p).is_absolute()) else { return false };
  if stand.in_stand(p) || Path::new(p).starts_with(data_root.join("remote")) {
    return false;
  }
  (stand.dialect == Dialect::Claude && claude_temp(p)) || roots.iter().any(|r| Path::new(p).starts_with(r))
}

/// Whether a local write (`runs_here`) targets the CLI's own plan / artifact folder, which its native tools write
/// without showing a permission card
pub fn is_agent_own_write(dialect: Dialect, path: &str, home: &Path, roots: &[PathBuf]) -> bool {
  let p = Path::new(path);
  match dialect {
    // Claude's config folders are the roots except `data_root` at index 1 (`here_roots`)
    Dialect::Claude => roots.iter().enumerate().filter(|(i, _)| *i != 1).any(|(_, r)| p.starts_with(r.join("plans"))),
    Dialect::Antigravity => {
      let gemini = home.join(".gemini");
      p.starts_with(gemini.join("antigravity-cli").join("brain")) || p.starts_with(gemini.join("antigravity").join("brain"))
    }
  }
}

/// Defense in depth alongside `.agents/hooks.json`: if Antigravity sends `session/request_permission` for one of its
/// built-in local workspace or shell tools outside `~/.gemini`, deny it even under synthesized yolo
pub fn is_blocked_local_tool(stand: &Stand, tc: &Value, home: &Path, data_root: &Path) -> bool {
  if stand.dialect != Dialect::Antigravity || remote_name(tc).is_some() {
    return false;
  }
  let raw_name = tc
    .get("name")
    .and_then(Value::as_str)
    .or_else(|| tc.get("title").and_then(Value::as_str))
    .map(|s| s.strip_prefix("Run ").unwrap_or(s))
    .unwrap_or("");
  if raw_name.starts_with("mcp_") {
    return false;
  }
  if AGY_SHELL_SEARCH_TOOLS.contains(&raw_name) {
    return true;
  }
  if !AGY_FILE_TOOLS.contains(&raw_name) {
    return false;
  }
  let input = tc.get("rawInput").or_else(|| tc.get("args"));
  let path = ["Path", "TargetFile", "DirectoryPath", "SearchPath", "path", "file_path"]
    .iter()
    .find_map(|k| input.and_then(|v| v.get(*k)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()));
  let Some(p) = path else { return true };
  if stand.in_stand(p) || Path::new(p).starts_with(data_root.join("remote")) {
    return true;
  }
  !Path::new(p).starts_with(home.join(".gemini"))
}

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

/// The session request of a Claude Code or Antigravity session in a folder on an SSH host: the stand-in as its cwd,
/// Claude's native tools switched off and aliased to the remote ones, and a system prompt section that says where the
/// project is, with the host's instructions file (`instructions`: path and text) since the CLI cannot find it from here
pub fn with_stand(mut req: Value, stand: &Stand, places: Option<&Value>, instructions: &[(String, String)]) -> Value {
  req["cwd"] = json!(stand.local);
  if let Some(meta) = req.get_mut("_meta").and_then(Value::as_object_mut) {
    meta.remove(crate::acp::session::remote::META);
  }
  let text = system_note(stand, places, instructions);
  match stand.dialect {
    Dialect::Claude => {
      let options = &mut req["_meta"]["claudeCode"]["options"];
      let mut disallowed: Vec<Value> = options.get("disallowedTools").and_then(Value::as_array).cloned().unwrap_or_default();
      disallowed.extend(DISALLOWED.iter().map(|t| json!(t)));
      options["disallowedTools"] = json!(disallowed);
      let aliases: Map<String, Value> = acpira_shared::host_tools::REMOTE.iter().map(|t| ((*t).to_owned(), json!(format!("{PREFIX}{t}")))).collect();
      options["toolAliases"] = Value::Object(aliases);
      let mut allowed: Vec<Value> = options.get("allowedTools").and_then(Value::as_array).cloned().unwrap_or_default();
      allowed.extend(READ_ONLY.iter().map(|t| json!(format!("{PREFIX}{t}"))));
      options["allowedTools"] = json!(allowed);
      req["systemPrompt"] = json!({ "mode": "append", "content": [{ "type": "text", "text": text }] });
    }
    Dialect::Antigravity => {
      let sp = &mut req["_meta"]["antigravity"]["systemPrompt"];
      let next = match sp.get("append").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(prev) => format!("{prev}\n\n{text}"),
        None => text,
      };
      sp["append"] = json!(next);
    }
  }
  req
}

fn system_note(stand: &Stand, places: Option<&Value>, instructions: &[(String, String)]) -> String {
  let s = |k: &str| places.and_then(|p| p.get(k)).and_then(Value::as_str).filter(|v| !v.is_empty());
  let platform = match (s("os"), s("arch"), s("shell")) {
    (Some(os), Some(arch), Some(shell)) => format!(" ({os} {arch}, shell {shell})"),
    _ => String::new(),
  };
  let prefix = stand.dialect.prefix();
  let own = match stand.dialect {
    Dialect::Claude => {
      "Your own files on this machine (plans, memory, saved outputs) stay here, and the same tools reach them."
    }
    Dialect::Antigravity => {
      "Built-in local file, search and command tools (read_file, write_file, edit_file, grep_search, find_by_name, \
       list_dir, run_command) target this machine and are blocked for project files; always call the mcp_acpira_* \
       tools above instead. Your own artifacts, scratch files and skills under ~/.gemini stay on this machine."
    }
  };
  let mut out = format!(
    "# Remote project\n\nThis session's project is on the SSH host {alias}{platform}, in {host}. Your working directory {local} is an \
     empty stand-in on this machine for that folder: it holds none of the project's files, and it is not a git repository even when \
     the project is one. The project tools {prefix}Read, {prefix}Write, {prefix}Edit, {prefix}Bash, {prefix}Grep and {prefix}Glob run \
     on {alias}: a relative path resolves against {host}, a path under the stand-in means the same file there, and commands (git, \
     builds, tests) run there. Write paths as {host}/… {own}",
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
/// `_meta.claudeCode.toolName`, its top-level `name`, else its title (`mcp__acpira__<Tool>`, `mcp_acpira_<Tool>`, or
/// Antigravity's `Run mcp_acpira_<Tool>`)
pub fn remote_name(u: &Value) -> Option<&'static str> {
  let named = |s: Option<&str>| {
    let s = s?.strip_prefix("Run ").unwrap_or(s?);
    let n = s.strip_prefix(PREFIX).or_else(|| s.strip_prefix(ANTIGRAVITY_PREFIX))?;
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

  fn agy_stand() -> Stand {
    Stand::for_dialect(Dialect::Antigravity, "gen1", "/home/u/proj/", Path::new("/Users/me/.acpira"))
  }

  #[cfg(unix)]
  #[test]
  fn a_closed_folder_on_the_way_is_opened_for_the_stand_in() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let remote = root.path().join("remote");
    std::fs::create_dir_all(remote.join("hosts")).unwrap();
    std::fs::set_permissions(&remote, std::fs::Permissions::from_mode(0o000)).unwrap();
    let s = Stand::new("gen1", "/mnt/proj", root.path());
    s.make(root.path()).unwrap();
    assert!(s.local.is_dir());
    assert!(!s.local.join(".agents").exists());
    assert_eq!(std::fs::metadata(&remote).unwrap().permissions().mode() & 0o777, 0o700);
  }

  #[test]
  fn antigravity_stand_in_writes_pre_tool_use_hooks_keeping_claude_empty() {
    let root = tempfile::tempdir().unwrap();
    let s = Stand::for_dialect(Dialect::Antigravity, "gen1", "/mnt/proj", root.path());
    s.make(root.path()).unwrap();
    let hooks: Value = serde_json::from_slice(&std::fs::read(s.local.join(".agents/hooks.json")).unwrap()).unwrap();
    let rules = hooks["hooks"]["PreToolUse"].as_array().unwrap();
    assert_eq!(rules.len(), 2);
    assert!(rules[0]["matcher"].as_str().unwrap().contains("run_command"));
    assert!(rules[1]["matcher"].as_str().unwrap().contains("read_file"));
  }

  #[test]
  fn the_stand_in_maps_to_the_host_folder() {
    let data = Path::new("/Users/me/.acpira");
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
    assert_eq!(resolve_host_path("gen1", "/home/u/proj", data, "/Users/me/.acpira/remote/antigravity/gen1/home/u/proj/src/lib.rs"), "/home/u/proj/src/lib.rs");
  }

  #[test]
  fn clis_own_files_stay_here_and_the_rest_goes_to_the_host() {
    let data = Path::new("/Users/me/.acpira");
    let home = Path::new("/Users/me");
    let s = stand();
    let roots = here_roots(Dialect::Claude, home, data);
    let here = |name: &str, args: Value| runs_here(name, &args, &s, data, &roots);
    assert!(here("Write", json!({ "file_path": "/Users/me/.claude/plans/p.md", "content": "x" })));
    assert!(here("Read", json!({ "file_path": "/private/tmp/claude-501/out.txt" })));
    assert!(!here("Read", json!({ "file_path": "/Users/me/.acpira/remote/hosts/gen1/home/u/proj/a.rs" })));
    assert!(!here("Read", json!({ "file_path": "src/a.rs" })));
    assert!(!here("Bash", json!({ "command": "cat /Users/me/.claude/x" })));
    assert!(!here("Grep", json!({ "pattern": "x" })));

    let agy = agy_stand();
    let agy_roots = here_roots(Dialect::Antigravity, home, data);
    assert!(runs_here("Write", &json!({ "file_path": "/Users/me/.gemini/antigravity-cli/brain/id/plan.md", "content": "x" }), &agy, data, &agy_roots));
    assert!(is_agent_own_write(Dialect::Antigravity, "/Users/me/.gemini/antigravity-cli/brain/id/plan.md", home, &agy_roots));
    assert!(!runs_here("Read", &json!({ "file_path": "/Users/me/.acpira/remote/antigravity/gen1/home/u/proj/a.rs" }), &agy, data, &agy_roots));
    assert!(is_blocked_local_tool(&agy, &json!({ "name": "run_command", "rawInput": { "CommandLine": "ls" } }), home, data));
    assert!(is_blocked_local_tool(&agy, &json!({ "name": "read_file", "rawInput": { "Path": "/etc/hosts" } }), home, data));
    assert!(!is_blocked_local_tool(&agy, &json!({ "name": "read_file", "rawInput": { "Path": "/Users/me/.gemini/skills/a/SKILL.md" } }), home, data));
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

    let agy = agy_stand();
    let agy_req = json!({ "cwd": "/home/u/proj", "_meta": { "acpira/remote": { "alias": "gen1" } } });
    let agy_out = with_stand(agy_req, &agy, Some(&places), &[("/home/u/proj/GEMINI.md".into(), "Use cargo.".into())]);
    assert_eq!(agy_out["cwd"], json!(agy.local));
    assert!(agy_out["_meta"].get("acpira/remote").is_none());
    let agy_text = agy_out["_meta"]["antigravity"]["systemPrompt"]["append"].as_str().unwrap();
    assert!(agy_text.contains("mcp_acpira_Read"));
    assert!(agy_text.contains("Use cargo."));
  }

  #[test]
  fn rows_read_as_the_native_tools() {
    let data = Path::new("/Users/me/.acpira");
    let s = stand();
    let u = json!({ "sessionUpdate": "tool_call", "title": "mcp__acpira__Edit", "_meta": { "claudeCode": { "toolName": "mcp__acpira__Edit" } } });
    assert_eq!(remote_name(&u), Some("Edit"));
    assert_eq!(remote_name(&json!({ "name": "mcp_acpira_Read", "title": "Run mcp_acpira_Read" })), Some("Read"));
    assert_eq!(remote_name(&json!({ "title": "Run mcp_acpira_Bash" })), Some("Bash"));
    assert_eq!(remote_name(&json!({ "title": "mcp__acpira__show_image" })), None);
    assert_eq!(remote_name(&json!({ "name": "mcp_acpira_show_image", "title": "Run mcp_acpira_show_image" })), None);
    assert_eq!(remote_name(&json!({ "title": "Edit" })), None);
    let v = view("Edit", &json!({ "file_path": "src/a.rs", "old_string": "a", "new_string": "b", "toolSummary": "Edit a.rs" }), &s, data, &[]);
    assert_eq!((v.kind, v.title.as_str()), ("edit", "Edit src/a.rs"));
    assert_eq!(v.locations, vec![json!({ "path": "/home/u/proj/src/a.rs" })]);
    assert_eq!(v.diff.unwrap()["oldText"], "a");
    assert_eq!(v.input["file_path"], "/home/u/proj/src/a.rs");
    assert!(v.input.get("toolSummary").is_none());
    let v = view("Bash", &json!({ "command": "cargo test" }), &s, data, &[]);
    assert_eq!((v.kind, v.title.as_str(), v.diff), ("execute", "cargo test", None));
    let v = view("Read", &json!({ "file_path": "/Users/me/.acpira/remote/hosts/gen1/home/u/proj/README.md", "offset": 5 }), &s, data, &[]);
    assert_eq!(v.locations, vec![json!({ "path": "/home/u/proj/README.md", "line": 5 })]);
  }
}
