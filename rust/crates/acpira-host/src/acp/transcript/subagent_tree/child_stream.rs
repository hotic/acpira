//! Lifecycle updates and the child streams: a node's state transitions, its own updates, and the updates that arrive
//! before their node (held as orphans, replayed once it is announced)

use serde_json::Value;

use acpira_shared::subagents::{StateSource, SubagentPeer, SubagentState, SubagentVisibility};

use super::{
  ORPHAN_MAX_IDS, ORPHAN_MAX_UPDATES, RouteCtx, SubagentTree, resumed_task, s, seal_launch_receipt, seal_node, state_name, terminal,
  tool_count,
};
use crate::acp::session::failure::failure_of;
use crate::acp::transcript::normalize::{apply_session_failure, apply_update};
use crate::acp::transport::wire::SubagentLifecycle;

impl SubagentTree {
  /// Lifecycle updates (RFD subagent_update / claude subagent_spawned + subagent_state_update)
  pub fn lifecycle(&mut self, parent_peer: &str, root_peer: Option<&str>, l: SubagentLifecycle, ctx: &mut RouteCtx) {
    if Some(l.peer_session_id.as_str()) == root_peer || l.peer_session_id == parent_peer {
      self.log(&format!("subagent lifecycle for own session {} ignored", l.peer_session_id));
      return;
    }
    let mut parent_id = None;
    if Some(parent_peer) != root_peer {
      match self.by(&self.by_session, parent_peer) {
        Some(p) => parent_id = Some(self.nodes[p].id.clone()),
        None => {
          self.log(&format!("subagent {} announced under unknown parent session {parent_peer}", l.peer_session_id));
          return;
        }
      }
    }
    let launch = self.pending_launches.remove(&l.peer_session_id);
    let mut n = self.by(&self.by_session, &l.peer_session_id);
    let is_new = n.is_none();
    if n.is_none()
      && let Some(launch) = &launch
      && let Some(prior) = self.by(&self.by_tool, &launch.tool_call_id)
      && self.nodes[prior].visibility != SubagentVisibility::Session
      && self.nodes[prior].peer.session_id.is_none()
    {
      let id = self.nodes[prior].id.clone();
      self.adopt_call_node(&id, &l.peer_session_id);
      n = self.idx(&id);
    }
    let i = match n {
      Some(i) => i,
      None => {
        let mut node = self.new_node(
          SubagentVisibility::Session,
          SubagentPeer { session_id: Some(l.peer_session_id.clone()), ..Default::default() },
          ctx.turn_index,
          l.cancel == Some(true),
          None,
        );
        let task = if self.resumed_logs { resumed_task(&l.peer_session_id).map(str::to_owned) } else { None };
        if let Some(task) = &task {
          node.resumed = true;
          node.peer.sidechain_id = Some(task.clone());
        }
        self.by_session.insert(l.peer_session_id.clone(), node.id.clone());
        self.nodes.push(node);
        let i = self.nodes.len() - 1;
        if let Some(task) = task
          && let Some(pos) = self.send_messages.iter().rposition(|(_, to)| *to == task)
        {
          let (call, _) = self.send_messages.remove(pos);
          self.link_resume_call(i, &call, ctx);
        }
        i
      }
    };
    if let Some(launch) = launch {
      let node = &mut self.nodes[i];
      if node.peer.tool_call_id.is_none() {
        node.peer.tool_call_id = Some(launch.tool_call_id.clone());
        self.by_tool.insert(launch.tool_call_id.clone(), node.id.clone());
      }
      if node.model.is_none() {
        node.model = launch.model;
      }
      if node.title.is_none() {
        node.title = launch.title;
      }
      let id = node.id.clone();
      if let Some(block) = ctx.root_tool(&launch.tool_call_id) {
        block.subagent_id = Some(id.clone());
        seal_launch_receipt(block);
        ctx.drop_anchor(&id);
      }
    }
    let node = &mut self.nodes[i];
    if parent_id.is_some() && node.parent_id.is_none() {
      node.parent_id = parent_id;
    }
    if l.title.is_some() {
      node.title = l.title;
    }
    if l.task.is_some() {
      node.task = l.task;
    }
    if let Some(c) = l.cancel {
      node.cancel = c;
    }
    node.meta = Some(l.meta);
    let id = node.id.clone();
    // A root child announced with no launch row to sit on keeps its place in the stream
    if is_new && node.parent_id.is_none() {
      let title = node.title.clone();
      ctx.place_anchor(&id, title);
    }
    // Content buffered before the announce is still the child's, even when this update carries a terminal state
    self.replay_orphans(&l.peer_session_id.clone(), &id, ctx);
    if let Some(st) = l.state {
      self.transition(&id, st);
    }
    self.bump(&id);
  }

