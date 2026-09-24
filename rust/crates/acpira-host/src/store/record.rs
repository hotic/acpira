//! The persisted session record: view fields plus what resuming needs

use serde::{Deserialize, Serialize};

use acpira_shared::subagents::SubagentRecord;
use acpira_shared::transcript::{AgentId, SessionControls, SessionSummary, SlashCommand, Turn, Usage};

fn is_false(v: &bool) -> bool {
  !*v
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedFrom {
  pub session_id: String,
  #[serde(deserialize_with = "acpira_shared::num::lenient_i64")]
  pub turn_index: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFrom {
  pub session_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
  pub id: String,
  pub agent: AgentId,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub account_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub acp_session_id: Option<String>,
  pub cwd: String,
  #[serde(default)]
  pub title: String,
  #[serde(default)]
  pub created_at: String,
  pub updated_at: String,
  pub turns: Vec<Turn>,
  #[serde(default)]
  pub controls: SessionControls,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usage: Option<Usage>,
  #[serde(default)]
  pub commands: Vec<SlashCommand>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pinned: Option<bool>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub history_pending: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub forked_from: Option<ForkedFrom>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub import_pending: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub imported_from: Option<ImportedFrom>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub subagents: Option<Vec<SubagentRecord>>,
}

impl SessionRecord {
  pub fn summary(&self) -> SessionSummary {
    SessionSummary {
      id: self.id.clone(),
      external: None,
      title: self.title.clone(),
      agent: self.agent.clone(),
      account_id: self.account_id.clone(),
      acp_session_id: self.acp_session_id.clone(),
      cwd: self.cwd.clone(),
      updated_at: self.updated_at.clone(),
      pinned: self.pinned,
      state: None,
    }
  }
}

/// Something the store can persist: a live session serializes itself at write time (no snapshot clone per save),
/// a plain record is its own source
pub trait RecordSource: Send + Sync {
  fn record_id(&self) -> String;
  fn record_json(&self) -> Vec<u8>;
  fn record(&self) -> SessionRecord;
}

impl RecordSource for SessionRecord {
  fn record_id(&self) -> String {
    self.id.clone()
  }
  fn record_json(&self) -> Vec<u8> {
    serde_json::to_vec(self).unwrap_or_default()
  }
  fn record(&self) -> SessionRecord {
    self.clone()
  }
}
