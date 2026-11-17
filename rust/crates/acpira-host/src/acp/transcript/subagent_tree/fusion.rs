//! Devin Fusion's sidekick: the lead's second model works inside the lead's own session, and of all it does only its tool
//! calls reach the root stream, under `sk::` ids (`agent-quirks-devin.md`). The wire has no handoff, brief or handback, so its
//! calls are cut into rounds here, each a node the clients draw as a sidekick relay

use serde_json::Value;

use acpira_shared::subagents::{RelayMode, SIDEKICK_ID, StateSource, SubagentHarness, SubagentPeer, SubagentState, SubagentVisibility};
use acpira_shared::transcript::{AgentBlock, Turn};

use super::{Dialect, Node, RouteCtx, SubagentTree, seal_node};

/// What Devin puts in front of the id of every tool call its Fusion sidekick makes
const SIDEKICK_CALL: &str = "sk::";
/// The only CLI with a sidekick of its own on the wire
const FUSION_AGENT: &str = "devin";

/// A tool call (or its update) of the agent's own sidekick
pub fn native_sidekick_call(u: &Value) -> bool {
  matches!(u.get("sessionUpdate").and_then(Value::as_str), Some("tool_call" | "tool_call_update"))
    && u.get("toolCallId").and_then(Value::as_str).is_some_and(|id| id.starts_with(SIDEKICK_CALL))
}

/// The lead itself did something on the root stream: called a tool of its own, or spoke. Thinking is not acting
fn lead_acted(u: &Value) -> bool {
  match u.get("sessionUpdate").and_then(Value::as_str) {
    Some("tool_call") => true,
    Some("agent_message_chunk") => {
      let content = u.get("content");
      content.and_then(|c| c.get("type")).and_then(Value::as_str) == Some("text")
        && content.and_then(|c| c.get("text")).and_then(Value::as_str).is_some_and(|t| !t.trim().is_empty())
    }
    _ => false,
  }
}

/// A foreground call the round still waits on: the sidekick is evidently at work
fn has_open_call(turns: &[Turn]) -> bool {
  turns
    .iter()
    .filter_map(Turn::as_agent)
    .flat_map(|t| &t.blocks)
    .any(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.status.is_open() && tc.background != Some(true)))
}

impl SubagentTree {
  /// A round of the agent's own sidekick. Told from a summoned sidekick round (a `session` node Acpira runs itself) by
  /// its tier, which a record keeps
  pub(super) fn native_round(n: &Node) -> bool {
    n.visibility == SubagentVisibility::Nested && Self::is_sidekick(n)
  }

  /// The harness of a child Acpira runs itself in another CLI. A native sidekick round carries one too, for the clients'
  /// sake, but has no process, thread session or caller: nothing of the relay may take it for a summoned round
  pub(super) fn summoned(n: &Node) -> Option<&SubagentHarness> {
    n.harness.as_ref().filter(|_| !Self::native_round(n))
  }

  /// The sidekick half of the session's model pair, for the rounds opened from now on
  pub fn set_native_sidekick(&mut self, model: Option<String>, effort: Option<String>) {
    self.native_sidekick = (model, effort);
  }

  /// A sidekick call on the root stream goes to the round it opened in, else to the running round, else it opens one
  pub(super) fn route_native_sidekick(&mut self, u: &Value, ctx: &mut RouteCtx) {
    let call = u.get("toolCallId").and_then(Value::as_str).unwrap_or("");
    // A round that ended keeps its calls: a late update is dropped there, never taken for new work
    if let Some(owner) = self.tool_owner.get(call).cloned()
      && self.idx(&owner).is_some()
    {
      self.apply_child(&owner, u, ctx);
      return;
    }
    let running = self.nodes.iter().rposition(|n| Self::native_round(n) && n.status == SubagentState::Running);
    let id = match running {
      Some(i) => self.nodes[i].id.clone(),
      None if u.get("sessionUpdate").and_then(Value::as_str) == Some("tool_call") => self.open_native_round(ctx),
      None => {
        self.log(&format!("sidekick update {call} has no round; dropped"));
        return;
      }
    };
    self.apply_child(&id, u, ctx);
  }

