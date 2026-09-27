//! Plan snapshot dedup

use acpira_shared::todo_tools::is_todo_tool;
use acpira_shared::transcript::{AgentBlock, PlanBlock, PlanEntry, PlanStatus, Turn};

pub fn same_plan_entries(a: &[PlanEntry], b: &[PlanEntry]) -> bool {
  a == b
}

pub fn last_plan_snapshot(turns: &[Turn]) -> Option<&PlanBlock> {
  turns.iter().rev().filter_map(Turn::as_agent).find_map(|t| {
    t.blocks.iter().rev().find_map(|b| match b {
      AgentBlock::Plan(p) => Some(p),
      _ => None,
    })
  })
}

/// Older hosts appended Grok's unchanged completed snapshot to every follow-up; drop those copies when opening a record
pub fn restore_plan_snapshots(mut turns: Vec<Turn>) -> Vec<Turn> {
  let mut previous: Option<Vec<PlanEntry>> = None;
  for turn in &mut turns {
    let Turn::Agent(t) = turn else { continue };
    let edited = t.blocks.iter().any(|b| matches!(b, AgentBlock::ToolCall(tc) if is_todo_tool(tc)));
    t.blocks.retain(|b| {
      let AgentBlock::Plan(p) = b else { return true };
      let repeated = !p.changed
        && !edited
        && !p.entries.is_empty()
        && p.entries.iter().all(|e| e.status == PlanStatus::Completed)
        && previous.as_ref().is_some_and(|prev| same_plan_entries(prev, &p.entries));
      previous = Some(p.entries.clone());
      !repeated
    });
  }
  turns
}
