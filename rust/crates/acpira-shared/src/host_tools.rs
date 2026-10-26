//! The tools of the engine's own MCP server (`acpira mcp`, `acpira-host/src/host_mcp.rs`), named here so the built-in
//! agent's permission rules can list them one by one: a tool a plugin adds to the same server is not one of them and
//! asks like any other MCP tool (`acpira-agent/src/permission.rs`). No TypeScript counterpart

/// Server name in `mcpServers`; adapters prefix the tool with it (`mcp__acpira__show_image`, `mcp.acpira.show_image`)
pub const SERVER: &str = "acpira";
pub const SHOW_IMAGE: &str = "show_image";
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

/// Every tool the engine itself serves; a name outside it on the same server belongs to a plugin
pub const ALL: &[&str] = &[SHOW_IMAGE, SHOW_WIDGET, WORKSPACE_HOOKS, OPEN_BROWSER, SEARCH_SESSIONS, READ_SESSION, ASK_AGENT, ASK_CALLER, PLUGIN_GUIDE, PLUGIN_ACTIVATE];

/// The tool groups the plugin page can switch off for every agent (a built-in plugin each). The rest has no switch:
/// `workspace_hooks` belongs to the hooks, `ask_caller` to a round already summoned, the plugin tools to the kernel
pub const GROUPS: &[(&str, &[&str])] =
  &[("summon", &[ASK_AGENT]), ("sessions", &[SEARCH_SESSIONS, READ_SESSION]), ("display", &[SHOW_IMAGE, SHOW_WIDGET]), ("browser", &[OPEN_BROWSER])];

/// The group `tool` belongs to, if it has one
pub fn group_of(tool: &str) -> Option<&'static str> {
  GROUPS.iter().find(|(_, tools)| tools.contains(&tool)).map(|(g, _)| *g)
}

/// Whether `tool` is one the engine serves itself
pub fn is_engine_tool(tool: &str) -> bool {
  ALL.contains(&tool)
}