  /// The next round of the sidekick's one thread, placed where the root turn stands now
  fn open_native_round(&mut self, ctx: &mut RouteCtx) -> String {
    let prior = self.nodes.iter().rev().find(|n| Self::native_round(n)).and_then(|n| n.harness.as_ref()).map(|h| (h.thread.clone(), h.round));
    let mut node = self.new_node(SubagentVisibility::Nested, SubagentPeer::default(), ctx.turn_index, false, Some(Dialect::Fusion));
    let (thread, round) = prior.map_or_else(|| (node.id.clone(), 1), |(thread, round)| (thread, round + 1));
    let (model, effort) = self.native_sidekick.clone();
    node.model = model;
    // The name a summoned sidekick's persona carries (`host_sidekick.rs`): what an inspector titles a round with no brief
    node.role = Some("Sidekick".into());
    node.harness = Some(SubagentHarness {
      agent: FUSION_AGENT.into(),
      persona: Some(SIDEKICK_ID.into()),
      mode: RelayMode::Work,
      thread,
      round,
      session_id: None,
      effort,
      group: None,
    });
    let id = node.id.clone();
    self.nodes.push(node);
    ctx.place_anchor(&id, None);
    self.bump(&id);
    id
  }

  /// What reached the root may be the lead taking over again: the running round is handed back unless a call of it is
  /// still open, which means the two work side by side
  pub(super) fn native_handback(&mut self, u: &Value) {
    if !lead_acted(u) {
      return;
    }
    for i in 0..self.nodes.len() {
      let n = &self.nodes[i];
      if Self::native_round(n) && n.status == SubagentState::Running && !has_open_call(&n.state.turns) {
        self.end_native_round(i, SubagentState::Completed);
      }
    }
  }

  /// Every running native round ends as `state`: the session calls it before a turn that was stopped or failed settles
  pub fn native_rounds_end(&mut self, state: SubagentState) {
    for i in 0..self.nodes.len() {
      let n = &self.nodes[i];
      if Self::native_round(n) && n.status == SubagentState::Running {
        self.end_native_round(i, state);
      }
    }
  }

