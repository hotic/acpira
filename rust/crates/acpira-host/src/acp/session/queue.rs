//! The follow-up queue: prompts parked behind the running turn, send-now, steering into the running turn, in-place edits

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::transcript::*;

use crate::acp::session::attachments::{PreparedPrompt, prepare_prompt, restore_drafts};
use crate::acp::session::prompt::Staged;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::push_steer;
use crate::acp::vendors::steering;
use crate::i18n::{t, tp};
use crate::util::random_uuid;

pub(crate) struct QueuedEntry {
  pub id: String,
  pub text: String,
  pub prepared: PreparedPrompt,
}

/// Prompts parked behind the running turn, and the entry a send-now or a steer has claimed
#[derive(Default)]
pub(crate) struct PromptQueue {
  pub entries: Vec<QueuedEntry>,
  pub sending_id: Option<String>,
  /// The queued entry whose `_session/steering` request is on the wire; the queue holds its flush until the answer
  pub steering_id: Option<String>,
}

impl PromptQueue {
  /// The entry is on its way out (send-now or steer) and can no longer be edited or removed
  pub(crate) fn in_flight(&self, id: &str) -> bool {
    self.sending_id.as_deref() == Some(id) || self.steering_id.as_deref() == Some(id)
  }

  /// Some entry is on its way out: a second send-now or steer waits
  pub(crate) fn claimed(&self) -> bool {
    self.sending_id.is_some() || self.steering_id.is_some()
  }
}

/// What the peer reports about its own turns (Codex `threadStatus`); starts over with every process
#[derive(Default)]
pub(crate) struct PeerTurn {
  /// The peer brackets its turns, so a turn it starts on its own has an observable end
  pub status_seen: bool,
  /// The running turn's peer already reported its thread idle: the prompt response is on its way and a steer would miss
  pub idle: bool,
  /// A steer landed after the turn it aimed at had ended and the peer started a turn of its own (Codex `startedNewTurn`):
  /// the session stays running until the peer's thread reports idle
  pub detached: bool,
}

impl AcpSession {
  async fn stage(&self, text: &str, drafts: &[Draft]) -> PreparedPrompt {
    let caps = self.caps(&self.core.lock());
    let prepared = prepare_prompt(&self.id, text, drafts, &self.deps.blobs, Some(caps)).await;
    for p in &prepared.problems {
      self.log(p);
      self.notify(p);
    }
    PreparedPrompt { problems: vec![], ..prepared }
  }

  /// Queue a prompt behind the running turn (or while starting); staged now so the row can show attachments
  pub(crate) async fn enqueue(self: &Arc<Self>, text: String, drafts: Vec<Draft>, staged: Option<PreparedPrompt>) {
    let prepared = match staged {
      Some(p) => p,
      None => self.stage(&text, &drafts).await,
    };
    let flush = {
      let mut c = self.core.lock();
      if prepared.blocks.is_empty() || !matches!(c.status, SessionStatus::Ready | SessionStatus::Starting) {
        return;
      }
      c.queue.entries.push(QueuedEntry { id: random_uuid(), text, prepared });
      self.bump(&mut c);
      !(c.phase.running || c.pending_prompt.is_some())
    };
    if flush {
      self.flush_queue();
    }
  }

  /// Send the first queued prompt, if any; nobody awaits it
  pub(crate) fn flush_queue(self: &Arc<Self>) -> bool {
    let next = {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready
        || c.phase.running
        || c.switching
        || c.picks.adopt_pending
        || c.pending_prompt.is_some()
        || c.queue.steering_id.is_some()
        || c.peer.detached
        || c.queue.entries.is_empty()
      {
        return false;
      }
      c.queue.sending_id = None;
      c.queue.entries.remove(0)
    };
    let me = self.clone();
    crate::util::run_prefix(me.prompt(next.text, vec![], false, Some(Staged { prepared: next.prepared, edited: false }), None));
    true
  }

