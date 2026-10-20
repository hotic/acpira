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
  /// A Claude workflow agent's own `agentId`: it names the CLI's sidechain transcript the node's process is read from
  /// (`peer.agentId` is the run-and-position key)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sidechain_id: Option<String>,
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
  /// A receipt node whose transcript has content of its own (a Claude workflow agent read from the CLI's sidechain log):
  /// the inspector shows that process instead of the receipt-only note
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub has_process: Option<bool>,
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
  /// The reasoning effort the child session really runs, by the CLI's own option name; absent when it has none
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub effort: Option<String>,
  /// The group the round was summoned through (`SubagentGroup.id`), when a call named a group
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub group: Option<String>,
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
  /// Work without approvals: the CLI runs in its own full-access mode where it has one, and Acpira answers the child's
  /// permission requests itself instead of showing cards
  Bypass,
  /// A persona setting only: each round takes the caller's current access (a read-only or plan caller summons a
  /// consult, a full-access one a bypass, anything else work). A node's round mode is never this
  Inherit,
}

impl RelayMode {
  /// A stored or requested mode name; anything unknown is a consult
  pub fn parse(s: Option<&str>) -> Self {
    match s {
      Some("work") => Self::Work,
      Some("bypass") => Self::Bypass,
      Some("inherit") => Self::Inherit,
      _ => Self::Consult,
    }
  }
}

/// What a persona is for. Picking one on the settings page fills in its default access and standing instructions
/// (`duty_defaults`, both still the user's to change), and it decides how a call without `thread` continues
/// (`keeps_thread`, `fresh_thread`)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentDuty {
  /// No defaults and no thread policy: a call continues a conversation only when it names the thread
  #[default]
  General,
  /// Finds out and reports with `path:line`; read-only, keeps its conversation within a session
  Scout,
  /// Carries out the caller's plan and is invited to push back on it; the caller's access, keeps its conversation
  Executor,
  /// Looks for defects in a change; read-only, a fresh conversation every time so it stays independent
  Reviewer,
}

impl SubagentDuty {
  /// A stored duty name; anything unknown is general
  pub fn parse(s: Option<&str>) -> Self {
    match s {
      Some("scout") => Self::Scout,
      Some("executor") => Self::Executor,
      Some("reviewer") => Self::Reviewer,
      _ => Self::General,
    }
  }

  pub fn is_general(&self) -> bool {
    *self == Self::General
  }

  /// A call without `thread` continues this persona's idle conversation in the session (unless it asks for a new one)
  pub fn keeps_thread(self) -> bool {
    matches!(self, Self::Scout | Self::Executor)
  }

  /// A call never continues a conversation on its own: each one starts fresh (an explicit `thread` is still honoured)
  pub fn fresh_thread(self) -> bool {
    self == Self::Reviewer
  }
}

/// The standing instructions each duty starts with (mirror of `DUTY_BRIEFS` in src/shared/subagents.ts; both are checked
/// against test/fixtures/duty-presets.json). Written for the model, so in English whatever the UI language
pub const SCOUT_BRIEF: &str = "You are a scout: find things out and report them; do not change anything. Back every claim with \
where it comes from as path:line (or path:start-end), quoting the line when it matters, so the caller can act on your \
report without opening the files again. Say what you looked for and did not find as well as what you found, and mark \
guesses as guesses.";
pub const EXECUTOR_BRIEF: &str = "You carry out a plan another agent made. If a step looks wrong, would break something, or \
misses a case, stop before doing it and say so, with the reason and what you would do instead: pushing back early is part \
of the job. Otherwise do what was asked and no more, and run the checks the task names (or the obvious ones).";
pub const REVIEWER_BRIEF: &str = "You are a reviewer. Look for real defects in the change: bugs, regressions, missed cases, \
broken contracts or docs that no longer match. Read the changed code, its callers and its tests yourself, and judge the \
change against the requirement, not against how its author describes it. Leave out style preferences.";

/// The access and standing instructions a duty fills in when it is picked (None: general leaves both as they are)
pub fn duty_defaults(duty: SubagentDuty) -> Option<(RelayMode, &'static str)> {
  match duty {
    SubagentDuty::General => None,
    SubagentDuty::Scout => Some((RelayMode::Consult, SCOUT_BRIEF)),
    SubagentDuty::Executor => Some((RelayMode::Inherit, EXECUTOR_BRIEF)),
    SubagentDuty::Reviewer => Some((RelayMode::Consult, REVIEWER_BRIEF)),
  }
}

fn is_false(b: &bool) -> bool {
  !*b
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
  #[serde(default, skip_serializing_if = "SubagentDuty::is_general")]
  pub duty: SubagentDuty,
  /// Every new conversation starts with the handoff pack of the session's changes (`handoff.rs`), whatever the call says
  #[serde(default, skip_serializing_if = "is_false")]
  pub changes: bool,
}

fn enabled_default() -> bool {
  true
}

pub const PERSONA_MAX: usize = 32;

