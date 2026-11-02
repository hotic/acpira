//! One engine drives a session at a time: the leases it holds, record edits and turns that pin them, takeovers
//! between engines, and the read-only mirror of a session another engine holds

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};

use super::{FIRST_PUSH_GRACE, MIRROR_POLL, PUSH_QUANTUM, RECORD_HOLD_WAIT, SessionManager, TAKEOVER_POLL, TAKEOVER_WAIT, YIELD_SETTLE};
use crate::acp::session::{AcpSession, SessionDeps, SessionPolicy};
use crate::i18n::{t, tp};
use crate::store::record::{RecordSource, SessionRecord};
use crate::store::session_lease::{Claim, SessionLeases};

/// A mirror's policy: no compaction or plan auto-approval, and it never runs a turn; the live policy's model shapes
/// still show
struct MirrorPolicy(Arc<dyn SessionPolicy>);

impl SessionPolicy for MirrorPolicy {
  fn model_shapes(&self, agent: &str) -> Option<acpira_shared::model_shapes::ModelShapes> {
    self.0.model_shapes(agent)
  }

  fn claim(&self, _session: &str) -> Option<Result<u64, String>> {
    Some(Err(t("host.sessionBusyElsewhere")))
  }
}

/// One claim on a session's lease (`store::session_lease`), released when it drops
pub(super) struct LeaseHold<'a> {
  pub(super) leases: &'a SessionLeases,
  pub(super) id: &'a str,
  pub(super) pin: u64,
}

impl LeaseHold<'_> {
  /// The pin outlives this hold: whoever takes it releases it
  pub(super) fn keep(mut self) -> u64 {
    std::mem::take(&mut self.pin)
  }
}

impl Drop for LeaseHold<'_> {
  fn drop(&mut self) {
    if self.pin != 0 {
      self.leases.release(self.id, self.pin);
    }
  }
}

