//! The hub's request / reply lines, shared by the sidecar side (`hub.rs`) and the MCP server side (`host_mcp.rs`)

use serde::{Deserialize, Serialize};

use acpira_shared::subagents::SubagentPersona;

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
  Personas(Vec<SubagentPersona>),
  /// The child's current activity, while an `ask` waits
  Progress(String),
  /// The tool's answer text
  Done(String),
  /// A failure the model should read (unknown persona, depth, spawn error …)
  Error(String),
  /// The answer to `Features`
  Features(HostFeatures),
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
}
