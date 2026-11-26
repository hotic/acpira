//! Antigravity (`agy` 1.5.16) native `/swarm` workers and `invoke_subagent` children:
//! - Child tool calls arrive on the root stream with `toolCallId` prefixed `<child_uuid>:<inner_call_id>` and route into
//!   their own `Session` node so they never mix into the root transcript or split streamed prose.
//! - Root `swarm { action: "add" }` and `invoke_subagent` calls open delegation nodes in place before the child's first
//!   wire call or disk poll lands; once disk metadata (`vendors/antigravity_subagents.rs`) maps roster names / roles to
//!   conversation UUIDs, wire-only and call-created nodes merge seamlessly.

use serde_json::{Value, json};

use acpira_shared::subagents::{SubagentPeer, SubagentState, SubagentVisibility};
use acpira_shared::transcript::{AgentBlock, Turn};

use super::{Dialect, RouteCtx, SubagentTree, has_content, last_text, seal_launch_receipt, terminal, tool_count};
use crate::acp::transcript::normalize::apply_update;
use crate::acp::vendors::antigravity::{AntigravityDelegation, delegation_of, split_child_tool_call_id};
use crate::acp::vendors::antigravity_subagents::{AntigravitySnapshot, first_plain_line};
use crate::i18n::t;

fn swarm_key(name: &str) -> String {
  format!("swarm:{name}")
}

fn role_key(role: &str) -> String {
  format!("role:{role}")
}

fn short_uuid(cid: &str) -> String {
  cid.chars().take(8).collect()
}

struct LaunchSpec {
  key: Option<String>,
  title: Option<String>,
  role: Option<String>,
  task: Option<String>,
  model: Option<String>,
}