  pub(super) fn transition(&mut self, id: &str, state: SubagentState) {
    let Some(i) = self.idx(id) else { return };
    let n = &self.nodes[i];
    if terminal(n.status) {
      if !terminal(state) {
        self.log(&format!("subagent {} is {}; wire state {} ignored", Self::label(n), state_name(n.status), state_name(state)));
        return;
      }
      if n.state_source == StateSource::Agent {
        if n.status != state {
          self.log(&format!("subagent {} is {}; wire state {} ignored", Self::label(n), state_name(n.status), state_name(state)));
        }
        return;
      }
    }
    let n = &mut self.nodes[i];
    n.status = state;
    n.state_source = StateSource::Agent;
    if terminal(state) {
      seal_node(n, state);
      self.terminal_events.push(id.to_owned());
    }
    self.nodes[i].rev += 1;
  }

  /// Child-stream updates
  pub fn apply_child(&mut self, id: &str, u: &Value, ctx: &mut RouteCtx) {
    let Some(mut i) = self.idx(id) else { return };
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    if terminal(self.nodes[i].status) {
      if kind == "usage_update" {
        let n = &mut self.nodes[i];
        n.usage = Some((u.get("used").and_then(Value::as_f64).unwrap_or(0.0), u.get("size").and_then(Value::as_f64).unwrap_or(0.0)));
        n.rev += 1;
      } else if !self.nodes[i].late_logged {
        self.nodes[i].late_logged = true;
        let n = &self.nodes[i];
        self.log(&format!("subagent {} is {}; {kind} dropped", Self::label(n), state_name(n.status)));
      }
      return;
    }
    match kind {
      "user_message_chunk" | "available_commands_update" | "current_mode_update" | "config_option_update" => return,
      "session_info_update" => {
        let log = self.log.clone();
        if let Some(f) = failure_of(u.get("_meta"), Some(&*log))
          && apply_session_failure(&mut self.nodes[i].state, &f)
        {
          self.nodes[i].rev += 1;
        }
        return;
      }
      "usage_update" => {
        let n = &mut self.nodes[i];
        n.usage = Some((u.get("used").and_then(Value::as_f64).unwrap_or(0.0), u.get("size").and_then(Value::as_f64).unwrap_or(0.0)));
        n.rev += 1;
        return;
      }
      _ => {}
    }
    // A resumed run read from its log takes no wire copy of the same chunks; wire text before any log line means this
    // adapter delivers the run after all, and the log is left alone
    if self.nodes[i].resumed && matches!(kind, "agent_message_chunk" | "agent_thought_chunk") {
      if self.nodes[i].log_fed {
        return;
      }
      self.nodes[i].resumed = false;
      let label = Self::label(&self.nodes[i]);
      self.log(&format!("resumed subagent {label} streams on the wire; its sidechain log is not read"));
    }
    // Claude native mirror link: the child's own updates may name the parent's Task call
    let n = &self.nodes[i];
    if n.visibility == SubagentVisibility::Session
      && n.peer.tool_call_id.is_none()
      && let Some(session) = n.peer.session_id.clone()
      // A resumed run still names the first run's Agent call, which stays the first run's
      && resumed_task(&session).is_none()
      && let Some(link) = s(u.get("_meta").and_then(|m| m.get("claudeCode")).and_then(|c| c.get("parentToolUseId")))
    {
      if let Some(prior) = self.by(&self.by_tool, &link)
        && prior != i
        && self.nodes[prior].visibility != SubagentVisibility::Session
        && self.nodes[prior].peer.session_id.is_none()
      {
        let pid = self.nodes[prior].id.clone();
        self.adopt_call_node(&pid, &session);
        let Some(adopted) = self.idx(&pid) else { return };
        i = adopted;
      }
      let nid = self.nodes[i].id.clone();
      self.nodes[i].peer.tool_call_id = Some(link.clone());
      self.by_tool.insert(link.clone(), nid.clone());
      if let Some(block) = ctx.root_tool(&link) {
        block.subagent_id = Some(nid.clone());
        seal_launch_receipt(block);
        ctx.drop_anchor(&nid);
      }
      self.replay_orphans(&format!("tool:{link}"), &nid, ctx);
      i = match self.idx(&nid) {
        Some(x) => x,
        None => return,
      };
    }
    if self.nodes[i].visibility != SubagentVisibility::Session
      && matches!(kind, "tool_call" | "tool_call_update")
      && let Some(tid) = u.get("toolCallId").and_then(Value::as_str)
    {
      self.tool_owner.insert(tid.to_owned(), self.nodes[i].id.clone());
    }
    let n = &mut self.nodes[i];
    apply_update(&mut n.state, u);
    n.tool_count = tool_count(&n.state.turns);
    n.rev += 1;
  }

