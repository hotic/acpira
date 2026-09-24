//! Plan execution instruction (mirror of src/shared/planExecution.ts)

use crate::transcript::{AgentBlock, PlanDocStatus, Turn};

pub fn plan_execution_prompt(markdown: &str) -> String {
  format!("Implement the following approved plan:\n\n{markdown}")
}

pub fn plan_execution_id(turn: &Turn, previous: Option<&Turn>) -> Option<String> {
  let Turn::User(u) = turn else { return None };
  if let Some(id) = &u.plan_id {
    return Some(id.clone());
  }
  if u.attachments.as_ref().is_some_and(|a| !a.is_empty()) {
    return None;
  }
  let Some(Turn::Agent(prev)) = previous else { return None };
  prev.blocks.iter().find_map(|b| match b {
    AgentBlock::PlanDocument(p) if p.status == PlanDocStatus::Executing && u.text == plan_execution_prompt(&p.markdown) => {
      Some(p.id.clone())
    }
    _ => None,
  })
}
