//! Viewers and pushes: the session list and views each page gets, attaching and detaching pages, mirrored sessions'
//! pushed views and forwarded actions, subagent subscriptions, and the flush that turns dirty sessions into pushes

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::protocol::{HostMsg, RawJson, WebviewMsg};
use acpira_shared::settings::in_workspace;
use acpira_shared::transcript::*;

use super::dispatch::list_edit_target;
use super::{PUSH_QUANTUM, SessionManager, Viewer, ViewerState, is_forwarded, session_id_of};
use crate::acp::session::AcpSession;
use crate::acp::session::errors::error_text;
use crate::i18n::t;
use crate::store::transcript_store::sort_index;

/// Dirty sessions become pushes: the first change goes out at once, later ones within a quantum coalesce
pub(super) async fn flush_loop(weak: Weak<SessionManager>) {
  loop {
    let Some(m) = weak.upgrade() else { return };
    let wake = m.wake.clone();
    let notified = wake.notified();
    tokio::pin!(notified);
    notified.as_mut().enable();
    let batch: Vec<(String, bool)> = m.state.lock().dirty.drain().collect();
    if batch.is_empty() {
      drop(m);
      notified.await;
      continue;
    }
    for (id, _) in &batch {
      m.process_change(id);
    }
    drop(m);
    tokio::time::sleep(PUSH_QUANTUM).await;
  }
}

impl SessionManager {
  pub fn sessions(&self) -> Vec<SessionSummary> {
    let (index, live, unread, elsewhere, deleting, trash) = {
      let st = self.state.lock();
      // Run by another engine: its entry as that engine announced it (one owner per session, so ids never collide)
      let elsewhere: HashMap<String, SessionSummary> = st.elsewhere.values().flatten().map(|s| (s.id.clone(), s.clone())).collect();
      (st.index.clone(), st.live.clone(), st.unread.clone(), elsewhere, st.deleting.clone(), st.trash.keys().cloned().collect::<HashSet<_>>())
    };
    let mark = |mut s: SessionSummary, live_state: Option<SummaryState>| {
      let seen = !unread.contains(&s.id);
      s.state = live_state.or_else(|| (!seen).then_some(SummaryState::Unread));
      // A reply that came in while background work runs: the ring brightens instead of the finished dot showing
      s.unread = (live_state == Some(SummaryState::Background) && !seen).then_some(true);
      if s.state != Some(SummaryState::Waiting) {
        s.question = None;
      }
      s
    };
    let mut merged: Vec<SessionSummary> = index
      .into_iter()
      .map(|s| match (live.get(&s.id), elsewhere.get(&s.id)) {
        (Some(l), _) => {
          let (state, question) = l.list_marks();
          // The index entry is written with the record; the live session knows a turn's end first
          let s = SessionSummary { question: question.then_some(true), completed_at: l.completed_at(), ..s };
          mark(s, state)
        }
        (None, Some(e)) => mark(e.clone(), e.state),
        (None, None) => mark(s, None),
      })
      .collect();
    // Sessions another engine started since this list last read the directory
    let known: HashSet<String> = merged.iter().map(|s| s.id.clone()).collect();
    for e in elsewhere.values().filter(|e| !known.contains(&e.id) && !live.contains_key(&e.id)) {
      merged.push(mark(e.clone(), e.state));
    }
    // A handover drops the old owner's announcement before the requesting engine finishes the disk move.
    for snapshot in deleting.into_values() {
      match merged.iter().position(|s| s.id == snapshot.id) {
        Some(i) => merged[i] = snapshot,
        None => merged.push(snapshot),
      }
    }
    merged.retain(|s| !trash.contains(&s.id));
    if let Some(c) = &self.deps.chatgpt {
      merged.extend(c.summaries());
    }
    let hosted = self.host_summaries(&merged);
    merged.extend(hosted);
    sort_index(&mut merged);
    merged
  }

