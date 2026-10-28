//! First-class subagent nodes for one session: per-dialect
//! normalization (RFD / claude native sessions, Devin's nested cognition.ai updates, Claude legacy Agent, Kimi's
//! receipt), lifecycle, early-update buffering and record round trips. All mutations are synchronous. Nodes are found
//! through peer-id → node-id maps; the root transcript is reached through `RouteCtx`

use std::collections::HashMap;

use serde_json::{Map, Value};

use acpira_shared::num::Num;
use acpira_shared::subagents::{
  StateSource, SubagentControls, SubagentCore, SubagentHarness, SubagentPeer, SubagentRecord, SubagentState, SubagentSummary,
  SubagentUsage, SubagentVisibility,
};
use acpira_shared::transcript::{
  AgentBlock, AgentTurn, PermissionBlock, QuestionBlock, ToolCallBlock, ToolContent, ToolStatus, Turn, TurnStop,
};

use crate::acp::session::restore_turns::restore_interrupted_turns;
use crate::acp::transcript::normalize::{
  Log, NormalizeState, ToolCtx, activity_of, async_task_live, empty_tool, end_turn, find_tool_mut, latest_step, live_activity,
};
use crate::acp::vendors::claude_workflow_log::LogWindow;
use crate::i18n::t;
use crate::util::{ms_of_iso, now_iso, now_ms, random_uuid};

mod child_stream;
mod root_stream;
mod summoned;
mod workflow;

const ORPHAN_MAX_IDS: usize = 8;
const ORPHAN_MAX_UPDATES: usize = 64;
/// How far around its announce / end a resumed run's log window reaches: the CLI writes the resume prompt just before the
/// adapter announces the run, and the run's last lines just before its terminal word
const RESUMED_LOG_SLACK_MS: i64 = 2000;
/// `SendMessage` calls remembered until the run they resume is announced
const SEND_MESSAGES_MAX: usize = 16;

/// What routing may touch on the root side: the anchor turn index and the root transcript itself
pub struct RouteCtx<'a> {
  pub turn_index: usize,
  pub root_turns: &'a mut Vec<Turn>,
}

impl RouteCtx<'_> {
  fn root_tool(&mut self, id: &str) -> Option<&mut ToolCallBlock> {
    find_tool_mut(self.root_turns, id)
  }

  /// A stand-in delegation row at the live root turn's tail, for a root child whose launch call never reached the
  /// stream: claude-agent-acp 0.93.0 keeps native `Agent` / `Task` calls out of the parent transcript, so without it the
  /// child has no delegation point and renders after the whole turn. Skipped when a root row already names the node
  fn place_anchor(&mut self, node: &str, title: Option<String>) {
    let linked = self
      .root_turns
      .iter()
      .filter_map(Turn::as_agent)
      .flat_map(|t| &t.blocks)
      .any(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.subagent_id.as_deref() == Some(node)));
    if linked {
      return;
    }
    let Some(Turn::Agent(turn)) = self.root_turns.get_mut(self.turn_index) else { return };
    let now = now_ms();
    turn.blocks.push(AgentBlock::ToolCall(ToolCallBlock {
      id: anchor_id(node),
      verb_key: Some("verb.delegate".into()),
      verb: t("verb.delegate"),
      target: title,
      status: ToolStatus::Completed,
      started_at: Some(now),
      ended_at: Some(now),
      subagent_id: Some(node.to_owned()),
      ..empty_tool(String::new())
    }));
  }

  /// The node's real launch row linked: its stand-in goes, so the delegation sits where the call was
  fn drop_anchor(&mut self, node: &str) {
    let id = anchor_id(node);
    for turn in self.root_turns.iter_mut().filter_map(Turn::as_agent_mut) {
      turn.blocks.retain(|b| !matches!(b, AgentBlock::ToolCall(tc) if tc.id == id));
    }
  }
}