impl SessionManager {
  /// Record edits (rename, pin, move) hold the session's lease until they are on disk: refused while another
  /// engine drives the session (or this one only mirrors it), since they would race that engine's saves. A short hold
  /// (another engine opening the session or saving an edit of its own) is waited out rather than refused
  pub(super) async fn hold_record<'a>(&'a self, id: &'a str) -> Result<LeaseHold<'a>> {
    if self.mirror_of(id).is_some() {
      return Err(anyhow!(t("host.sessionBusyElsewhere")));
    }
    let deadline = tokio::time::Instant::now() + RECORD_HOLD_WAIT;
    loop {
      match self.leases.claim(id) {
        Claim::Held(pin) => return Ok(LeaseHold { leases: &self.leases, id, pin }),
        Claim::Elsewhere if tokio::time::Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(50)).await,
        Claim::Elsewhere => return Err(anyhow!(t("host.sessionBusyElsewhere"))),
        Claim::Failed(e) => return Err(anyhow!(tp("host.sessionLeaseFailed", &[("error", &e)]))),
      }
    }
  }

  /// Deletion ends the session wherever it runs. Keep its undo in the requesting engine rather than forwarding the
  /// delete to an owner whose lease (and action route) goes away with it. Other machines' copies stay read-only
  pub(super) async fn hold_deletion<'a>(&'a self, id: &'a str) -> Result<LeaseHold<'a>> {
    if self.mirror_of(id).is_some_and(|s| !s.can_take_over_elsewhere()) {
      return Err(anyhow!(t("host.sessionBusyElsewhere")));
    }
    match self.leases.claim(id) {
      Claim::Held(pin) => Ok(LeaseHold { leases: &self.leases, id, pin }),
      Claim::Elsewhere => self.take_lease_elsewhere(id).await,
      Claim::Failed(e) => Err(anyhow!(tp("host.sessionLeaseFailed", &[("error", &e)]))),
    }
  }

  /// The session's `claim` hook: the lease for a turn about to start
  pub(super) fn claim_turn(&self, id: &str) -> Result<u64, String> {
    if self.yielding.lock().contains(id) || self.state.lock().deleting.contains_key(id) {
      return Err(t("host.sessionBusyElsewhere"));
    }
    match self.leases.claim(id) {
      Claim::Held(pin) => Ok(pin),
      Claim::Elsewhere => Err(t("host.sessionBusyElsewhere")),
      Claim::Failed(e) => {
        self.log(&format!("session {id}: lease failed: {e}"));
        Err(tp("host.sessionLeaseFailed", &[("error", &e)]))
      }
    }
  }

  /// A finished turn's lease pin goes once its final record is on disk. A failed save keeps the lease (another engine
  /// would read a record without the end of the turn) and is retried, until it lands or the session is closed here
  pub(super) fn release_after_save(&self, s: Arc<AcpSession>, pin: u64) {
    let me = self.me.clone();
    tokio::spawn(async move {
      let mut wait = Duration::from_secs(1);
      loop {
        let Some(m) = me.upgrade() else { return };
        let closed = m.live(&s.id).is_none_or(|live| !Arc::ptr_eq(&live, &s));
        if closed {
          m.leases.release(&s.id, pin);
          return;
        }
        match m.deps.store.flush(s.clone() as Arc<dyn RecordSource>).await {
          Ok(()) => {
            m.leases.release(&s.id, pin);
            m.sync_leases();
            return;
          }
          Err(e) => m.log(&format!("session {}: final save failed, lease kept, retrying in {}s: {e}", s.id, wait.as_secs())),
        }
        drop(m);
        tokio::time::sleep(wait).await;
        wait = (wait * 2).min(Duration::from_secs(30));
      }
    });
  }

  pub(super) fn mirror_of(&self, id: &str) -> Option<Arc<AcpSession>> {
    self.state.lock().mirrors.get(id).cloned()
  }

  /// Hold exactly the leases of the sessions this engine has open: a session is driven from one engine at a time, every
  /// other one shows it read-only until it takes the session over (or this engine closes it, or ends)
  pub fn sync_leases(&self) {
    let wanted: HashSet<String> = self.live_sessions().into_iter().map(|s| s.id.clone()).collect();
    self.leases.sync(&wanted);
    self.bus.announce(self.leases.held());
  }

  /// Watch for other engines asking to take over a session this one has open; ends with the manager
  pub(super) fn watch_takeovers(&self) {
    let me = self.me.clone();
    tokio::spawn(async move {
      loop {
        tokio::time::sleep(TAKEOVER_POLL).await;
        let Some(m) = me.upgrade() else { return };
        if m.state.lock().disposed {
          return;
        }
        for id in m.leases.held() {
          if m.leases.takeover_requested(&id) && m.live(&id).is_some() && m.yielding.lock().insert(id.clone()) {
            let m2 = m.clone();
            tokio::spawn(async move {
              m2.yield_session(&id).await;
              m2.yielding.lock().remove(&id);
            });
          } else if m.leases.handover_requested(&id) && m.handover_ready(&id) && m.yielding.lock().insert(id.clone()) {
            let m2 = m.clone();
            tokio::spawn(async move {
              m2.hand_over_idle(&id).await;
              m2.yielding.lock().remove(&id);
            });
          }
        }
      }
    });
  }

  /// Another engine takes this session over: the running turn stops, the record is written, the agent ends and the lease
  /// goes; whoever was looking at it here keeps a read-only copy that can take it back
  async fn yield_session(self: &Arc<Self>, id: &str) {
    let _op = self.session_op(id).await;
    let Some(s) = self.live(id) else { return };
    self.log(&format!("session {id}: another engine takes it over, letting go"));
    if s.is_running() {
      s.cancel_for_handover().await;
      let t0 = tokio::time::Instant::now();
      while s.is_running() && t0.elapsed() < YIELD_SETTLE {
        tokio::time::sleep(Duration::from_millis(50)).await;
      }
    }
    if s.is_running() {
      self.log(&format!("session {id}: turn has not settled, lease kept"));
      return;
    }
    if let Err(e) = self.deps.store.flush(s.clone() as Arc<dyn RecordSource>).await {
      self.log(&format!("session {id}: save before handing over failed: {e}"));
      return;
    }
    let record = s.to_record();
    self.forget(id);
    // The copy goes up before the lease goes, so the viewers never see the session vanish
    if !self.viewers_on(id, None).is_empty() {
      self.start_mirror(id, record, t("host.sessionTakenOver"), true);
    }
    self.sync_leases();
    self.emit_sessions();
  }

  /// A session another engine wants can go without asking anyone: live and idle here, and no window of this engine shows
  /// it (the usual case is a window reloaded onto a new engine after an upgrade, leaving this one with no window at all)
  fn handover_ready(&self, id: &str) -> bool {
    self.live(id).is_some_and(|s| s.can_let_go()) && self.viewers_on(id, None).is_empty()
  }

  /// Hand an idle, unseen session to the engine that wants it: the agent process closes like an idle release, and the
  /// lease goes once the native session is closed, so the other engine's read-only copy opens by itself
  async fn hand_over_idle(self: &Arc<Self>, id: &str) {
    let Some(s) = self.live(id) else { return };
    if self.release(&s).await {
      self.log(&format!("session {id}: another engine wants it and no window here shows it, handing it over"));
      self.sync_leases();
      self.emit_sessions();
    }
  }

  /// Ask the owner to stop and save, then hold its lease without opening an agent here. Deletion uses the same
  /// handshake as a takeover but must not resume the session just to close it again
  async fn take_lease_elsewhere<'a>(&'a self, id: &'a str) -> Result<LeaseHold<'a>> {
    if let Err(e) = self.leases.request_takeover(id) {
      self.leases.clear_takeover(id);
      return Err(anyhow!(tp("host.sessionLeaseFailed", &[("error", &e.to_string())])));
    }
    let deadline = tokio::time::Instant::now() + TAKEOVER_WAIT;
    let pin = loop {
      match self.leases.claim(id) {
        Claim::Held(pin) => break pin,
        Claim::Elsewhere if tokio::time::Instant::now() < deadline => tokio::time::sleep(Duration::from_millis(100)).await,
        Claim::Elsewhere => {
          self.leases.clear_takeover(id);
          return Err(anyhow!(t("host.takeOverTimeout")));
        }
        Claim::Failed(e) => {
          self.leases.clear_takeover(id);
          return Err(anyhow!(tp("host.sessionLeaseFailed", &[("error", &e)])));
        }
      }
    };
    self.leases.clear_takeover(id);
    Ok(LeaseHold { leases: &self.leases, id, pin })
  }

  /// Take over a session another engine has open: ask it to let go, wait for the lease, then open the session here
  pub(super) async fn take_over_elsewhere(self: &Arc<Self>, id: &str) -> Result<()> {
    // Refuse before asking the healthy owner to stop: SSH may see the same accounts but a locked keychain.
    if let Some(s) = self.mirror_of(id)
      && let Some(accounts) = &self.deps.accounts
    {
      accounts.check_locks(Some(&s.agent)).await;
      if accounts.credentials_locked(&s.agent) {
        let name = self.registry().try_get(&s.agent).map(|a| a.name.clone()).unwrap_or_else(|| s.agent.clone());
        return Err(anyhow!(tp("notice.locked.title", &[("agent", &name)])));
      }
    }
    let hold = self.take_lease_elsewhere(id).await?;
    self.log(&format!("session {id}: taken over from another engine"));
    self.state.lock().mirrors.remove(id);
    // The first viewer loads it (fresh from disk, the other engine's last word); the rest find it live
    for v in self.viewers_on(id, None) {
      self.select_session_for(&v, id).await;
    }
    drop(hold);
    Ok(())
  }

  /// Some session is mid-turn (a permission card or question waiting on the user counts: the turn is still running)
  pub fn busy(&self) -> bool {
    self.live_sessions().iter().any(|s| s.is_running())
  }

  /// Sessions whose lease this engine holds (tests)
  pub fn leased(&self) -> Vec<String> {
    self.leases.held()
  }

  /// On the engines' push bus right now (tests)
  pub fn bus_connected(&self) -> bool {
    self.bus.connected()
  }

  /// Inert deps for a mirror: it never saves (the record belongs to the engine holding the lease), warms or switches
  fn mirror_deps(&self) -> SessionDeps {
    let live = self.session_deps();
    SessionDeps {
      on_change: Arc::new(|_: &str, _: bool| {}),
      notify: None,
      accounts: None,
      policy: Arc::new(MirrorPolicy(live.policy.clone())),
      pool: None,
      remote: None,
      ..live
    }
  }

  fn record_path(&self, id: &str) -> std::path::PathBuf {
    self.deps.store.dir().join(format!("{id}.json"))
  }

  /// Show `record` read-only to the viewers on `id` and poll: the copy follows the record the other engine keeps saving,
  /// and once its lease comes free the session opens here like any other (the viewers re-select it)
  pub(super) fn start_mirror(self: &Arc<Self>, id: &str, record: SessionRecord, note: String, elsewhere: bool) {
    // One epoch for this copy and every copy the poll below makes: their revs grow, so a viewer is patched between them
    let epoch = crate::acp::session::next_view_epoch();
    let mirror = AcpSession::mirror(record, self.mirror_deps(), note.clone(), self.next_mirror_rev(), elsewhere, epoch);
    let fresh = self.state.lock().mirrors.insert(id.to_owned(), mirror.clone()).is_none();
    self.log(&format!("session {id}: lease not taken, mirroring its record ({note})"));
    // Held by an engine that may push its view: the record copy waits for that view a moment (`FIRST_PUSH_GRACE`)
    let await_push = fresh && elsewhere && self.bus.connected();
    if !await_push {
      self.emit_session(&mirror);
    }
    if !fresh {
      return;
    }
    // Settled below once the pushed view came or the grace ended (`first_view_settled`)
    if await_push {
      self.state.lock().first_push.insert(id.to_owned(), tokio::sync::watch::channel(false).0);
    }
    // Held by another engine: ask it to hand the session over once nobody there needs it (refreshed on every poll below),
    // and to push the session's view meanwhile
    if elsewhere {
      self.want_handover(id);
      self.bus.subscribe(id);
    }
    let me = self.me.clone();
    let id = id.to_owned();
    let mut note = note;
    tokio::spawn(async move {
      if await_push {
        let until = tokio::time::Instant::now() + FIRST_PUSH_GRACE;
        while tokio::time::Instant::now() < until {
          tokio::time::sleep(PUSH_QUANTUM).await;
          let Some(m) = me.upgrade() else { return };
          if m.remote_msg(&id).is_some() || m.mirror_of(&id).is_none() {
            break;
          }
        }
        // No pushed view in time (an owner from before the bus, or one not running the session): the record copy
        if let Some(m) = me.upgrade()
          && m.remote_msg(&id).is_none()
          && let Some(copy) = m.mirror_of(&id)
        {
          m.emit_session(&copy);
        }
      }
      if await_push
        && let Some(m) = me.upgrade()
        && let Some(tx) = m.state.lock().first_push.remove(&id)
      {
        tx.send_replace(true);
      }
      let mut seen = None;
      loop {
        tokio::time::sleep(MIRROR_POLL).await;
        let Some(m) = me.upgrade() else { return };
        {
          let st = m.state.lock();
          if st.disposed {
            return;
          }
          if st.loading.contains_key(&id) {
            continue;
          }
        }
        // Replaced by the session itself (taken over, deleted): this poll is done
        if m.mirror_of(&id).is_none() {
          m.leases.clear_handover(&id);
          m.end_remote(&id);
          return;
        }
        let watchers = m.viewers_on(&id, None);
        if watchers.is_empty() {
          // Nobody looks at it any more; a later select starts over
          m.state.lock().mirrors.remove(&id);
          m.leases.clear_handover(&id);
          m.end_remote(&id);
          return;
        }
        // Free and nobody else is about to take it (a takeover in flight frees it for the requester, not for this copy).
        // Another machine's copy is never free here
        if !m.state.lock().copies.contains(&id) && !m.leases.held_elsewhere(&id) && !m.leases.takeover_requested(&id) {
          // An engine that cannot read the login (an SSH login's locked keychain) would only stop it on sign-in and then
          // hold its lease: stay read-only, saying why, until the store unlocks (`check_locks` ran when this copy started)
          if let Some(agent) = m.mirror_of(&id).map(|s| s.agent.clone())
            && m.deps.accounts.as_ref().is_some_and(|a| a.credentials_locked(&agent))
          {
            let name = m.registry().try_get(&agent).map(|a| a.name.clone()).unwrap_or(agent);
            let locked = tp("notice.locked.title", &[("agent", &name)]);
            if note != locked {
              note = locked;
              // The owner's last pushed view would otherwise stay on screen, still ready. Subscribed on, so an engine
              // that takes the session later is shown live again
              m.state.lock().remote.remove(&id);
              m.renote_mirror(&id, &note, elsewhere, epoch);
            }
            continue;
          }
          {
            let mut st = m.state.lock();
            if st.loading.contains_key(&id) {
              continue;
            }
            st.mirrors.remove(&id);
          }
          m.leases.clear_handover(&id);
          m.end_remote(&id);
          m.log(&format!("session {id}: the other engine let go, opening it here"));
          // The first watcher loads it (fresh from disk); the rest wait on that load inside select_session_for
          for v in watchers {
            let (m2, id2) = (m.clone(), id.clone());
            tokio::spawn(async move { m2.select_session_for(&v, &id2).await });
          }
          return;
        }
        if elsewhere {
          m.want_handover(&id);
        }
        // Re-read only when the other engine saved since the last look (or its pushed view just ended)
        let forced = m.state.lock().refresh.remove(&id);
        let stamp = tokio::fs::metadata(m.record_path(&id)).await.and_then(|md| md.modified()).ok();
        if stamp.is_none() || (stamp == seen && !forced) {
          continue;
        }
        seen = stamp;
        if let Ok(record) = m.deps.store.load_detailed(&id).await {
          let copy = AcpSession::mirror(record, m.mirror_deps(), note.clone(), m.next_mirror_rev(), elsewhere, epoch);
          let current = {
            let mut st = m.state.lock();
            match st.mirrors.get_mut(&id) {
              Some(slot) => {
                *slot = copy.clone();
                true
              }
              None => false,
            }
          };
          if !current {
            m.end_remote(&id);
            return;
          }
          // The copy still serves reads (diff sources, exports); the screen keeps the pushed view while there is one
          m.emit_session(&copy);
        }
      }
    });
  }

  /// Whether a fresh mirror still waits for its owner's first pushed view: nothing of it goes to a page meanwhile, since
  /// the grace's end shows the record copy to every page on it anyway
  pub(super) fn awaiting_first_push(&self, id: &str) -> bool {
    self.state.lock().first_push.get(id).is_some_and(|tx| !*tx.borrow())
  }

  /// Until the session's mirror has its owner's first pushed view, or its grace ended: a page's first view (init) is then
  /// the owner's live view instead of the read-only record copy it would replace a few milliseconds later. Returns at
  /// once for anything else; a dropped sender (the engine closing) ends the wait too. The push settles it the moment it
  /// lands (`bus_view`), the grace's end otherwise
  pub async fn first_view_settled(&self, id: Option<&str>) {
    let Some(id) = id else { return };
    let rx = self.state.lock().first_push.get(id).map(|tx| tx.subscribe());
    if let Some(mut rx) = rx {
      let _ = rx.wait_for(|done| *done).await;
    }
  }

  /// Swap a mirror for a copy of itself carrying another note, and show it
  fn renote_mirror(&self, id: &str, note: &str, elsewhere: bool, epoch: u64) {
    let Some(old) = self.mirror_of(id) else { return };
    let copy = AcpSession::mirror(old.to_record(), self.mirror_deps(), note.to_owned(), self.next_mirror_rev(), elsewhere, epoch);
    if let Some(slot) = self.state.lock().mirrors.get_mut(id) {
      *slot = copy.clone();
    } else {
      return;
    }
    self.emit_session(&copy);
  }

  /// Keep this engine's handover request for a mirrored session fresh
  fn want_handover(&self, id: &str) {
    // Viewing must not move a working session onto an SSH login that cannot read its credentials. Keep subscribing
    // and forwarding actions to its owner; unlock / refresh can make a later poll eligible for handover again.
    if self.mirror_of(id).is_some_and(|s| self.deps.accounts.as_ref().is_some_and(|a| a.credentials_locked(&s.agent))) {
      self.leases.clear_handover(id);
      return;
    }
    if let Err(e) = self.leases.request_handover(id) {
      self.log(&format!("session {id}: handover request failed: {e}"));
    }
  }
}
