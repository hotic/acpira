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
  /// How long the call may wait before it answers "still working" (seconds; the MCP side derives it from the client)
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
}
