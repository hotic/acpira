//! Goal controls from the dock (`vendors::goal`): what the agent advertises decides what may be sent, and whether the
//! action would start a turn of the agent's own decides how. An agent that leaves continuing its goal to the client
//! (the built-in one) also gets the goal's next turns from here (`goal_wake`)

use std::sync::Arc;

use anyhow::Result;

use serde_json::json;
use tokio::sync::oneshot;

use acpira_shared::transcript::{GoalAction, GoalStatus};

use crate::acp::session::prompt::Origin;
use crate::acp::session::queue::{PromptAnswer, PromptSteer, PromptSteerEnd};
use crate::acp::session::{AcpSession, Core};
use crate::acp::transport::process::AgentProcess;
use crate::acp::vendors::{antigravity, goal};

/// Internal steer id for an Antigravity goal clear: absent from `queue.entries`, so `land_prompt_steer` hands the turn
/// over to the cancel prompt without inserting a visible `SteerBlock`
const GOAL_CLEAR_STEER_ID: &str = "__acpira_goal_clear__";

enum Route {
  Prompt(String),
  Request(Arc<AgentProcess>, String),
  SteerClear(Arc<AgentProcess>, String, oneshot::Sender<PromptAnswer>, oneshot::Receiver<PromptSteerEnd>),
  Deferred,
}

