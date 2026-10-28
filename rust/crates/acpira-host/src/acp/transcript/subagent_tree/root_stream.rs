//! Routing of the root stream's delegation rows: the nested and receipt dialects (Devin's cognition.ai updates, Claude's
//! legacy Agent, Kimi's receipt), delegation links on the root rows, and `SendMessage` resumes

use serde_json::{Map, Value};

use acpira_shared::subagents::{SubagentPeer, SubagentState, SubagentVisibility};

use super::{
  Dialect, PendingDelegation, PendingLaunch, Route, RouteCtx, SEND_MESSAGES_MAX, SubagentTree, rec, result_text, s, seal_launch_receipt,
  terminal,
};
use crate::i18n::t;

impl SubagentTree {
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
    // A finished child of an earlier turn whose id a fresh agent session reused (a historical edit, see `normalize.rs`
    // `settled_before`): the new row is not linked to it, so the stale mapping goes and the call may open its own child
    if let Some(i) = self.by(&self.by_tool, &tool_call_id)
      && terminal(self.nodes[i].status)
      && self.nodes[i].turn_index < ctx.turn_index as u64
      && ctx.root_tool(&tool_call_id).is_some_and(|b| b.subagent_id.as_deref() != Some(self.nodes[i].id.as_str()))
    {
      self.by_tool.remove(&tool_call_id);
    }
    let meta = rec(u.get("_meta"));
    let raw = rec(u.get("rawInput"));
    if s(meta.and_then(|m| m.get("cognition.ai/inferenceToolName"))).as_deref() == Some("read_subagent") {
      let title =
        s(raw.and_then(|r| r.get("agent_id"))).and_then(|a| self.by(&self.by_agent, &a)).and_then(|i| self.nodes[i].title.clone());
      let Some(block) = ctx.root_tool(&tool_call_id) else { return };
      block.verb_key = Some("verb.awaitSubagent".into());
      block.verb = t("verb.awaitSubagent");
      if let Some(title) = title {
        block.target = Some(title);
        block.target_mono = None;
      }
      return;
    }
    let claude = meta.and_then(|m| rec(m.get("claudeCode")));
    if self.resumed_logs
      && self.by(&self.by_tool, &tool_call_id).is_none()
      && s(claude.and_then(|c| c.get("toolName"))).as_deref() == Some("SendMessage")
      && let Some(to) = s(raw.and_then(|r| r.get("to")))
    {
      self.note_send_message(&tool_call_id, &to, ctx);
      return;
    }
    let mut n = self.by(&self.by_tool, &tool_call_id);
    if claude.and_then(|c| c.get("subagent")) == Some(&Value::Bool(true)) {
      let i = match n {
        Some(i) => i,
        None => {
          let Some(i) = self.upsert_call_node(&tool_call_id, SubagentVisibility::Nested, ctx) else { return };
          i
        }
      };
      self.nodes[i].dialect = Some(Dialect::Claude);
      n = Some(i);
    } else if n.is_none() && s(raw.and_then(|r| r.get("subagent_type"))).is_some() && s(raw.and_then(|r| r.get("prompt"))).is_some() {
      n = self.upsert_call_node(&tool_call_id, SubagentVisibility::Receipt, ctx);
    }
    let Some(i) = n else { return };
    let id = self.nodes[i].id.clone();
    let Some(block) = ctx.root_tool(&tool_call_id) else { return };
    block.subagent_id = Some(id.clone());
    if self.nodes[i].visibility == SubagentVisibility::Session {
      seal_launch_receipt(block);
      ctx.drop_anchor(&id);
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
          let Some(block) = ctx.root_tool(&tool_call_id) else { return };
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

  /// A root `SendMessage` naming an agent: tied to that agent's resumed run if it is already announced in this turn,
  /// else remembered for its announce. The call streams in with an empty input first, so `to` may come with an update
  fn note_send_message(&mut self, call: &str, to: &str, ctx: &mut RouteCtx) {
    let turn = ctx.turn_index as u64;
    let run = self.nodes.iter().rposition(|n| {
      n.resumed && n.peer.tool_call_id.is_none() && n.turn_index == turn && n.peer.sidechain_id.as_deref() == Some(to)
    });
    if let Some(i) = run {
      self.link_resume_call(i, call, ctx);
      return;
    }
    if self.send_messages.iter().any(|(c, _)| c == call) {
      return;
    }
    if self.send_messages.len() >= SEND_MESSAGES_MAX {
      self.send_messages.remove(0);
    }
    self.send_messages.push((call.to_owned(), to.to_owned()));
  }

  /// A resumed run sits where the `SendMessage` that started it is, the way a first run sits at its Agent call
  pub(super) fn link_resume_call(&mut self, i: usize, call: &str, ctx: &mut RouteCtx) {
    let id = self.nodes[i].id.clone();
    let Some(block) = ctx.root_tool(call) else { return };
    if block.subagent_id.is_some() {
      return;
    }
    block.subagent_id = Some(id.clone());
    seal_launch_receipt(block);
    ctx.drop_anchor(&id);
    self.nodes[i].peer.tool_call_id = Some(call.to_owned());
    self.by_tool.insert(call.to_owned(), id.clone());
    self.bump(&id);
  }

  fn upsert_call_node(&mut self, tool_call_id: &str, visibility: SubagentVisibility, ctx: &mut RouteCtx) -> Option<usize> {
    if let Some(i) = self.by(&self.by_tool, tool_call_id) {
      return Some(i);
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
    // None only if a replayed update took the new node out again
    self.idx(&id)
  }
}
