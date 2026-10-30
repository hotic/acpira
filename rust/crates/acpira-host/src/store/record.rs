//! The persisted session record: view fields plus what resuming needs

use serde::{Deserialize, Serialize};

use acpira_shared::model_shapes::session_model;
use acpira_shared::subagents::SubagentRecord;
use acpira_shared::transcript::{AgentId, SessionControls, SessionGoal, SessionSummary, SlashCommand, Turn, Usage, completed_at};

fn is_false(v: &bool) -> bool {
  !*v
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedFrom {
  pub session_id: String,
  #[serde(deserialize_with = "acpira_shared::num::lenient_i64")]
  pub turn_index: i64,
  /// The source's agent when the fork moved to another one (absent: same agent, and every record written before)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<AgentId>,
  /// The source's model as shown when it was forked, for the cross-agent handoff (the fork no longer has its controls)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedFrom {
  pub session_id: String,
}

/// The machine that drives a session whose record this is a copy of (`remote::host_store`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CopyOf {
  /// Its machine id (`remote::helper::machine_id`)
  pub machine: String,
  /// Its name, for the note a copy shows
  #[serde(default)]
  pub name: String,
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
  /// The user renamed the session: the agent's own titles are ignored from then on, since an adapter re-announces
  /// the title it keeps (claude-agent-acp after every restart)
  #[serde(default, skip_serializing_if = "is_false")]
  pub renamed: bool,
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
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub archived: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub category: Option<String>,
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
  /// The agent's goal when the record was written (`vendors::goal`); the agent's own snapshot replaces it once connected
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub goal: Option<SessionGoal>,
  /// Set on a copy of a session another machine drives in a folder of this one: shown read-only here, never driven
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub copy_of: Option<CopyOf>,
}

impl SessionRecord {
  /// Records written before `TurnUsage::fold_cache_into_input` carry the vendor's own `input` semantics; folding on
  /// load is idempotent, so already folded turns stay as they are
  pub fn fold_turn_usage(&mut self) {
    for turn in &mut self.turns {
      if let Turn::Agent(a) = turn
        && let Some(u) = a.usage.as_mut()
      {
        u.fold_cache_into_input();
      }
    }
  }

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
      created_at: Some(self.created_at.clone()).filter(|c| !c.is_empty()),
      pinned: self.pinned,
      archived: self.archived,
      category: self.category.clone(),
      state: None,
      unread: None,
      question: None,
      completed_at: completed_at(self.turns.iter()),
      model: session_model(&self.controls),
    }
  }
}

/// Something the store can persist: a live session serializes itself at write time (no snapshot clone per save),
/// a plain record is its own source
pub trait RecordSource: Send + Sync {
  fn record_id(&self) -> String;
  fn record_json(&self) -> Vec<u8>;
  fn record(&self) -> SessionRecord;
  /// The session's folder, without serializing the rest
  fn record_cwd(&self) -> String {
    self.record().cwd
  }
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
  fn record_cwd(&self) -> String {
    self.cwd.clone()
  }
}