/// The id the built-in agent's sidekick goes by (`acp/session/relay.rs`): the executor the hub lists first for a session
/// whose built-in agent has one picked. No persona or group of the roster takes it, so `agent: "sidekick"` names one thing
pub const SIDEKICK_ID: &str = "sidekick";
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
/// and unique (a missing or taken one, `SIDEKICK_ID` included, is derived from the name, then numbered)
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
    while id == SIDEKICK_ID || out.iter().any(|o| o.id == id) {
      id = format!("{base}-{n}");
      n += 1;
    }
    out.push(SubagentPersona {
      id,
      name,
      agent,
      model: opt("model", ID_MAX),
      effort: opt("effort", ID_MAX),
      mode: RelayMode::parse(o.get("mode").and_then(serde_json::Value::as_str)),
      when: text("when", TEXT_MAX),
      brief: opt("brief", TEXT_MAX),
      enabled: o.get("enabled") != Some(&serde_json::Value::Bool(false)),
      duty: SubagentDuty::parse(o.get("duty").and_then(serde_json::Value::as_str)),
      changes: o.get("changes") == Some(&serde_json::Value::Bool(true)),
    });
  }
  out
}

/// Personas summoned together (`subagents.json` `groups`): one `ask_agent` call naming the group starts a round for
/// each enabled member, with the same task and the same handoff pack, each on its own; their replies come back together
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentGroup {
  /// Stable slug, unique among the groups and never a persona's id, since the tool's `agent` value names either
  #[serde(default)]
  pub id: String,
  #[serde(default)]
  pub name: String,
  /// Persona ids, in the order their rounds start
  #[serde(default)]
  pub members: Vec<String>,
  /// Tells the model when to summon the group
  #[serde(default)]
  pub when: String,
  #[serde(default = "enabled_default")]
  pub enabled: bool,
}

pub const GROUP_MAX: usize = 16;
/// Members of one group, at most: the summoned rounds a session runs at once
pub const GROUP_MEMBERS_MAX: usize = 6;

/// Like `sanitize_personas` (mirror of `sanitizeGroups`): unnamed entries drop, text is clipped, ids are unique slugs
/// that no persona has (a missing or taken one is derived from the name, then numbered), and members are the ids of
/// `personas` it names, each once, in order, at most `GROUP_MEMBERS_MAX`
pub fn sanitize_groups(v: &serde_json::Value, personas: &[SubagentPersona]) -> Vec<SubagentGroup> {
  let mut out: Vec<SubagentGroup> = vec![];
  for item in v.as_array().into_iter().flatten().take(GROUP_MAX) {
    let Some(o) = item.as_object() else { continue };
    let text = |k: &str, max: usize| o.get(k).and_then(serde_json::Value::as_str).map(|s| clip(s, max)).unwrap_or_default();
    let name = text("name", NAME_MAX);
    if name.is_empty() {
      continue;
    }
    let mut base = persona_slug(&text("id", ID_MAX));
    if base.is_empty() {
      base = persona_slug(&name);
    }
    if base.is_empty() {
      base = "group".into();
    }
    let taken = |id: &str| id == SIDEKICK_ID || out.iter().any(|g| g.id == id) || personas.iter().any(|p| p.id == id);
    let mut id = base.clone();
    let mut n = 2;
    while taken(&id) {
      id = format!("{base}-{n}");
      n += 1;
    }
    let mut members: Vec<String> = vec![];
    for m in o.get("members").and_then(serde_json::Value::as_array).into_iter().flatten().filter_map(serde_json::Value::as_str) {
      let m = m.trim();
      if members.len() < GROUP_MEMBERS_MAX && personas.iter().any(|p| p.id == m) && !members.iter().any(|x| x == m) {
        members.push(m.to_owned());
      }
    }
    out.push(SubagentGroup { id, name, members, when: text("when", TEXT_MAX), enabled: o.get("enabled") != Some(&serde_json::Value::Bool(false)) });
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

  /// The group cases `test/relay.test.ts` runs through `sanitizeGroups`
  #[test]
  fn sanitizes_groups_exactly_like_the_typescript_mirror() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/groups-sanitize.json")).unwrap();
    let personas = sanitize_personas(&fixture["personas"]);
    let out = serde_json::to_value(sanitize_groups(&fixture["input"], &personas)).unwrap();
    assert_eq!(out, fixture["output"]);
  }

  /// The duty presets the settings page fills in (TS) and the engine uses for the implicit sidekick (Rust)
  #[test]
  fn duty_presets_match_the_typescript_mirror() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/duty-presets.json")).unwrap();
    for duty in [SubagentDuty::General, SubagentDuty::Scout, SubagentDuty::Executor, SubagentDuty::Reviewer] {
      let key = serde_json::to_value(duty).unwrap();
      let want = &fixture[key.as_str().unwrap()];
      let got = match duty_defaults(duty) {
        Some((mode, brief)) => serde_json::json!({ "mode": mode, "brief": brief, "keepsThread": duty.keeps_thread(), "freshThread": duty.fresh_thread() }),
        None => serde_json::json!({ "keepsThread": false, "freshThread": false }),
      };
      assert_eq!(&got, want, "{duty:?}");
    }
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
