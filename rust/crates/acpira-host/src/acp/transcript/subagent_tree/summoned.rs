//! Summoned children (`acp/session/relay.rs`): a `session` node per round, bound to the child's native session once it
//! is known, and what the relay reads back of it (its thread, reply, controls and outcome)

use acpira_shared::subagents::{SubagentHarness, SubagentPeer, SubagentState, SubagentVisibility};
use acpira_shared::transcript::{AgentBlock, AgentTurn, Turn, TurnStop};

use super::{RelayOpen, RouteCtx, SubagentTree};
use crate::acp::transcript::normalize::end_turn;
use crate::i18n::t;
use crate::util::now_ms;

impl SubagentTree {
  /// A child Acpira is about to run in another CLI: a `session` node without a peer yet (`relay_bind` adds it once
  /// session/new answers). `parent` is another node when a summoned child summons in turn
  /// An empty `harness.thread` starts a new thread named after this node
  pub fn relay_open(&mut self, o: RelayOpen) -> String {
    let RelayOpen { parent, turn_index, mut harness, role, title, task, model } = o;
    let mut node = self.new_node(SubagentVisibility::Session, SubagentPeer::default(), turn_index, true, None);
    if harness.thread.is_empty() {
      harness.thread = node.id.clone();
    }
    node.parent_id = parent.filter(|p| self.idx(p).is_some());
    node.title = Some(title);
    node.task = Some(task);
    node.role = Some(role);
    node.model = model;
    node.harness = Some(harness);
    let id = node.id.clone();
    self.nodes.push(node);
    self.bump(&id);
    id
  }

  /// The child's native session id is known: its updates and requests route to the node from now on, and whatever
  /// arrived under that id before (session/new still in flight) is replayed first
  /// `route` is the key the child's handlers stamp on its traffic (`relay:<thread>`), `native` the CLI's own session id
  pub fn relay_bind(&mut self, id: &str, route: &str, native: &str, ctx: &mut RouteCtx) {
    let Some(i) = self.idx(id) else { return };
    self.nodes[i].peer.session_id = Some(route.to_owned());
    if let Some(h) = self.nodes[i].harness.as_mut() {
      h.session_id = Some(native.to_owned());
    }
    self.by_session.insert(route.to_owned(), id.to_owned());
    self.replay_orphans(route, id, ctx);
    self.bump(id);
  }

  /// The child's prompt answered while rounds it sent off are still out: its row says it waits for them
  pub fn relay_waiting(&mut self, id: &str) {
    let Some(i) = self.idx(id) else { return };
    let label = t("verb.awaitSubagent");
    let Some(Turn::Agent(turn)) = self.nodes[i].state.turns.last_mut() else { return };
    if turn.activity.as_ref().is_some_and(|a| a.label == label) {
      return;
    }
    turn.activity = Some(acpira_shared::transcript::Activity { kind: acpira_shared::transcript::ToolKind::Other, label });
    self.bump(id);
  }

  /// The results of rounds the child sent off go back to it within its own round: its answer so far is sealed, and the
  /// automatic message carrying them opens the next turn of its transcript
  pub fn relay_follow_up(&mut self, id: &str, text: &str) {
    let Some(i) = self.idx(id) else { return };
    let st = &mut self.nodes[i].state;
    end_turn(st, TurnStop::EndTurn);
    st.turns.push(Turn::User(acpira_shared::transcript::UserTurn {
      text: text.to_owned(),
      auto: Some(true),
      auto_reason: Some(acpira_shared::transcript::AutoReason::Subagents),
      ..Default::default()
    }));
    st.turns.push(Turn::Agent(AgentTurn { started_at: Some(now_ms()), ..Default::default() }));
    self.bump(id);
  }

  /// The round ended (`result` = the child's reply, or the error shown in its place)
  pub fn relay_end(&mut self, id: &str, state: SubagentState, result: Option<String>) {
    let Some(i) = self.idx(id) else { return };
    if result.is_some() {
      self.nodes[i].result = result;
    }
    if self.nodes[i].status == SubagentState::Running {
      self.transition(id, state);
    }
    self.bump(id);
  }

  /// The root's `ask_agent` call that summoned this node: the row hides behind the node, as a delegation row does
  pub fn relay_link_call(&mut self, id: &str, tool_call_id: &str, ctx: &mut RouteCtx) -> bool {
    let Some(i) = self.idx(id) else { return false };
    if self.nodes[i].peer.tool_call_id.is_none() {
      self.nodes[i].peer.tool_call_id = Some(tool_call_id.to_owned());
      self.by_tool.insert(tool_call_id.to_owned(), id.to_owned());
    }
    let Some(block) = ctx.root_tool(tool_call_id) else { return false };
    block.subagent_id = Some(id.to_owned());
    self.bump(id);
    true
  }

  /// The latest round of a thread: (node id, harness, persona name, native session id, state)
  pub fn relay_latest(&self, thread: &str) -> Option<(String, SubagentHarness, Option<String>, Option<String>, SubagentState)> {
    let n = self.nodes.iter().rev().find(|n| n.harness.as_ref().is_some_and(|h| h.thread == thread || n.id == thread))?;
    let h = n.harness.clone()?;
    let native = h.session_id.clone();
    Some((n.id.clone(), h, n.role.clone(), native, n.status))
  }

