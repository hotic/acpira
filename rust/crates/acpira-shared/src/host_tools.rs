//! The tools of the engine's own MCP server (`acpira mcp`, `acpira-host/src/host_mcp.rs`), named here so the built-in
//! agent's permission rules can list them one by one: a tool a plugin adds to the same server is not one of them and
//! asks like any other MCP tool (`acpira-agent/src/permission.rs`). No TypeScript counterpart

/// Server name in `mcpServers`; adapters prefix the tool with it (`mcp__acpira__show_image`, `mcp.acpira.show_image`)
pub const SERVER: &str = "acpira";
pub const SHOW_IMAGE: &str = "show_image";
pub const SHOW_VIDEO: &str = "show_video";
pub const SHOW_WIDGET: &str = "show_widget";
pub const WORKSPACE_HOOKS: &str = "workspace_hooks";
pub const OPEN_BROWSER: &str = "open_browser";
pub const SEARCH_SESSIONS: &str = "search_sessions";
pub const READ_SESSION: &str = "read_session";
pub const ASK_AGENT: &str = "ask_agent";
pub const ASK_CALLER: &str = "ask_caller";
/// The plugin tools of the server itself (`docs/dev/plugins.md`): the contract, and an activation the user confirms
pub const PLUGIN_GUIDE: &str = "plugin_guide";
pub const PLUGIN_ACTIVATE: &str = "plugin_activate";

/// Computer use (`acpira-host/src/computer.rs`): the running apps, a window as an outline, steps in a window, a
/// picture of a window. The engine asks the user once per app before any of them touches it
pub const COMPUTER_APPS: &str = "computer_apps";
pub const COMPUTER_OBSERVE: &str = "computer_observe";
pub const COMPUTER_ACT: &str = "computer_act";
pub const COMPUTER_SEE: &str = "computer_see";
/// The computer-use tools. Not in `ALL`: they act outside the workspace, so the built-in agent asks for them like for
/// any other MCP tool (the ones that only read carry `readOnlyHint`)
pub const COMPUTER: &[&str] = &[COMPUTER_APPS, COMPUTER_OBSERVE, COMPUTER_ACT, COMPUTER_SEE];

/// Every tool the engine itself serves; a name outside it on the same server belongs to a plugin
pub const ALL: &[&str] = &[SHOW_IMAGE, SHOW_VIDEO, SHOW_WIDGET, WORKSPACE_HOOKS, OPEN_BROWSER, SEARCH_SESSIONS, READ_SESSION, ASK_AGENT, ASK_CALLER, PLUGIN_GUIDE, PLUGIN_ACTIVATE];

/// The tool groups the plugin page can switch off for every agent (a built-in plugin each). The rest has no switch:
/// `workspace_hooks` belongs to the hooks, `ask_caller` to a round already summoned, the plugin tools to the kernel
pub const GROUPS: &[(&str, &[&str])] = &[
  ("summon", &[ASK_AGENT]),
  ("sessions", &[SEARCH_SESSIONS, READ_SESSION]),
  ("display", &[SHOW_IMAGE, SHOW_VIDEO, SHOW_WIDGET]),
  ("browser", &[OPEN_BROWSER]),
  (COMPUTER_GROUP, COMPUTER),
];
pub const COMPUTER_GROUP: &str = "computer";

/// The groups that stay off until the user switches them on
pub const OFF_BY_DEFAULT: &[&str] = &[COMPUTER_GROUP];

/// Whether `group` is on when the user never touched its switch
pub fn group_default(group: &str) -> bool {
  !OFF_BY_DEFAULT.contains(&group)
}

/// The group `tool` belongs to, if it has one
pub fn group_of(tool: &str) -> Option<&'static str> {
  GROUPS.iter().find(|(_, tools)| tools.contains(&tool)).map(|(g, _)| *g)
}

/// The project tools a Claude Code session in a folder on an SSH host gets under local run, named after the native tools
/// they stand in for (`mcp__acpira__Read` …): they run on the host, while Claude's own are switched off
/// (`acpira-host/src/acp/vendors/claude_remote.rs`). Offered only to such a session, so not in `ALL`
pub const REMOTE: &[&str] = &["Read", "Write", "Edit", "MultiEdit", "Bash", "BashOutput", "KillShell", "Grep", "Glob", "LS"];

/// Built-in agent identifier (`acpira agent`).
pub const SELF_AGENT_ID: &str = "acpira";

/// Whether `tool` is one the engine serves itself
pub fn is_engine_tool(tool: &str) -> bool {
  ALL.contains(&tool) || REMOTE.contains(&tool) || COMPUTER.contains(&tool)
}

pub fn video_mime_of(path: &str) -> Option<&'static str> {
  match std::path::Path::new(path).extension()?.to_str()?.to_ascii_lowercase().as_str() {
    "mp4" => Some("video/mp4"),
    "webm" => Some("video/webm"),
    _ => None,
  }
}

pub fn widget_mime_of(path: &str) -> Option<&'static str> {
  let tail = path.rsplit(['/', '\\']).next().unwrap_or(path);
  let ext = tail.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())?;
  matches!(ext.as_str(), "html" | "htm").then_some("text/html")
}

/// Add the host's entry to a session's `mcpServers`, dropping any user/project entry with the same reserved server name.
pub fn push_entry(servers: &mut Vec<serde_json::Value>, entry: Option<serde_json::Value>) -> bool {
  let Some(entry) = entry else { return false };
  let before = servers.len();
  servers.retain(|s| s.get("name").and_then(serde_json::Value::as_str) != Some(SERVER));
  let shadowed = servers.len() < before;
  servers.push(entry);
  shadowed
}

/// True when a session request's `mcpServers` carries the host's own entry.
pub fn has_server(req: &serde_json::Value) -> bool {
  req.get("mcpServers").and_then(serde_json::Value::as_array).is_some_and(|l| l.iter().any(is_host_entry))
}

/// The same request without the host's entry (for an agent that refuses it).
pub fn without_server(mut req: serde_json::Value) -> serde_json::Value {
  if let Some(list) = req.get_mut("mcpServers").and_then(serde_json::Value::as_array_mut) {
    list.retain(|s| !is_host_entry(s));
  }
  req
}

pub fn is_host_entry(s: &serde_json::Value) -> bool {
  s.get("name").and_then(serde_json::Value::as_str) == Some(SERVER)
    && s.get("args").and_then(serde_json::Value::as_array).is_some_and(|a| a.first().and_then(serde_json::Value::as_str) == Some("mcp"))
}