  /// The end is the engine's reading, never the agent's word
  fn end_native_round(&mut self, i: usize, state: SubagentState) {
    let n = &mut self.nodes[i];
    n.status = state;
    n.state_source = StateSource::Local;
    seal_node(n, state);
    n.rev += 1;
    self.terminal_events.push(n.id.clone());
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use serde_json::{Value, json};

  use acpira_shared::subagents::{SubagentState, SubagentVisibility};
  use acpira_shared::transcript::{AgentBlock, AgentTurn, ToolStatus, Turn};

  use super::super::{Route, RouteCtx, SubagentTree};
  use crate::acp::transcript::normalize::ToolCtx;

  fn tree() -> SubagentTree {
    let mut tree = SubagentTree::new(Arc::new(|_: &str| {}), ToolCtx::default(), None, None, false);
    tree.set_native_sidekick(Some("SWE-2".into()), Some("Medium".into()));
    tree
  }

  fn call(id: &str, status: &str) -> Value {
    json!({ "sessionUpdate": "tool_call", "toolCallId": id, "title": "ls", "kind": "execute", "status": status })
  }

  fn done(id: &str) -> Value {
    json!({ "sessionUpdate": "tool_call_update", "toolCallId": id, "status": "completed" })
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
  fn a_stretch_of_sidekick_calls_is_one_round_on_an_anchor_row() {
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    assert!(feed(&mut tree, &mut turns, call("sk::exec:0", "in_progress")));
    assert!(feed(&mut tree, &mut turns, done("sk::exec:0")));
    assert!(!feed(&mut tree, &mut turns, thinks()));
    assert!(feed(&mut tree, &mut turns, call("sk::read:1", "completed")));
    let list = tree.summaries();
    assert_eq!(list.len(), 1);
    let n = &list[0];
    assert_eq!((n.core.state, n.core.visibility, n.core.tool_count), (SubagentState::Running, SubagentVisibility::Nested, 2));
    assert_eq!(n.core.model.as_deref(), Some("SWE-2"));
    let h = n.core.harness.as_ref().unwrap();
    assert_eq!((h.agent.as_str(), h.persona.as_deref(), h.round, h.effort.as_deref()), ("devin", Some("sidekick"), 1, Some("Medium")));
    assert_eq!(h.thread, n.core.id);
    assert_eq!(n.steps.as_ref().map(Vec::len), Some(2));
    // Nothing of the sidekick's sits on the root but the row its relay hangs on
    assert_eq!(anchors(&turns), vec![n.core.id.clone()]);
    assert_eq!(turns[0].as_agent().unwrap().blocks.len(), 1);
  }

  #[test]
  fn the_lead_takes_over_only_from_a_sidekick_with_no_call_open() {
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, call("sk::exec:0", "in_progress"));
    // Side by side: the sidekick's command still runs while the lead speaks and calls a tool
    assert!(!feed(&mut tree, &mut turns, says("Meanwhile")));
    assert!(!feed(&mut tree, &mut turns, call("toolu_1", "completed")));
    assert_eq!(tree.summaries()[0].core.state, SubagentState::Running);
    feed(&mut tree, &mut turns, done("sk::exec:0"));
    assert!(!feed(&mut tree, &mut turns, thinks()));
    assert!(!feed(&mut tree, &mut turns, says("  ")));
    assert_eq!(tree.summaries()[0].core.state, SubagentState::Running);
    assert!(!feed(&mut tree, &mut turns, says("Reviewed.")));
    let first = tree.summaries()[0].clone();
    assert_eq!(first.core.state, SubagentState::Completed);
    assert!(first.core.ended_at.is_some());
    assert_eq!(tree.terminal_events, vec![first.core.id.clone()]);
    // A late word on a call of the ended round stays there and opens nothing
    assert!(feed(&mut tree, &mut turns, done("sk::exec:0")));
    assert_eq!(tree.summaries().len(), 1);
    // The next stretch is the same thread's second round, on a row of its own
    assert!(feed(&mut tree, &mut turns, call("sk::exec:1", "completed")));
    let list = tree.summaries();
    assert_eq!(list.len(), 2);
    let h = list[1].core.harness.as_ref().unwrap();
    assert_eq!((h.thread.as_str(), h.round), (first.core.id.as_str(), 2));
    assert_eq!(list[1].core.state, SubagentState::Running);
    assert_eq!(anchors(&turns), vec![first.core.id.clone(), list[1].core.id.clone()]);
  }

  #[test]
  fn the_prompt_returning_hands_the_round_back() {
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, call("sk::exec:0", "completed"));
    tree.settle("prompt-returned", false);
    assert_eq!(tree.summaries()[0].core.state, SubagentState::Completed);

    let (mut tree, mut turns) = (self::tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, call("sk::exec:0", "in_progress"));
    tree.native_rounds_end(SubagentState::Cancelled);
    tree.settle("prompt-returned", false);
    let n = tree.summaries()[0].clone();
    assert_eq!(n.core.state, SubagentState::Cancelled);
    assert_eq!(n.steps.unwrap()[0].status, ToolStatus::Cancelled);

    let (mut tree, mut turns) = (self::tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, call("sk::exec:0", "completed"));
    tree.settle("connection-lost", false);
    assert_eq!(tree.summaries()[0].core.state, SubagentState::Disconnected);
  }

  #[test]
  fn the_relay_never_takes_a_native_round_for_a_summoned_one() {
    let (mut tree, mut turns) = (tree(), vec![Turn::Agent(AgentTurn::default())]);
    feed(&mut tree, &mut turns, call("sk::exec:0", "in_progress"));
    let id = tree.summaries()[0].core.id.clone();
    assert!(tree.relay_running().is_empty());
    assert!(tree.relay_running_from(0).is_empty());
    assert!(tree.relay_latest(&id).is_none());
    assert!(tree.relay_thread_of(&id).is_none());
    assert!(tree.relay_harness(&id).is_none());
    assert!(tree.relay_turns(&id).is_none());
    assert!(tree.relay_thread_turns(&id).is_empty());
    assert!(tree.relay_unlinked("", 0, |_, _| true).is_none());
    assert!(!tree.relay_cancel(&id));
    // A record brings the round back without its dialect: it is still no summoned round, and no longer running
    let mut back = SubagentTree::new(Arc::new(|_: &str| {}), ToolCtx::default(), Some(tree.to_records()), None, false);
    assert_eq!(back.summaries()[0].core.state, SubagentState::Disconnected);
    assert!(back.relay_latest(&id).is_none());
    assert!(back.relay_thread_of(&id).is_none());
    // The thread goes on where the record left it
    let mut turns = vec![Turn::Agent(AgentTurn::default())];
    feed(&mut back, &mut turns, call("sk::exec:0", "completed"));
    let h = back.summaries()[1].core.harness.clone().unwrap();
    assert_eq!((h.thread.as_str(), h.round), (id.as_str(), 2));
  }
}
