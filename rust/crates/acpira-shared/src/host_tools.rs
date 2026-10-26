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

/// Every tool the engine itself serves; a name outside it on the same server belongs to a plugin
pub const ALL: &[&str] = &[SHOW_IMAGE, SHOW_WIDGET, WORKSPACE_HOOKS, OPEN_BROWSER, SEARCH_SESSIONS, READ_SESSION, ASK_AGENT, ASK_CALLER];

/// Whether `tool` is one the engine serves itself
pub fn is_engine_tool(tool: &str) -> bool {
  ALL.contains(&tool)
}