fn anchor_id(node: &str) -> String {
  format!("subagent:{node}")
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dialect {
  Claude,
  Devin,
  /// An agent of a Claude dynamic workflow (`claude_workflow.rs`)
  Workflow,
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
  /// The async task id of the workflow run a workflow agent belongs to (connection-local, never persisted)
  workflow: Option<String>,
  /// A child Acpira runs itself in another CLI (`acp/session/relay.rs`); its process is the session's, not the parent agent's
  harness: Option<SubagentHarness>,
  state: NormalizeState,
  rev: i64,
  cached: Option<(i64, SubagentSummary)>,
  restored: bool,
  late_logged: bool,
  /// A `SendMessage` resume of a Claude native child (`resumed_task`): claude-agent-acp drops the run's own updates, so
  /// its transcript is read from the child's sidechain log until the wire proves otherwise
  resumed: bool,
  /// Some of that log has landed: a late wire copy of the same run is no longer applied
  log_fed: bool,
}

/// The task id of a resumed native child run: claude-agent-acp announces the run a `SendMessage` starts on a finished
/// child as `<taskId>:generation:<n>` (n ≥ 2; source 0.83.0 `native-subagents.js` `nextChildSessionId`). Its updates
/// still name the first run's `parentToolUseId`, which the adapter routes to that finished first run and drops (stderr
/// "ignoring late update for terminal subagent"; observed 2026-10-11, 0.83.0, source unchanged in 0.89.1)
fn resumed_task(peer_session: &str) -> Option<&str> {
  let (task, n) = peer_session.split_once(":generation:")?;
  (!task.is_empty() && n.parse::<u32>().is_ok_and(|n| n >= 2)).then_some(task)
}

/// What `session/workflow_logs.rs` reads for one node: its log, whether it still runs, and the window of its own lines
pub struct LogTarget {
  pub node: String,
  pub agent_id: String,
  pub running: bool,
  pub window: Option<LogWindow>,
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

/// Any block at all: a node starts with one empty agent turn
fn has_content(turns: &[Turn]) -> bool {
  turns.iter().any(|t| t.as_agent().is_none_or(|a| !a.blocks.is_empty()))
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
  /// The adapter drops resumed native runs (`resumed_task`): read them from the sidechain log (`Vendor::resumed_child_logs`)
  resumed_logs: bool,
  /// Root `SendMessage` calls not yet tied to the run they resume: (tool call id, the agent id it names)
  send_messages: Vec<(String, String)>,
}

#[derive(serde::Serialize)]
pub struct RecordRef<'a> {
  #[serde(flatten)]
  pub core: SubagentCore,
  pub turns: &'a [Turn],
  pub rev: i64,
}

/// A summoned round's node (`relay_open`)
pub struct RelayOpen {
  pub parent: Option<String>,
  pub turn_index: usize,
  pub harness: SubagentHarness,
  pub role: String,
  pub title: String,
  pub task: String,
  pub model: Option<String>,
}

pub enum Route {
  Consumed,
  Root,
}

impl SubagentTree {
  pub fn new(log: Log, child_ctx: ToolCtx, records: Option<Vec<SubagentRecord>>, updated_at: Option<&str>, resumed_logs: bool) -> Self {
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
      resumed_logs,
      send_messages: vec![],
    };
    for r in records.unwrap_or_default() {
      let mut c = r.core;
      let was_running = c.state == SubagentState::Running;
      // A record from before the sidechain read still names its log through the peer id
      let resumed = resumed_logs && c.peer.session_id.as_deref().and_then(resumed_task).is_some();
      if resumed && c.peer.sidechain_id.is_none() {
        c.peer.sidechain_id = c.peer.session_id.as_deref().and_then(resumed_task).map(str::to_owned);
      }
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
        workflow: None,
        harness: c.harness,
        state,
        rev: r.rev.unwrap_or(1),
        cached: None,
        restored: true,
        late_logged: false,
        resumed,
        log_fed: false,
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

  /// Some subagent (a background workflow agent or a summoned child included) is still working
  pub fn any_running(&self) -> bool {
    self.nodes.iter().any(|n| n.status == SubagentState::Running)
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
      workflow: None,
      harness: None,
      state,
      rev: 0,
      cached: None,
      restored: false,
      late_logged: false,
      resumed: false,
      log_fed: false,
    }
  }

  /// End of the connection / turn
  pub fn settle(&mut self, reason: &str) {
    let prompt_returned = reason == "prompt-returned";
    for n in &mut self.nodes {
      if n.status != SubagentState::Running {
        continue;
      }
      // A workflow runs in the background past the prompt that launched it; its agents end with the run (`workflow_ended`).
      // A summoned child is Acpira's own process: it keeps working after a tool call that returned early, and the
      // session ends it explicitly (`relay.rs`)
      if prompt_returned && (n.dialect == Some(Dialect::Workflow) || n.harness.is_some()) {
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

  /// The live activity line of a node, as its row shows it
  pub fn activity(&self, id: &str) -> Option<String> {
    let n = self.nodes.iter().find(|n| n.id == id)?;
    let turns = &n.state.turns;
    live_activity(turns).or_else(|| latest_step(turns)).or_else(|| activity_of(turns)).map(|a| a.label)
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
      has_process: (n.visibility == SubagentVisibility::Receipt && has_content(&n.state.turns)).then_some(true),
      harness: n.harness.clone(),
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
        // The row's second line is the only live view of a child. Read-only tools finish within a frame, so between
        // steps it keeps the latest one instead of dropping back to "Working", which read as a flicker
        let turns = &n.state.turns;
        core.activity = live_activity(turns).or_else(|| latest_step(turns)).or_else(|| activity_of(turns)).map(|a| a.label);
      }
      let summary = SubagentSummary { core, permissions: (!permissions.is_empty()).then_some(permissions), question };
      n.cached = Some((n.rev, summary));
    }
    self.nodes.iter().filter_map(|n| n.cached.as_ref().map(|c| c.1.clone())).collect()
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

#[cfg(test)]
mod tests {
  use super::resumed_task;

  #[test]
  fn only_a_later_generation_names_a_resumed_task() {
    assert_eq!(resumed_task("af556d847b5f299c3:generation:2"), Some("af556d847b5f299c3"));
    assert_eq!(resumed_task("af556d847b5f299c3:generation:13"), Some("af556d847b5f299c3"));
    assert_eq!(resumed_task("thr:generation:1"), None);
    assert_eq!(resumed_task(":generation:2"), None);
    assert_eq!(resumed_task("k1:generation:x"), None);
    assert_eq!(resumed_task("k1"), None);
  }
}