/// Appends a board post as a standalone settled Markdown text block inside the child's transcript
fn append_board_post(turns: &mut crate::acp::transcript::normalize::NormalizeState, text: &str) {
  let ends_with_text = matches!(
    turns.turns.last().and_then(Turn::as_agent).and_then(|t| t.blocks.last()),
    Some(AgentBlock::Text(tb)) if !tb.markdown.is_empty()
  );
  let chunk = if ends_with_text { format!("\n\n{text}") } else { text.to_owned() };
  apply_update(turns, &json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": chunk } }));
  // Board posts from disk are already complete messages, not in-flight token streams
  if let Some(AgentBlock::Text(tb)) = turns.turns.last_mut().and_then(Turn::as_agent_mut).and_then(|t| t.blocks.last_mut()) {
    tb.streaming = Some(false);
  }
}

impl SubagentTree {
  /// Routes an Antigravity child `tool_call` / `tool_call_update` (`<child_uuid>:<inner_call_id>`) off the root stream
  /// into its child node. Returns `true` when consumed
  pub fn route_antigravity_child(&mut self, u: &Value, root_peer: Option<&str>, ctx: &mut RouteCtx) -> bool {
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "tool_call" | "tool_call_update") {
      return false;
    }
    let Some(tid) = u.get("toolCallId").and_then(Value::as_str) else { return false };
    let Some((child_cid, _)) = split_child_tool_call_id(tid, root_peer) else { return false };
    let node_id = self.open_or_find_antigravity_child(child_cid, ctx);
    self.apply_child(&node_id, u, ctx);
    true
  }

  /// Ensures a live node exists for `child_cid` when a permission or elicitation request arrives for `<child_cid>:...`
  /// before the child's first tool call notification. Returns `None` if that child already ended in this session
  pub fn ensure_antigravity_child(&mut self, child_cid: &str, ctx: &mut RouteCtx) -> Option<String> {
    if let Some(i) = self.by(&self.by_session, child_cid) {
      let n = &self.nodes[i];
      if !terminal(n.status) {
        return Some(n.id.clone());
      }
      let revivable = n.status == SubagentState::Completed
        || (n.restored && n.status == SubagentState::Disconnected && n.turn_index < ctx.turn_index as u64);
      if !revivable {
        return None;
      }
    }
    Some(self.open_or_find_antigravity_child(child_cid, ctx))
  }

  fn open_or_find_antigravity_child(&mut self, child_cid: &str, ctx: &mut RouteCtx) -> String {
    if let Some(i) = self.by(&self.by_session, child_cid) {
      let n = &self.nodes[i];
      // A restored swarm worker or previously settled non-swarm child woken for a follow-up round gets a fresh node
      // instead of mutating a terminal record
      let reopen = n.status == SubagentState::Completed
        || (n.restored && n.status == SubagentState::Disconnected && n.turn_index < ctx.turn_index as u64);
      if !reopen {
        return n.id.clone();
      }
      let (title, role, model, agy_key, settled_step) = (n.title.clone(), n.role.clone(), n.model.clone(), n.agy_key.clone(), n.agy_settled_step);
      let mut fresh = self.new_node(
        SubagentVisibility::Session,
        SubagentPeer { session_id: Some(child_cid.to_owned()), ..Default::default() },
        ctx.turn_index,
        false,
        Some(Dialect::Antigravity),
      );
      fresh.background = true;
      fresh.title = title;
      fresh.role = role;
      fresh.model = model;
      fresh.agy_key = agy_key;
      fresh.agy_settled_step = settled_step;
      let id = fresh.id.clone();
      let anchor_title = fresh.title.clone();
      self.by_session.insert(child_cid.to_owned(), id.clone());
      self.nodes.push(fresh);
      ctx.place_anchor(&id, anchor_title);
      self.bump(&id);
      return id;
    }
    let mut node = self.new_node(
      SubagentVisibility::Session,
      SubagentPeer { session_id: Some(child_cid.to_owned()), ..Default::default() },
      ctx.turn_index,
      false,
      Some(Dialect::Antigravity),
    );
    node.background = true;
    node.title = Some(short_uuid(child_cid));
    let id = node.id.clone();
    let anchor_title = node.title.clone();
    self.by_session.insert(child_cid.to_owned(), id.clone());
    self.nodes.push(node);
    ctx.place_anchor(&id, anchor_title);
    self.bump(&id);
    id
  }

  /// Links root `swarm { action: "add" }` and `invoke_subagent` tool calls to Antigravity delegation nodes right at
  /// their call site. Returns `true` when the call is an Antigravity delegation
  pub fn annotate_antigravity_root(&mut self, u: &Value, ctx: &mut RouteCtx) -> bool {
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("");
    if !matches!(kind, "tool_call" | "tool_call_update") {
      return false;
    }
    let Some(call_id) = u.get("toolCallId").and_then(Value::as_str).filter(|s| !s.is_empty()) else { return false };
    let status = u.get("status").and_then(Value::as_str);

    // Follow-up update on a call already linked to an Antigravity node
    if let Some(i) = self.by(&self.by_tool, call_id)
      && self.nodes[i].dialect == Some(Dialect::Antigravity)
    {
      let id = self.nodes[i].id.clone();
      if let Some(del) = delegation_of(u) {
        self.apply_delegation_fields(i, &del);
      }
      if let Some(block) = ctx.root_tool(call_id) {
        block.subagent_id = Some(id.clone());
        block.verb_key = Some("verb.delegate".into());
        block.verb = t("verb.delegate");
        if matches!(status, Some("completed")) {
          seal_launch_receipt(block);
        }
      }
      // If the launch call itself failed or was cancelled before a child ever started, settle the node
      if self.nodes[i].peer.session_id.is_none() {
        match status {
          Some("failed") => self.transition(&id, SubagentState::Failed),
          Some("cancelled") => self.transition(&id, SubagentState::Cancelled),
          _ => {}
        }
      }
      self.bump(&id);
      return true;
    }

    let Some(del) = delegation_of(u) else { return false };
    if ctx.root_tool(call_id).is_none() {
      return false;
    }
    match &del {
      AntigravityDelegation::SwarmAdd { names, brief, model } => {
        for (idx, name) in names.iter().enumerate() {
          let key = swarm_key(name);
          if idx == 0 {
            let spec = LaunchSpec {
              key: Some(key),
              title: Some(name.clone()),
              role: Some(name.clone()),
              task: brief.clone(),
              model: model.clone(),
            };
            self.link_or_create_antigravity_call(call_id, spec, status, ctx);
          } else {
            // Sharded `swarm add` (`n > 1` / `tasks`): extra workers anchor right behind the launch call
            let existing = self.nodes.iter().position(|n| n.dialect == Some(Dialect::Antigravity) && n.agy_key.as_deref() == Some(key.as_str()));
            if existing.is_none() {
              let mut node = self.new_node(SubagentVisibility::Session, SubagentPeer::default(), ctx.turn_index, false, Some(Dialect::Antigravity));
              node.background = true;
              node.title = Some(name.clone());
              node.role = Some(name.clone());
              node.task = brief.clone();
              node.model = model.clone();
              node.agy_key = Some(key);
              let id = node.id.clone();
              self.nodes.push(node);
              ctx.place_anchor(&id, Some(name.clone()));
              if matches!(status, Some("failed")) {
                self.transition(&id, SubagentState::Failed);
              } else if matches!(status, Some("cancelled")) {
                self.transition(&id, SubagentState::Cancelled);
              }
              self.bump(&id);
            }
          }
        }
      }
      AntigravityDelegation::InvokeSubagent { role, type_name, prompt, model } => {
        let title = role.clone().or_else(|| type_name.clone());
        let node_role = type_name.clone().filter(|tn| tn != "self" && Some(tn.as_str()) != title.as_deref());
        let spec = LaunchSpec {
          key: role.as_deref().map(role_key),
          title,
          role: node_role,
          task: prompt.clone(),
          model: model.clone(),
        };
        self.link_or_create_antigravity_call(call_id, spec, status, ctx);
      }
    }
    true
  }

  fn link_or_create_antigravity_call(&mut self, call_id: &str, spec: LaunchSpec, status: Option<&str>, ctx: &mut RouteCtx) -> String {
    // Reuse an already-discovered disk/wire node for this worker if it has no launch call yet
    let existing = spec.key.as_deref().and_then(|k| {
      self.nodes.iter().position(|n| {
        n.dialect == Some(Dialect::Antigravity)
          && n.peer.tool_call_id.is_none()
          && n.turn_index == ctx.turn_index as u64
          && n.agy_key.as_deref() == Some(k)
      })
    });
    let i = match existing {
      Some(i) => {
        let n = &mut self.nodes[i];
        n.peer.tool_call_id = Some(call_id.to_owned());
        if n.task.is_none() {
          n.task = spec.task;
        }
        if n.model.is_none() {
          n.model = spec.model;
        }
        let id = n.id.clone();
        self.by_tool.insert(call_id.to_owned(), id.clone());
        ctx.drop_anchor(&id);
        i
      }
      None => {
        let mut node = self.new_node(
          SubagentVisibility::Session,
          SubagentPeer { tool_call_id: Some(call_id.to_owned()), ..Default::default() },
          ctx.turn_index,
          false,
          Some(Dialect::Antigravity),
        );
        node.background = true;
        node.title = spec.title;
        node.role = spec.role;
        node.task = spec.task;
        node.model = spec.model;
        node.agy_key = spec.key;
        let id = node.id.clone();
        self.by_tool.insert(call_id.to_owned(), id);
        self.nodes.push(node);
        self.nodes.len() - 1
      }
    };
    let id = self.nodes[i].id.clone();
    if let Some(block) = ctx.root_tool(call_id) {
      block.subagent_id = Some(id.clone());
      block.verb_key = Some("verb.delegate".into());
      block.verb = t("verb.delegate");
      if matches!(status, Some("completed")) {
        seal_launch_receipt(block);
      }
    }
    if self.nodes[i].peer.session_id.is_none() {
      match status {
        Some("failed") => self.transition(&id, SubagentState::Failed),
        Some("cancelled") => self.transition(&id, SubagentState::Cancelled),
        _ => {}
      }
    }
    self.bump(&id);
    id
  }

  fn apply_delegation_fields(&mut self, i: usize, del: &AntigravityDelegation) {
    let n = &mut self.nodes[i];
    match del {
      AntigravityDelegation::SwarmAdd { names, brief, model } => {
        if let Some(name) = names.first() {
          if n.title.is_none() {
            n.title = Some(name.clone());
          }
          if n.role.is_none() {
            n.role = Some(name.clone());
          }
          if n.agy_key.is_none() {
            n.agy_key = Some(swarm_key(name));
          }
        }
        if n.task.is_none() && brief.is_some() {
          n.task = brief.clone();
        }
        if n.model.is_none() && model.is_some() {
          n.model = model.clone();
        }
      }
      AntigravityDelegation::InvokeSubagent { role, type_name, prompt, model } => {
        if n.title.is_none() {
          n.title = role.clone().or_else(|| type_name.clone());
        }
        if n.role.is_none() {
          n.role = type_name.clone().filter(|tn| tn != "self" && Some(tn.as_str()) != n.title.as_deref());
        }
        if n.task.is_none() && prompt.is_some() {
          n.task = prompt.clone();
        }
        if n.model.is_none() && model.is_some() {
          n.model = model.clone();
        }
        if n.agy_key.is_none() {
          n.agy_key = role.as_deref().map(role_key);
        }
      }
    }
  }

  /// Applies a disk metadata snapshot (`subagents/*.json`, `swarm/members.json`, `swarm/plan.json`, `swarm/board.jsonl`)
  /// onto the tree. Merges wire-only nodes with pending launch-call nodes and transitions killed children to completed.
  /// Returns `true` when any node changed
  pub fn sync_antigravity(&mut self, snap: AntigravitySnapshot, ctx: &mut RouteCtx) -> bool {
    let mut changed = false;
    for child in snap.children {
      let match_key = child
        .swarm_name
        .as_deref()
        .map(swarm_key)
        .or_else(|| child.title.as_deref().map(role_key));
      let by_cid = self.by(&self.by_session, &child.conversation_id);
      let pending_call = match_key.as_deref().and_then(|k| {
        self.nodes.iter().position(|n| {
          n.dialect == Some(Dialect::Antigravity)
            && n.peer.session_id.is_none()
            && !terminal(n.status)
            && n.agy_key.as_deref() == Some(k)
        })
      });

      let i = match (pending_call, by_cid) {
        (Some(keep), Some(wire)) if keep != wire => {
          // Wire tool calls arrived before disk linked the launch call to this uuid: fold wire work into the launch node
          let keep_id = self.nodes[keep].id.clone();
          let wire_id = self.nodes[wire].id.clone();
          self.merge_antigravity_wire_into_call(&keep_id, &wire_id, &child.conversation_id, ctx);
          changed = true;
          let Some(idx) = self.idx(&keep_id) else { continue };
          idx
        }
        (Some(keep), _) => {
          let id = self.nodes[keep].id.clone();
          self.nodes[keep].peer.session_id = Some(child.conversation_id.clone());
          self.by_session.insert(child.conversation_id.clone(), id);
          changed = true;
          keep
        }
        (None, Some(idx)) => {
          // Do not mutate or recreate a restored terminal node from an earlier run
          if self.nodes[idx].restored && terminal(self.nodes[idx].status) {
            continue;
          }
          idx
        }
        (None, None) => {
          let mut node = self.new_node(
            SubagentVisibility::Session,
            SubagentPeer { session_id: Some(child.conversation_id.clone()), ..Default::default() },
            ctx.turn_index,
            false,
            Some(Dialect::Antigravity),
          );
          node.background = true;
          node.agy_key = match_key.clone();
          let id = node.id.clone();
          self.by_session.insert(child.conversation_id.clone(), id.clone());
          self.nodes.push(node);
          ctx.place_anchor(&id, child.title.clone());
          changed = true;
          self.nodes.len() - 1
        }
      };

      let stale_idle = child
        .idle_step
        .is_some_and(|step| self.nodes[i].agy_settled_step.is_some_and(|prev| step <= prev));
      let mut node_changed = false;
      {
        let n = &mut self.nodes[i];
        if n.dialect.is_none() {
          n.dialect = Some(Dialect::Antigravity);
        }
        if n.agy_key.is_none() && match_key.is_some() {
          n.agy_key = match_key;
        }
        if let Some(title) = child.title
          && n.title.as_deref() != Some(title.as_str())
        {
          n.title = Some(title);
          node_changed = true;
        }
        if let Some(role) = child.role
          && n.role.as_deref() != Some(role.as_str())
        {
          n.role = Some(role);
          node_changed = true;
        }
        if n.task.is_none()
          && let Some(task) = child.task
        {
          n.task = Some(task);
          node_changed = true;
        }
        if let Some(model) = child.model
          && n.model.as_deref() != Some(model.as_str())
        {
          n.model = Some(model);
          node_changed = true;
        }
        if n.agy_waiting != child.waiting {
          n.agy_waiting = child.waiting;
          node_changed = true;
        }
        for post in child.new_posts {
          append_board_post(&mut n.state, &post.text);
          n.result = Some(post.text);
          node_changed = true;
        }
        // Ignore a stale previous-round report while a re-woken non-swarm child has not flushed its new step yet
        if !stale_idle {
          let note = child.latest_report.as_deref().and_then(first_plain_line);
          if note.is_some() && n.agy_note != note {
            n.agy_note = note;
            node_changed = true;
          }
          if n.result.is_none()
            && let Some(report) = child.latest_report
          {
            if last_text(&n.state.turns).is_none() {
              append_board_post(&mut n.state, &report);
            }
            n.result = Some(report);
            node_changed = true;
          }
        }
      }
      let id = self.nodes[i].id.clone();
      if child.ended && !stale_idle && self.nodes[i].status == SubagentState::Running {
        self.nodes[i].agy_waiting = false;
        if let Some(step) = child.idle_step {
          self.nodes[i].agy_settled_step = Some(step);
        }
        self.transition(&id, SubagentState::Completed);
        node_changed = true;
      }
      if node_changed {
        self.bump(&id);
        changed = true;
      }
    }
    changed
  }

  /// Folds a wire-created node's transcript into the root launch-call node once disk metadata links them
  fn merge_antigravity_wire_into_call(&mut self, keep_id: &str, wire_id: &str, child_cid: &str, ctx: &mut RouteCtx) {
    let Some(wi) = self.idx(wire_id) else { return };
    let wire = self.nodes.remove(wi);
    ctx.drop_anchor(wire_id);
    // Retarget any tool_owner entries that belonged to the merged wire node
    for owner in self.tool_owner.values_mut() {
      if owner == wire_id {
        *owner = keep_id.to_owned();
      }
    }
    let Some(ki) = self.idx(keep_id) else { return };
    let keep = &mut self.nodes[ki];
    keep.peer.session_id = Some(child_cid.to_owned());
    if has_content(&wire.state.turns) {
      keep.state = wire.state;
      keep.tool_count = tool_count(&keep.state.turns);
    }
    if keep.result.is_none() {
      keep.result = wire.result;
    }
    if wire.ended_at.is_some() {
      keep.ended_at = wire.ended_at;
    }
    self.by_session.insert(child_cid.to_owned(), keep_id.to_owned());
    self.bump(keep_id);
  }

  /// Whether any Antigravity child node or orchestrator round is still running (drives disk metadata polling)
  pub fn has_running_antigravity(&self) -> bool {
    self.nodes.iter().any(|n| matches!(n.dialect, Some(Dialect::Antigravity | Dialect::Orchestrator)) && n.status == SubagentState::Running)
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use serde_json::json;

  use acpira_shared::subagents::{SubagentState, SubagentVisibility};
  use acpira_shared::transcript::{AgentBlock, AgentTurn, Turn};

  use super::*;
  use crate::acp::transcript::normalize::{NormalizeState, ToolCtx, apply_update};
  use crate::acp::vendors::antigravity_subagents::{AntigravityChildMeta, BoardPost};

  fn tree() -> SubagentTree {
    SubagentTree::new(Arc::new(|_| {}), ToolCtx::default(), None, None, false)
  }

  #[test]
  fn child_wire_tool_calls_route_into_subagent_and_merge_with_swarm_launch_call() {
    let mut t = tree();
    let mut root = NormalizeState::new(vec![Turn::Agent(AgentTurn::default())]);
    let root_id = "10000000-0000-4000-8000-000000000001";
    let child_id = "30000000-0000-4000-8000-000000000003";

    // 1. Coordinator launches `ui` via `swarm add`
    let add = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": "call_add_ui",
      "kind": "other",
      "title": "Run swarm",
      "status": "completed",
      "rawInput": { "action": "add", "name": "ui", "brief": "Implement the settings form" }
    });
    assert!(apply_update(&mut root, &add));
    assert!(t.annotate_antigravity_root(&add, &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    assert_eq!(t.len(), 1);
    let launch_node_id = match &root.turns[0].as_agent().unwrap().blocks[0] {
      AgentBlock::ToolCall(tc) => tc.subagent_id.clone().expect("linked to launch call"),
      _ => panic!("expected tool call"),
    };

    // 2. Worker tool call arrives BEFORE disk metadata linked its uuid: must not pollute root
    let child_call = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": format!("{child_id}:call_read_1"),
      "kind": "read",
      "title": "Read settings.rs",
      "status": "completed",
      "locations": [{ "path": "/Users/dev/example-app/src/settings.rs" }]
    });
    assert!(t.route_antigravity_child(&child_call, Some(root_id), &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    // Wire created a temporary node with a stand-in anchor; root still has zero worker tool rows
    assert_eq!(t.len(), 2);

    // 3. Disk metadata arrives mapping `ui` -> `child_id` + `plan.json` task title + board post
    let snap = AntigravitySnapshot {
      children: vec![AntigravityChildMeta {
        conversation_id: child_id.into(),
        swarm_name: Some("ui".into()),
        title: Some("Settings form".into()),
        role: Some("ui".into()),
        task: Some("Implement the settings form".into()),
        model: Some("gemini-4-neon-xhigh".into()),
        waiting: true,
        ended: false,
        idle_step: None,
        new_posts: vec![BoardPost { seq: 4, text: "@core The form is ready".into() }],
        latest_report: Some("@core The form is ready".into()),
      }],
      ..Default::default()
    };
    assert!(t.sync_antigravity(snap, &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    // Temporary wire node merged cleanly into the launch call node; stand-in anchor removed
    assert_eq!(t.len(), 1);
    let summaries = t.summaries();
    let s = &summaries[0].core;
    assert_eq!(s.id, launch_node_id);
    assert_eq!(s.visibility, SubagentVisibility::Session);
    assert_eq!(s.title.as_deref(), Some("Settings form"));
    assert_eq!(s.role.as_deref(), Some("ui"));
    assert_eq!(s.task.as_deref(), Some("Implement the settings form"));
    assert_eq!(s.model.as_deref(), Some("gemini-4-neon-xhigh"));
    assert_eq!(s.tool_count, 1);
    assert_eq!(s.state, SubagentState::Running);
    assert_eq!(s.activity.as_deref(), Some("@core The form is ready"));
    // Root turn has only the launch call block, no leftover stand-in anchor
    assert_eq!(root.turns[0].as_agent().unwrap().blocks.len(), 1);

    // 4. Parent prompt finishes (`end_turn`): worker outlives prompt under `native_outlive = true`
    t.settle("prompt-returned", true);
    assert_eq!(t.summaries()[0].core.state, SubagentState::Running);

    // 5. Subsequent wire call routes directly to the merged node
    let child_call_2 = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": format!("{child_id}:call_run_2"),
      "kind": "execute",
      "title": "Run cargo test",
      "status": "in_progress",
      "rawInput": { "command": "cargo test" }
    });
    assert!(t.route_antigravity_child(&child_call_2, Some(root_id), &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    assert_eq!(t.summaries()[0].core.tool_count, 2);
    assert!(t.summaries()[0].core.activity.as_deref().unwrap_or("").contains("cargo test"));

    // 6. Worker killed on disk -> completed with latest board post as result
    let end_snap = AntigravitySnapshot {
      children: vec![AntigravityChildMeta {
        conversation_id: child_id.into(),
        swarm_name: Some("ui".into()),
        title: Some("Settings form".into()),
        role: Some("ui".into()),
        task: None,
        model: Some("gemini-4-neon-xhigh".into()),
        waiting: false,
        ended: true,
        idle_step: None,
        new_posts: vec![],
        latest_report: Some("@core The form is ready".into()),
      }],
      ..Default::default()
    };
    assert!(t.sync_antigravity(end_snap, &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    let done = &t.summaries()[0].core;
    assert_eq!(done.state, SubagentState::Completed);
    assert_eq!(done.result.as_deref(), Some("@core The form is ready"));
  }

  #[test]
  fn settled_non_swarm_child_wakes_as_fresh_node_and_ignores_stale_idle_step() {
    let mut t = tree();
    let mut root = NormalizeState::new(vec![Turn::Agent(AgentTurn::default())]);
    let root_id = "10000000-0000-4000-8000-000000000001";
    let child_id = "40000000-0000-4000-8000-000000000004";

    let inv = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": "call_img_1",
      "kind": "other",
      "title": "Run invoke_subagent",
      "status": "completed",
      "rawInput": { "TypeName": "image-generator", "Role": "Icon generator", "Prompt": "Draw an icon" }
    });
    assert!(apply_update(&mut root, &inv));
    assert!(t.annotate_antigravity_root(&inv, &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));

    // 1. Child finishes its first turn at step 4 (state still SUBAGENT_STATE_ALIVE on disk)
    let snap1 = AntigravitySnapshot {
      children: vec![AntigravityChildMeta {
        conversation_id: child_id.into(),
        swarm_name: None,
        title: Some("Icon generator".into()),
        role: Some("image-generator".into()),
        task: None,
        model: None,
        waiting: false,
        ended: true,
        idle_step: Some(4),
        new_posts: vec![],
        latest_report: Some("Generated icon.png".into()),
      }],
      ..Default::default()
    };
    assert!(t.sync_antigravity(snap1.clone(), &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    assert_eq!(t.summaries()[0].core.state, SubagentState::Completed);
    assert_eq!(t.summaries()[0].core.result.as_deref(), Some("Generated icon.png"));
    assert!(!t.has_running_antigravity());

    // 2. Parent follows up via `send_message`, waking the same child uuid for round 2
    let round2_call = json!({
      "sessionUpdate": "tool_call",
      "toolCallId": format!("{child_id}:call_2"),
      "kind": "other",
      "title": "Generate variant",
      "status": "in_progress"
    });
    assert!(t.route_antigravity_child(&round2_call, Some(root_id), &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    assert_eq!(t.len(), 2);
    assert_eq!(t.summaries()[1].core.state, SubagentState::Running);
    assert!(t.has_running_antigravity());

    // 3. A poll before round 2 flushes to disk still sees idle_step=4 -> must NOT close round 2 early
    t.sync_antigravity(snap1, &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns });
    assert_eq!(t.summaries()[1].core.state, SubagentState::Running);

    // 4. Round 2 finishes at step 9 -> closes cleanly
    let snap2 = AntigravitySnapshot {
      children: vec![AntigravityChildMeta {
        conversation_id: child_id.into(),
        swarm_name: None,
        title: Some("Icon generator".into()),
        role: Some("image-generator".into()),
        task: None,
        model: None,
        waiting: false,
        ended: true,
        idle_step: Some(9),
        new_posts: vec![],
        latest_report: Some("Generated icon_v2.png".into()),
      }],
      ..Default::default()
    };
    assert!(t.sync_antigravity(snap2, &mut RouteCtx { turn_index: 0, root_turns: &mut root.turns }));
    assert_eq!(t.summaries()[1].core.state, SubagentState::Completed);
    assert_eq!(t.summaries()[1].core.result.as_deref(), Some("Generated icon_v2.png"));
    assert!(!t.has_running_antigravity());
  }
}
