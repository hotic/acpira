//! Antigravity's orchestrator models (agy 1.5.16, `agent-quirks-antigravity.md`): a planning model and a faster execution model take
//! turns inside the one conversation. The planner's `exit_plan_mode` hands its plan over and the executor's
//! `enter_plan_mode` hands back; nothing else on the wire tells the two apart, so all the root streams in between is the
//! executor's. Each stretch is a round of one sidekick thread, drawn as a relay like Devin Fusion's (`fusion.rs`), the
//! plan its task and the report its result. The executor's words stay on the root: its last ones are the turn's reply

use serde_json::Value;

use acpira_shared::subagents::SubagentState;

use super::{Dialect, Route, RouteCtx, SubagentTree};
use crate::acp::vendors::antigravity::{PlanRelay, plan_relay_of, plan_title};

/// The CLI whose orchestrators these are
const ORCHESTRATOR_AGENT: &str = "antigravity";

impl SubagentTree {
  /// Whether the session runs an orchestrator now, `worker` naming its execution model so far. A name the session's
  /// transcript already gave (`set_orchestrator_worker`) is kept over the stand-in
  pub fn set_orchestrator(&mut self, worker: Option<String>) {
    self.orchestrator = worker.map(|w| self.orchestrator.take().unwrap_or(w));
  }

  /// The execution model the session's transcript names: the running round and the rounds after it take it
  pub fn set_orchestrator_worker(&mut self, name: String) -> bool {
    if self.orchestrator.is_none() {
      return false;
    }
    let mut changed = false;
    for n in self.nodes.iter_mut().filter(|n| n.dialect == Some(Dialect::Orchestrator) && n.status == SubagentState::Running) {
      if n.model.as_deref() != Some(name.as_str()) {
        n.model = Some(name.clone());
        n.rev += 1;
        changed = true;
      }
    }
    self.orchestrator = Some(name);
    changed
  }

  fn running_orchestrator_round(&self) -> Option<usize> {
    self.nodes.iter().rposition(|n| n.dialect == Some(Dialect::Orchestrator) && n.status == SubagentState::Running)
  }

  pub fn has_running_orchestrator_round(&self) -> bool {
    self.running_orchestrator_round().is_some()
  }

  /// The running round whose execution model made call `tool_call_id`: the call's permission request belongs there
  pub fn orchestrator_owner(&self, tool_call_id: &str) -> Option<String> {
    let id = &self.nodes[self.running_orchestrator_round()?].id;
    (self.tool_owner.get(tool_call_id) == Some(id)).then(|| id.clone())
  }

  /// The orchestrator's part of the root stream; `None` leaves the update to the other dialects
  pub(super) fn route_orchestrator(&mut self, u: &Value, ctx: &mut RouteCtx) -> Option<Route> {
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    let call = u.get("toolCallId").and_then(Value::as_str).unwrap_or("");
    let relay = plan_relay_of(u);
    // A hand-off row's later updates: the relay already stands in for it, and they may bring what its first one lacked
    if let Some(node) = self.relay_calls.get(call).cloned() {
      if let Some(i) = self.idx(&node) {
        match relay {
          Some(PlanRelay::Handoff { plan: Some(plan) }) if self.nodes[i].task.is_none() => self.set_plan(i, plan),
          Some(PlanRelay::Handback { reason: Some(reason) }) if self.nodes[i].result.is_none() => {
            self.nodes[i].result = Some(reason);
            self.nodes[i].rev += 1;
          }
          _ => {}
        }
      }
      return Some(Route::Consumed);
    }
    let running = self.running_orchestrator_round();
    match relay {
      Some(PlanRelay::Handoff { plan }) if self.orchestrator.is_some() => {
        // A plan handed over again with no report between: the round before ended there
        if let Some(i) = running {
          self.end_native_round(i, SubagentState::Completed);
        }
        let worker = self.orchestrator.clone();
        let id = self.open_round_of(ctx, ORCHESTRATOR_AGENT, Dialect::Orchestrator, worker, None);
        if let (Some(plan), Some(i)) = (plan, self.idx(&id)) {
          self.set_plan(i, plan);
        }
        self.relay_calls.insert(call.to_owned(), id);
        return Some(Route::Consumed);
      }
      Some(PlanRelay::Handback { reason }) => {
        let i = running?;
        self.relay_calls.insert(call.to_owned(), self.nodes[i].id.clone());
        self.nodes[i].result = reason;
        self.end_native_round(i, SubagentState::Completed);
        return Some(Route::Consumed);
      }
      _ => {}
    }
    let id = self.nodes[running?].id.clone();
    match kind {
      "tool_call" | "agent_thought_chunk" => {}
      // A call the planner left running before the hand-off stays the planner's
      "tool_call_update" if self.tool_owner.get(call) == Some(&id) => {}
      _ => return None,
    }
    self.apply_child(&id, u, ctx);
    Some(Route::Consumed)
  }

