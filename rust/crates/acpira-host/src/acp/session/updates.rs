//! `session/update` routing: extension updates, subagent sessions, replay filtering and vendor corrections before the
//! normalizer, then the usage / plan / activity bookkeeping after it

use std::sync::Arc;

use serde_json::Value;

use acpira_shared::transcript::*;

use crate::acp::session::failure::failure_of;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::{activity_of, apply_async_task, apply_update};
use crate::acp::transcript::plans::{capture_plan, plan_documents};
use crate::acp::transcript::subagent_tree::{Route, RouteCtx};
use crate::acp::transport::wire::{ExtensionUpdate, extension_of};
use crate::acp::vendors::{claude_autonomous, claude_window, steering};

const RUNNING_KINDS: [&str; 6] =
  ["user_message_chunk", "agent_message_chunk", "agent_thought_chunk", "tool_call", "tool_call_update", "plan"];

impl AcpSession {
  pub(crate) fn on_update(self: &Arc<Self>, n: Value) {
    let session_id = n.get("sessionId").and_then(Value::as_str).unwrap_or("").to_owned();
    let mut u = n.get("update").cloned().unwrap_or(Value::Null);
    let kind = u.get("sessionUpdate").and_then(Value::as_str).unwrap_or("").to_owned();
    let mut c = self.core.lock();
    if c.phase.editing {
      if matches!(kind.as_str(), "available_commands_update" | "usage_update") {
        c.phase.edit_notifications.push(n);
      }
      return;
    }
    // Extension updates: subagent lifecycle and AIR async tasks
    let log = |line: &str| self.log(line);
    if let Some(ext) = extension_of(&u, &log) {
      match ext {
        ExtensionUpdate::Ignored(k) => self.log(&format!("{k} ignored")),
        ExtensionUpdate::AsyncTask(e) => self.async_task_update(&mut c, &session_id, &e),
        ExtensionUpdate::Workflow(w) => {
          // Raw frames only arrive live; a workflow run under a child session is not observed, so only the root's count
          if c.replaying || c.acp_session_id.as_deref() != Some(session_id.as_str()) {
            self.log(&format!("workflow progress on {session_id} ignored"));
          } else {
            let turn_index = current_turn_index(&c);
            let Core { tree, state, .. } = &mut *c;
            let changed = tree.workflow_progress(w, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
            // An agent with an id has a sidechain log to follow (`workflow_logs.rs`)
            self.schedule_workflow_logs(&mut c);
            if !changed {
              return;
            }
            self.drain_terminal(&mut c);
          }
        }
        ExtensionUpdate::Lifecycle(l) => {
          let root = c.acp_session_id.clone();
          let turn_index = current_turn_index(&c);
          let Core { tree, state, .. } = &mut *c;
          tree.lifecycle(&session_id, root.as_deref(), l, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
          self.drain_terminal(&mut c);
        }
      }
      self.touch(&mut c);
      return;
    }
    if c.acp_session_id.as_deref().is_some_and(|a| a != session_id) {
      if c.tree.has_peer_session(&session_id) {
        // Replay content is dropped for a node restored from the record
        if !c.replaying || !c.tree.peer_restored(&session_id) {
          let turn_index = current_turn_index(&c);
          let node = c.tree.node_for_peer(&session_id);
          let Core { tree, state, .. } = &mut *c;
          if let Some(node) = node {
            tree.apply_child(&node, &u, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
          }
          self.drain_terminal(&mut c);
        }
        self.touch(&mut c);
      } else {
        c.tree.buffer_orphan(&session_id, u);
      }
      return;
    }
    // Codex brackets every turn with threadStatus: idle during a prompt means that turn is over (no steering into it), and
    // idle while a steer's detached turn runs ends that turn
    if kind == "session_info_update"
      && !c.replaying
      && let Some(idle) = steering::thread_idle(&u)
    {
      c.peer.status_seen = true;
      if !idle {
        c.peer.idle = false;
      } else if c.peer.detached {
        self.end_detached(&mut c, TurnStop::EndTurn);
      } else if c.phase.running {
        c.peer.idle = true;
      }
    }
    // Conversation content cannot belong to a session that does not exist yet (pi-acp's banner during session/new)
    if c.acp_session_id.is_none()
      && c.status == SessionStatus::Starting
      && ["agent_message_chunk", "agent_thought_chunk", "tool_call", "tool_call_update", "plan"].contains(&kind.as_str())
    {
      drop(c);
      self.log(&format!("startup {kind} ignored (no session yet)"));
      return;
    }
    // Hidden modes, or none at all: pi-acp echoes every thinking pick as a mode update that selects nothing here
    if kind == "current_mode_update" && (self.def().ignore_modes || c.state.controls.modes.is_empty()) {
      return;
    }
    // yolo is host-side state: a mode pushed by the CLI must not drag the UI back
    if c.perms.auto_approve && kind == "current_mode_update" {
      u["currentModeId"] = Value::from("yolo");
    }
    if c.replaying && RUNNING_KINDS.contains(&kind.as_str()) {
      return;
    }
    if c.replaying && kind == "session_info_update" && failure_of(u.get("_meta"), None).is_some() {
      return;
    }
    if c.phase.running
      && kind == "session_info_update"
      && let Some(f) = failure_of(u.get("_meta"), None).filter(|f| f.severity == Severity::Error)
    {
      c.turn_failure = Some(f);
    }
    if !c.replaying
      && let Some(comp) = c.compaction.completion.as_mut()
    {
      comp.update(&u);
    }
    // A user_message_chunk echoed mid-turn is the one just sent
    if c.phase.running && kind == "user_message_chunk" {
      return;
    }
    if kind == "session_info_update"
      && c.lineage.agent_title_muted
      && let Some(title) = u.get("title").and_then(Value::as_str).filter(|x| !x.is_empty())
    {
      let head: String = title.chars().take(60).collect();
      self.log(&format!("agent title ignored: {head}"));
      u["title"] = Value::Null;
    }
    if self.vendor.corrects_window()
      && kind == "usage_update"
      && let Some(size) = u.get("size").and_then(Value::as_f64)
    {
      c.usage.reported_window = Some(size);
      if let Some(w) = claude_window::correct(c.account_id.as_deref(), &c.state.controls, size, &crate::model_catalog::current()) {
        u["size"] = w.into();
      }
    }
    if kind == "agent_message_chunk"
      && let Some(banner) = c.startup_banner.clone()
      && u.get("content").and_then(|x| x.get("type")).and_then(Value::as_str) == Some("text")
      && u["content"].get("text").and_then(Value::as_str) == Some(banner.as_str())
    {
      self.log("startup banner ignored");
      c.startup_banner = None;
      return;
    }
    let turn_index = current_turn_index(&c);
    let routed = {
      let Core { tree, state, .. } = &mut *c;
      tree.route_root(&u, &mut RouteCtx { turn_index, root_turns: &mut state.turns })
    };
    self.drain_terminal(&mut c);
    if matches!(routed, Route::Consumed) {
      self.touch(&mut c);
      return;
    }
    // Claude's task-notification followups stream with no prompt on the wire: show them running only after routing
    // child updates. A nested tool from a detached child can look like ordinary content, but it must not reopen the root.
    self.open_autonomous(&mut c, &kind);
    if !apply_update(&mut c.state, &u) {
      return;
    }
    if kind == "usage_update" {
      c.usage.notifications = true;
      c.usage.revision += 1;
      if let Some(f) = c.usage.finish_refresh.take() {
        let _ = f.send(false);
      }
      c.usage.clear_timer();
      // The autonomous cycle's result: its snapshot is stamped on the turn above, now it settles
      if c.peer.autonomous && claude_autonomous::cycle_ended(&u) {
        self.end_detached(&mut c, TurnStop::EndTurn);
        self.touch(&mut c);
        return;
      }
    } else if c.phase.running {
      self.schedule_usage_poll(&mut c);
    }
    if matches!(kind.as_str(), "tool_call" | "tool_call_update") {
      {
        let Core { tree, state, .. } = &mut *c;
        tree.annotate_root(&u, &mut RouteCtx { turn_index, root_turns: &mut state.turns });
      }
      self.relay_annotate(&mut c, &u, turn_index);
      self.drain_terminal(&mut c);
      c.questions.raw.remember(&u);
      let plan = capture_plan(&mut c.state.turns, &u);
      // Kimi 0.41.0 confirms the plan exit in tool output but omits current_mode_update
      let tool_call_id = u.get("toolCallId").and_then(Value::as_str).unwrap_or("");
      if self.vendor.plan_exit_in_tool_output()
        && let Some(pid) = plan
        && plan_documents(&c.state.turns).iter().any(|p| p.id == pid && p.approval_tool_call_id.as_deref() == Some(tool_call_id))
        && u.get("status").and_then(Value::as_str) == Some("completed")
        && u.get("rawOutput").and_then(Value::as_str).is_some_and(|o| o.starts_with("Exited plan mode. Plan mode deactivated."))
      {
        c.state.controls.mode_id = Some("default".into());
      }
    }
    if c.phase.running {
      let activity = activity_of(&c.state.turns);
      if let Some(Turn::Agent(last)) = c.state.turns.last_mut() {
        last.activity = activity;
      }
    }
    self.touch(&mut c);
    // Kimi reports usage after the prompt response: re-evaluate only the live, successfully completed user turn
    let reevaluate =
      !c.replaying && !c.phase.running && c.compaction.auto_eligible && matches!(kind.as_str(), "usage_update" | "available_commands_update");
    drop(c);
    if reevaluate {
      self.after_prompt(false, TurnStop::EndTurn);
    }
  }

  /// An AIR async task lands on the transcript of the session whose stream carried it
  fn async_task_update(&self, c: &mut Core, peer: &str, e: &crate::acp::transport::wire::AsyncTaskEvent) {
    if c.replaying {
      return;
    }
    let root = c.acp_session_id.as_deref() == Some(peer);
    c.task_peer.insert(e.async_task_id.clone(), peer.to_owned());
    if root {
      apply_async_task(&mut c.state, e);
      // A workflow's agents end with its run (`SubagentTree::workflow_ended`); only workflow agents carry the task id
      if let Some(st) = e.state.filter(|s| matches!(s, AsyncTaskState::Completed | AsyncTaskState::Failed | AsyncTaskState::Stopped)) {
        c.tree.workflow_ended(&e.async_task_id, st);
        self.drain_terminal(c);
        // The agents it ended still get their last lines read
        self.schedule_workflow_logs(c);
      }
      return;
    }
    match c.tree.task_state(peer) {
      Some((state, node)) => {
        if apply_async_task(state, e) {
          c.tree.bump(&node);
        }
      }
      None => {
        c.task_peer.remove(&e.async_task_id);
        self.log(&format!("async task {} on unknown session {peer} dropped", e.async_task_id));
      }
    }
  }
}

/// The root agent turn a new subagent anchors to: the live one, or the index the next update is about to open
pub(crate) fn current_turn_index(c: &Core) -> usize {
  match c.state.turns.last() {
    Some(Turn::Agent(_)) => c.state.turns.len() - 1,
    _ => c.state.turns.len(),
  }
}
