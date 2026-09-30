//! The automatic account switch when a turn runs out of quota: reserve the session, move to the fallback account and
//! continue the task there, or put the error back when the hand-off fails

use std::sync::Arc;

use acpira_shared::transcript::*;

use crate::acp::session::errors::is_quota_exhausted;
use crate::acp::session::prompt::{Origin, agent_turn_mut};
use crate::acp::session::{AcpSession, Core};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::{t, tp};
use crate::util::random_uuid;

/// What the exhausted turn looked like before the switch rewrote it, so a failed switch can put the error back
struct ExhaustedTurn {
  stop: Option<TurnStop>,
  error: Option<TurnError>,
  notice: Option<(usize, AgentBlock)>,
  switch_notice: String,
}

impl AcpSession {
  /// The hidden follow-up after an automatic account switch, in the host language: the native session came back on the
  /// new account, so the agent picks the task up from its own context
  fn continue_after_switch(self: &Arc<Self>) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move { me.prompt_inner(t("host.autoContinuePrompt"), vec![], Origin::Continue, None, None).await })
  }

  /// A turn that ran out of account quota reserves the session for the automatic switch (under the same lock that settles
  /// it, so no queued or new prompt can slip onto the exhausted account); only a session bound to a saved account qualifies
  pub(crate) fn reserve_switch(&self, c: &mut Core, error: &TurnError) -> bool {
    let yes = self.deps.accounts.is_some() && c.account_id.is_some() && is_quota_exhausted(error);
    if yes {
      c.switching = true;
    }
    yes
  }

  /// Move to the account the strategy picks and continue the task there: the exhausted turn keeps its output and gets a
  /// notice row instead of the error card, the native session is resumed on the new credential, and a hidden continue
  /// prompt picks the work up. With no account to move to (or a failed hand-off) the error stays where it was
  pub(crate) async fn switch_after_exhaustion(self: Arc<Self>, agent_idx: usize, started_at: i64) {
    let Some(hooks) = self.deps.accounts.clone() else { return self.release_switch() };
    let current = self.core.lock().account_id.clone();
    let Some((next, to)) = hooks.fallback(self.agent.clone(), current.clone()).await else {
      self.log("quota exhausted: no other account to switch to");
      return self.release_switch();
    };
    let from = current.as_ref().and_then(|id| hooks.label(id.clone())).or(current).unwrap_or_default();
    self.log(&format!("quota exhausted: switching {from} → {to}"));
    let saved = {
      let mut c = self.core.lock();
      let saved = agent_turn_mut(&mut c, agent_idx, started_at).map(|turn| {
        let failure_id = turn.error.as_ref().and_then(|e| e.failure_id.clone());
        let notice = failure_id
          .and_then(|id| turn.blocks.iter().position(|b| matches!(b, AgentBlock::Notice(n) if n.id == id)))
          .map(|i| (i, turn.blocks.remove(i)));
        let switch_notice = format!("account-switch:{}", random_uuid());
        // One line in the flow: the agent's quota text stays in the log and comes back with the error if the hand-off fails
        turn.blocks.push(AgentBlock::Notice(NoticeBlock {
          id: switch_notice.clone(),
          revision: acpira_shared::num::Num(1.0),
          category: FailureCategory::Limit,
          severity: Severity::Warning,
          title: tp("host.accountSwitched", &[("from", &from), ("to", &to)]),
          details: None,
          actions: vec![],
        }));
        let saved = ExhaustedTurn { stop: turn.stop, error: turn.error.take(), notice, switch_notice };
        turn.stop = Some(TurnStop::EndTurn);
        saved
      });
      self.touch(&mut c);
      saved
    };
    self.rebind_reserved(&next).await;
    if self.status() == SessionStatus::Ready {
      return self.continue_after_switch().await;
    }
    // The new account did not come up: the session's own error / login state explains it, the turn gets its error back
    if let Some(saved) = saved {
      let mut c = self.core.lock();
      let turn = c.state.turns.iter_mut().rev().filter_map(Turn::as_agent_mut).find(|t| {
        t.blocks.iter().any(|b| matches!(b, AgentBlock::Notice(n) if n.id == saved.switch_notice))
      });
      if let Some(turn) = turn {
        turn.blocks.retain(|b| !matches!(b, AgentBlock::Notice(n) if n.id == saved.switch_notice));
        if let Some((i, block)) = saved.notice {
          turn.blocks.insert(i.min(turn.blocks.len()), block);
        }
        turn.stop = saved.stop;
        turn.error = saved.error;
      }
    }
    self.release_switch();
  }

  fn release_switch(self: &Arc<Self>) {
    {
      let mut c = self.core.lock();
      c.switching = false;
      self.touch(&mut c);
    }
    self.after_prompt(false, TurnStop::Cancelled);
  }
}
