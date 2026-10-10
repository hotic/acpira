//! The goal extension codex-acp and claude-agent-acp implement (read from their sources: codex-acp 1.13.0 / 2.1.1,
//! claude-agent-acp 0.83.0 / 0.87.0). The agent keeps working toward an objective across turns until it judges it met.
//!
//! - Advertised at initialize as `{ version: 1, controlMethod: "_session/goal", actions }`: codex-acp 1.13.0 at
//!   `_meta.goal` (actions set / pause / resume / clear), codex-acp 2.x and claude-agent-acp under
//!   `_meta.jetbrains.air.goal` (Claude: set / clear), the latter only to clients advertising AIR.
//! - Every change arrives as a `session_info_update` whose `_meta` carries the snapshot at the same key (`null` = cleared):
//!   `objective`, `status` (active / paused / blocked / complete / limited; Claude only ever active), Claude's `iterations`,
//!   Codex's `tokenBudget` / `tokensUsed` / `timeUsedSeconds` / `updatedAt` (ms). codex-acp republishes the current goal
//!   when a session is created or loaded.
//! - `_session/goal` `{ sessionId, action, objective? }`. On codex-acp `pause` / `clear` only change the goal, while `set` /
//!   `resume` start a goal turn of their own that no host `session/prompt` owns. claude-agent-acp runs every action as the
//!   `/goal …` command, steered into a running turn or, idle, as a prompt of its own. Turn-starting actions are therefore
//!   sent as a `/goal …` prompt (both adapters parse it; Codex handles `/goal pause|resume|clear|<objective>`), so the
//!   turn stays the host's; the rest go over `_session/goal`.

use serde_json::{Value, json};

use acpira_shared::num::Num;
use acpira_shared::transcript::{GoalAction, GoalBlock, GoalEvent, GoalStatus, SessionGoal};

pub const METHOD: &str = "_session/goal";

/// Where a goal capability / snapshot sits: the AIR key first, then codex-acp 1.x's top-level one
fn goal_meta(meta: &Value) -> Option<&Value> {
  meta.pointer("/jetbrains/air/goal").or_else(|| meta.get("goal"))
}

/// The controls the agent advertises at initialize, or None when it has no goal extension
pub fn actions(init: &Value) -> Option<Vec<GoalAction>> {
  let cap = goal_meta(init.get("_meta")?)?;
  if cap.get("controlMethod").and_then(Value::as_str) != Some(METHOD) {
    return None;
  }
  let actions: Vec<GoalAction> = cap
    .get("actions")?
    .as_array()?
    .iter()
    .filter_map(|a| serde_json::from_value(a.clone()).ok())
    .collect();
  (!actions.is_empty()).then_some(actions)
}

/// A goal snapshot riding a `session_info_update`: `Some(None)` when the agent cleared it, `None` when the update says
/// nothing about goals (or carries a snapshot this host cannot read)
pub fn snapshot_of(update: &Value) -> Option<Option<SessionGoal>> {
  let raw = goal_meta(update.get("_meta")?)?;
  if raw.is_null() {
    return Some(None);
  }
  let objective = raw.get("objective")?.as_str()?.trim();
  if objective.is_empty() {
    return None;
  }
  let status = match raw.get("status").and_then(Value::as_str) {
    Some("paused") => GoalStatus::Paused,
    Some("blocked") => GoalStatus::Blocked,
    Some("limited" | "usageLimited" | "budgetLimited") => GoalStatus::Limited,
    Some("complete") => GoalStatus::Complete,
    _ => GoalStatus::Active,
  };
  let num = |k: &str| raw.get(k).and_then(Value::as_f64).filter(|n| n.is_finite() && *n >= 0.0).map(Num);
  Some(Some(SessionGoal {
    objective: objective.to_owned(),
    status,
    iterations: num("iterations"),
    token_budget: num("tokenBudget"),
    tokens_used: num("tokensUsed"),
    time_used_seconds: num("timeUsedSeconds"),
    updated_at: num("updatedAt"),
  }))
}

