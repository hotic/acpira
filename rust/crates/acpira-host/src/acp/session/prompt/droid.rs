//! Observe the retries the Droid ACP adapter drops without changing the ownership of the live prompt.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use acpira_shared::transcript::{AgentBlock, SessionStatus, TurnError, TurnRetry, TurnStop};
use serde_json::json;

use super::{AcpSession, AgentProcess, OpenTurn, Settled, agent_turn_mut};
use crate::acp::session::Core;
use crate::acp::transcript::normalize::{RetryReport, RetryText, report_retry};
use crate::acp::vendors::droid_core::{self, Swap};
use crate::acp::vendors::droid_retry::Cursor;
use crate::i18n::t;

pub(super) fn cut_error() -> TurnError {
  TurnError { message: t("host.droidGeminiCut"), kind: Some("droid_gemini_truncated".into()), retryable: Some(false), ..Default::default() }
}

impl AcpSession {
  /// An adapter ignoring the targeted cancel cannot safely take another prompt. Fence it before closing; queued work
  /// stays parked for an explicit reconnect, which resumes the native session rather than replaying the failed task.
  pub(super) async fn droid_cancel_unanswered(&self, turn: &OpenTurn) {
    let (closing, generation) = {
      let mut c = self.core.lock();
      if !turn.live(&c) || !c.phase.running { return; }
      self.log("Droid did not answer session/cancel after repeated Gemini stream failures; closing the adapter");
      let closing = self.drop_process(&mut c);
      (closing, c.proc_gen)
    };
    // Keep the turn claimed until the old CLI released the native session. Even an immediate Reconnect is then safe.
    if let Some(closing) = closing { closing.await; }
    let mut c = self.core.lock();
    if c.proc_gen != generation || !c.phase.running || c.status == SessionStatus::Closed { return; }
    let error = cut_error();
    c.error = Some(error.message.clone());
    c.status = SessionStatus::Error;
    self.settle(&mut c, TurnStop::Error, Some(error));
    self.touch(&mut c);
  }

  /// The caller still awaits the peer's answer: a success crossing the cancel remains a success and the queue
  /// cannot send the next prompt over a still-running one.
  pub(super) async fn watch_droid_cuts(&self, turn: &OpenTurn, proc: &AgentProcess, home: &Path, sid: &str, model: &str) -> bool {
    let epoch = self.core.lock().perms.epoch;
    let path = home.join("logs/droid-log-single.log");
    let since = turn.started_at;
    let mut cursor = Cursor::default();
    loop {
      tokio::time::sleep(Duration::from_secs(2)).await;
      let (path, session, model) = (path.clone(), sid.to_owned(), model.to_owned());
      let Ok((next, cut)) = tokio::task::spawn_blocking(move || {
        let cut = cursor.poll(&path, &session, &model, since);
        (cursor, cut)
      }).await else { return false };
      cursor = next;
      let Some(cut) = cut else { continue };
      let mut c = self.core.lock();
      if !turn.live(&c) || c.perms.epoch != epoch || !c.phase.running || c.status != SessionStatus::Ready || c.replaying { return false; }
      let Some(a) = agent_turn_mut(&mut c, turn.agent_idx, since) else { return false };
      // ACP may already have moved on while the log flushed: never interrupt text, a tool or a card. The first
      // warning itself can be the last row when the retry has not sent its first thought yet.
      let last = a.blocks.iter().rev().find(|b| !matches!(b, AgentBlock::Notice(n) if n.retry.is_some()));
      if a.stop.is_some() || !matches!(last, Some(AgentBlock::Thought(_))) { continue; }
      let detail = t("host.droidGeminiThinkingOnly");
      a.retry = Some(TurnRetry { attempt: Some(cut.attempt), max: None, detail: detail.clone() });
      report_retry(&mut c.state, RetryReport {
        attempt: Some(cut.attempt), max: None, text: detail, quality: RetryText::Summary, reconnect: false, wait_ms: None,
      });
      self.touch(&mut c);
      if cut.repeated {
        self.log("Droid: cancelling repeated thinking-only Gemini stream failures on the same request");
        proc.notify("session/cancel", json!({ "sessionId": sid }));
        return true;
      }
    }
  }
}

impl AcpSession {
  /// Droid moved the session onto a Droid Core model on its own (`droid_core.rs`): a running turn stops and settles with
  /// the exhaustion as its error (`settle_core_swap`), and the model the user picked is set back either way, so neither
  /// the composer nor the remembered settings ever take the Droid Core model
  pub(crate) fn refuse_core_swap(self: &Arc<Self>, c: &mut Core, swap: Swap) {
    let running = c.phase.running && c.status == SessionStatus::Ready;
    self.log(&format!(
      "Droid moved the session from {} to the Droid Core model {} on its own (Standard usage used up): {}",
      swap.from.0,
      swap.to.0,
      if running { "stopping the turn" } else { "setting the model back" }
    ));
    if running && c.droid_core.blocked.is_none() {
      if let (Some(proc), Some(sid)) = (c.proc.as_ref(), c.acp_session_id.as_deref()) {
        proc.notify("session/cancel", json!({ "sessionId": sid }));
      }
      c.droid_core.blocked = Some(swap.clone());
    }
    let Some(config_id) = c.state.controls.options.iter().find(|o| o.category.as_deref() == Some("model")).map(|o| o.id.clone()) else { return };
    let me = self.clone();
    tokio::spawn(async move {
      if let Err(e) = me.set_config_wire(config_id, swap.from.0.clone()).await {
        me.log(&format!("Droid: setting {} back after the Droid Core swap failed: {e}", swap.from.0));
      }
    });
  }

  /// The turn that met the swap settles as an error naming the used-up model and, when the bound account's quota tells,
  /// when Standard usage comes back. None = no swap was refused in it, or the session moved on meanwhile
  pub(super) async fn settle_core_swap(&self, turn: &OpenTurn) -> Option<Settled> {
    let (swap, account) = {
      let c = self.core.lock();
      (c.droid_core.blocked.clone()?, c.account_id.clone())
    };
    let quota = match (account, self.deps.accounts.as_ref()) {
      (Some(id), Some(hooks)) => tokio::time::timeout(Duration::from_secs(5), hooks.quota(id)).await.ok().flatten(),
      _ => None,
    };
    let error = droid_core::turn_error(&swap, quota.as_ref().and_then(droid_core::reset_at));
    let mut c = self.core.lock();
    c.droid_core.blocked = None;
    if !turn.live(&c) {
      return None;
    }
    let exhausted = self.reserve_switch(&mut c, &error);
    self.settle(&mut c, TurnStop::Error, Some(error));
    Some(Settled { stop: TurnStop::Error, exhausted })
  }
}
