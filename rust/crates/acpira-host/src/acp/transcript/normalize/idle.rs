//! Background waits: stretches of a live turn where only background work is open (a monitor, a backgrounded shell or
//! subagent) and the model says nothing. claude-agent-acp keeps the prompt open while the model sits waiting for such
//! work, and that wait is neither tool time nor generation, so the output speed leaves it out (`AgentTurn::idle_ms`)

use serde_json::Value;

use acpira_shared::transcript::*;

use crate::json::{str_of, truthy};
use crate::util::now_ms;

use super::NormalizeState;
use super::tasks::async_task_live;

/// A silence up to this long is still the model working (a prefill, a reply the agent sends in one piece at the end);
/// only what runs past it counts as waiting
const GRACE_MS: i64 = 20_000;

/// An update the model itself produced: text, reasoning, a call or its streamed input, a plan. Status changes,
/// progress beats and usage come from the agent around the model and do not end a silence
pub(super) fn model_activity(u: &Value) -> bool {
  match str_of(u, "sessionUpdate").unwrap_or("") {
    "agent_message_chunk" | "agent_thought_chunk" | "tool_call" | "plan" => true,
    "tool_call_update" => truthy(u.get("rawInput")),
    _ => false,
  }
}

fn background(tc: &ToolCallBlock) -> bool {
  tc.background == Some(true) || tc.async_task.is_some()
}

/// Background work is open and nothing in the foreground: a running call keeps its time as tool time instead
fn waiting(t: &AgentTurn) -> bool {
  let mut open_background = false;
  for b in &t.blocks {
    let AgentBlock::ToolCall(tc) = b else { continue };
    if background(tc) {
      open_background |= async_task_live(tc) || tc.status.is_open();
    } else if tc.status.is_open() {
      return false;
    }
  }
  open_background
}

fn live_turn(s: &mut NormalizeState) -> Option<&mut AgentTurn> {
  match s.turns.last_mut() {
    Some(Turn::Agent(t)) if t.started_at.is_some() && t.stop.is_none() && t.ended_at.is_none() => Some(t),
    _ => None,
  }
}

fn settle(s: &mut NormalizeState, now: i64) {
  let Some(since) = s.quiet_since.take() else { return };
  let over = now - since - GRACE_MS;
  if over > 0
    && let Some(t) = live_turn(s)
  {
    t.idle_ms = Some(t.idle_ms.unwrap_or(0) + over);
  }
}

/// After every event on the session: a model update ends the silence, a change that opens or closes work starts or
/// ends a waiting stretch
pub(super) fn tick(s: &mut NormalizeState, activity: bool) {
  let quiet = live_turn(s).is_some_and(|t| waiting(t));
  if s.quiet_since.is_none() && !quiet {
    return;
  }
  let now = now_ms();
  if activity || !quiet {
    settle(s, now);
  }
  if quiet && s.quiet_since.is_none() {
    s.quiet_since = Some(now);
  }
}

/// The turn ends: a stretch still open is counted up to now
pub(super) fn close(s: &mut NormalizeState) {
  settle(s, now_ms());
}
