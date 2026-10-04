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
  /// Set when Acpira itself ran the child in another CLI (a summoned persona, `relay`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub harness: Option<SubagentHarness>,
}

/// What a summoned child runs in: the CLI, the persona it was summoned as, and whether it may write
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentHarness {
  pub agent: String,
  /// The persona's id (its display name is the node's `role`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub persona: Option<String>,
  pub mode: RelayMode,
  /// The conversation this round belongs to: the id of the thread's first node. Every round is a node of its own, all
  /// rounds of a thread talk to the same native session
  #[serde(default)]
  pub thread: String,
  #[serde(default = "first_round")]
  pub round: u32,
  /// The child CLI's own session id (the node's `peer.sessionId` is Acpira's routing key, since two CLIs may mint the
  /// same id)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session_id: Option<String>,
}

fn first_round() -> u32 {
  1
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayMode {
  /// An opinion: the brief says not to edit, and the CLI runs in its own read-only mode where it has one (how strictly
  /// that blocks writes is the CLI's; without one its own permission settings apply)
  #[default]
  Consult,
  Work,
}

/// A cross-harness subagent defined once in the settings (`~/.acpira/subagents.json`); every session can summon it
/// through Acpira's MCP tool `ask_agent`, or the user names it with `@name`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentPersona {
  /// Stable slug, the value of the tool's `agent` argument
  #[serde(default)]
  pub id: String,
  #[serde(default)]
  pub name: String,
  /// The CLI it runs in (an agent id of the registry)
  #[serde(default)]
  pub agent: String,
  /// A value of that agent's model select; absent = the CLI's default
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  /// A value of that agent's reasoning-effort select; absent = the default
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub effort: Option<String>,
  #[serde(default)]
  pub mode: RelayMode,
  /// Tells the model when to summon it
  #[serde(default)]
  pub when: String,
  /// Appended to every task it is given
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub brief: Option<String>,
  #[serde(default = "enabled_default")]
  pub enabled: bool,
}

fn enabled_default() -> bool {
  true
}

pub const PERSONA_MAX: usize = 32;
const NAME_MAX: usize = 48;
const TEXT_MAX: usize = 2000;
/// Ids, agent ids, model and effort values
const ID_MAX: usize = 200;

fn clip(s: &str, max: usize) -> String {
  s.trim().chars().take(max).collect()
}

/// A lowercase ascii slug of a display name (`Codex Review` → `codex-review`); empty when nothing ascii is left
pub fn persona_slug(name: &str) -> String {
  let mut out = String::new();
  for ch in name.trim().chars() {
    if ch.is_ascii_alphanumeric() {
      out.push(ch.to_ascii_lowercase());
    } else if !out.ends_with('-') && !out.is_empty() {
      out.push('-');
    }
  }
  out.trim_end_matches('-').to_owned()
}

/// A hand-edited file or a forged message, field by field like `sanitizePersonas` (one bad field never drops the
/// entry): entries without a name or agent are dropped, text is clipped, an unknown mode is a consult, ids are slugs
/// and unique (a missing or taken one is derived from the name, then numbered)
pub fn sanitize_personas(v: &serde_json::Value) -> Vec<SubagentPersona> {
  let mut out: Vec<SubagentPersona> = vec![];
  for item in v.as_array().into_iter().flatten().take(PERSONA_MAX) {
    let Some(o) = item.as_object() else { continue };
    let text = |k: &str, max: usize| o.get(k).and_then(serde_json::Value::as_str).map(|s| clip(s, max)).unwrap_or_default();
    let opt = |k: &str, max: usize| Some(text(k, max)).filter(|s| !s.is_empty());
    let (name, agent) = (text("name", NAME_MAX), text("agent", ID_MAX));
    if name.is_empty() || agent.is_empty() {
      continue;
    }
    let mut base = persona_slug(&text("id", ID_MAX));
    if base.is_empty() {
      base = persona_slug(&name);
    }
    if base.is_empty() {
      base = "agent".into();
    }
    let mut id = base.clone();
    let mut n = 2;
    while out.iter().any(|o| o.id == id) {
      id = format!("{base}-{n}");
      n += 1;
    }
    out.push(SubagentPersona {
      id,
      name,
      agent,
      model: opt("model", ID_MAX),
      effort: opt("effort", ID_MAX),
      mode: if o.get("mode").and_then(serde_json::Value::as_str) == Some("work") { RelayMode::Work } else { RelayMode::Consult },
      when: text("when", TEXT_MAX),
      brief: opt("brief", TEXT_MAX),
      enabled: o.get("enabled") != Some(&serde_json::Value::Bool(false)),
    });
  }
  out
}

#[cfg(test)]
mod persona_tests {
  use super::*;

  /// The boundary cases `test/relay.test.ts` runs through `sanitizePersonas`: both sides must agree on every one
  #[test]
  fn sanitizes_exactly_like_the_typescript_mirror() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/personas-sanitize.json")).unwrap();
    let out = serde_json::to_value(sanitize_personas(&fixture["input"])).unwrap();
    assert_eq!(out, fixture["output"]);
  }
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