/// The milestone a snapshot change marks in the transcript; counter-only changes (a new round, more tokens) mark none
pub fn event_of(prev: Option<&SessionGoal>, next: Option<&SessionGoal>) -> Option<GoalBlock> {
  let block = |event: GoalEvent, g: &SessionGoal, objective: bool| GoalBlock {
    event,
    objective: objective.then(|| g.objective.clone()),
    iterations: g.iterations,
    tokens_used: g.tokens_used,
    time_used_seconds: g.time_used_seconds,
  };
  match (prev, next) {
    (None, None) => None,
    // A completed goal is dropped by the host when its turn ends; the agent clearing it afterwards is no news
    (Some(p), None) => (p.status != GoalStatus::Complete).then(|| GoalBlock { objective: None, ..block(GoalEvent::Cleared, p, false) }),
    (None, Some(n)) => Some(match n.status {
      GoalStatus::Active => block(GoalEvent::Set, n, true),
      s => block(event_for(s), n, true),
    }),
    (Some(p), Some(n)) if p.objective != n.objective => Some(block(GoalEvent::Set, n, true)),
    (Some(p), Some(n)) if p.status == n.status => None,
    (Some(p), Some(n)) => Some(match (p.status, n.status) {
      (_, GoalStatus::Active) => block(GoalEvent::Resumed, n, false),
      (_, s) => block(event_for(s), n, false),
    }),
  }
}

fn event_for(s: GoalStatus) -> GoalEvent {
  match s {
    GoalStatus::Active => GoalEvent::Set,
    GoalStatus::Paused => GoalEvent::Paused,
    GoalStatus::Blocked => GoalEvent::Blocked,
    GoalStatus::Limited => GoalEvent::Limited,
    GoalStatus::Complete => GoalEvent::Complete,
  }
}

/// Whether the action starts a turn of the agent's own when sent over `_session/goal` (see the module doc); those go
/// out as a `/goal …` prompt instead. `clear_starts_turn` is `Vendor::goal_clear_starts_turn`
pub fn starts_turn(action: GoalAction, clear_starts_turn: bool) -> bool {
  match action {
    GoalAction::Set | GoalAction::Resume => true,
    GoalAction::Pause => false,
    GoalAction::Clear => clear_starts_turn,
  }
}

/// The `/goal …` prompt text for a turn-starting action
pub fn command_text(action: GoalAction, objective: Option<&str>) -> Option<String> {
  match action {
    GoalAction::Set => objective.map(str::trim).filter(|o| !o.is_empty()).map(|o| format!("/goal {o}")),
    GoalAction::Pause => Some("/goal pause".into()),
    GoalAction::Resume => Some("/goal resume".into()),
    GoalAction::Clear => Some("/goal clear".into()),
  }
}

/// Claude Code answers its `/goal` command with the command's local output, which claude-agent-acp forwards as reply
/// text: "Goal set: <objective>", or "Goal cleared: <old objective>" for `/goal clear` (claude-agent-acp 0.87.0 / SDK
/// 0.3.287, seen live after a steered `/goal`). The goal row and strip already say as much, so the echo is dropped.
/// `sent` is the newest text the host sent (prompt or steer); whitespace is compared loosely
pub fn is_command_echo(text: &str, sent: &str) -> bool {
  let squash = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
  let Some(arg) = sent.trim().strip_prefix("/goal") else { return false };
  // "/goals" is another command
  if !arg.is_empty() && !arg.starts_with(char::is_whitespace) {
    return false;
  }
  let (arg, text) = (squash(arg), squash(text));
  match arg.as_str() {
    "" | "pause" | "resume" => false,
    "clear" => text.starts_with("Goal cleared: "),
    _ => text == format!("Goal set: {arg}"),
  }
}

