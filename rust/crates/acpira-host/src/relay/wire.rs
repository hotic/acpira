//! The hub's request / reply lines, shared by the sidecar side (`hub.rs`) and the MCP server side (`host_mcp.rs`)

use serde::{Deserialize, Serialize};

use acpira_shared::subagents::{SubagentGroup, SubagentPersona};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HubRequest {
  /// The grant of the MCP entry the caller was started with: it names the session, thread and depth (`hub.rs`)
  pub token: String,
  #[serde(flatten)]
  pub op: HubOp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum HubOp {
  List,
  Ask(AskArgs),
  /// `ask_caller`: a summoned child's question to the agent that summoned it (only for a summoned grant); progress
  /// lines while it waits, then `Done` / `Error`
  AskCaller(AskCallerArgs),
  /// Which host tools this sidecar's shell can carry out right now (`HubReply::Features`)
  Features,
  /// `open_browser`: show an http(s) page in the user's editor; answered `Done` / `Error`
  OpenBrowser { url: String },
  /// `search_sessions`: find Acpira sessions, or text inside one; answered `Done` / `Error`
  SearchSessions(SessionSearchArgs),
  /// `read_session`: one session's outline or a page of its turns; answered `Done` / `Error`
  ReadSession(SessionReadArgs),
  /// `show_image` / `show_widget` in a session whose folder is on an SSH host: the files are checked and read there,
  /// for the call's completion to show; answered `Done` / `Error` (what is wrong with them)
  HostFiles { paths: Vec<String>, widget: bool },
  /// `workspace_hooks` in a session whose folder is on an SSH host: the project's hooks file there, checked there;
  /// answered `Done` (the check's text) / `Error`
  HostHooks { cwd: String },
  /// A project stdio MCP server on the SSH host of the session's folder, by the name the session registered it under
  /// (`AcpSession::host_mcp_servers`): answered `Done` once it runs there (or `Error`), then the socket carries its stdin
  /// one way and `hub::Frame`s the other
  HostStdio { name: String },
  /// The plugin tools in effect and the built-in tool groups switched off (`HubReply::Plugins`)
  Plugins,
  /// A plugin's script tool, by the name it is served under; answered `Done` (its output) / `Error`
  PluginCall {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
  },
  /// `plugin_guide`: the contract and the state of every plugin; answered `Done`
  PluginGuide,
  /// `plugin_activate`: freeze and check the plugin, then ask the user on a card in the calling session; progress
  /// lines while the card waits, then `Done` / `Error`
  PluginActivate {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    wait_secs: Option<u64>,
  },
  /// A call to one of this server's tools, before it runs (`phase: "before"`, answered `Done` with `{ arguments?,
  /// block?, after }`) or after (`"after"` with its `result`, answered `Done` with `{ result? }`): the plugins'
  /// `tool.before` / `tool.after` hooks (`AcpSession::plugin_tool`)
  ToolHook {
    phase: String,
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
  },
  /// A project tool of a Claude Code session in a folder on an SSH host (`host_tools::REMOTE`, `vendors::claude_remote`),
  /// run there or, for Claude's own files, here; `tool_use_id` is the model's call (`_meta["claudecode/toolUseId"]`),
  /// which ties it to its row. Answered `Tool` (the MCP result) or `Error`. The caller closing the socket cancels it
  RemoteTool {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_use_id: Option<String>,
  },
}

/// `search_sessions`' arguments (`session_library.rs`)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSearchArgs {
  /// Words that must all occur (title or conversation); absent or empty = the most recent sessions
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub query: Option<String>,
  /// `project` (the caller's folder, the default) or `all`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub scope: Option<String>,
  /// Only sessions of this agent (id or name)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
  /// Search inside this one session instead (link, id or the agent's own session id)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub limit: Option<usize>,
}

/// `read_session`'s arguments
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionReadArgs {
  /// Link, id or the agent's own session id
  #[serde(default)]
  pub session: String,
  /// Turn range (1-based, inclusive); both absent = the outline
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub from: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub to: Option<u32>,
  /// Bytes into the `from` turn's text, where a page cut it (the page names it)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub offset: Option<usize>,
  /// `full` adds tool output and diffs to the turns
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
}

/// `ask_agent`'s arguments
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskArgs {
  /// Persona id (or name)
  #[serde(default)]
  pub agent: String,
  /// The task; empty with a thread = keep waiting for that thread's running round
  #[serde(default)]
  pub prompt: String,
  /// A few words naming the task, shown on the row
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  /// `consult` / `work`; absent = the persona's default
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode: Option<String>,
  /// Continue an earlier conversation with the same child
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub thread: Option<String>,
  /// true = start a new conversation even where the persona would continue its idle one (a scout, an executor)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub new_thread: Option<bool>,
  /// How long the call may wait before it answers "still working" (seconds; the MCP side derives it from the client)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub wait_secs: Option<u64>,
  /// A new task: true = the call waits for the reply; false = it answers at once and the reply is delivered to the
  /// caller when the round ends. Absent = the caller's default (the root does not wait)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub wait: Option<bool>,
  /// With a thread whose child waits on a question: one answer per question, in order (an option's label or free text)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub answers: Option<Vec<String>>,
  /// A new task: true = hand over the caller's uncommitted changes (the handoff pack on a thread's first round, what
  /// moved since on a later one). Absent = the persona's own switch
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub changes: Option<bool>,
}

/// `ask_caller`'s arguments
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskCallerArgs {
  #[serde(default)]
  pub question: String,
  /// Suggested answers; the caller may still answer in its own words
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub options: Vec<String>,
  /// How long the call may wait for the answer (seconds; the MCP side derives it from the client)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub wait_secs: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HubReply {
  /// The answer to `List` from a hub before groups: the personas alone (still read, for an MCP server that outlives an
  /// engine upgrade)
  Personas(Vec<SubagentPersona>),
  /// The answer to `List`: what the caller may summon
  Roster { personas: Vec<SubagentPersona>, groups: Vec<SubagentGroup> },
  /// The child's current activity, while an `ask` waits
  Progress(String),
  /// The tool's answer text
  Done(String),
  /// A failure the model should read (unknown persona, depth, spawn error …)
  Error(String),
  /// The answer to `Features`
  Features(HostFeatures),
  /// A whole MCP tool result (`RemoteTool`)
  Tool(serde_json::Value),
  /// The answer to `Plugins`: the MCP definitions of the plugin tools in effect, the tool groups switched off
  Plugins {
    tools: Vec<serde_json::Value>,
    #[serde(default)]
    off: Vec<String>,
  },
}

/// Host tools the MCP server offers only when the serving shell can carry them out
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostFeatures {
  /// The shell shows web pages inside the editor (`open_browser`)
  #[serde(default)]
  pub browser: bool,
  /// The grant is a summoned child's: it has a caller to ask (`ask_caller`)
  #[serde(default)]
  pub summoned: bool,
  /// The session's folder is on an SSH host: the files a tool names are read there (`HubOp::HostFiles`)
  #[serde(default)]
  pub remote: bool,
  /// The host's alias when the session's agent (Claude Code under local run) gets the project tools that run there
  /// (`HubOp::RemoteTool`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub remote_tools: Option<String>,
}
