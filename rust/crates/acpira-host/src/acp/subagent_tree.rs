//! First-class subagent nodes for one session (mirror of src/host/acp/subagents/SubagentTree.ts): per-dialect
//! normalization (RFD / claude native sessions, Devin's nested cognition.ai updates, Claude legacy Agent, Kimi's
//! receipt), lifecycle, early-update buffering and record round trips. All mutations are synchronous. Nodes are found
//! through peer-id → node-id maps; the root transcript is reached through `RouteCtx`

use std::collections::HashMap;

use serde_json::{Map, Value};

use acpira_shared::num::Num;
use acpira_shared::subagents::{
  StateSource, SubagentControls, SubagentCore, SubagentPeer, SubagentRecord, SubagentState, SubagentSummary, SubagentUsage,
  SubagentVisibility,
};
use acpira_shared::transcript::{
  AgentBlock, AgentTurn, PermissionBlock, QuestionBlock, ToolCallBlock, ToolContent, ToolStatus, Turn, TurnStop,
};

use super::normalize::{
  Log, NormalizeState, ToolCtx, activity_of, apply_session_failure, apply_update, async_task_live, end_turn, find_tool_mut,
};
use super::restore_turns::restore_interrupted_turns;
use super::session_failure::failure_of;
use super::wire::SubagentLifecycle;
use crate::i18n::t;
use crate::util::{ms_of_iso, now_iso, now_ms, random_uuid};

const ORPHAN_MAX_IDS: usize = 8;
const ORPHAN_MAX_UPDATES: usize = 64;

/// What routing may touch on the root side: the anchor turn index and the root transcript itself
pub struct RouteCtx<'a> {
  pub turn_index: usize,
  pub root_turns: &'a mut Vec<Turn>,
}

impl RouteCtx<'_> {
  fn root_tool(&mut self, id: &str) -> Option<&mut ToolCallBlock> {
    find_tool_mut(self.root_turns, id)
  }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
  Claude,
  Devin,
}

struct PendingDelegation {
  tool_call_id: String,
  title: Option<String>,
  task: Option<String>,
}

struct PendingLaunch {
  tool_call_id: String,
  model: Option<String>,
  title: Option<String>,
}

struct Node {
  id: String,
  parent_id: Option<String>,
  turn_index: u64,
  visibility: SubagentVisibility,
  title: Option<String>,
  task: Option<String>,
  role: Option<String>,
  status: SubagentState,
  state_source: StateSource,
  cancel: bool,
  cancel_requested: bool,
  background: bool,
  announced_at: i64,
  ended_at: Option<i64>,
  model: Option<String>,
  usage: Option<(f64, f64)>,
  peer: SubagentPeer,
  tool_count: u64,
  result: Option<String>,
  meta: Option<Value>,
  dialect: Option<Dialect>,
  state: NormalizeState,
  rev: i64,
  cached: Option<(i64, SubagentSummary)>,
  restored: bool,
  late_logged: bool,
}

fn terminal(s: SubagentState) -> bool {
  s.is_terminal()
}

fn rec(v: Option<&Value>) -> Option<&Map<String, Value>> {
  v.and_then(Value::as_object)
}

fn s(v: Option<&Value>) -> Option<String> {
  v.and_then(Value::as_str).filter(|x| !x.is_empty()).map(str::to_owned)
}

fn tool_count(turns: &[Turn]) -> u64 {
  turns.iter().filter_map(Turn::as_agent).map(|t| t.blocks.iter().filter(|b| matches!(b, AgentBlock::ToolCall(_))).count() as u64).sum()
}

fn state_name(st: SubagentState) -> &'static str {
  match st {
    SubagentState::Running => "running",
    SubagentState::Completed => "completed",
    SubagentState::Failed => "failed",
    SubagentState::Cancelled => "cancelled",
    SubagentState::Disconnected => "disconnected",
  }
}

pub struct SubagentTree {
  nodes: Vec<Node>,
  by_session: HashMap<String, String>,
  by_agent: HashMap<String, String>,
  by_tool: HashMap<String, String>,
  tool_owner: HashMap<String, String>,
  pending_delegations: Vec<PendingDelegation>,
  pending_launches: HashMap<String, PendingLaunch>,
  orphans: HashMap<String, Vec<Value>>,
  orphan_order: Vec<String>,
  orphan_logged: bool,
  log: Log,
  child_ctx: ToolCtx,
  /// Node ids that went terminal since the last drain: the session closes their pending cards
  pub terminal_events: Vec<String>,
}