  /// The serialized view of a session (live, or a ChatGPT mirror)
  pub fn view_of(&self, id: Option<&str>) -> Option<(RawJson, bool)> {
    let id = id?;
    if let Some(HostMsg::Session { session, running, .. }) = self.host_msg(id) {
      return Some((session, running));
    }
    if self.live(id).is_none()
      && let Some(HostMsg::Session { session, running, .. }) = self.remote_msg(id)
    {
      return Some((session, running));
    }
    if let Some(s) = self.live(id).or_else(|| self.mirror_of(id)) {
      return Some(s.view_json());
    }
    let view = self.deps.chatgpt.as_ref()?.view(id)?;
    Some((RawJson::new(&view), view.running))
  }

  /// The session push for one viewer, as `emit_session` would send it
  pub fn session_msg(&self, id: Option<&str>) -> Option<HostMsg> {
    let id = id?;
    if let Some(m) = self.host_msg(id) {
      return Some(m);
    }
    if self.live(id).is_none()
      && let Some(m) = self.remote_msg(id)
    {
      return Some(m);
    }
    if let Some(s) = self.live(id).or_else(|| self.mirror_of(id)) {
      return Some(s.view_msg());
    }
    let (session, running) = self.view_of(Some(id))?;
    Some(HostMsg::Session { session, running, parts: None })
  }

  pub(super) fn has_view(&self, id: &str) -> bool {
    self.host_msg(id).is_some() || self.live(id).is_some() || self.mirror_of(id).is_some() || self.deps.chatgpt.as_ref().is_some_and(|c| c.view(id).is_some())
  }

  pub(super) fn most_recent(&self) -> Option<String> {
    self.most_recent_except(None)
  }

  pub(super) fn most_recent_except(&self, except: Option<&str>) -> Option<String> {
    let scope = (self.deps.scope)();
    let cwd = (self.deps.cwd)();
    let deleting: HashSet<String> = self.state.lock().deleting.keys().cloned().collect();
    self.sessions().into_iter()
      .find(|s| except != Some(s.id.as_str()) && !deleting.contains(&s.id) && (scope == "all" || in_workspace(&s.cwd, &cwd)))
      .map(|s| s.id)
  }

  /// Attach a viewer: `initial` is the session it opens on, or the newest listed one
  pub fn attach(&self, initial: Option<acpira_shared::sidecar::InitialView>) -> Arc<Viewer> {
    use acpira_shared::sidecar::InitialView;
    let empty = matches!(initial, Some(InitialView::Empty { empty: true }));
    let id = match initial {
      Some(InitialView::Id(id)) => Some(id),
      Some(InitialView::MostRecent { most_recent: true }) => self.most_recent(),
      _ => None,
    };
    if let Some(id) = &id {
      self.mark_read(id);
    }
    let mut st = self.state.lock();
    let v = Arc::new(Viewer {
      id: self.viewer_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
      state: parking_lot::Mutex::new(ViewerState { active_id: id, empty, ..Default::default() }),
      sink: Default::default(),
    });
    st.viewers.push(v.clone());
    v
  }

  pub fn detach(&self, v: &Viewer) {
    *v.sink.lock() = None;
    self.state.lock().viewers.retain(|x| x.id != v.id);
    self.prune_host_views();
    self.sync_node_subs();
  }

  pub(super) fn viewers(&self) -> Vec<Arc<Viewer>> {
    self.state.lock().viewers.clone()
  }

  pub fn emit(&self, m: HostMsg) {
    for v in self.viewers() {
      v.emit(m.clone());
    }
  }

