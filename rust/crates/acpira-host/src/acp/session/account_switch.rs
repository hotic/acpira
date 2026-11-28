//! The automatic account switch when a turn runs out of quota: reserve the session, move to the fallback account and
//! continue the task there, or put the error back when the hand-off fails. A turn that failed on a sign-in its process
//! holds expired reconnects on the same account the same way, and a session whose saved account was removed moves to
//! another before its process starts

use std::sync::Arc;

use acpira_shared::transcript::*;

use crate::acp::session::errors::is_quota_exhausted;
use crate::acp::session::prompt::{Origin, agent_turn_mut};
use crate::acp::session::{AcpSession, Core};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::{t, tp};
use crate::util::{now_ms, random_uuid};

/// The least time between two reconnects on an expired sign-in: one sooner means the stored credential is stale as well
const REAUTH_GAP_MS: i64 = 5 * 60_000;

/// The notice row a hand-off leaves on the failed turn, and the continue prompt that follows it
struct Handoff {
  title: String,
  category: FailureCategory,
  info: bool,
  prompt_key: &'static str,
}

/// What the failed turn looked like before the hand-off rewrote it, so a failed hand-off can put the error back
struct ExhaustedTurn {
  stop: Option<TurnStop>,
  error: Option<TurnError>,
  notice: Option<(usize, AgentBlock)>,
  switch_notice: String,
}

impl AcpSession {
  /// The hidden follow-up after an automatic hand-off, in the host language: the native session came back on a fresh
  /// process, so the agent picks the task up from its own context
  fn continue_after_switch(self: &Arc<Self>, prompt_key: &'static str) -> BoxFuture<()> {
    let me = self.clone();
    Box::pin(async move { me.prompt_inner(t(prompt_key), vec![], Origin::Continue, None, None).await })
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

  /// A turn that failed on a sign-in its process holds expired reserves the session to reconnect on the stored
  /// credential, which may have been renewed since (a plugin's account may rewrite its token file). Only a session bound to an
  /// account qualifies, and not again within `REAUTH_GAP_MS`, so a credential expired in the store too shows its error.
  /// A `/compact` is no task to continue: it keeps its error and the next prompt (a retried `/compact` too) reconnects first
  pub(crate) fn reserve_reauth(&self, c: &mut Core, error: &TurnError, compacting: bool) {
    let now = now_ms();
    if self.deps.accounts.is_none() || c.account_id.is_none() || c.switching || error.kind.as_deref() != Some("auth_expired") {
      return;
    }
    if compacting {
      self.drop_stale_process(c);
      return;
    }
    if c.reauth_at.is_some_and(|at| now - at < REAUTH_GAP_MS) {
      return;
    }
    self.drop_warm();
    c.switching = true;
    c.reauth_due = true;
    c.reauth_at = Some(now);
  }

  fn drop_stale_process(&self, c: &mut Core) {
    c.adapter_stale = true;
    c.credential_stale = true;
    self.drop_warm();
  }

  /// Warm processes of the agent read their token when they started, possibly before the store's was renewed
  fn drop_warm(&self) {
    if let Some(pool) = &self.deps.pool {
      pool.invalidate(Some(&self.agent));
    }
  }

  /// Reconnect on the bound account's stored credential and continue the task (`hand_off`)
  pub(crate) async fn reauth_after_expiry(self: Arc<Self>, agent_idx: usize, started_at: i64) {
    let Some(account) = self.core.lock().account_id.clone() else { return self.release_switch() };
    self.log("sign-in expired in the running process: reconnecting on the stored credential");
    let notice = Handoff { title: t("host.signInRenewed"), category: FailureCategory::Access, info: true, prompt_key: "host.reauthContinuePrompt" };
    self.hand_off(agent_idx, started_at, &account, notice).await
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
    let title = tp("host.accountSwitched", &[("from", &from), ("to", &to)]);
    let notice = Handoff { title, category: FailureCategory::Limit, info: false, prompt_key: "host.autoContinuePrompt" };
    self.hand_off(agent_idx, started_at, &next, notice).await
  }

  /// Bind `account` on a fresh process and continue the task there: the failed turn keeps its output and gets a notice
  /// row instead of the error card, the native session is resumed on the account's credential, and a hidden continue
  /// prompt picks the work up. With the new process not coming up the error goes back where it was
  async fn hand_off(self: Arc<Self>, agent_idx: usize, started_at: i64, account: &str, h: Handoff) {
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
          category: h.category,
          severity: Severity::Warning,
          info: h.info,
          title: h.title,
          details: None,
          actions: vec![],
          retry: None,
        }));
        let saved = ExhaustedTurn { stop: turn.stop, error: turn.error.take(), notice, switch_notice };
        turn.stop = Some(TurnStop::EndTurn);
        saved
      });
      self.touch(&mut c);
      saved
    };
    self.rebind_reserved(account).await;
    if self.status() == SessionStatus::Ready {
      return self.continue_after_switch(h.prompt_key).await;
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

  /// The account to start on: the bound one, or, when that account was removed (saved or plugin) or the session has
  /// none while the agent has saved accounts, the replacement / default account, which becomes the session's binding.
  /// An old session opened after its account was deleted carries on with a remaining login instead of stopping on sign-in
  pub(crate) async fn replace_removed_account(&self, account: Option<String>) -> Option<String> {
    let Some(hooks) = self.deps.accounts.clone() else { return account };
    let (next, label) = match &account {
      Some(gone) => match hooks.replacement(self.agent.clone(), gone.clone()).await {
        Some(pair) => pair,
        None => return account,
      },
      None => match hooks.default_account(self.agent.clone()) {
        Some(next) => {
          let label = hooks.label(next.clone()).unwrap_or_else(|| next.chars().take(8).collect());
          (next, label)
        }
        None => return None,
      },
    };
    let mut c = self.core.lock();
    // A pick made while the replacement was looked up wins: its own reopen starts on it
    if c.account_id != account {
      return c.account_id.clone();
    }
    match &account {
      Some(gone) => {
        let short: String = if gone.starts_with(crate::accounts::plugin_accounts::ID_PREFIX) {
          gone.clone()
        } else {
          gone.chars().take(8).collect()
        };
        self.log(&format!("account {short} was removed: moving to {label}"));
      }
      None => self.log(&format!("unbound session: moving to {label}")),
    }
    c.account_id = Some(next.clone());
    self.touch(&mut c);
    Some(next)
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
