//! This engine on the push bus (`bus.rs`): the views and list entries it owns and posts, the ones it watches, and the
//! actions other engines forward to it

use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;
use serde_json::value::RawValue;

use acpira_shared::protocol::{HostMsg, RawJson};
use acpira_shared::transcript::*;

use super::{Remote, SessionManager, remote_view};
use crate::acp::transport::rpc::BoxFuture;
use crate::bus::{BusPeer, ViewFrame, apply_frame};
use crate::store::transcript_store::is_session_id;

impl BusPeer for SessionManager {
  fn bus_watch(&self, session: &str, on: bool) {
    // A new subscriber: the bus sends the next view whole, so push the current one now rather than at the next change
    if on
      && let Some(s) = self.live(session)
      && let HostMsg::Session { parts: Some(p), .. } = s.view_msg()
    {
      self.bus.publish(session, &p);
    }
  }

  fn bus_view(&self, session: &str, frame: ViewFrame) {
    let built = {
      let mut st = self.state.lock();
      // Only for a session still shown read-only here
      if !st.mirrors.contains_key(session) || st.live.contains_key(session) {
        return;
      }
      let r = st.remote.entry(session.to_owned()).or_insert_with(|| Remote {
        epoch: crate::acp::session::next_view_epoch(),
        parts: None,
        msg: None,
      });
      match apply_frame(r.parts.as_ref().map(|p| &*p.0), &frame) {
        None => None,
        Some((turns, last)) => {
          let rev = self.next_mirror_rev();
          remote_view(session, &frame.head, rev, r.epoch, turns, last).map(|(parts, msg)| {
            r.parts = Some(parts);
            r.msg = Some(msg.clone());
            msg
          })
        }
      }
    };
    match built {
      Some(msg) => {
        for v in self.viewers_on(session, None) {
          v.emit(msg.clone());
        }
      }
      // A step this engine cannot apply: subscribing again makes the owner send the whole view
      None => {
        self.log(&format!("session {session}: pushed view out of step, asking for the whole view"));
        self.bus.subscribe(session);
      }
    }
  }

  fn bus_gone(&self, session: &str) {
    let mut st = self.state.lock();
    if st.remote.remove(session).is_some() {
      // Back to the record copy: re-read it now, its rev counts on from the pushed views
      st.refresh.insert(session.to_owned());
    }
  }

  fn bus_act(&self, session: String, msg: Value) -> BoxFuture<Result<(), String>> {
    let me = self.me.clone();
    Box::pin(async move {
      let Some(m) = me.upgrade() else { return Err("engine closing".into()) };
      m.run_forwarded(session, msg).await
    })
  }

  fn bus_owned(&self) -> Vec<String> {
    self.leases.held()
  }

  fn bus_wanted(&self) -> Vec<String> {
    self.state.lock().mirrors.keys().cloned().collect()
  }

  fn bus_wanted_nodes(&self) -> Vec<(String, String)> {
    let wanted = self.wanted_nodes();
    self.state.lock().node_subs = wanted.clone();
    wanted.into_iter().collect()
  }

  fn bus_own_live(&self) -> Vec<Value> {
    self.own_live()
  }

  fn bus_live(&self, peer: u64, summaries: Vec<Value>) {
    let parsed: Vec<SessionSummary> = summaries.into_iter().filter_map(parse_live_summary).collect();
    let dropped = {
      let mut st = self.state.lock();
      let before = st.elsewhere.get(&peer);
      if before.map_or(parsed.is_empty(), |b| *b == parsed) {
        return;
      }
      let dropped = before.is_some_and(|b| b.iter().any(|s| !parsed.iter().any(|p| p.id == s.id)));
      if parsed.is_empty() {
        st.elsewhere.remove(&peer);
      } else {
        st.elsewhere.insert(peer, parsed);
      }
      dropped
    };
    // A session the other engine let go: this list falls back to its own entry, read again from the directory (the owner
    // wrote the record before letting go)
    if dropped {
      self.save_index();
    }
    self.emit_sessions();
  }

  fn bus_touch(&self, what: &str) {
    let Some(me) = self.me.upgrade() else { return };
    let what = what.to_owned();
    tokio::spawn(async move {
      match what.as_str() {
        "unread" => me.reload_unread().await,
        "categories" => me.sync_categories().await,
        "index" => me.sync_index().await,
        other => {
          let hooks: Vec<_> = me.touch_hooks.lock().iter().filter(|(w, _)| *w == other).map(|(_, f)| f.clone()).collect();
          for f in hooks {
            f();
          }
        }
      }
    });
  }

  fn bus_node_watch(&self, session: &str, node: &str, on: bool) {
    let key = (session.to_owned(), node.to_owned());
    if !on {
      self.state.lock().node_watch.remove(&key);
      return;
    }
    // A new observer: the transcript goes out whole now, whatever was sent before
    self.state.lock().node_watch.insert(key, None);
    if let Some(s) = self.live(session) {
      // A restored Claude workflow agent without a process reads its sidechain log once (`workflow_logs.rs`)
      s.observe_subagent(node);
      self.publish_node(&s, node);
    }
  }

  fn bus_node_view(&self, session: &str, node: &str, rev: i64, running: bool, turns: Box<RawValue>) {
    let key = (session.to_owned(), node.to_owned());
    let turns = RawJson(Arc::from(turns));
    {
      let mut st = self.state.lock();
      if st.live.contains_key(session) || !st.node_subs.contains(&key) {
        return;
      }
      st.remote_nodes.insert(key.clone(), (rev, running, turns.clone()));
    }
    for v in self.viewers_on(session, None) {
      let fresh = {
        let mut vs = v.state.lock();
        let fresh = vs.observing.as_ref() == Some(&key) && vs.last_subagent_rev != Some(rev);
        if fresh {
          vs.last_subagent_rev = Some(rev);
        }
        fresh
      };
      if fresh {
        v.emit(HostMsg::Subagent { session_id: session.to_owned(), subagent_id: node.to_owned(), rev, running, turns: turns.clone() });
      }
    }
  }

  fn bus_reset(&self) {
    let had = {
      let mut st = self.state.lock();
      st.node_watch.clear();
      // Sent again to the next router (`bus_wanted_nodes`)
      st.node_subs.clear();
      !std::mem::take(&mut st.elsewhere).is_empty()
    };
    if had {
      self.save_index();
      self.emit_sessions();
    }
  }
}

/// A list entry another engine announced: a state this engine does not know (a newer engine's) is dropped rather than the
/// whole entry, and the unread flag is this engine's own call
fn parse_live_summary(mut v: Value) -> Option<SessionSummary> {
  let state = v.as_object_mut().and_then(|o| o.remove("state"));
  let mut s: SessionSummary = serde_json::from_value(v).ok()?;
  s.state = state.and_then(|x| serde_json::from_value(x).ok());
  s.unread = None;
  is_session_id(&s.id).then_some(s)
}

impl SessionManager {
  /// The list entries of the sessions open here as the bus carries them, each with its state, in a stable order
  pub(super) fn own_live(&self) -> Vec<Value> {
    let mut live: Vec<(String, Value)> = self
      .live_sessions()
      .into_iter()
      .filter_map(|s| {
        let mut sum = s.summary();
        let (state, question) = s.list_marks();
        sum.state = state;
        sum.question = question.then_some(true);
        serde_json::to_value(&sum).ok().map(|v| (s.id.clone(), v))
      })
      .collect();
    live.sort_by(|a, b| a.0.cmp(&b.0));
    live.into_iter().map(|(_, v)| v).collect()
  }
}