  fn set_plan(&mut self, i: usize, plan: String) {
    let n = &mut self.nodes[i];
    n.title = plan_title(&plan);
    n.task = Some(plan);
    n.rev += 1;
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use serde_json::{Value, json};

  use acpira_shared::subagents::{SubagentState, SubagentVisibility};
  use acpira_shared::transcript::{AgentBlock, AgentTurn, Turn};

  use super::super::{Route, RouteCtx, SubagentTree};
  use crate::acp::transcript::normalize::ToolCtx;

  const PLAN: &str = "### Plan\n\n1. **Create** `fib.py` printing the first 20 numbers\n2. Run it\n\n### Re-plan if:\n- it fails";

  fn tree() -> SubagentTree {
    let mut tree = SubagentTree::new(Arc::new(|_: &str| {}), ToolCtx::default(), None, None, false);
    tree.set_orchestrator(Some("Gemini Flash Worker".into()));
    tree
  }

  /// A call as agy 1.5.16 sends it: its tool name beside the ACP fields, the input twice
  fn call(id: &str, name: &str, input: Value, status: &str) -> Value {
    let title = if name == "run_command" { "ls".to_owned() } else { format!("Run {name}") };
    let kind = if name == "run_command" { "execute" } else { "other" };
    json!({ "sessionUpdate": "tool_call", "toolCallId": id, "name": name, "args": input, "rawInput": input, "title": title, "kind": kind, "status": status })
  }

  fn done(id: &str, name: &str) -> Value {
    json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "name": name, "status": "completed" })
  }

  fn handoff(id: &str) -> Value {
    call(id, "exit_plan_mode", json!({ "plan": PLAN }), "in_progress")
  }

  fn says(text: &str) -> Value {
    json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } })
  }

  fn thinks() -> Value {
    json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "hm" } })
  }

  /// Routes one root update as the session does; true when a child took it
  fn feed(tree: &mut SubagentTree, turns: &mut Vec<Turn>, u: Value) -> bool {
    matches!(tree.route_root(&u, &mut RouteCtx { turn_index: 0, root_turns: turns }), Route::Consumed)
  }

  /// The rows the tree put on the root (root rows proper come from the session's own normalization)
  fn anchors(turns: &[Turn]) -> Vec<String> {
    turns
      .iter()
      .filter_map(Turn::as_agent)
      .flat_map(|t| &t.blocks)
      .filter_map(|b| match b {
        AgentBlock::ToolCall(tc) => tc.subagent_id.clone(),
        _ => None,
      })
      .collect()
  }

  #[test]
  fn a_handed_off_plan_is_a_sidekick_round_until_the_prompt_returns() {
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    // The planner looks around on the root
    assert!(!feed(&mut tree, &mut turns, thinks()));
    assert!(!feed(&mut tree, &mut turns, call("c1", "run_command", json!({ "CommandLine": "ls" }), "in_progress")));
    assert!(!feed(&mut tree, &mut turns, done("c1", "run_command")));
    // The hand-off opens the round and its row never shows
    assert!(feed(&mut tree, &mut turns, handoff("c2")));
    assert!(feed(&mut tree, &mut turns, done("c2", "exit_plan_mode")));
    // The executor thinks and works inside the round, and says its piece on the root
    assert!(feed(&mut tree, &mut turns, thinks()));
    assert!(feed(&mut tree, &mut turns, call("c3", "write_file", json!({ "Path": "fib.py" }), "in_progress")));
    assert!(feed(&mut tree, &mut turns, done("c3", "write_file")));
    assert!(!feed(&mut tree, &mut turns, says("Done: fib.py prints 20 numbers.")));
    let n = tree.summaries()[0].clone();
    assert_eq!((n.core.state, n.core.visibility, n.core.tool_count), (SubagentState::Running, SubagentVisibility::Nested, 1));
    assert_eq!(n.core.model.as_deref(), Some("Gemini Flash Worker"));
    assert_eq!(n.core.title.as_deref(), Some("Create fib.py printing the first 20 numbers"));
    assert_eq!(n.core.task.as_deref(), Some(PLAN));
    let h = n.core.harness.as_ref().unwrap();
    assert_eq!((h.agent.as_str(), h.persona.as_deref(), h.round), ("antigravity", Some("sidekick"), 1));
    assert_eq!(anchors(&turns), vec![n.core.id.clone()]);
    // Its words never end the round: the prompt returning does
    tree.settle("prompt-returned", false);
    let n = tree.summaries()[0].clone();
    assert_eq!(n.core.state, SubagentState::Completed);
    assert_eq!(n.core.result, None);
  }

  #[test]
  fn the_executor_hands_back_with_its_report_and_the_next_plan_is_round_two() {
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, handoff("c1"));
    feed(&mut tree, &mut turns, call("c2", "run_command", json!({ "CommandLine": "pytest" }), "completed"));
    assert_eq!(tree.orchestrator_owner("c2").as_deref(), Some(tree.summaries()[0].core.id.as_str()));
    assert!(feed(&mut tree, &mut turns, call("c3", "enter_plan_mode", json!({ "reason": "pytest fails twice: no fixtures" }), "in_progress")));
    let first = tree.summaries()[0].clone();
    assert_eq!(first.core.state, SubagentState::Completed);
    assert_eq!(first.core.result.as_deref(), Some("pytest fails twice: no fixtures"));
    assert!(tree.orchestrator_owner("c2").is_none());
    // Its row's later updates stay hidden; the planner reviews on the root
    assert!(feed(&mut tree, &mut turns, done("c3", "enter_plan_mode")));
    assert!(!feed(&mut tree, &mut turns, thinks()));
    assert!(!feed(&mut tree, &mut turns, call("c4", "run_command", json!({ "CommandLine": "ls tests" }), "completed")));
    // The revised plan opens the thread's second round
    assert!(feed(&mut tree, &mut turns, handoff("c5")));
    let list = tree.summaries();
    let h = list[1].core.harness.as_ref().unwrap();
    assert_eq!((h.thread.as_str(), h.round), (first.core.id.as_str(), 2));
    assert_eq!(anchors(&turns), vec![first.core.id.clone(), list[1].core.id.clone()]);
  }

  #[test]
  fn only_an_orchestrator_hands_off_and_a_planner_call_stays_the_planners() {
    // Plain plan mode on another model: exit_plan_mode is the CLI's own row
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    tree.set_orchestrator(None);
    assert!(!feed(&mut tree, &mut turns, handoff("c1")));
    assert!(tree.summaries().is_empty());
    // A command the planner sent to the background before the hand-off finishes on the root
    let (mut tree, mut turns) = (self::tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, call("c1", "run_command", json!({ "CommandLine": "sleep 9" }), "in_progress"));
    feed(&mut tree, &mut turns, handoff("c2"));
    assert!(!feed(&mut tree, &mut turns, done("c1", "run_command")));
    assert_eq!(tree.summaries()[0].core.tool_count, 0);
  }

  #[test]
  fn the_transcripts_worker_renames_the_running_round_and_outlives_the_stand_in() {
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, handoff("c1"));
    assert!(tree.set_orchestrator_worker("Claude Sonnet 5.5".into()));
    assert!(!tree.set_orchestrator_worker("Claude Sonnet 5.5".into()));
    assert_eq!(tree.summaries()[0].core.model.as_deref(), Some("Claude Sonnet 5.5"));
    tree.set_orchestrator(Some("Gemini Flash Worker".into()));
    feed(&mut tree, &mut turns, call("c2", "enter_plan_mode", json!({}), "in_progress"));
    feed(&mut tree, &mut turns, handoff("c3"));
    assert_eq!(tree.summaries()[1].core.model.as_deref(), Some("Claude Sonnet 5.5"));
    // A stopped turn cancels the running round
    tree.native_rounds_end(SubagentState::Cancelled);
    assert_eq!(tree.summaries()[1].core.state, SubagentState::Cancelled);
  }
}
