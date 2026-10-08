//! The follow-up queue: prompts parked behind the running turn, send-now, steering into the running turn, in-place edits

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::transcript::*;

use crate::acp::session::attachments::{PreparedPrompt, prepare_prompt, restore_drafts};
use crate::acp::session::prompt::{Origin, Staged};
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::push_steer;
use crate::acp::vendors::{claude_autonomous, steering};
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
  /// The detached turn is one the peer started with nothing on the wire (a Claude autonomous cycle,
  /// `vendors::claude_autonomous`): its result's `usage_update` ends it, and a stop settles it locally since no prompt
  /// response will
  pub autonomous: bool,
  /// Late packets from a settled autonomous cycle must not reopen its turn. A new prompt clears this latch.
  pub autonomous_blocked: bool,
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

  /// The prompt a failed ultracode rebuild left without a native session (`rebuild_before_prompt`): its claim is undone
  /// and a user's prompt waits at the head of the queue, which flushes once Retry has the session Ready again
  pub(crate) async fn park_after_failed_rebuild(&self, text: String, drafts: Vec<Draft>, origin: Origin, staged: Option<Staged>) {
    let parked = match (origin, staged) {
      (Origin::User, Some(s)) => Some((s.id, s.prepared)),
      (Origin::User, None) => Some((None, self.stage(&text, &drafts).await)),
      _ => None,
    };
    let mut c = self.core.lock();
    c.phase.running = false;
    c.phase.staging = false;
    if let Some((id, prepared)) = parked.filter(|(_, p)| !p.blocks.is_empty()) {
      c.queue.entries.insert(0, QueuedEntry { id: id.unwrap_or_else(random_uuid), text, prepared });
    }
    self.touch(&mut c);
  }

  /// Queue a prompt behind the running turn (or while starting); staged now so the row can show attachments.
  /// Returns the new entry's id, or None when nothing was queued
  pub(crate) async fn enqueue(self: &Arc<Self>, text: String, drafts: Vec<Draft>, staged: Option<PreparedPrompt>) -> Option<String> {
    let prepared = match staged {
      Some(p) => p,
      None => self.stage(&text, &drafts).await,
    };
    let id = random_uuid();
    let flush = {
      let mut c = self.core.lock();
      if prepared.blocks.is_empty() || !matches!(c.status, SessionStatus::Ready | SessionStatus::Starting) {
        return None;
      }
      c.queue.entries.push(QueuedEntry { id: id.clone(), text, prepared });
      self.bump(&mut c);
      !(c.phase.running || c.pending_prompt.is_some() || c.hooks.gating)
    };
    if flush {
      self.flush_queue();
    }
    Some(id)
  }

  /// A prompt sent from the composer with steering on: while a turn runs it joins that turn over `_session/steering`
  /// right away. It goes through the queue so a steer that cannot land (no support, the turn ending, another steer on
  /// the wire, the peer refusing) leaves it there as an ordinary follow-up; it never cancels the running turn.
  /// Without a running turn it is the ordinary prompt
  pub async fn steer_prompt(self: &Arc<Self>, text: String, drafts: Vec<Draft>) -> Result<()> {
    let running = {
      let c = self.core.lock();
      c.status == SessionStatus::Ready && c.phase.running
    };
    if !running {
      self.prompt(text, drafts, false, None, None).await;
      return Ok(());
    }
    match self.enqueue(text, drafts, None).await {
      Some(id) => self.steer_entry(&id, false).await,
      None => Ok(()),
    }
  }

  /// Send the first queued prompt, if any; nobody awaits it
  pub(crate) fn flush_queue(self: &Arc<Self>) -> bool {
    let next = {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready
        || c.phase.running
        || c.switching
        || c.hooks.gating
        || c.picks.adopt_pending
        || c.ultracode.rebuilding
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
    crate::util::run_prefix(me.prompt(next.text, vec![], false, Some(Staged { prepared: next.prepared, edited: false, id: Some(next.id) }), None));
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
    self.steer_entry(id, true).await
  }

  /// send_now: a steer that cannot go out falls back to send-now (the queue row's button); without it the entry just
  /// stays queued (a composer send, which must not cancel the running turn)
  async fn steer_entry(self: &Arc<Self>, id: &str, send_now: bool) -> Result<()> {
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
        Some(_) if !steerable && !send_now => return Ok(()),
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

  /// The detached peer turn reported its thread idle (or, for an autonomous cycle, its result or a stop): settle it like a
  /// prompt response and let the queue move
  pub(crate) fn end_detached(self: &Arc<Self>, c: &mut Core, stop: TurnStop) {
    c.peer.detached = false;
    let autonomous = std::mem::take(&mut c.peer.autonomous);
    if autonomous {
      c.peer.autonomous_blocked = true;
    }
    if c.phase.running {
      self.settle(c, stop, None);
    }
    self.log(if autonomous { "autonomous cycle ended" } else { "detached turn ended (thread idle)" });
    let me = self.clone();
    tokio::spawn(async move { me.after_prompt(false, stop) });
  }

  /// Turn content with nothing on the wire from a peer that runs cycles of its own (Claude's task-notification
  /// followups): the session shows the last agent turn running again until the cycle's result arrives
  pub(crate) fn open_autonomous(&self, c: &mut Core, kind: &str) {
    if !self.vendor.autonomous_cycles()
      || c.replaying
      || c.phase.running
      || c.peer.detached
      || c.peer.autonomous_blocked
      || c.status != SessionStatus::Ready
      || !claude_autonomous::CYCLE_START_KINDS.contains(&kind)
      || !matches!(c.state.turns.last(), Some(Turn::Agent(_)))
    {
      return;
    }
    c.peer.detached = true;
    c.peer.autonomous = true;
    self.reopen_for_detached(c);
    self.log(&format!("autonomous cycle started ({kind})"));
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
