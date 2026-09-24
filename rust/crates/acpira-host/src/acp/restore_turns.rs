//! Interrupted-turn repair when a record is opened: a disk snapshot carries
//! display history, never ownership of a live request or background shell

use acpira_shared::num::Num;
use acpira_shared::transcript::{AgentBlock, AsyncTaskState, CompactionStatus, Observation, QuestionOutcome, ToolStatus, Turn, TurnStop};

use crate::util::ms_of_iso;

fn unfinished(b: &AgentBlock) -> bool {
  match b {
    AgentBlock::Text(x) => x.streaming == Some(true),
    AgentBlock::Thought(x) => x.streaming == Some(true),
    AgentBlock::ToolCall(x) => x.status.is_open(),
    AgentBlock::Compaction(x) => x.status == CompactionStatus::InProgress,
    AgentBlock::Permission(_) => true,
    AgentBlock::Question(x) => x.outcome.is_none(),
    _ => false,
  }
}

pub fn restore_interrupted_turns(mut turns: Vec<Turn>, updated_at: &str) -> Vec<Turn> {
  let mut observed = ms_of_iso(updated_at).unwrap_or(0);
  for t in turns.iter().filter_map(Turn::as_agent) {
    observed = observed.max(t.started_at.unwrap_or(0)).max(t.ended_at.unwrap_or(0));
    for b in &t.blocks {
      match b {
        AgentBlock::ToolCall(x) => observed = observed.max(x.started_at.unwrap_or(0)).max(x.ended_at.unwrap_or(0)),
        AgentBlock::Thought(x) => observed = observed.max(x.started_at.unwrap_or(0)),
        _ => {}
      }
    }
  }
  for turn in &mut turns {
    let Turn::Agent(t) = turn else { continue };
    let active = t.blocks.iter().any(unfinished);
    if !active && (t.stop.is_some() || t.started_at.is_none()) {
      continue;
    }
    let ended = t.ended_at.or((observed != 0).then_some(observed));
    t.blocks.retain(|b| !matches!(b, AgentBlock::Permission(_)));
    for b in &mut t.blocks {
      if !unfinished(b) {
        continue;
      }
      match b {
        AgentBlock::Text(x) => x.streaming = Some(false),
        AgentBlock::Thought(x) => {
          x.streaming = Some(false);
          if let (Some(s), Some(e)) = (x.started_at, ended) {
            x.duration_sec = Some(Num((((e - s) as f64) / 1000.0).round().max(0.0)));
          }
        }
        AgentBlock::ToolCall(x) => {
          x.status = ToolStatus::Cancelled;
          if let Some(task) = x.async_task.as_mut()
            && matches!(task.state, AsyncTaskState::Running | AsyncTaskState::Paused)
          {
            task.stop_requested = false;
            task.can_stop = false;
            x.observation = Some(Observation::Unknown);
          }
          if let (Some(s), Some(e)) = (x.started_at, ended) {
            x.ended_at = Some(s.max(e));
          }
        }
        AgentBlock::Compaction(x) => x.status = CompactionStatus::Cancelled,
        AgentBlock::Question(x) => x.outcome = Some(QuestionOutcome::Cancelled),
        _ => {}
      }
    }
    t.activity = None;
    if t.stop.is_none() {
      t.stop = Some(TurnStop::Cancelled);
    }
    if t.started_at.is_some() {
      t.ended_at = ended;
    }
  }
  turns
}