#[derive(serde::Serialize)]
pub struct RecordRef<'a> {
  #[serde(flatten)]
  pub core: SubagentCore,
  pub turns: &'a [Turn],
  pub rev: i64,
}

pub enum Route {
  Consumed,
  Root,
}

impl SubagentTree {
  pub fn new(log: Log, child_ctx: ToolCtx, records: Option<Vec<SubagentRecord>>, updated_at: Option<&str>) -> Self {
    let at = updated_at.map(str::to_owned).unwrap_or_else(now_iso);
    let mut tree = SubagentTree {
      nodes: vec![],
      by_session: HashMap::new(),
      by_agent: HashMap::new(),
      by_tool: HashMap::new(),
      tool_owner: HashMap::new(),
      pending_delegations: vec![],
      pending_launches: HashMap::new(),
      orphans: HashMap::new(),
      orphan_order: vec![],
      orphan_logged: false,
      log,
      child_ctx,
      terminal_events: vec![],
    };
    for r in records.unwrap_or_default() {
      let c = r.core;
      let was_running = c.state == SubagentState::Running;
      let mut state = tree.child_state();
      state.turns = restore_interrupted_turns(r.turns, &at);
      tree.nodes.push(Node {
        id: c.id,
        parent_id: c.parent_id,
        turn_index: c.turn_index,
        visibility: c.visibility,
        title: c.title,
        task: c.task,
        role: c.role,
        status: if was_running { SubagentState::Disconnected } else { c.state },
        state_source: if was_running { StateSource::Local } else { c.state_source },
        cancel: c.controls.cancel,
        cancel_requested: c.cancel_requested == Some(true),
        background: c.background == Some(true),
        announced_at: c.announced_at,
        ended_at: if was_running { Some(ms_of_iso(&at).filter(|x| *x != 0).unwrap_or_else(now_ms)) } else { c.ended_at },
        model: c.model,
        usage: c.usage.map(|u| (u.used.0, u.size.0)),
        peer: c.peer,
        tool_count: c.tool_count,
        result: c.result,
        meta: None,
        dialect: None,
        state,
        rev: r.rev.unwrap_or(1),
        cached: None,
        restored: true,
        late_logged: false,
      });
    }
    tree.reindex();
    tree
  }

  fn log(&self, line: &str) {
    (self.log)(line);
  }

  fn child_state(&self) -> NormalizeState {
    NormalizeState { ctx: self.child_ctx.clone(), log: Some(self.log.clone()), ..Default::default() }
  }

  pub fn len(&self) -> usize {
    self.nodes.len()
  }

  pub fn is_empty(&self) -> bool {
    self.nodes.is_empty()
  }

  fn idx(&self, id: &str) -> Option<usize> {
    self.nodes.iter().position(|n| n.id == id)
  }

  fn by(&self, map: &HashMap<String, String>, key: &str) -> Option<usize> {
    map.get(key).and_then(|id| self.idx(id))
  }

  fn label(n: &Node) -> String {
    n.peer
      .session_id
      .clone()
      .or_else(|| n.peer.agent_id.clone())
      .or_else(|| n.peer.tool_call_id.clone())
      .unwrap_or_else(|| n.id.chars().take(8).collect())
  }

  pub fn reindex(&mut self) {
    self.by_session.clear();
    self.by_agent.clear();
    self.by_tool.clear();
    self.tool_owner.clear();
    for n in &self.nodes {
      if let Some(x) = &n.peer.session_id {
        self.by_session.insert(x.clone(), n.id.clone());
      }
      if let Some(x) = &n.peer.agent_id {
        self.by_agent.insert(x.clone(), n.id.clone());
      }
      if let Some(x) = &n.peer.tool_call_id {
        self.by_tool.insert(x.clone(), n.id.clone());
      }
    }
  }

  pub fn has_peer_session(&self, peer: &str) -> bool {
    self.by(&self.by_session, peer).is_some()
  }

  /// The node id a peer session id belongs to, terminal or not
  pub fn node_for_peer(&self, peer: &str) -> Option<String> {
    self.by(&self.by_session, peer).map(|i| self.nodes[i].id.clone())
  }

