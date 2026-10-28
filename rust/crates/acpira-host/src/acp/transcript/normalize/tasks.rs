//! Background tasks (`asyncTasks` events): the tool row that hosts each task, its state written through to that row,
//! and what happens to live tasks when their process goes

use acpira_shared::transcript::*;

use crate::acp::transport::wire::{AsyncTaskEvent, TaskEventKind};
use crate::util::now_ms;

use super::NormalizeState;
use super::notices::push_standalone;
use super::tool_call::{empty_tool, time_tool};
use super::tool_kind::verb_of;

fn task_terminal(st: AsyncTaskState) -> bool {
  matches!(st, AsyncTaskState::Completed | AsyncTaskState::Failed | AsyncTaskState::Stopped)
}

fn task_state_moves(from: AsyncTaskState, to: AsyncTaskState) -> bool {
  !task_terminal(from) || (from == AsyncTaskState::Stopped && matches!(to, AsyncTaskState::Completed | AsyncTaskState::Failed))
}

fn task_status(st: AsyncTaskState) -> ToolStatus {
  match st {
    AsyncTaskState::Completed => ToolStatus::Completed,
    AsyncTaskState::Failed => ToolStatus::Failed,
    AsyncTaskState::Stopped => ToolStatus::Cancelled,
    _ => ToolStatus::InProgress,
  }
}

/// The task is still observable and owns its row across turn ends
pub fn async_task_live(b: &ToolCallBlock) -> bool {
  b.async_task.as_ref().is_some_and(|a| b.observation != Some(Observation::Unknown) && !task_terminal(a.state))
}

/// The row at `loc` belongs to a call that is over (its turn is earlier than `current`, its status is final and no
/// background task keeps it alive). ACP keeps tool call ids unique per agent session only, and a historical edit
/// continues the transcript on a fresh `session/new`: an agent numbering its calls per session (the built-in agent's
/// `call-N`) then announces ids those rows already hold. Merging into them rewrote old turns and left the new turn
/// without its tool rows (acpira 1.10.7, 2026-10-22)
pub(super) fn settled_before(turns: &[Turn], (ti, bi): (usize, usize), current: usize) -> bool {
  ti < current
    && matches!(turns[ti].as_agent().and_then(|t| t.blocks.get(bi)),
      Some(AgentBlock::ToolCall(tc)) if !tc.status.is_open() && !async_task_live(tc))
}

/// Some background task of this transcript is still running and observed: ending the process would orphan it
pub fn has_live_async_task(s: &NormalizeState) -> bool {
  s.turns.iter().filter_map(Turn::as_agent).any(|t| t.blocks.iter().any(|b| matches!(b, AgentBlock::ToolCall(tc) if async_task_live(tc))))
}

fn block_by_task(turns: &[Turn], task_id: &str) -> Option<(usize, usize)> {
  for (ti, t) in turns.iter().enumerate() {
    let Turn::Agent(a) = t else { continue };
    if let Some(bi) =
      a.blocks.iter().position(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.async_task.as_ref().is_some_and(|x| x.id == task_id)))
    {
      return Some((ti, bi));
    }
  }
  None
}

#[expect(
  clippy::expect_used,
  clippy::unreachable,
  reason = "every locator comes from find_tool / block_by_task on the same turns, which only point at tool rows"
)]
pub(super) fn tool_at(turns: &mut [Turn], (ti, bi): (usize, usize)) -> &mut ToolCallBlock {
  match &mut turns[ti].as_agent_mut().expect("agent turn").blocks[bi] {
    AgentBlock::ToolCall(tc) => tc,
    _ => unreachable!("locator points at a tool call"),
  }
}

pub fn find_tool(turns: &[Turn], id: &str) -> Option<(usize, usize)> {
  for (ti, t) in turns.iter().enumerate().rev() {
    let Turn::Agent(a) = t else { continue };
    if let Some(bi) = a.blocks.iter().position(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.id == id)) {
      return Some((ti, bi));
    }
  }
  None
}

pub fn find_tool_mut<'a>(turns: &'a mut [Turn], id: &str) -> Option<&'a mut ToolCallBlock> {
  let loc = find_tool(turns, id)?;
  Some(tool_at(turns, loc))
}

/// The named row adopts the task; a synthesized placeholder hosting it first comes out. Returns the row's new locator
fn attach_async_task(s: &mut NormalizeState, mut loc: (usize, usize), task_id: &str) -> (usize, usize) {
  if let Some(prev) = block_by_task(&s.turns, task_id)
    && prev != loc
  {
    let synthesized = tool_at(&mut s.turns, prev).id == format!("async:{task_id}");
    if synthesized {
      if let Some(t) = s.turns[prev.0].as_agent_mut() {
        t.blocks.remove(prev.1);
      }
      if prev.0 == loc.0 && prev.1 < loc.1 {
        loc.1 -= 1;
      }
    } else {
      tool_at(&mut s.turns, prev).async_task = None;
    }
  }
  let info = s.tasks.tasks.get(task_id).cloned();
  let b = tool_at(&mut s.turns, loc);
  b.async_task = info;
  b.background = Some(true);
  s.tasks.linked.insert(task_id.to_owned());
  loc
}

/// Write a linked task's info through to the row hosting it
fn sync_task(s: &mut NormalizeState, task_id: &str) {
  if !s.tasks.linked.contains(task_id) {
    return;
  }
  let Some(info) = s.tasks.tasks.get(task_id).cloned() else { return };
  if let Some(loc) = block_by_task(&s.turns, task_id) {
    tool_at(&mut s.turns, loc).async_task = Some(info);
  }
}

