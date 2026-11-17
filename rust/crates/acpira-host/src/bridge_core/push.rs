//! What goes back to the page: manager events coalesced per batch window, session views and the list sent as patches when the page takes them

use acpira_shared::protocol::HostMsg;
use acpira_shared::session_patch::Delivery;
use acpira_shared::session_window::VIEW_WINDOW_BYTES;

use super::{BATCH_WINDOW, BridgeCore};

impl BridgeCore {
  pub(super) fn queue(&self, m: HostMsg) {
    if !self.ready.load(std::sync::atomic::Ordering::Acquire) {
      // Dropped: init carries the current state. A session view means the one being opened can go into init now
      // (`notify_one` keeps the wake-up when the ready handler is not waiting yet)
      if matches!(m, HostMsg::Session { .. }) {
        self.view_shown.notify_one();
      }
      // Not state: init says nothing of a session that could not be opened, and the page waiting for it would wait on
      if matches!(m, HostMsg::SessionUnavailable { .. }) {
        self.refused.lock().push(m);
      }
      return;
    }
    let _order = self.order.lock();
    let pushed = self.batch.lock().push(m);
    match pushed {
      Pushed::Flush(queued) => queued.into_iter().for_each(|m| self.deliver(m)),
      Pushed::Arm => {
        let weak = self.me.clone();
        tokio::spawn(async move {
          tokio::time::sleep(BATCH_WINDOW).await;
          if let Some(c) = weak.upgrade() {
            c.flush();
          }
        });
      }
      Pushed::Wait => {}
    }
  }

  fn flush(&self) {
    let _order = self.order.lock();
    let queued = self.batch.lock().flush();
    for m in queued {
      self.deliver(m);
    }
  }

  /// Post a batched message; a session view goes out as a patch against the last view of that session the page was sent,
  /// the one on screen or one it keeps from before a switch (`SentViews`). The diff and the post happen under one lock, so
  /// two flushes racing on different threads can never post patches out of order
  pub(super) fn deliver(&self, m: HostMsg) {
    let m = match m {
      HostMsg::Sessions { sessions } => return self.deliver_list(sessions),
      m => m,
    };
    let HostMsg::Session { parts, running, .. } = &m else { return self.emit(m) };
    let mut sent = self.sent.lock();
    let next = parts.as_ref().filter(|_| self.patches.load(std::sync::atomic::Ordering::Acquire));
    let delivery = match next {
      Some(next) if self.window.load(std::sync::atomic::Ordering::Acquire) => sent.deliver_window(next, VIEW_WINDOW_BYTES),
      next => sent.deliver(next),
    };
    match delivery {
      // Older than what the page shows: it carries nothing the page does not have and must not become a patch base
      Delivery::Stale => {}
      Delivery::Patch(patch) => self.emit(HostMsg::SessionPatch { patch }),
      Delivery::Window(session) => self.emit(HostMsg::Session { session: session.into(), running: *running, parts: None }),
      Delivery::Whole => self.emit(m),
    }
  }

  /// A session list goes out as runs over the list the page holds when the page takes list patches; an unchanged list
  /// is not sent at all. Diffed and posted under `sent`, ordered against init like session views
  fn deliver_list(&self, sessions: Vec<acpira_shared::transcript::SessionSummary>) {
    if !self.list_patches.load(std::sync::atomic::Ordering::Acquire) {
      return self.emit(HostMsg::Sessions { sessions });
    }
    let _order = self.sent.lock();
    let mut listed = self.listed.lock();
    match listed.take() {
      Some((rev, held)) => {
        let next = match acpira_shared::sessions_patch::diff(&held, &sessions, rev) {
          Some(patch) => {
            self.emit(HostMsg::SessionsPatch { patch });
            rev + 1
          }
          None => rev,
        };
        *listed = Some((next, sessions));
      }
      None => {
        *listed = Some((0, sessions.clone()));
        self.emit(HostMsg::Sessions { sessions });
      }
    }
  }

  pub fn post_now(&self, m: HostMsg) {
    self.emit(m);
  }

  /// Every post to the page goes through here. The read guard is held across the post, so once `dispose` has taken the
  /// write side nothing is mid-post and nothing posts again: a batch timer or a handler still running after detachView
  /// cannot reach the shell behind the detach
  pub(super) fn emit(&self, m: HostMsg) {
    let disposed = self.disposed.read();
    if !*disposed {
      (self.post)(m);
    }
  }
}

/// Streaming updates are dense: within one batch window only the latest message per key survives, and an idle session edge
/// goes out at once together with whatever was pending. Only the edge: a session whose own turn is over may still stream
/// (a summoned child the turn left running), and those idle views coalesce like running ones
#[derive(Default)]
pub struct MsgBatch {
  pending: Vec<(String, HostMsg)>,
  armed: bool,
  /// The running flag of the last session view pushed; None before the first
  running: Option<bool>,
  /// The id of the last session view pushed: switching to another session flushes at once instead of waiting out the window
  session_id: Option<String>,
}

/// What a push asks of the owner
pub enum Pushed {
  /// Post these now
  Flush(Vec<HostMsg>),
  /// Start the window timer, then flush
  Arm,
  /// A timer is already running
  Wait,
}

impl MsgBatch {
  pub fn push(&mut self, m: HostMsg) -> Pushed {
    let immediate = match &m {
      HostMsg::Session { running, parts, .. } => {
        let switched = match parts.as_ref().map(|p| p.0.id.as_str()) {
          Some(id) => self.session_id.replace(id.to_owned()).as_deref() != Some(id),
          None => false,
        };
        let idle = self.running.replace(*running) != Some(false) && !*running;
        switched || idle
      }
      HostMsg::DeleteSessionResult { .. } => true,
      _ => false,
    };
    let key = m.batch_key();
    match self.pending.iter().position(|(k, _)| *k == key) {
      Some(i) => self.pending[i].1 = m,
      None => self.pending.push((key, m)),
    }
    if immediate {
      return Pushed::Flush(self.flush());
    }
    if self.armed {
      return Pushed::Wait;
    }
    self.armed = true;
    Pushed::Arm
  }

  pub fn flush(&mut self) -> Vec<HostMsg> {
    self.armed = false;
    std::mem::take(&mut self.pending).into_iter().map(|(_, m)| m).collect()
  }

  pub fn clear(&mut self) {
    self.pending.clear();
  }
}