  pub fn dequeue(&self, id: &str) {
    let mut c = self.core.lock();
    if c.queue.in_flight(id) {
      return;
    }
    let before = c.queue.entries.len();
    c.queue.entries.retain(|q| q.id != id);
    if c.queue.entries.len() != before {
      self.touch(&mut c);
    }
  }

  /// Reserve the selected entry before cancelling the active turn
  pub async fn send_queued(self: &Arc<Self>, id: &str) -> Result<()> {
    let running = {
      let mut c = self.core.lock();
      if c.queue.claimed() || c.status != SessionStatus::Ready {
        return Ok(());
      }
      let Some(i) = c.queue.entries.iter().position(|q| q.id == id) else { return Ok(()) };
      let entry = c.queue.entries.remove(i);
      c.queue.entries.insert(0, entry);
      c.queue.sending_id = Some(id.to_owned());
      self.touch(&mut c);
      c.phase.running
    };
    // cancel is only a notification; prompt completion owns the next flush, so prompts never overlap
    if running {
      self.cancel().await
    } else {
      self.flush_queue();
    }
    Ok(())
  }

  /// Inject the selected entry into the running turn over `_session/steering` instead of cancelling that turn. The entry
  /// stays queued (marked sending, its flush held) until the peer answers: `injected` moves it into the turn as a steer
  /// block, `promptRequired` (the turn ended meanwhile) puts it first in line for the normal flush, a failure leaves it
  /// where it was. Without a running turn or steering support this is the ordinary send-now
  pub async fn steer_queued(self: &Arc<Self>, id: &str) -> Result<()> {
    let claimed = {
      let mut c = self.core.lock();
      if c.queue.claimed() || c.status != SessionStatus::Ready {
        return Ok(());
      }
      // Staging, a pre-send compaction or an account switch own the wire: the steer would land in the wrong request
      // A peer that already reported its thread idle has finished the turn: the steer would start a turn of its own
      let steerable = c.phase.running
        && !c.phase.staging
        && c.pending_prompt.is_none()
        && !c.switching
        && !c.peer.idle
        && !c.peer.detached
        && self.can_steer_of(&c);
      match c.queue.entries.iter().find(|q| q.id == id) {
        None => return Ok(()),
        Some(_) if !steerable => None,
        Some(entry) => {
          let blocks = entry.prepared.blocks.clone();
          c.queue.steering_id = Some(id.to_owned());
          self.touch(&mut c);
          Some((c.proc.clone(), c.acp_session_id.clone(), blocks))
        }
      }
    };
    let Some((proc, acp_id, blocks)) = claimed else { return self.send_queued(id).await };
    let (Some(proc), Some(acp_id)) = (proc, acp_id) else {
      self.release_steer(id, false);
      return Ok(());
    };
    match proc.request_ordered(steering::METHOD, steering::params(&acp_id, &blocks)).await {
      Ok((r, handoff)) => {
        let outcome = steering::outcome_of(&r);
        self.log(&format!("steer: {}", r.get("outcome").and_then(Value::as_str).unwrap_or("?")));
        match outcome {
          // startedNewTurn comes from a peer that ignored the opt-in (codex-acp once its turn had ended): the content went out
          // all the same, so it is recorded where it was sent rather than sent a second time, and its answer streams onto
          // the same agent turn. When the peer brackets its turns the session runs until that turn reports idle
          Some(outcome @ (steering::Outcome::Injected | steering::Outcome::StartedNewTurn)) => {
            let mut c = self.core.lock();
            c.queue.steering_id = None;
            if let Some(i) = c.queue.entries.iter().position(|q| q.id == id) {
              let entry = c.queue.entries.remove(i);
              let attachments = (!entry.prepared.attachments.is_empty()).then_some(entry.prepared.attachments);
              push_steer(&mut c.state, SteerBlock { id: entry.id, text: entry.text, attachments });
            }
            if outcome == steering::Outcome::StartedNewTurn && c.peer.status_seen {
              c.peer.detached = true;
              self.reopen_for_detached(&mut c);
            }
            self.bump(&mut c);
            drop(handoff);
          }
          Some(steering::Outcome::PromptRequired) | None => {
            drop(handoff);
            self.release_steer(id, true);
          }
        }
      }
      Err(e) => {
        self.log(&format!("steer failed: {e}"));
        self.notify(&tp("queue.steerFailed", &[("error", &e.to_string())]));
        self.release_steer(id, false);
      }
    }
    Ok(())
  }

