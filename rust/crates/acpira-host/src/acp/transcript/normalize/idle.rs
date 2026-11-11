//! Non-generation stretches inside a live turn (`AgentTurn::idle_ms`), which the output speed leaves out:
//! - Concurrent background work (a monitor, backgrounded shell, AIR async task, subagent or scheduled wakeup) while
//!   nothing runs in the foreground and the model stays silent past `GRACE_MS`
//! - Full pauses where the main model cannot be decoding turn output (in-progress compaction, open API retry, or an
//!   open permission / question card with no foreground tool timing it)
//! - Detached continuations (`reopen_for_detached`: autonomous cycles, goal continuations, detached steer turns),
//!   whose tokens never enter the prompt's `usage`

use serde_json::Value;

use acpira_shared::transcript::*;

use crate::json::{str_of, truthy};
use crate::util::now_ms;

use super::NormalizeState;
use super::tasks::{async_task_live, has_unlinked_live_task};

/// A silence up to this long beside background work is still the model working (a prefill, a reply the agent sends in
/// one piece at the end); only what runs past it counts as waiting
const GRACE_MS: i64 = 20_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Wait {
  /// Concurrent background work: silence counts only past `GRACE_MS`
  Background,
  /// Full pause (compaction, API retry, or user card with no foreground tool timing it): counts in full
  Paused,
}

#[derive(Default, Clone)]
pub struct IdleState {
  /// Session-level background work outside `turns` (running subagents, summoned rounds, pending scheduled wakeup)
  background: bool,
  /// `started_at` of the turn `quiet` / `detached_since` belong to, so a history rewrite never bleeds across turns
  turn_started_at: Option<i64>,
  quiet: Option<(Wait, i64)>,
  /// `ended_at` of the settled prompt turn a detached cycle reopened: its `usage` covers only up to that point, so all
  /// non-tool time after it joins `idle_ms` when the detached cycle closes
  detached_since: Option<i64>,
}

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

/// What the live turn is waiting on while nothing runs in the foreground (a running foreground call keeps its time as
/// tool time instead)
fn wait_of(t: &AgentTurn, external_bg: bool) -> Option<Wait> {
  let mut open_background = external_bg;
  let mut paused = false;
  for b in &t.blocks {
    match b {
      AgentBlock::ToolCall(tc) => {
        if background(tc) {
          open_background |= async_task_live(tc) || tc.status.is_open();
        } else if tc.status.is_open() {
          return None;
        }
      }
      AgentBlock::Compaction(c) if c.status == CompactionStatus::InProgress => paused = true,
      AgentBlock::Notice(n) if n.retry.as_ref().is_some_and(|r| r.status == ToolStatus::InProgress) => paused = true,
      AgentBlock::Permission(_) => paused = true,
      AgentBlock::Question(q) if q.outcome.is_none() => paused = true,
      _ => {}
    }
  }
  if paused {
    Some(Wait::Paused)
  } else if open_background {
    Some(Wait::Background)
  } else {
    None
  }
}

fn align_turn(s: &mut NormalizeState) {
  let cur = match s.turns.last() {
    Some(Turn::Agent(t)) => t.started_at,
    _ => None,
  };
  if s.idle.turn_started_at != cur {
    s.idle.turn_started_at = cur;
    s.idle.quiet = None;
    s.idle.detached_since = None;
  }
}

/// Only inside `[responded_at, ended_at]`, the window `turnOutputTps` divides by
fn live_turn(s: &mut NormalizeState) -> Option<&mut AgentTurn> {
  match s.turns.last_mut() {
    Some(Turn::Agent(t)) if t.started_at.is_some() && t.responded_at.is_some() && t.stop.is_none() && t.ended_at.is_none() => Some(t),
    _ => None,
  }
}