  pub fn buffer_orphan(&mut self, key: &str, u: Value) {
    if !self.orphans.contains_key(key) {
      if self.orphans.len() >= ORPHAN_MAX_IDS {
        self.log_orphan_drop();
        return;
      }
      self.orphans.insert(key.to_owned(), vec![]);
      self.orphan_order.push(key.to_owned());
    }
    let Some(list) = self.orphans.get_mut(key) else { return };
    if list.len() >= ORPHAN_MAX_UPDATES {
      self.log_orphan_drop();
      return;
    }
    list.push(u);
  }

  fn log_orphan_drop(&mut self) {
    if self.orphan_logged {
      return;
    }
    self.orphan_logged = true;
    self.log("subagent updates dropped: too many buffered for unannounced sessions");
  }

  pub(super) fn replay_orphans(&mut self, key: &str, id: &str, ctx: &mut RouteCtx) {
    let Some(buffered) = self.orphans.remove(key) else { return };
    self.orphan_order.retain(|k| k != key);
    for u in buffered {
      self.apply_child(id, &u, ctx);
    }
  }

  /// One delegation announced twice collapses into the node the root block already points at
  pub(super) fn adopt_call_node(&mut self, id: &str, peer_session: &str) {
    let stale_id = self.by_session.get(peer_session).cloned();
    let Some(i) = self.idx(id) else { return };
    self.nodes[i].visibility = SubagentVisibility::Session;
    self.nodes[i].peer.session_id = Some(peer_session.to_owned());
    self.by_session.insert(peer_session.to_owned(), id.to_owned());
    if let Some(stale_id) = stale_id.filter(|s| s != id)
      && let Some(si) = self.idx(&stale_id)
    {
      let stale = self.nodes.remove(si);
      let Some(i) = self.idx(id) else { return };
      let n = &mut self.nodes[i];
      if stale.state.turns.len() > 1 || stale.state.turns.iter().any(|t| t.as_agent().is_some_and(|a| !a.blocks.is_empty())) {
        n.state = stale.state;
        n.tool_count = tool_count(&n.state.turns);
      }
      n.status = stale.status;
      n.state_source = stale.state_source;
      n.cancel = stale.cancel;
      n.title = n.title.take().or(stale.title);
      n.task = n.task.take().or(stale.task);
      n.role = n.role.take().or(stale.role);
      n.model = n.model.take().or(stale.model);
      n.usage = n.usage.or(stale.usage);
      n.result = n.result.take().or(stale.result);
      n.meta = n.meta.take().or(stale.meta);
      if stale.ended_at.is_some() {
        n.ended_at = stale.ended_at;
      }
      if stale.parent_id.is_some() && n.parent_id.is_none() {
        n.parent_id = stale.parent_id;
      }
      n.cancel_requested |= stale.cancel_requested;
      n.background |= stale.background;
      n.late_logged |= stale.late_logged;
    }
    self.bump(id);
  }
}