pub fn params(session_id: &str, action: GoalAction) -> Value {
  json!({ "sessionId": session_id, "action": action })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn goal(status: &str) -> Value {
    json!({ "objective": " ship it ", "status": status, "tokensUsed": 1200, "tokenBudget": 5000, "timeUsedSeconds": 30 })
  }

  #[test]
  fn capabilities_are_read_at_both_keys() {
    let codex1 = json!({ "_meta": { "goal": { "version": 1, "controlMethod": "_session/goal", "actions": ["set", "pause", "resume", "clear"] } } });
    assert_eq!(actions(&codex1), Some(vec![GoalAction::Set, GoalAction::Pause, GoalAction::Resume, GoalAction::Clear]));
    let claude = json!({ "_meta": { "jetbrains": { "air": { "goal": { "version": 1, "controlMethod": "_session/goal", "actions": ["set", "clear"] } } } } });
    assert_eq!(actions(&claude), Some(vec![GoalAction::Set, GoalAction::Clear]));
    assert_eq!(actions(&json!({ "_meta": { "steering": { "supported": true } } })), None);
    // Another control method is another protocol
    assert_eq!(actions(&json!({ "_meta": { "goal": { "controlMethod": "_x/goal", "actions": ["set"] } } })), None);
  }

  #[test]
  fn snapshots_parse_clear_and_ignore_other_updates() {
    let u = |g: Value| json!({ "sessionUpdate": "session_info_update", "_meta": { "jetbrains": { "air": { "goal": g } } } });
    let g = snapshot_of(&u(goal("budgetLimited"))).unwrap().unwrap();
    assert_eq!(g.objective, "ship it");
    assert_eq!(g.status, GoalStatus::Limited);
    assert_eq!(g.tokens_used, Some(Num(1200.0)));
    assert_eq!(snapshot_of(&u(Value::Null)), Some(None));
    assert_eq!(snapshot_of(&json!({ "sessionUpdate": "session_info_update", "_meta": { "goal": null } })), Some(None));
    assert_eq!(snapshot_of(&json!({ "sessionUpdate": "session_info_update", "title": "x" })), None);
    assert_eq!(snapshot_of(&u(json!({ "objective": "  " }))), None);
  }

  #[test]
  fn only_status_and_objective_changes_mark_the_transcript() {
    let g = |status: GoalStatus, objective: &str| SessionGoal {
      objective: objective.into(),
      status,
      iterations: None,
      token_budget: None,
      tokens_used: None,
      time_used_seconds: None,
      updated_at: None,
    };
    let active = g(GoalStatus::Active, "a");
    assert_eq!(event_of(None, Some(&active)).map(|b| (b.event, b.objective)), Some((GoalEvent::Set, Some("a".into()))));
    assert_eq!(event_of(Some(&active), Some(&active)), None);
    assert_eq!(event_of(Some(&active), Some(&g(GoalStatus::Paused, "a"))).map(|b| b.event), Some(GoalEvent::Paused));
    assert_eq!(event_of(Some(&g(GoalStatus::Paused, "a")), Some(&active)).map(|b| b.event), Some(GoalEvent::Resumed));
    assert_eq!(event_of(Some(&active), Some(&g(GoalStatus::Active, "b"))).map(|b| b.event), Some(GoalEvent::Set));
    assert_eq!(event_of(Some(&active), Some(&g(GoalStatus::Complete, "a"))).map(|b| b.event), Some(GoalEvent::Complete));
    assert_eq!(event_of(Some(&active), None).map(|b| b.event), Some(GoalEvent::Cleared));
    assert_eq!(event_of(Some(&g(GoalStatus::Complete, "a")), None), None);
  }

  #[test]
  fn turn_starting_actions_become_commands() {
    assert!(starts_turn(GoalAction::Resume, false));
    assert!(!starts_turn(GoalAction::Clear, false));
    assert!(starts_turn(GoalAction::Clear, true));
    assert!(!starts_turn(GoalAction::Pause, true));
    assert_eq!(command_text(GoalAction::Set, Some(" fix it ")).as_deref(), Some("/goal fix it"));
    assert_eq!(command_text(GoalAction::Set, Some(" ")), None);
    assert_eq!(command_text(GoalAction::Clear, None).as_deref(), Some("/goal clear"));
    assert_eq!(params("s", GoalAction::Pause)["action"], "pause");
  }

  #[test]
  fn only_the_echo_of_the_sent_command_is_dropped() {
    let sent = "/goal ship it\nwith  docs";
    assert!(is_command_echo("Goal set: ship it\nwith docs", sent));
    assert!(!is_command_echo("Goal set: something else", sent));
    assert!(!is_command_echo("Goal set: ship it with docs. Starting now", sent));
    assert!(!is_command_echo("Goal set: ship it", "ship it"));
    assert!(!is_command_echo("Goal set: ship it", "/goals ship it"));
    assert!(is_command_echo("Goal cleared: ship it", "/goal clear"));
    assert!(!is_command_echo("No goal set", "/goal clear"));
  }
}