fn settle_quiet(s: &mut NormalizeState, now: i64) {
  let Some((kind, since)) = s.idle.quiet.take() else { return };
  let grace = match kind {
    Wait::Background => GRACE_MS,
    Wait::Paused => 0,
  };
  let over = now - since - grace;
  if over > 0
    && let Some(t) = live_turn(s)
  {
    t.idle_ms = Some(t.idle_ms.unwrap_or(0) + over);
  }
}

/// Merged foreground tool execution inside `[from, to]`, matching `turnActionHelpers` / `TurnStats`
fn foreground_tool_ms(t: &AgentTurn, from: i64, to: i64) -> i64 {
  let mut spans: Vec<(i64, i64)> = t
    .blocks
    .iter()
    .filter_map(|b| match b {
      AgentBlock::ToolCall(tc) if tc.background != Some(true) => {
        let start = from.max(tc.started_at?);
        let end = to.min(tc.ended_at?);
        (end > start).then_some((start, end))
      }
      _ => None,
    })
    .collect();
  if spans.is_empty() {
    return 0;
  }
  spans.sort_unstable_by_key(|s| s.0);
  let mut total = 0;
  let (mut cur_s, mut cur_e) = spans[0];
  for &(s, e) in &spans[1..] {
    if s <= cur_e {
      cur_e = cur_e.max(e);
    } else {
      total += cur_e - cur_s;
      (cur_s, cur_e) = (s, e);
    }
  }
  total + (cur_e - cur_s)
}

/// After every event on the session: a model update ends the silence, a change that opens or closes work starts or
/// ends a waiting stretch
pub(super) fn tick(s: &mut NormalizeState, activity: bool) {
  align_turn(s);
  if s.idle.detached_since.is_some() {
    s.idle.quiet = None;
    return;
  }
  let bg = s.idle.background || has_unlinked_live_task(s);
  let next = live_turn(s).and_then(|t| wait_of(t, bg));
  if s.idle.quiet.is_none() && next.is_none() {
    return;
  }
  let now = now_ms();
  if activity || s.idle.quiet.is_some_and(|(cur, _)| next != Some(cur)) {
    settle_quiet(s, now);
  }
  if let Some(kind) = next
    && s.idle.quiet.is_none()
  {
    s.idle.quiet = Some((kind, now));
  }
}

/// Sync session-level background work (running subagents, summoned rounds, pending scheduled wakeup) into `idle_ms`
pub fn sync_idle(s: &mut NormalizeState, background: bool) {
  s.idle.background = background;
  tick(s, false);
}

/// A settled prompt turn is reopened for a detached continuation whose tokens never enter `usage`: remember where the
/// counted window ended, before `ended_at` is cleared
pub fn reopen_idle(s: &mut NormalizeState) {
  align_turn(s);
  s.idle.quiet = None;
  if s.idle.detached_since.is_none()
    && let Some(Turn::Agent(t)) = s.turns.last()
    && t.started_at.is_some()
    && t.responded_at.is_some()
    && let Some(ended) = t.ended_at
  {
    s.idle.detached_since = Some(ended);
  }
}

/// Before `end_turn` stamps `stop` / `ended_at`: close any open live-prompt wait
pub(super) fn close_before_seal(s: &mut NormalizeState, now: i64) {
  align_turn(s);
  if s.idle.detached_since.is_some() {
    s.idle.quiet = None;
  } else {
    settle_quiet(s, now);
  }
}

/// After `end_turn` stamped `ended_at` and closed open foreground tools: fold a detached continuation's non-tool span
/// into `idle_ms`, so `endedAt - respondedAt - foregroundToolMs - idleMs` stays the counted prompt's decode window
pub(super) fn close_after_seal(s: &mut NormalizeState) {
  let Some(since) = s.idle.detached_since.take() else { return };
  let Some(Turn::Agent(t)) = s.turns.last_mut() else { return };
  let Some(ended) = t.ended_at else { return };
  let uncounted = (ended - since) - foreground_tool_ms(t, since, ended);
  if uncounted > 0 {
    t.idle_ms = Some(t.idle_ms.unwrap_or(0) + uncounted);
  }
}