  pub(super) fn emit_session(&self, s: &AcpSession) {
    // A mirrored session whose owner pushes its view: that view, not the record copy
    if self.live(&s.id).is_none()
      && let Some(m) = self.remote_msg(&s.id)
    {
      for v in self.viewers_on(&s.id, None) {
        v.emit(m.clone());
      }
      return;
    }
    let mut view: Option<HostMsg> = None;
    for v in self.viewers() {
      if v.active_id().as_deref() != Some(s.id.as_str()) {
        continue;
      }
      v.emit(view.get_or_insert_with(|| s.view_msg()).clone());
      let obs = v.state.lock().observing.clone();
      let Some((sid, sub)) = obs.filter(|(sid, _)| *sid == s.id) else { continue };
      if let Some((turns, rev, running)) = s.subagent_transcript(&sub) {
        let fresh = {
          let mut vs = v.state.lock();
          let fresh = vs.last_subagent_rev != Some(rev);
          if fresh {
            vs.last_subagent_rev = Some(rev);
          }
          fresh
        };
        if fresh {
          v.emit(HostMsg::Subagent { session_id: sid, subagent_id: sub, rev, running, turns });
        }
      }
    }
    let owned = self.live(&s.id).is_some_and(|l| std::ptr::eq(&*l, s));
    // Shown by another engine too: the same view goes out over the bus
    if owned
      && self.bus.watching(&s.id)
      && let HostMsg::Session { parts: Some(p), .. } = view.unwrap_or_else(|| s.view_msg())
    {
      self.bus.publish(&s.id, &p);
    }
    // And the subagents its pages observe
    if owned {
      let nodes: Vec<String> = self.state.lock().node_watch.keys().filter(|(sid, _)| *sid == s.id).map(|(_, n)| n.clone()).collect();
      for node in nodes {
        self.publish_node(s, &node);
      }
    }
  }

  /// Owner: a watched subagent's transcript goes to the engines observing it when its rev moved since the last send
  pub(super) fn publish_node(&self, s: &AcpSession, node: &str) {
    let Some((turns, rev, running)) = s.subagent_transcript(node) else { return };
    {
      let mut st = self.state.lock();
      match st.node_watch.get_mut(&(s.id.clone(), node.to_owned())) {
        Some(last) if *last != Some(rev) => *last = Some(rev),
        _ => return,
      }
    }
    self.bus.node_publish(&s.id, node, rev, running, turns.0.as_ref().to_owned());
  }

  /// The subagents this engine's pages observe on sessions another engine owns and pushes
  pub(super) fn wanted_nodes(&self) -> HashSet<(String, String)> {
    let observed: Vec<(String, String)> = self.viewers().iter().filter_map(|v| v.state.lock().observing.clone()).collect();
    let st = self.state.lock();
    observed.into_iter().filter(|(sid, _)| !st.live.contains_key(sid) && st.mirrors.contains_key(sid)).collect()
  }

  /// Bring the bus's subagent subscriptions in line with what the pages here observe (cheap: called on every move)
  pub(super) fn sync_node_subs(&self) {
    if !self.deps.bus {
      return;
    }
    let wanted = self.wanted_nodes();
    let (add, gone) = {
      let mut st = self.state.lock();
      if st.node_subs == wanted {
        return;
      }
      let add: Vec<(String, String)> = wanted.difference(&st.node_subs).cloned().collect();
      let gone: Vec<(String, String)> = st.node_subs.difference(&wanted).cloned().collect();
      for k in &gone {
        st.remote_nodes.remove(k);
      }
      st.node_subs = wanted;
      (add, gone)
    };
    for (s, n) in add {
      self.bus.node_subscribe(&s, &n);
    }
    for (s, n) in gone {
      self.bus.node_unsubscribe(&s, &n);
    }
  }

  /// The pushed view of a mirrored session, once its owner sent one. Only while the mirror is there: a deletion or a
  /// reconcile that drops the mirror ends the pushed view with it, before the mirror's poll unsubscribes
  pub(super) fn remote_msg(&self, id: &str) -> Option<HostMsg> {
    let st = self.state.lock();
    if !st.mirrors.contains_key(id) {
      return None;
    }
    st.remote.get(id).and_then(|r| r.msg.clone())
  }