  pub fn peer_restored(&self, peer: &str) -> bool {
    self.by(&self.by_session, peer).is_some_and(|i| self.nodes[i].restored)
  }

  /// A request under this peer session id → the node's transcript; a terminal node takes no new requests
  pub fn state_for_peer(&mut self, peer: &str) -> Option<(&mut NormalizeState, String)> {
    let i = self.by(&self.by_session, peer)?;
    if terminal(self.nodes[i].status) {
      return None;
    }
    let n = &mut self.nodes[i];
    Some((&mut n.state, n.id.clone()))
  }

  /// AIR tasks outlive the child's terminal word, so this lookup ignores the terminal gate
  pub fn task_state(&mut self, peer: &str) -> Option<(&mut NormalizeState, String)> {
    let i = self.by(&self.by_session, peer)?;
    let n = &mut self.nodes[i];
    Some((&mut n.state, n.id.clone()))
  }

  pub fn state_of(&mut self, node_id: &str) -> Option<&mut NormalizeState> {
    let i = self.idx(node_id)?;
    Some(&mut self.nodes[i].state)
  }

  pub fn bump(&mut self, node_id: &str) {
    if let Some(i) = self.idx(node_id) {
      self.nodes[i].rev += 1;
    }
  }

  /// (node id, state) of every node, for sweeps over all transcripts
  pub fn states_mut(&mut self) -> impl Iterator<Item = (&str, &mut NormalizeState)> {
    self.nodes.iter_mut().map(|n| (n.id.as_str(), &mut n.state))
  }

  /// Every node id below this one, multi-level, cycle-safe
  pub fn descendants(&self, id: &str) -> Vec<String> {
    let mut seen = vec![id.to_owned()];
    let mut queue = std::collections::VecDeque::from([id.to_owned()]);
    let mut out = vec![];
    while let Some(cur) = queue.pop_front() {
      for n in &self.nodes {
        if n.parent_id.as_deref() == Some(cur.as_str()) && !seen.contains(&n.id) {
          seen.push(n.id.clone());
          queue.push_back(n.id.clone());
          out.push(n.id.clone());
        }
      }
    }
    out
  }