  /// A detached peer turn shows as the agent turn it continues, running again, once the prompt that turn belonged to has
  /// settled (until then that prompt still owns the running state and settles it itself)
  pub(crate) fn reopen_for_detached(&self, c: &mut Core) {
    if c.phase.running {
      return;
    }
    c.phase.running = true;
    if let Some(Turn::Agent(t)) = c.state.turns.last_mut() {
      t.stop = None;
      t.ended_at = None;
    }
  }

  /// The detached peer turn reported its thread idle: settle it like a prompt response and let the queue move
  pub(crate) fn end_detached(self: &Arc<Self>, c: &mut Core) {
    c.peer.detached = false;
    if c.phase.running {
      self.settle(c, TurnStop::EndTurn, None);
    }
    self.log("detached turn ended (thread idle)");
    let me = self.clone();
    tokio::spawn(async move { me.after_prompt(false, TurnStop::EndTurn) });
  }

  /// The steer did not join the turn: the entry is queued again, first in line when the turn it missed has already ended
  fn release_steer(self: &Arc<Self>, id: &str, first: bool) {
    {
      let mut c = self.core.lock();
      c.queue.steering_id = None;
      if first && let Some(i) = c.queue.entries.iter().position(|q| q.id == id) {
        let entry = c.queue.entries.remove(i);
        c.queue.entries.insert(0, entry);
      }
      self.touch(&mut c);
    }
    self.flush_queue();
  }

  /// Replace a queued prompt in place: kept attachments come back from their blobs, new drafts are staged alongside
  pub async fn edit_queued(self: &Arc<Self>, id: &str, text: String, retained: Vec<i64>, drafts: Vec<Draft>) -> Result<()> {
    let kept = {
      let c = self.core.lock();
      if c.queue.in_flight(id) {
        return Ok(());
      }
      let entry = c.queue.entries.iter().find(|q| q.id == id).ok_or_else(|| anyhow!(t("queue.gone")))?;
      retained.iter().filter_map(|i| usize::try_from(*i).ok().and_then(|i| entry.prepared.attachments.get(i)).cloned()).collect::<Vec<_>>()
    };
    let mut all = restore_drafts(&self.id, &kept, &self.deps.blobs).await?;
    all.extend(drafts);
    let prepared = self.stage(&text, &all).await;
    let mut c = self.core.lock();
    if c.queue.in_flight(id) {
      return Err(anyhow!(t("queue.gone")));
    }
    let Some(i) = c.queue.entries.iter().position(|q| q.id == id) else { return Err(anyhow!(t("queue.gone"))) };
    let has_content = prepared.blocks.iter().any(|b| {
      b.get("type").and_then(Value::as_str) != Some("text") || b.get("text").and_then(Value::as_str).is_some_and(|x| !x.trim().is_empty())
    });
    if !has_content {
      c.queue.entries.remove(i);
      self.touch(&mut c);
      return Ok(());
    }
    c.queue.entries[i].text = text;
    c.queue.entries[i].prepared = prepared;
    self.touch(&mut c);
    Ok(())
  }
}

pub(crate) fn queue_snapshot(c: &Core) -> Option<Vec<QueuedPrompt>> {
  if c.queue.entries.is_empty() {
    return None;
  }
  Some(
    c.queue.entries
      .iter()
      .map(|q| QueuedPrompt {
        id: q.id.clone(),
        text: q.text.clone(),
        attachments: q.prepared.attachments.clone(),
        sending: c.queue.in_flight(&q.id).then_some(true),
      })
      .collect(),
  )
}
