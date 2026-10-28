//! Claude dynamic workflows: one receipt node per workflow agent from the run's progress frames, its end, and the
//! updates read from each agent's sidechain log

use std::collections::HashSet;

use serde_json::Value;

use acpira_shared::subagents::{StateSource, SubagentPeer, SubagentState, SubagentVisibility};
use acpira_shared::transcript::{AsyncTaskState, TurnStop};

use super::{Dialect, LogTarget, RESUMED_LOG_SLACK_MS, RouteCtx, SubagentTree, has_content, seal_node, terminal, tool_count};
use crate::acp::transcript::normalize::{apply_update, end_turn};
use crate::acp::transport::wire::WorkflowProgress;
use crate::acp::vendors::claude_workflow_log::LogWindow;
use crate::util::now_ms;

impl SubagentTree {
  /// A Claude dynamic workflow's progress (`claude_workflow.rs`): one receipt node per agent, keyed by run and position
  /// (`<task>#<index>` in `peer.agentId`, since a queued agent has no agentId yet). The nodes hang off no delegation row:
  /// the Workflow tool row stays the background task with its own state and stop button, so the webview lists the agents
  /// after the announcing turn's content. The phase reads as the role, the prompt preview as the task and the result
  /// preview as the result. True when anything changed
  pub fn workflow_progress(&mut self, w: WorkflowProgress, ctx: &mut RouteCtx) -> bool {
    let mut changed = false;
    for a in w.agents {
      let key = format!("{}#{}", w.async_task_id, a.index);
      let i = match self.by(&self.by_agent, &key) {
        Some(i) => i,
        None => {
          let peer = SubagentPeer { agent_id: Some(key.clone()), ..Default::default() };
          let mut node = self.new_node(SubagentVisibility::Receipt, peer, ctx.turn_index, false, Some(Dialect::Workflow));
          node.background = true;
          node.workflow = Some(w.async_task_id.clone());
          self.by_agent.insert(key, node.id.clone());
          self.nodes.push(node);
          changed = true;
          self.nodes.len() - 1
        }
      };
      let n = &mut self.nodes[i];
      // The agent's own id names its sidechain log (`claude_workflow_log.rs`); a queued agent gets it once it starts
      if n.peer.sidechain_id.is_none()
        && let Some(agent_id) = a.agent_id
      {
        n.peer.sidechain_id = Some(agent_id);
        n.rev += 1;
        changed = true;
      }
      let before = (n.title.clone(), n.task.clone(), n.role.clone(), n.model.clone(), n.result.clone(), n.tool_count);
      n.title = a.label.or(n.title.take());
      n.task = a.prompt.or(n.task.take());
      n.role = a.phase.or(n.role.take());
      n.model = a.model.or(n.model.take());
      n.result = a.result.or(n.result.take());
      n.tool_count = a.tool_calls.unwrap_or(n.tool_count);
      if before != (n.title.clone(), n.task.clone(), n.role.clone(), n.model.clone(), n.result.clone(), n.tool_count) {
        n.rev += 1;
        changed = true;
      }
      if terminal(a.state) && n.status != a.state {
        let id = n.id.clone();
        self.transition(&id, a.state);
        changed = true;
      }
    }
    changed
  }

  /// The workflow run's async task reached a terminal state: agents its last progress frame still showed running end
  /// with it. `stopped` is held as a local verdict, because claude-agent-acp sends a best-effort `stopped` and corrects it
  /// to `completed` / `failed` in the same millisecond, and a local terminal state yields to the agent's
  pub fn workflow_ended(&mut self, task_id: &str, state: AsyncTaskState) {
    let open: Vec<String> = self
      .nodes
      .iter()
      .filter(|n| n.workflow.as_deref() == Some(task_id) && (n.status == SubagentState::Running || n.state_source == StateSource::Local))
      .map(|n| n.id.clone())
      .collect();
    for id in open {
      match state {
        AsyncTaskState::Completed => self.transition(&id, SubagentState::Completed),
        AsyncTaskState::Failed => self.transition(&id, SubagentState::Failed),
        AsyncTaskState::Stopped => {
          let Some(i) = self.idx(&id) else { continue };
          let n = &mut self.nodes[i];
          if n.status != SubagentState::Running {
            continue;
          }
          n.status = SubagentState::Cancelled;
          n.state_source = StateSource::Local;
          seal_node(n, SubagentState::Cancelled);
          n.rev += 1;
          self.terminal_events.push(id);
        }
        AsyncTaskState::Running | AsyncTaskState::Paused => {}
      }
    }
  }

  /// Workflow agents whose sidechain log is due a read (`session/workflow_logs.rs`): running ones, a live one that just
  /// ended (its last lines), and a restored one a viewer asked for (`wanted`) whose record holds no process. `done` are
  /// the nodes whose final read already ran. (node id, the agent's own id, running)
  pub fn workflow_log_targets(&self, wanted: &HashSet<String>, done: &HashSet<String>) -> Vec<LogTarget> {
    self
      .nodes
      .iter()
      .filter(|n| !done.contains(&n.id))
      .filter_map(|n| {
        let agent_id = n.peer.sidechain_id.clone()?;
        let running = n.status == SubagentState::Running;
        let due = if n.restored {
          !running && wanted.contains(&n.id) && !has_content(&n.state.turns)
        } else {
          n.dialect == Some(Dialect::Workflow) || n.resumed
        };
        // A resumed run shares its log with every other run of the agent: only the lines between its announce and its end
        let window = n.resumed.then(|| LogWindow {
          from: n.announced_at - RESUMED_LOG_SLACK_MS,
          to: if running { None } else { Some(n.ended_at.unwrap_or_else(now_ms) + RESUMED_LOG_SLACK_MS) },
        });
        due.then(|| LogTarget { node: n.id.clone(), agent_id, running, window })
      })
      .collect()
  }

  /// Updates read from a workflow agent's sidechain log. They land past the terminal gate (the last lines are read once
  /// the agent ended), and a terminal node's turn is sealed again so nothing is left streaming or open. True when the
  /// transcript changed
  pub fn workflow_log(&mut self, id: &str, updates: &[Value]) -> bool {
    let Some(i) = self.idx(id) else { return false };
    let n = &mut self.nodes[i];
    // A native child whose run turned out to stream on the wire while this read was under way
    if n.dialect != Some(Dialect::Workflow) && !n.resumed && !n.restored {
      return false;
    }
    let mut changed = false;
    for u in updates {
      changed |= apply_update(&mut n.state, u);
    }
    if !changed {
      return false;
    }
    if terminal(n.status) {
      let stop = if matches!(n.status, SubagentState::Cancelled | SubagentState::Disconnected) { TurnStop::Cancelled } else { TurnStop::EndTurn };
      end_turn(&mut n.state, stop);
    }
    n.log_fed |= n.resumed;
    // The progress frame counts the agent's tool calls too; the log may be behind it or ahead
    n.tool_count = n.tool_count.max(tool_count(&n.state.turns));
    n.rev += 1;
    true
  }
}