impl AcpSession {
  /// Turn-starting actions go out as a hidden `/goal …` prompt (`AutoReason::Goal`: no bubble, the goal row stands for
  /// it), which queues behind a running turn like any prompt; the rest (Codex's pause and clear) go over
  /// `_session/goal`, and the agent's snapshot that follows updates the view. Claude's clear during a turn is a request
  /// too: its goal holds that turn until met, so a queued clear would never get its turn, while claude-agent-acp steers
  /// the requested command into it. An agent that leaves continuing to the client starts no turn of its own on any
  /// action, so its clear is a request as well. Antigravity has neither `_session/goal` nor `/goal clear`, so its clear
  /// steers or sends `antigravity::GOAL_CANCEL_PROMPT` so its Stop hook sees `<!-- GOAL_CANCELLED -->`
  pub async fn control_goal(self: &Arc<Self>, action: GoalAction, objective: Option<String>) -> Result<()> {
    // Decided under the lock, sent after it: the guard must not live across an await
    let route = {
      let mut c = self.core.lock();
      let synth = self.synthesizes_goal(&c);
      let steer_clear = action == GoalAction::Clear && c.phase.running;
      let clear_starts_turn = self.vendor.goal_clear_starts_turn() && c.proc.as_ref().is_none_or(|p| goal::continue_prompt(&p.init).is_none());
      if !self.goal_actions_of(&c).is_some_and(|a| a.contains(&action)) {
        None
      } else if synth && action == GoalAction::Clear {
        if c.state.goal.is_none() {
          return Ok(());
        }
        if c.phase.running {
          let can_steer = !c.phase.staging
            && c.pending_prompt.is_none()
            && !c.switching
            && !c.peer.idle
            && !c.peer.detached
            && !self.card_holds_steer(&c)
            && c.queue.prompt_steer.is_none()
            && c.proc.as_ref().is_some_and(|p| self.vendor.steers_by_prompt(&p.init));
          match (can_steer, c.proc.clone().filter(|p| p.alive()), c.acp_session_id.clone()) {
            (true, Some(proc), Some(peer)) => {
              let (answer_tx, answer) = oneshot::channel();
              let (end, end_rx) = oneshot::channel();
              c.queue.prompt_steer = Some(PromptSteer { id: GOAL_CLEAR_STEER_ID.into(), answer, end });
              c.goal_clearing = true;
              Some(Route::SteerClear(proc, peer, answer_tx, end_rx))
            }
            _ => Some(Route::Deferred),
          }
        } else {
          c.goal_clearing = true;
          Some(Route::Prompt(antigravity::GOAL_CANCEL_PROMPT.into()))
        }
      } else if goal::starts_turn(action, clear_starts_turn) && !steer_clear {
        goal::command_text(action, objective.as_deref()).map(Route::Prompt)
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
      Some(Route::Deferred) => {
        self.log("goal Clear: deferred while turn cannot steer");
        return Ok(());
      }
      Some(Route::Prompt(text)) => {
        self.clone().prompt_inner(text, vec![], Origin::Goal, None, None).await;
        return Ok(());
      }
      Some(Route::SteerClear(proc, peer, answer, end)) => {
        self.steer_goal_clear(proc, peer, answer, end).await;
        return Ok(());
      }
      Some(Route::Request(proc, peer)) => (proc, peer),
    };
    proc.request(goal::METHOD, goal::params(&peer, action)).await?;
    Ok(())
  }

  /// Steers `antigravity::GOAL_CANCEL_PROMPT` into a running Antigravity turn without adding a visible steer row
  async fn steer_goal_clear(
    self: &Arc<Self>,
    proc: Arc<AgentProcess>,
    peer: String,
    answer: oneshot::Sender<PromptAnswer>,
    end: oneshot::Receiver<PromptSteerEnd>,
  ) {
    let blocks = vec![json!({ "type": "text", "text": antigravity::GOAL_CANCEL_PROMPT })];
    let params = self.prompt_params(Some(&peer), blocks);
    let me = self.clone();
    tokio::spawn(async move {
      let r = proc.request_ordered("session/prompt", params).await;
      let outcome = match &r {
        Err(e) if antigravity::steer_bounced(&e.message) => PromptSteerEnd::Held,
        _ => PromptSteerEnd::Refused,
      };
      if let Err(e) = &r
        && me.refuse_prompt_steer(GOAL_CLEAR_STEER_ID, outcome)
      {
        me.core.lock().goal_clearing = false;
        me.log(&format!("goal clear steer {}: {e}", if outcome == PromptSteerEnd::Held { "bounced" } else { "failed" }));
        return;
      }
      let _ = answer.send(r);
    });
    match end.await {
      Ok(PromptSteerEnd::Landed) => self.log("goal clear: steered as a prompt"),
      Ok(PromptSteerEnd::Missed) => {
        // The running turn finished just before the steer reached it; fall back to an idle cancel turn if still active
        let send_idle = {
          let mut c = self.core.lock();
          let active = !c.phase.running && c.state.goal.as_ref().is_some_and(|g| g.status == GoalStatus::Active);
          c.goal_clearing = active;
          active
        };
        if send_idle {
          self.clone().prompt_inner(antigravity::GOAL_CANCEL_PROMPT.into(), vec![], Origin::Goal, None, None).await;
        }
      }
      Ok(PromptSteerEnd::Refused | PromptSteerEnd::Held) | Err(_) => {
        self.core.lock().goal_clearing = false;
      }
    }
  }

  /// The workspace gate gave up (`hooks.rs`): the goal's run ends there instead of continuing past the gate's round
  /// limit. true when the goal is the engine's to continue and still active, so the caller pauses it at the agent
  /// (`_session/goal`) and the dock shows it waiting for the user
  pub(crate) fn end_goal_run(c: &mut Core) -> bool {
    c.goal_due = false;
    c.state.goal.as_ref().is_some_and(|g| g.status == GoalStatus::Active) && c.proc.as_ref().is_some_and(|p| goal::continue_prompt(&p.init).is_some())
  }

  /// An active goal's next turn, for an agent that leaves continuing to the client (`goal::continue_prompt`): its
  /// continue prompt goes out hidden (`UserTurn::goal_continue`) once the turn before ended with the goal still active
  /// (`Core::goal_due`) and nothing of the user's waits (`flush_queue`). The agent's rules end the run (met, blocked,
  /// budget, its own pause); a stop, a failure, the workspace gate giving up or the dock's pause ends it here. true when
  /// it went out
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
