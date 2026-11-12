//! The persisted session record: view fields plus what resuming needs

use std::collections::HashMap;

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
  /// Some recorded turns reached the native peer only as clipped handoff context; self-read must stay available.
  #[serde(default, skip_serializing_if = "is_false")]
  pub history_context: bool,
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
  /// The `clientId`s of the latest actions a paired device drove the session with (`AcpSession::claim_client_id`),
  /// kept so an engine that opens the session after another one exited still runs a resent action once
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub client_ids: Vec<String>,
  /// What the user picked in plugin command buttons' menus, by command name (`CommandDef::menu_picks`)
  #[serde(default, skip_serializing_if = "PluginMenus::is_empty")]
  pub plugin_menus: PluginMenus,
}

/// Menu picks by command name, each a map of group name or toggle id to the pick
pub type PluginMenus = std::collections::BTreeMap<String, serde_json::Map<String, serde_json::Value>>;

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
      wake_at: None,
      completed_at: completed_at(self.turns.iter()),
      model: session_model(&self.controls),
    }
  }
}

/// One turn in the pieces the row store keeps: a user turn whole, an agent turn as its head (`blocks` emptied) and each
/// block on its own, so a streaming reply rewrites only the blocks that changed
pub struct TurnPart {
  pub head: Vec<u8>,
  pub blocks: Vec<Vec<u8>>,
}

/// One subagent: its record without turns, and the turns unless the caller already holds them (`stamp` unchanged)
pub struct SubagentPart {
  pub id: String,
  /// Changes whenever the subagent's transcript may have; 0 never matches a stored one
  pub stamp: u64,
  pub head: Vec<u8>,
  pub turns: Option<Vec<TurnPart>>,
}

/// A record serialized piecewise for the row store (`store/db/body.rs`)
pub struct RecordParts {
  pub summary: SessionSummary,
  /// The record with `turns` (and `subagents`, when it has any) emptied
  pub head: Vec<u8>,
  pub turns: Vec<TurnPart>,
  pub subagents: Vec<SubagentPart>,
}

/// Encode turns one by one. An agent turn's blocks are taken out for its head encoding and handed back, so nothing is
/// cloned; a value that cannot serialize comes back empty and the row store refuses the save
pub fn turn_parts(turns: &mut [Turn]) -> Vec<TurnPart> {
  turns
    .iter_mut()
    .map(|turn| match turn {
      Turn::User(_) => TurnPart { head: serde_json::to_vec(turn).unwrap_or_default(), blocks: vec![] },
      Turn::Agent(agent) => {
        let blocks = std::mem::take(&mut agent.blocks);
        let head = serde_json::to_vec(turn).unwrap_or_default();
        let parts = blocks.iter().map(|b| serde_json::to_vec(b).unwrap_or_default()).collect();
        if let Turn::Agent(agent) = turn {
          agent.blocks = blocks;
        }
        TurnPart { head, blocks: parts }
      }
    })
    .collect()
}

impl RecordParts {
  /// A plain record split whole: nothing is skipped
  pub fn of(mut record: SessionRecord) -> Self {
    let summary = record.summary();
    let mut turns = std::mem::take(&mut record.turns);
    let mut subagents = record.subagents.take();
    if subagents.is_some() {
      record.subagents = Some(vec![]);
    }
    let head = serde_json::to_vec(&record).unwrap_or_default();
    let subagents = subagents
      .iter_mut()
      .flatten()
      .map(|sub| {
        let mut turns = std::mem::take(&mut sub.turns);
        SubagentPart { id: sub.core.id.clone(), stamp: 0, head: serde_json::to_vec(sub).unwrap_or_default(), turns: Some(turn_parts(&mut turns)) }
      })
      .collect();
    RecordParts { summary, head, turns: turn_parts(&mut turns), subagents }
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
  /// The record in the pieces the row store writes. `known` maps a subagent id to the stamp of the transcript the store
  /// already holds: unless `full`, such a subagent comes back without its turns, so finished children cost nothing
  fn record_parts(&self, known: &HashMap<String, u64>, full: bool) -> RecordParts {
    let _ = (known, full);
    RecordParts::of(self.record())
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
