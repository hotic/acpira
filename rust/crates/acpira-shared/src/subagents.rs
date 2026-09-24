//! First-class subagent nodes shared by host and webview (mirror of src/shared/subagents.ts)

use serde::{Deserialize, Serialize};

use crate::num::Num;
use crate::transcript::{PermissionBlock, QuestionBlock, Turn};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentState {
  Running,
  Completed,
  Failed,
  Cancelled,
  Disconnected,
}

impl SubagentState {
  pub fn is_terminal(self) -> bool {
    !matches!(self, SubagentState::Running)
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentVisibility {
  Session,
  Nested,
  Receipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateSource {
  Agent,
  Local,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentControls {
  pub cancel: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentPeer {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SubagentUsage {
  pub used: Num,
  pub size: Num,
}

/// Fields common to the summary pushed to the webview and the persisted record
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentCore {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub parent_id: Option<String>,
  #[serde(deserialize_with = "crate::num::lenient_u64")]
  pub turn_index: u64,
  pub visibility: SubagentVisibility,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub task: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub role: Option<String>,
  pub state: SubagentState,
  pub state_source: StateSource,
  #[serde(default)]
  pub controls: SubagentControls,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cancel_requested: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub background: Option<bool>,
  #[serde(deserialize_with = "crate::num::lenient_i64")]
  pub announced_at: i64,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::num::lenient_opt_i64")]
  pub ended_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usage: Option<SubagentUsage>,
  #[serde(default)]
  pub peer: SubagentPeer,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub activity: Option<String>,
  #[serde(default, deserialize_with = "crate::num::lenient_u64")]
  pub tool_count: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentSummary {
  #[serde(flatten)]
  pub core: SubagentCore,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub permissions: Option<Vec<PermissionBlock>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub question: Option<QuestionBlock>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentRecord {
  #[serde(flatten)]
  pub core: SubagentCore,
  #[serde(default)]
  pub turns: Vec<Turn>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::num::lenient_opt_i64")]
  pub rev: Option<i64>,
}