  fn new_node(
    &self,
    visibility: SubagentVisibility,
    peer: SubagentPeer,
    turn_index: usize,
    cancel: bool,
    dialect: Option<Dialect>,
  ) -> Node {
    let announced_at = now_ms();
    let mut state = self.child_state();
    // The eager turn gives tool rows real startedAt/endedAt and end_turn something to seal
    state.turns.push(Turn::Agent(AgentTurn { started_at: Some(announced_at), ..Default::default() }));
    Node {
      id: random_uuid(),
      parent_id: None,
      turn_index: turn_index as u64,
      visibility,
      title: None,
      task: None,
      role: None,
      status: SubagentState::Running,
      state_source: StateSource::Agent,
      cancel,
      cancel_requested: false,
      background: false,
      announced_at,
      ended_at: None,
      model: None,
      usage: None,
      peer,
      tool_count: 0,
      result: None,
      meta: None,
      dialect,
      state,
      rev: 0,
      cached: None,
      restored: false,
      late_logged: false,
    }
  }

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
        let node = self.new_node(
          SubagentVisibility::Session,
          SubagentPeer { session_id: Some(l.peer_session_id.clone()), ..Default::default() },
          ctx.turn_index,
          l.cancel == Some(true),
          None,
        );
        self.by_session.insert(l.peer_session_id.clone(), node.id.clone());
        self.nodes.push(node);
        self.nodes.len() - 1
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
        block.subagent_id = Some(id);
        seal_launch_receipt(block);
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
    // Content buffered before the announce is still the child's, even when this update carries a terminal state
    self.replay_orphans(&l.peer_session_id.clone(), &id, ctx);
    if let Some(st) = l.state {
      self.transition(&id, st);
    }
    self.bump(&id);
  }

  fn transition(&mut self, id: &str, state: SubagentState) {
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
    // Claude native mirror link: the child's own updates may name the parent's Task call
    let n = &self.nodes[i];
    if n.visibility == SubagentVisibility::Session
      && n.peer.tool_call_id.is_none()
      && let Some(session) = n.peer.session_id.clone()
      && let Some(link) = s(u.get("_meta").and_then(|m| m.get("claudeCode")).and_then(|c| c.get("parentToolUseId")))
    {
      if let Some(prior) = self.by(&self.by_tool, &link)
        && prior != i
        && self.nodes[prior].visibility != SubagentVisibility::Session
        && self.nodes[prior].peer.session_id.is_none()
      {
        let pid = self.nodes[prior].id.clone();
        self.adopt_call_node(&pid, &session);
        i = self.idx(&pid).expect("adopted node");
      }
      let nid = self.nodes[i].id.clone();
      self.nodes[i].peer.tool_call_id = Some(link.clone());
      self.by_tool.insert(link.clone(), nid.clone());
      if let Some(block) = ctx.root_tool(&link) {
        block.subagent_id = Some(nid.clone());
        seal_launch_receipt(block);
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
    let list = self.orphans.get_mut(key).expect("inserted");
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

  fn replay_orphans(&mut self, key: &str, id: &str, ctx: &mut RouteCtx) {
    let Some(buffered) = self.orphans.remove(key) else { return };
    self.orphan_order.retain(|k| k != key);
    for u in buffered {
      self.apply_child(id, &u, ctx);
    }
  }

  /// One delegation announced twice collapses into the node the root block already points at
  fn adopt_call_node(&mut self, id: &str, peer_session: &str) {
    let stale_id = self.by_session.get(peer_session).cloned();
    let Some(i) = self.idx(id) else { return };
    self.nodes[i].visibility = SubagentVisibility::Session;
    self.nodes[i].peer.session_id = Some(peer_session.to_owned());
    self.by_session.insert(peer_session.to_owned(), id.to_owned());
    if let Some(stale_id) = stale_id.filter(|s| s != id)
      && let Some(si) = self.idx(&stale_id)
    {
      let stale = self.nodes.remove(si);
      let i = self.idx(id).expect("still present");
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

  /// Root-stream routing: the nested / receipt dialects
  pub fn route_root(&mut self, u: &Value, ctx: &mut RouteCtx) -> Route {
    let meta = rec(u.get("_meta"));
    if let Some(started) = meta.and_then(|m| rec(m.get("cognition.ai/subagent_started"))) {
      let started = started.clone();
      self.devin_started(&started, meta.cloned(), ctx);
      return Route::Consumed;
    }
    if let Some(done) = meta.and_then(|m| rec(m.get("cognition.ai/subagent_completed"))) {
      let done = done.clone();
      self.devin_completed(&done);
      return Route::Consumed;
    }
    let parent_agent = s(meta.and_then(|m| m.get("cognition.ai/subagent_context")).and_then(|c| c.get("parentAgentId")));
    if let Some(pa) = parent_agent.filter(|p| p != "root") {
      match self.by(&self.by_agent, &pa) {
        None => self.buffer_orphan(&format!("agent:{pa}"), u.clone()),
        Some(i) => {
          let id = self.nodes[i].id.clone();
          self.apply_child(&id, u, ctx);
        }
      }
      return Route::Consumed;
    }
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "tool_call" | "tool_call_update") {
      return Route::Root;
    }
    let tool_call_id = u.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_owned();
    if let Some(owner) = self.tool_owner.get(&tool_call_id).cloned() {
      if self.idx(&owner).is_some() {
        self.apply_child(&owner, u, ctx);
        return Route::Consumed;
      }
      self.tool_owner.remove(&tool_call_id);
    }
    if let Some(other) = meta.and_then(|m| m.keys().find(|k| k.starts_with("cognition.ai/subagent_")))
      && self.by_agent.contains_key(&tool_call_id)
    {
      self.log(&format!("unhandled {other} on subagent {tool_call_id}"));
      return Route::Consumed;
    }
    let claude = meta.and_then(|m| rec(m.get("claudeCode")));
    let tool_response = claude.and_then(|c| rec(c.get("toolResponse")));
    if let Some(tr) = tool_response.filter(|tr| tr.get("isAsync") == Some(&Value::Bool(true)))
      && let Some(agent_id) = s(tr.get("agentId"))
    {
      let resolved_model = s(tr.get("resolvedModel"));
      let description = s(tr.get("description"));
      let mut n = self.by(&self.by_session, &agent_id);
      if let Some(prior) = self.by(&self.by_tool, &tool_call_id)
        && Some(prior) != n
        && self.nodes[prior].visibility != SubagentVisibility::Session
        && self.nodes[prior].peer.session_id.is_none()
      {
        let pid = self.nodes[prior].id.clone();
        self.adopt_call_node(&pid, &agent_id);
        n = self.idx(&pid);
      }
      match n {
        Some(i) => {
          let node = &mut self.nodes[i];
          node.peer.tool_call_id = Some(tool_call_id.clone());
          if node.model.is_none() {
            node.model = resolved_model;
          }
          if node.title.is_none() {
            node.title = description;
          }
          node.rev += 1;
          let id = node.id.clone();
          self.by_tool.insert(tool_call_id.clone(), id.clone());
          self.replay_orphans(&format!("tool:{tool_call_id}"), &id, ctx);
        }
        None => {
          self.pending_launches.insert(agent_id, PendingLaunch { tool_call_id, model: resolved_model, title: description });
        }
      }
      return Route::Root;
    }
    if let Some(parent_tool) = s(claude.and_then(|c| c.get("parentToolUseId"))) {
      match self.by(&self.by_tool, &parent_tool) {
        Some(i) => {
          let id = self.nodes[i].id.clone();
          self.apply_child(&id, u, ctx);
        }
        None => self.buffer_orphan(&format!("tool:{parent_tool}"), u.clone()),
      }
      return Route::Consumed;
    }
    if s(meta.and_then(|m| m.get("cognition.ai/inferenceToolName"))).as_deref() == Some("run_subagent")
      && let Some(raw) = rec(u.get("rawInput"))
    {
      self.pending_delegations.push(PendingDelegation { tool_call_id, title: s(raw.get("title")), task: s(raw.get("task")) });
    }
    Route::Root
  }

  fn devin_started(&mut self, started: &Map<String, Value>, meta: Option<Map<String, Value>>, ctx: &mut RouteCtx) {
    let Some(agent_id) = s(started.get("agentId")) else {
      self.log("subagent_started without agentId dropped");
      return;
    };
    let existing = self.by(&self.by_agent, &agent_id);
    let is_new = existing.is_none();
    let i = match existing {
      Some(i) => i,
      None => {
        let node = self.new_node(
          SubagentVisibility::Nested,
          SubagentPeer { agent_id: Some(agent_id.clone()), ..Default::default() },
          ctx.turn_index,
          false,
          Some(Dialect::Devin),
        );
        self.by_agent.insert(agent_id.clone(), node.id.clone());
        self.nodes.push(node);
        self.nodes.len() - 1
      }
    };
    if self.nodes[i].parent_id.is_none()
      && let Some(pa) =
        s(meta.as_ref().and_then(|m| m.get("cognition.ai/subagent_context")).and_then(|c| c.get("parentAgentId"))).filter(|p| p != "root")
    {
      match self.by(&self.by_agent, &pa) {
        Some(p) => self.nodes[i].parent_id = Some(self.nodes[p].id.clone()),
        None => self.log(&format!("subagent_started {agent_id} names unknown parent {pa}; kept top-level")),
      }
    }
    let (title, task, profile, model) =
      (s(started.get("title")), s(started.get("task")), s(started.get("profile")), s(started.get("model")));
    let n = &mut self.nodes[i];
    if title.is_some() {
      n.title = title.clone();
    }
    if task.is_some() {
      n.task = task.clone();
    }
    if profile.is_some() {
      n.role = profile;
    }
    if model.is_some() {
      n.model = model;
    }
    if started.get("isBackground") == Some(&Value::Bool(true)) {
      n.background = true;
    }
    n.meta = Some(Value::Object(started.clone()));
    let id = n.id.clone();
    let di = self.pending_delegations.iter().position(|d| d.title == title && (d.task.is_none() || task.is_none() || d.task == task));
    if let Some(di) = di {
      let d = self.pending_delegations.remove(di);
      self.nodes[i].peer.tool_call_id = Some(d.tool_call_id.clone());
      self.by_tool.insert(d.tool_call_id.clone(), id.clone());
      if let Some(block) = ctx.root_tool(&d.tool_call_id) {
        block.subagent_id = Some(id.clone());
      }
    } else if self.nodes[i].peer.tool_call_id.is_none() {
      self.log(&format!("subagent {agent_id} has no matching run_subagent call"));
    }
    self.bump(&id);
    if is_new {
      self.replay_orphans(&format!("agent:{agent_id}"), &id, ctx);
    }
  }

  fn devin_completed(&mut self, done: &Map<String, Value>) {
    let agent_id = s(done.get("agentId"));
    let Some(i) = agent_id.as_deref().and_then(|a| self.by(&self.by_agent, a)) else {
      self.log(&format!("subagent_completed for unknown agent {}", agent_id.as_deref().unwrap_or("?")));
      return;
    };
    if let Some(summary) = s(done.get("summary")) {
      self.nodes[i].result = Some(summary);
    }
    let id = self.nodes[i].id.clone();
    self.transition(&id, if done.get("success") == Some(&Value::Bool(false)) { SubagentState::Failed } else { SubagentState::Completed });
    self.bump(&id);
  }

  /// After a root tool_call / tool_call_update is applied: stamp delegation links, keep nested / receipt nodes in sync
  pub fn annotate_root(&mut self, u: &Value, ctx: &mut RouteCtx) {
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "tool_call" | "tool_call_update") {
      return;
    }
    let tool_call_id = u.get("toolCallId").and_then(Value::as_str).unwrap_or("").to_owned();
    if ctx.root_tool(&tool_call_id).is_none() {
      return;
    }
    let meta = rec(u.get("_meta"));
    let raw = rec(u.get("rawInput"));
    if s(meta.and_then(|m| m.get("cognition.ai/inferenceToolName"))).as_deref() == Some("read_subagent") {
      let title =
        s(raw.and_then(|r| r.get("agent_id"))).and_then(|a| self.by(&self.by_agent, &a)).and_then(|i| self.nodes[i].title.clone());
      let block = ctx.root_tool(&tool_call_id).expect("checked");
      block.verb_key = Some("verb.awaitSubagent".into());
      block.verb = t("verb.awaitSubagent");
      if let Some(title) = title {
        block.target = Some(title);
        block.target_mono = None;
      }
      return;
    }
    let claude = meta.and_then(|m| rec(m.get("claudeCode")));
    let mut n = self.by(&self.by_tool, &tool_call_id);
    if claude.and_then(|c| c.get("subagent")) == Some(&Value::Bool(true)) {
      let i = match n {
        Some(i) => i,
        None => self.upsert_call_node(&tool_call_id, SubagentVisibility::Nested, ctx),
      };
      self.nodes[i].dialect = Some(Dialect::Claude);
      n = Some(i);
    } else if n.is_none() && s(raw.and_then(|r| r.get("subagent_type"))).is_some() && s(raw.and_then(|r| r.get("prompt"))).is_some() {
      n = Some(self.upsert_call_node(&tool_call_id, SubagentVisibility::Receipt, ctx));
    }
    let Some(i) = n else { return };
    let id = self.nodes[i].id.clone();
    let block = ctx.root_tool(&tool_call_id).expect("checked");
    block.subagent_id = Some(id.clone());
    if self.nodes[i].visibility == SubagentVisibility::Session {
      seal_launch_receipt(block);
      self.bump(&id);
      return;
    }
    block.verb_key = Some("verb.delegate".into());
    block.verb = t("verb.delegate");
    let node = &mut self.nodes[i];
    if let Some(d) = s(raw.and_then(|r| r.get("description"))) {
      node.title = Some(d);
    }
    if let Some(p) = s(raw.and_then(|r| r.get("prompt"))) {
      node.task = Some(p);
    }
    if let Some(r) = s(raw.and_then(|r| r.get("subagent_type"))) {
      node.role = Some(r);
    }
    if let Some(m) = s(raw.and_then(|r| r.get("model"))) {
      node.model = Some(m);
    }
    if node.dialect == Some(Dialect::Devin) {
      self.bump(&id);
      return;
    }
    match u.get("status").and_then(Value::as_str) {
      Some("completed") => {
        let tr = claude.and_then(|c| rec(c.get("toolResponse")));
        if node.dialect == Some(Dialect::Claude) && tr.and_then(|t| t.get("isAsync")) == Some(&Value::Bool(true)) {
          node.background = true;
          if let Some(agent_id) = s(tr.and_then(|t| t.get("agentId"))) {
            node.peer.agent_id = Some(agent_id.clone());
            self.by_agent.insert(agent_id, id.clone());
          }
          self.transition(&id, SubagentState::Running);
        } else {
          let block = ctx.root_tool(&tool_call_id).expect("checked");
          let result = result_text(u, block);
          if result.is_some() {
            self.nodes[i].result = result;
          }
          self.transition(&id, SubagentState::Completed);
        }
      }
      Some("failed") => self.transition(&id, SubagentState::Failed),
      Some("cancelled") => self.transition(&id, SubagentState::Cancelled),
      _ => {}
    }
    self.bump(&id);
  }

  fn upsert_call_node(&mut self, tool_call_id: &str, visibility: SubagentVisibility, ctx: &mut RouteCtx) -> usize {
    if let Some(i) = self.by(&self.by_tool, tool_call_id) {
      return i;
    }
    let node = self.new_node(
      visibility,
      SubagentPeer { tool_call_id: Some(tool_call_id.to_owned()), ..Default::default() },
      ctx.turn_index,
      false,
      None,
    );
    let id = node.id.clone();
    self.by_tool.insert(tool_call_id.to_owned(), id.clone());
    self.nodes.push(node);
    self.replay_orphans(&format!("tool:{tool_call_id}"), &id, ctx);
    self.idx(&id).expect("just pushed")
  }

  /// End of the connection / turn
  pub fn settle(&mut self, reason: &str) {
    let prompt_returned = reason == "prompt-returned";
    for n in &mut self.nodes {
      if n.status != SubagentState::Running {
        continue;
      }
      if prompt_returned
        && n
          .state
          .turns
          .iter()
          .filter_map(Turn::as_agent)
          .any(|t| t.blocks.iter().any(|b| matches!(b, AgentBlock::ToolCall(tc) if async_task_live(tc))))
      {
        continue;
      }
      n.status = SubagentState::Disconnected;
      n.state_source = StateSource::Local;
      n.ended_at = Some(now_ms());
      end_turn(&mut n.state, TurnStop::Cancelled);
      self.terminal_events.push(n.id.clone());
      n.rev += 1;
    }
    let dropped: usize = self.orphans.values().map(Vec::len).sum();
    if dropped > 0 {
      self.log(&format!("{dropped} buffered subagent update(s) dropped on {reason}"));
    }
    self.orphans.clear();
    self.orphan_order.clear();
    self.orphan_logged = false;
    self.pending_delegations.clear();
    self.pending_launches.clear();
    self.tool_owner.clear();
  }

  /// Edit / retry rewrote history at turn_index: nodes announced in the removed turns go with them
  pub fn truncate(&mut self, turn_index: usize) {
    self.pending_delegations.clear();
    self.pending_launches.clear();
    let removed: Vec<String> = self.nodes.iter().filter(|n| n.turn_index >= turn_index as u64).map(|n| n.id.clone()).collect();
    if removed.is_empty() {
      return;
    }
    self.nodes.retain(|n| !removed.contains(&n.id));
    // reindex clears the connection-local tool owners as well, as the TS tree does
    self.reindex();
  }

  fn core_of(n: &Node) -> SubagentCore {
    SubagentCore {
      id: n.id.clone(),
      parent_id: n.parent_id.clone(),
      turn_index: n.turn_index,
      visibility: n.visibility,
      title: n.title.clone(),
      task: n.task.clone(),
      role: n.role.clone(),
      state: n.status,
      state_source: n.state_source,
      controls: SubagentControls { cancel: n.cancel },
      cancel_requested: n.cancel_requested.then_some(true),
      background: n.background.then_some(true),
      announced_at: n.announced_at,
      ended_at: n.ended_at,
      model: n.model.clone(),
      usage: n.usage.map(|(used, size)| SubagentUsage { used: Num(used), size: Num(size) }),
      peer: n.peer.clone(),
      activity: None,
      tool_count: n.tool_count,
      result: n.result.clone(),
    }
  }

  pub fn summaries(&mut self) -> Vec<SubagentSummary> {
    for n in &mut self.nodes {
      if n.cached.as_ref().is_some_and(|(rev, _)| *rev == n.rev) {
        continue;
      }
      let mut permissions: Vec<PermissionBlock> = vec![];
      let mut question: Option<QuestionBlock> = None;
      for t in n.state.turns.iter().filter_map(Turn::as_agent) {
        for b in &t.blocks {
          match b {
            AgentBlock::Permission(p) => permissions.push(p.clone()),
            AgentBlock::Question(q) if q.outcome.is_none() => question = Some(q.clone()),
            _ => {}
          }
        }
      }
      let mut core = Self::core_of(n);
      if n.status == SubagentState::Running {
        core.activity = activity_of(&n.state.turns).map(|a| a.label);
      }
      let summary = SubagentSummary { core, permissions: (!permissions.is_empty()).then_some(permissions), question };
      n.cached = Some((n.rev, summary));
    }
    self.nodes.iter().map(|n| n.cached.as_ref().expect("filled").1.clone()).collect()
  }

  pub fn transcript(&self, id: &str) -> Option<(&[Turn], i64, bool)> {
    let n = self.nodes.iter().find(|n| n.id == id)?;
    Some((&n.state.turns, n.rev, n.status == SubagentState::Running))
  }

  pub fn cancel(&mut self, id: &str) -> Option<String> {
    let i = self.idx(id)?;
    let n = &mut self.nodes[i];
    if !n.cancel || n.status != SubagentState::Running {
      return None;
    }
    let peer = n.peer.session_id.clone()?;
    n.cancel_requested = true;
    n.rev += 1;
    Some(peer)
  }

  pub fn to_records(&self) -> Vec<SubagentRecord> {
    self.nodes.iter().map(|n| SubagentRecord { core: Self::core_of(n), turns: n.state.turns.clone(), rev: Some(n.rev) }).collect()
  }

  /// The records as borrowed views: serializing them never clones a child transcript
  pub fn record_refs(&self) -> Vec<RecordRef<'_>> {
    self.nodes.iter().map(|n| RecordRef { core: Self::core_of(n), turns: &n.state.turns, rev: n.rev }).collect()
  }
}

fn seal_node(n: &mut Node, state: SubagentState) {
  if n.ended_at.is_none() {
    n.ended_at = Some(now_ms());
  }
  // A native child's result is what the child last said
  if n.visibility == SubagentVisibility::Session && n.result.is_none() {
    'outer: for t in n.state.turns.iter().rev().filter_map(Turn::as_agent) {
      for b in t.blocks.iter().rev() {
        if let AgentBlock::Text(tx) = b
          && !tx.markdown.trim().is_empty()
        {
          n.result = Some(tx.markdown.clone());
          break 'outer;
        }
      }
    }
  }
  if matches!(n.state.turns.last(), Some(Turn::Agent(a)) if a.stop.is_none()) {
    end_turn(
      &mut n.state,
      if matches!(state, SubagentState::Cancelled | SubagentState::Disconnected) { TurnStop::Cancelled } else { TurnStop::EndTurn },
    );
  }
}

