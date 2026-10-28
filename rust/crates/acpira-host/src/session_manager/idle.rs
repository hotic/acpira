//! Agent processes given back: idle sessions nobody looks at release theirs on a sweep, and shutdown ends them all

use std::collections::HashSet;
use std::sync::{Arc, Weak};
use std::time::Duration;

use super::{DISPOSE_GRACE, IDLE_KEEP, IDLE_KEEP_MIN, IDLE_RELEASE, IDLE_SWEEP, SessionManager, Trash};
use crate::acp::session::AcpSession;
use crate::store::record::RecordSource;
use crate::store::session_lease::Claim;

/// Every IDLE_SWEEP, background sessions with nothing in flight give their agent process back (`release_idle`)
pub(super) async fn idle_loop(weak: Weak<SessionManager>) {
  loop {
    tokio::time::sleep(IDLE_SWEEP).await;
    let Some(m) = weak.upgrade() else { return };
    if m.state.lock().disposed {
      return;
    }
    m.release_idle(IDLE_RELEASE, IDLE_KEEP).await;
  }
}

impl SessionManager {
  /// End the agent processes of sessions nobody looks at: a live session no viewer shows, with nothing in flight
  /// (`AcpSession::releasable`), goes once it has been like that for `after`; beyond the `keep` most recently seen ones,
  /// once it has been like that for IDLE_KEEP_MIN (or `after`, when shorter). Its record is written, its lease released
  /// and the list entry stays; opening it again resumes the native session like after a window reload and replays its
  /// controls (`load_session`). Returns the ids released
  pub async fn release_idle(self: &Arc<Self>, after: Duration, keep: usize) -> Vec<String> {
    let now = tokio::time::Instant::now();
    let live = self.live_sessions();
    let shown: HashSet<String> = self.viewers().iter().filter_map(|v| v.active_id()).collect();
    let yielding = self.yielding.lock().clone();
    // Read outside the manager lock: a session reports its changes into the manager while holding its own lock
    let quiet: Vec<(Arc<AcpSession>, bool)> = live
      .into_iter()
      .map(|s| {
        let quiet = !shown.contains(&s.id) && !yielding.contains(&s.id) && s.releasable();
        (s, quiet)
      })
      .collect();
    let mut idle: Vec<(tokio::time::Instant, Arc<AcpSession>)> = vec![];
    {
      let mut st = self.state.lock();
      if st.disposed {
        return vec![];
      }
      let ids: HashSet<String> = st.live.keys().cloned().collect();
      st.idle_since.retain(|id, _| ids.contains(id));
      for (s, quiet) in quiet {
        if quiet {
          idle.push((*st.idle_since.entry(s.id.clone()).or_insert(now), s));
        } else {
          st.idle_since.remove(&s.id);
        }
      }
    }
    // Most recently seen first: the first `keep` of them only go after the full wait
    idle.sort_by_key(|(since, _)| std::cmp::Reverse(*since));
    let mut released = vec![];
    for (i, (since, s)) in idle.into_iter().enumerate() {
      let unseen = now.duration_since(since);
      if (unseen >= after || (i >= keep && unseen >= IDLE_KEEP_MIN.min(after))) && self.release(&s).await {
        released.push(s.id.clone());
      }
    }
    if !released.is_empty() {
      self.sync_leases();
      self.emit_sessions();
    }
    released
  }

  /// Close one idle session's agent process; false when a viewer landed on it or it changed meanwhile. Until the old
  /// process has closed the native session and exited, this engine keeps the lease (no other engine resumes it) and a
  /// reopen here waits (`load_session`)
  pub(super) async fn release(self: &Arc<Self>, s: &Arc<AcpSession>) -> bool {
    if let Err(e) = self.deps.store.flush(s.clone() as Arc<dyn RecordSource>).await {
      self.log(&format!("session {}: idle release skipped, save failed: {e}", s.id));
      return false;
    }
    if !s.releasable() {
      return false;
    }
    let (closed_tx, closed_rx) = tokio::sync::watch::channel(false);
    {
      // A viewer selecting the session sets its active id before it looks the session up here: either this check sees
      // the viewer, or the lookup finds the session gone and loads it from the record just written
      let mut st = self.state.lock();
      let current = st.live.get(&s.id).is_some_and(|l| Arc::ptr_eq(l, s));
      if !current || st.viewers.iter().any(|v| v.active_id().as_deref() == Some(s.id.as_str())) {
        return false;
      }
      st.live.remove(&s.id);
      st.was_running.remove(&s.id);
      st.was_busy.remove(&s.id);
      st.was_waiting.remove(&s.id);
      st.settle_seq.remove(&s.id);
      st.health_seen.remove(&s.id);
      st.dirty.remove(&s.id);
      st.idle_since.remove(&s.id);
      st.closing.insert(s.id.clone(), closed_rx);
    }
    // A pin of its own holds the lease across the close; the session's turn pin (if any) can go now
    let close_pin = match self.leases.claim(&s.id) {
      Claim::Held(pin) => Some(pin),
      _ => None,
    };
    if let Some(pin) = s.drop_lease() {
      self.leases.release(&s.id, pin);
    }
    let (_, closing) = s.shutdown();
    let me = self.clone();
    let id = s.id.clone();
    tokio::spawn(async move {
      if let Some(f) = closing {
        f.await;
      }
      if let Some(pin) = close_pin {
        me.leases.release(&id, pin);
      }
      me.state.lock().closing.remove(&id);
      let _ = closed_tx.send(true);
    });
    self.log(&format!("session {}: idle, agent process released", s.id));
    true
  }

  pub async fn dispose(self: &Arc<Self>) {
    self.installs.cancel_all();
    self.bus.stop();
    self.stop_hosts();
    {
      let mut st = self.state.lock();
      st.disposed = true;
      if let Some(h) = st.probe_timer.take() {
        h.abort();
      }
    }
    if let Some(c) = &self.deps.chatgpt {
      c.dispose().await;
    }
    // Agent processes are ended before the host returns (it exits right after): each session closes and kills its process, the pool
    // kills its idle ones, all at once; whatever has not exited within DISPOSE_GRACE is SIGKILLed so nothing outlives the host
    let live: Vec<Arc<AcpSession>> = self.state.lock().live.drain().map(|(_, s)| s).collect();
    let mut procs = Vec::new();
    let mut exits = tokio::task::JoinSet::new();
    for s in live {
      let (proc, closing) = s.shutdown();
      procs.extend(proc);
      if let Some(f) = closing {
        exits.spawn(f);
      }
      if let Err(e) = self.deps.store.flush(s as Arc<dyn RecordSource>).await {
        self.log(&format!("dispose flush failed: {e}"));
      }
    }
    let (pooled, pool_exit) = self.pool.dispose();
    exits.spawn(pool_exit);
    if tokio::time::timeout(DISPOSE_GRACE, async { while exits.join_next().await.is_some() {} }).await.is_err() {
      self.log("agent processes still running at shutdown; killing them");
      procs.extend(pooled.lock().iter().cloned());
      for p in procs {
        p.kill_hard();
      }
    }
    let trash: Vec<(String, Trash)> = self.state.lock().trash.drain().collect();
    for (id, t) in trash {
      t.timer.abort();
      self.deps.store.remove(&id).await;
    }
    self.state.lock().mirrors.clear();
    self.leases.release_all();
    let saves: Vec<_> = self.prefs_saves.lock().drain(..).collect();
    for h in saves {
      let _ = h.await;
    }
    self.state.lock().sync_due = None;
    self.deps.store.dispose().await;
    self.sync_index().await;
  }
}
