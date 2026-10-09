//! Goal controls from the dock (`vendors::goal`): what the agent advertises decides what may be sent, and whether the
//! action would start a turn of the agent's own decides how

use std::sync::Arc;

use anyhow::Result;

use acpira_shared::transcript::GoalAction;

use crate::acp::session::AcpSession;
use crate::acp::transport::process::AgentProcess;
use crate::acp::vendors::goal;

enum Route {
  Prompt,
  Request(Arc<AgentProcess>, String),
}

impl AcpSession {
  /// Turn-starting actions go out as a `/goal …` prompt, which queues behind a running turn like any prompt; the rest
  /// (Codex's pause and clear) go over `_session/goal`, and the agent's snapshot that follows updates the view
  pub async fn control_goal(self: &Arc<Self>, action: GoalAction, objective: Option<String>) -> Result<()> {
    // Decided under the lock, sent after it: the guard must not live across an await
    let route = {
      let c = self.core.lock();
      if !Self::goal_actions_of(&c).is_some_and(|a| a.contains(&action)) {
        None
      } else if goal::starts_turn(action, self.vendor.goal_clear_starts_turn()) {
        Some(Route::Prompt)
      } else {
        match (c.proc.clone().filter(|p| p.alive()), c.acp_session_id.clone()) {
          (Some(proc), Some(peer)) => Some(Route::Request(proc, peer)),
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
          self.prompt(text, vec![], false, None, None).await;
        }
        return Ok(());
      }
      Some(Route::Request(proc, peer)) => (proc, peer),
    };
    proc.request(goal::METHOD, goal::params(&peer, action)).await?;
    Ok(())
  }
}
