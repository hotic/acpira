//! Observe the retries the Droid ACP adapter drops without changing the ownership of the live prompt.

use std::path::Path;
use std::time::Duration;

use acpira_shared::transcript::{AgentBlock, SessionStatus, TurnError, TurnRetry, TurnStop};
use serde_json::json;

use super::{AcpSession, AgentProcess, OpenTurn, agent_turn_mut};
use crate::acp::transcript::normalize::{RetryReport, RetryText, report_retry};
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
