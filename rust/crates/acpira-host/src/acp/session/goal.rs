//! Goal controls from the dock (`vendors::goal`): what the agent advertises decides what may be sent, and whether the
//! action would start a turn of the agent's own decides how

use std::sync::Arc;

use anyhow::Result;

use acpira_shared::transcript::GoalAction;

use crate::acp::session::AcpSession;
use crate::acp::session::prompt::Origin;
use crate::acp::transport::process::AgentProcess;
use crate::acp::vendors::goal;

enum Route {
  Prompt,
  Request(Arc<AgentProcess>, String),
}

impl AcpSession {
  /// Turn-starting actions go out as a hidden `/goal …` prompt (`AutoReason::Goal`: no bubble, the goal row stands for
  /// it), which queues behind a running turn like any prompt; the rest (Codex's pause and clear) go over
  /// `_session/goal`, and the agent's snapshot that follows updates the view. Claude's clear during a turn is a request
  /// too: its goal holds that turn until met, so a queued clear would never get its turn, while claude-agent-acp steers
  /// the requested command into it
  pub async fn control_goal(self: &Arc<Self>, action: GoalAction, objective: Option<String>) -> Result<()> {
    // Decided under the lock, sent after it: the guard must not live across an await
    let route = {
      let mut c = self.core.lock();
      let steer_clear = action == GoalAction::Clear && c.phase.running;
      if !Self::goal_actions_of(&c).is_some_and(|a| a.contains(&action)) {
        None
      } else if goal::starts_turn(action, self.vendor.goal_clear_starts_turn()) && !steer_clear {
        Some(Route::Prompt)
      } else {
        match (c.proc.clone().filter(|p| p.alive()), c.acp_session_id.clone()) {
          (Some(proc), Some(peer)) => {
            if self.vendor.echoes_goal_command() {
              c.goal_steered = goal::command_text(action, objective.as_deref());
            }
            Some(Route::Request(proc, peer))
          }
          _ => return Ok(()),
        }
      }
    };
    let (proc, peer) = match route {
      None => {
        self.log(&format!("goal {action:?}: not advertised"));
        return Ok(());
      }
      Some(Route::Prompt) => {
        if let Some(text) = goal::command_text(action, objective.as_deref()) {
          self.clone().prompt_inner(text, vec![], Origin::Goal, None, None).await;
        }
        return Ok(());
      }
      Some(Route::Request(proc, peer)) => (proc, peer),
    };
    proc.request(goal::METHOD, goal::params(&peer, action)).await?;
    Ok(())
  }
}