  pub(super) fn next_mirror_rev(&self) -> i64 {
    self.mirror_rev.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1
  }

  /// A mirror is no longer shown or no longer a mirror: its owner stops pushing to this engine
  pub(super) fn end_remote(&self, id: &str) {
    self.state.lock().remote.remove(id);
    self.bus.unsubscribe(id);
    self.sync_node_subs();
  }

  /// A page's action on a session another engine owns and pushes: run there
  pub(super) async fn forward(&self, id: &str, m: &WebviewMsg) -> Result<()> {
    let msg = serde_json::to_value(m)?;
    self.bus.act(id, msg).await.map_err(|e| anyhow!(e))
  }

  /// Run an action a page of another engine took on a session this one owns, as if from a page showing it here
  pub(super) async fn run_forwarded(self: Arc<Self>, session: String, msg: Value) -> Result<(), String> {
    let m: WebviewMsg = serde_json::from_value(msg).map_err(|e| e.to_string())?;
    let target = session_id_of(&m).or_else(|| list_edit_target(&m).map(str::to_owned));
    if !(is_forwarded(&m) || list_edit_target(&m).is_some()) || target.is_some_and(|id| id != session) {
      return Err(format!("not an action on {session}"));
    }
    if self.live(&session).is_none() {
      return Err(t("history.unavailable"));
    }
    // A page of its own that nobody sees: not in the viewer list, so it neither receives pushes nor keeps the session here
    let v = Arc::new(Viewer {
      id: self.viewer_seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
      state: parking_lot::Mutex::new(ViewerState { active_id: Some(session), ..Default::default() }),
      sink: Default::default(),
    });
    self.dispatch(&v, m).await.map_err(|e| error_text(&e))
  }

  /// Pages that join later get the list in their init state, so skipping an unchanged list leaves none behind. Built and
  /// posted under one lock: two racing callers can never post an older list after a newer one
  pub(super) fn emit_sessions(&self) {
    // Every change that can move a list entry comes through here: the other engines hear about it too (unchanged: not sent)
    self.bus.announce_live(self.own_live());
    let mut last = self.last_sessions.lock();
    let sessions = self.sessions();
    if last.as_ref() == Some(&sessions) {
      return;
    }
    *last = Some(sessions.clone());
    self.emit(HostMsg::Sessions { sessions });
  }

  /// The current list for one page, taken under the same lock as `emit_sessions`: whatever `send` queues lands after any
  /// list already emitted and before any later one, so the page cannot end on an older list that the next emit skips
  pub fn sessions_in_order(&self, send: impl FnOnce(Vec<SessionSummary>)) {
    let _order = self.last_sessions.lock();
    send(self.sessions());
  }

  /// Landing on a session also reads it; callers push the session list afterwards
  pub(super) fn set_active(&self, v: &Viewer, id: Option<String>) {
    if let Some(id) = &id {
      self.mark_read(id);
    }
    {
      let mut st = v.state.lock();
      if st.active_id != id {
        st.observing = None;
        st.last_subagent_rev = None;
      }
      st.active_id = id;
    }
    self.sync_node_subs();
  }

  pub(super) fn viewers_on(&self, id: &str, except: Option<&Viewer>) -> Vec<Arc<Viewer>> {
    self.viewers().into_iter().filter(|v| except.is_none_or(|e| e.id != v.id) && v.active_id().as_deref() == Some(id)).collect()
  }

  pub(crate) fn mark_dirty(&self, id: &str, running: bool) {
    let mut st = self.state.lock();
    let prev = st.dirty.insert(id.to_owned(), running);
    // Every change reports here, so a turn shorter than one flush quantum still leaves its running edge for the idle check
    if running {
      st.was_running.insert(id.to_owned());
      st.was_busy.insert(id.to_owned());
    }
    drop(st);
    // A fresh entry or an idle edge wakes the flusher at once
    if prev.is_none() || !running {
      self.wake.notify_one();
    }
  }
}