/// A task event named a toolCallId before the row existed; the row's arrival adopts the parked task
pub(super) fn link_async_task(s: &mut NormalizeState, loc: (usize, usize)) {
  let id = tool_at(&mut s.turns, loc).id.clone();
  let Some(task_id) = s.tasks.by_tool.remove(&id) else { return };
  let Some(state) = s.tasks.tasks.get(&task_id).map(|i| i.state) else { return };
  let loc = attach_async_task(s, loc, &task_id);
  tool_at(&mut s.turns, loc).status = task_status(state);
}

pub(super) fn time_tool_at(s: &mut NormalizeState, current: usize, loc: (usize, usize)) {
  let live = loc.0 == current && s.turns[current].as_agent().is_some_and(|t| t.started_at.is_some() && t.stop.is_none());
  // The block may have moved (a placeholder removal); find it again by position validity
  if let Some(Turn::Agent(t)) = s.turns.get_mut(loc.0)
    && let Some(AgentBlock::ToolCall(tc)) = t.blocks.get_mut(loc.1)
  {
    time_tool(tc, live);
  }
}

/// One asyncTasks event on the owning session's transcript; returns whether the transcript changed
pub fn apply_async_task(s: &mut NormalizeState, e: &AsyncTaskEvent) -> bool {
  let id = e.async_task_id.clone();
  {
    let info = s.tasks.tasks.entry(id.clone()).or_insert_with(|| AsyncTaskInfo {
      id: id.clone(),
      state: AsyncTaskState::Running,
      can_stop: false,
      task_type: None,
      name: None,
      description: None,
      summary: None,
      last_tool_name: None,
      output_file_path: None,
      usage: None,
      stop_requested: false,
    });
    if e.name.is_some() {
      info.name = e.name.clone();
    }
    if e.task_type.is_some() {
      info.task_type = e.task_type.clone();
    }
    if e.description.is_some() {
      info.description = e.description.clone();
    }
    if e.summary.is_some() {
      info.summary = e.summary.clone();
    }
    if e.last_tool_name.is_some() {
      info.last_tool_name = e.last_tool_name.clone();
    }
    if e.output_file_path.is_some() {
      info.output_file_path = e.output_file_path.clone();
    }
    if let Some(u) = &e.usage {
      let cur = info.usage.get_or_insert_with(Default::default);
      if u.total_tokens.is_some() {
        cur.total_tokens = u.total_tokens;
      }
      if u.tool_uses.is_some() {
        cur.tool_uses = u.tool_uses;
      }
      if u.duration_ms.is_some() {
        cur.duration_ms = u.duration_ms;
      }
    }
    if let Some(c) = e.can_stop {
      info.can_stop = c;
    }
    if e.event == TaskEventKind::State
      && let Some(st) = e.state
    {
      if task_state_moves(info.state, st) {
        info.state = st;
      }
      if task_terminal(info.state) {
        info.stop_requested = false;
      }
    }
  }
  sync_task(s, &id);
  let state = s.tasks.tasks[&id].state;
  let named = e.tool_call_id.as_deref().and_then(|tid| find_tool(&s.turns, tid));
  if let Some(tid) = &e.tool_call_id {
    if named.is_some() {
      s.tasks.by_tool.remove(tid);
    } else {
      s.tasks.by_tool.insert(tid.clone(), id.clone());
    }
  }
  let block = named.or_else(|| block_by_task(&s.turns, &id));
  if let Some(mut loc) = block {
    if named.is_some() {
      loc = attach_async_task(s, loc, &id);
    }
    let b = tool_at(&mut s.turns, loc);
    // A live wire event is observation itself
    b.observation = None;
    b.status = task_status(state);
    if task_terminal(state) {
      if b.ended_at.is_none() {
        b.ended_at = Some(now_ms());
      }
    } else {
      time_tool(b, true);
    }
    return true;
  }
  if e.show_in_transcript {
    let kind = if e.task_type.as_deref() == Some("shell") { ToolKind::Execute } else { ToolKind::Other };
    let row = ToolCallBlock {
      id: format!("async:{id}"),
      kind,
      verb: verb_of(kind),
      target: e.name.clone().or_else(|| e.description.clone()),
      status: task_status(state),
      background: Some(true),
      started_at: Some(now_ms()),
      async_task: Some(s.tasks.tasks[&id].clone()),
      ..empty_tool(String::new())
    };
    s.tasks.linked.insert(id);
    push_standalone(s, AgentBlock::ToolCall(row));
    return true;
  }
  s.log(&format!("async task {id}: no transcript row (no toolCallId match, showInTranscript off)"));
  false
}

/// Mark a stop request on a task (shared with its row); false when the task is unknown
pub fn set_stop_requested(s: &mut NormalizeState, task_id: &str, on: bool) -> bool {
  let Some(info) = s.tasks.tasks.get_mut(task_id) else { return false };
  info.stop_requested = on;
  sync_task(s, task_id);
  true
}

/// The process carrying the tasks is gone: live tasks lose their observer
pub fn disconnect_async_tasks(s: &mut NormalizeState) {
  let mut touched = vec![];
  for t in s.turns.iter_mut().filter_map(Turn::as_agent_mut) {
    for b in &mut t.blocks {
      let AgentBlock::ToolCall(tc) = b else { continue };
      let Some(a) = tc.async_task.as_mut() else { continue };
      if task_terminal(a.state) {
        continue;
      }
      tc.observation = Some(Observation::Unknown);
      a.can_stop = false;
      a.stop_requested = false;
      touched.push(a.id.clone());
    }
  }
  for id in touched {
    if s.tasks.linked.contains(&id)
      && let Some(info) = s.tasks.tasks.get_mut(&id)
    {
      info.can_stop = false;
      info.stop_requested = false;
    }
  }
}