  /// A summoned node's harness and persona name
  pub fn relay_harness(&self, id: &str) -> Option<(SubagentHarness, Option<String>)> {
    self.nodes.iter().find(|n| n.id == id).and_then(|n| Some((n.harness.clone()?, n.role.clone())))
  }

  /// Whether a summoned node runs without approvals (its round's mode is bypass)
  pub fn relay_bypass(&self, id: &str) -> bool {
    self.nodes.iter().find(|n| n.id == id).and_then(|n| n.harness.as_ref()).is_some_and(|h| h.mode == acpira_shared::subagents::RelayMode::Bypass)
  }

  /// The thread a node belongs to, when it is a summoned one
  pub fn relay_thread_of(&self, id: &str) -> Option<String> {
    self.nodes.iter().find(|n| n.id == id).and_then(|n| n.harness.as_ref()).map(|h| h.thread.clone())
  }

  /// The oldest summoned node of `turn_index` not yet tied to the root call that summoned it, with the call's task and
  /// a harness / persona name the call fits (`fits`: two personas asked the same thing must not swap rows)
  pub fn relay_unlinked(&self, task: &str, turn_index: usize, fits: impl Fn(&SubagentHarness, Option<&str>) -> bool) -> Option<String> {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let want = norm(task);
    self
      .nodes
      .iter()
      .find(|n| {
        n.turn_index == turn_index as u64
          && n.peer.tool_call_id.is_none()
          && n.harness.as_ref().is_some_and(|h| fits(h, n.role.as_deref()))
          && n.task.as_deref().map(norm).as_deref() == Some(want.as_str())
      })
      .map(|n| n.id.clone())
  }

  /// Whether a root tool row already belongs to a node
  pub fn relay_linked(&self, tool_call_id: &str) -> bool {
    self.by_tool.contains_key(tool_call_id)
  }

  /// The model and reasoning effort the child really runs (what its config options say after the persona's choice was
  /// applied)
  pub fn relay_set_controls(&mut self, id: &str, model: Option<String>, effort: Option<String>) {
    let Some(i) = self.idx(id) else { return };
    let n = &mut self.nodes[i];
    let effort_changed = n.harness.as_ref().is_some_and(|h| h.effort != effort);
    if n.model == model && !effort_changed {
      return;
    }
    n.model = model;
    if let Some(h) = n.harness.as_mut() {
      h.effort = effort;
    }
    self.bump(id);
  }

  /// Mark a running summoned node cancelled-on-request, bound to its native session or not yet (a round still
  /// connecting checks `relay_cancel_requested` before it sends the prompt)
  pub fn relay_cancel(&mut self, id: &str) -> bool {
    let Some(i) = self.idx(id) else { return false };
    let n = &mut self.nodes[i];
    if n.harness.is_none() || n.status != SubagentState::Running {
      return false;
    }
    n.cancel_requested = true;
    self.bump(id);
    true
  }

  /// Cancelled on request, or gone: an edit truncating the turn removes the node right after flagging it, so a round
  /// still connecting must read a missing node as cancelled too
  pub fn relay_cancel_requested(&self, id: &str) -> bool {
    self.nodes.iter().find(|n| n.id == id).is_none_or(|n| n.cancel_requested)
  }

  /// How a summoned round ended: its state, the last text the child wrote, and the error shown in its place
  pub fn relay_outcome(&self, id: &str) -> Option<(SubagentState, String, Option<String>)> {
    let n = self.nodes.iter().find(|n| n.id == id)?;
    Some((n.status, self.relay_reply(id).unwrap_or_default(), n.result.clone()))
  }

  /// A summoned node's own transcript: the turns of its round
  pub fn relay_turns(&self, id: &str) -> Option<&[Turn]> {
    self.nodes.iter().find(|n| n.id == id && n.harness.is_some()).map(|n| n.state.turns.as_slice())
  }

  /// Every round of a thread so far, oldest first: the turns of each
  pub fn relay_thread_turns(&self, thread: &str) -> Vec<&[Turn]> {
    self.nodes.iter().filter(|n| n.harness.as_ref().is_some_and(|h| h.thread == thread)).map(|n| n.state.turns.as_slice()).collect()
  }

  /// The last text the child wrote in its current round
  pub fn relay_reply(&self, id: &str) -> Option<String> {
    let n = self.nodes.iter().find(|n| n.id == id)?;
    let t = n.state.turns.last().and_then(Turn::as_agent)?;
    t.blocks.iter().rev().find_map(|b| match b {
      AgentBlock::Text(tx) if !tx.markdown.trim().is_empty() => Some(tx.markdown.clone()),
      _ => None,
    })
  }

  /// Running summoned nodes (the session ends their processes when it settles or closes)
  pub fn relay_running(&self) -> Vec<String> {
    self.nodes.iter().filter(|n| n.harness.is_some() && n.status == SubagentState::Running).map(|n| n.id.clone()).collect()
  }

  /// Running summoned nodes announced at or after turn_index: the ones an edit's `truncate` would take away
  pub fn relay_running_from(&self, turn_index: usize) -> Vec<String> {
    self
      .nodes
      .iter()
      .filter(|n| n.harness.is_some() && n.status == SubagentState::Running && n.turn_index >= turn_index as u64)
      .map(|n| n.id.clone())
      .collect()
  }
}
