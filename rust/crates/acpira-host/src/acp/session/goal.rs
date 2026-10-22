//! Goal controls from the dock (`vendors::goal`): what the agent advertises decides what may be sent, and whether the
//! action would start a turn of the agent's own decides how. An agent that leaves continuing its goal to the client
//! (the built-in one) also gets the goal's next turns from here (`goal_wake`)

use std::sync::Arc;

use anyhow::Result;

use acpira_shared::transcript::{GoalAction, GoalStatus};

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
  /// the requested command into it. An agent that leaves continuing to the client starts no turn of its own on any
  /// action, so its clear is a request as well
  pub async fn control_goal(self: &Arc<Self>, action: GoalAction, objective: Option<String>) -> Result<()> {
    // Decided under the lock, sent after it: the guard must not live across an await
    let route = {
      let mut c = self.core.lock();
      let steer_clear = action == GoalAction::Clear && c.phase.running;
      let clear_starts_turn = self.vendor.goal_clear_starts_turn() && c.proc.as_ref().is_none_or(|p| goal::continue_prompt(&p.init).is_none());
      if !Self::goal_actions_of(&c).is_some_and(|a| a.contains(&action)) {
        None
      } else if goal::starts_turn(action, clear_starts_turn) && !steer_clear {
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

  /// An active goal's next turn, for an agent that leaves continuing to the client (`goal::continue_prompt`): its
  /// continue prompt goes out hidden (`AutoReason::GoalContinue`) once the turn before ended with the goal still active
  /// (`Core::goal_due`) and nothing of the user's waits (`flush_queue`). The agent's rules end the run (met, blocked,
  /// budget, its own pause); a stop, a failure or the dock's pause ends it here. true when it went out
  pub(crate) fn goal_wake(self: &Arc<Self>) -> bool {
    let text = {
      let mut c = self.core.lock();
      if !std::mem::take(&mut c.goal_due) || !c.state.goal.as_ref().is_some_and(|g| g.status == GoalStatus::Active) {
        return false;
      }
      let Some(text) = c.proc.as_ref().and_then(|p| goal::continue_prompt(&p.init)) else { return false };
      text
    };
    self.log("goal: continuing");
    let me = self.clone();
    crate::util::run_prefix(Box::pin(async move { me.prompt_inner(text, vec![], Origin::GoalContinue, None, None).await }));
    true
  }
}
