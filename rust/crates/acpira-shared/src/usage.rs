//! The usage page (mirror of packages/shared/usage.ts): what the agents did across every session, hour by hour. The engine
//! keeps one entry per turn it has seen (`acpira-host` `store/usage_index.rs`) and hands the page the hours folded per
//! session and model; the page folds them again into days, local hours, models, agents and projects itself

use serde::{Deserialize, Serialize};

use crate::transcript::AgentId;

/// A session the rows point into
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageSession {
  pub agent: AgentId,
  /// The project: the folder the session runs in
  pub cwd: String,
}

/// A model the rows point into: the id the turns name and the family the picker shows for it (the id's last segment when
/// no option names it)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageModel {
  pub id: String,
  pub name: String,
}

/// One session's turns in one model within one clock hour. Zero fields are left out
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
  /// The hour's start, Unix milliseconds (whole UTC hours)
  pub at: i64,
  /// Index into `UsageView::sessions`
  pub session: u32,
  /// Index into `UsageView::models`; absent when nothing names the model
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<u32>,
  /// Tokens: the whole prompt (cache included) plus the output, as the turn's own card counts them
  #[serde(default, skip_serializing_if = "is_zero")]
  pub tokens: u64,
  #[serde(default, skip_serializing_if = "is_zero")]
  pub input: u64,
  #[serde(default, skip_serializing_if = "is_zero")]
  pub output: u64,
  #[serde(default, skip_serializing_if = "is_zero")]
  pub cached_read: u64,
  /// Agent turns that started in the hour
  #[serde(default, skip_serializing_if = "is_zero")]
  pub turns: u64,
  /// The user's own messages among them (automatic turns left out)
  #[serde(default, skip_serializing_if = "is_zero")]
  pub prompts: u64,
  /// Working time: from the first streamed output to the turn's end, background waits left out
  #[serde(default, skip_serializing_if = "is_zero")]
  pub work_ms: u64,
  /// The longest single turn's working time
  #[serde(default, skip_serializing_if = "is_zero")]
  pub longest_ms: u64,
  #[serde(default, skip_serializing_if = "is_zero")]
  pub tools: u64,
  /// Lines the agents' edits added and removed (`diffStat` of edit rows; edits made through a shell are not seen)
  #[serde(default, skip_serializing_if = "is_zero")]
  pub added: u64,
  #[serde(default, skip_serializing_if = "is_zero")]
  pub removed: u64,
}

fn is_zero(n: &u64) -> bool {
  *n == 0
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageView {
  pub sessions: Vec<UsageSession>,
  pub models: Vec<UsageModel>,
  /// Ordered by `at`
  pub rows: Vec<UsageRow>,
}