/// The launch receipt is the delegation call returning; an unswept pending row would be marked failed at turn end
fn seal_launch_receipt(block: &mut ToolCallBlock) {
  if block.status.is_open() {
    block.status = ToolStatus::Completed;
    if block.started_at.is_some() && block.ended_at.is_none() {
      block.ended_at = Some(now_ms());
    }
  }
}

fn result_text(u: &Value, block: &ToolCallBlock) -> Option<String> {
  match u.get("rawOutput") {
    Some(Value::String(x)) if !x.trim().is_empty() => return Some(x.clone()),
    Some(Value::Array(items)) => {
      let parts: Vec<&str> = items
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .filter(|x| !x.trim().is_empty())
        .collect();
      if !parts.is_empty() {
        return Some(parts.join("\n"));
      }
    }
    _ => {}
  }
  if let Some(content) = u.get("content").and_then(Value::as_array) {
    let parts: Vec<&str> = content
      .iter()
      .filter(|c| c.get("type").and_then(Value::as_str) == Some("content"))
      .filter_map(|c| c.get("content"))
      .filter(|inner| inner.get("type").and_then(Value::as_str) == Some("text"))
      .filter_map(|inner| inner.get("text").and_then(Value::as_str))
      .filter(|x| !x.trim().is_empty())
      .collect();
    if !parts.is_empty() {
      return Some(parts.join("\n"));
    }
  }
  match &block.content {
    Some(ToolContent::Text { text }) => Some(text.clone()),
    _ => None,
  }
}
