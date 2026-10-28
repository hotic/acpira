//! A live session's changes: its list entry, record, pushes, health and quota, the attention edges a user away
//! from it hears about, the deps a session is built with, and prompts a plugin sends elsewhere

use std::sync::Arc;

use anyhow::{Result, anyhow};

use acpira_shared::inventory::{AgentHealth, HealthSource};
use acpira_shared::transcript::*;

use super::{ATTENTION_SETTLE, Attention, AttentionKind, SessionManager};
use crate::accounts::account_manager::AccountHooks;
use crate::acp::session::{AcpSession, SessionAccountHooks, SessionDeps};
use crate::store::record::RecordSource;
use crate::store::transcript_store::sort_index;

impl SessionManager {
  /// What TS onChange did, for one session: list entry, disk, pushes, health, quota, warm pool
  pub(super) fn process_change(&self, id: &str) {
    let Some(s) = self.live(id) else { return };
    self.learn_shape_of(&s);
    let sum = s.summary();
    {
      let mut st = self.state.lock();
      match st.index.iter().position(|x| x.id == sum.id) {
        Some(i) => st.index[i] = sum,
        None => st.index.insert(0, sum),
      }
      sort_index(&mut st.index);
    }
    self.deps.store.save(s.clone() as Arc<dyn RecordSource>);
    self.save_index();
    let running = s.is_running();
    let idle_edge = {
      let mut st = self.state.lock();
      if running {
        st.was_running.insert(id.to_owned());
        false
      } else {
        st.was_running.remove(id)
      }
    };
    // The lease outlives the turn until its final record is on disk: another engine opening it must read the ended turn.
    // Taken on any change that finds the turn over (not only the observed edge), so a pin is never left behind
    if let Some(pin) = s.take_ended_lease() {
      self.release_after_save(s.clone(), pin);
    }
    // A turn starting or ending moves this session's lease (another engine must not open it mid-turn)
    self.sync_leases();
    // Decided before the list goes out, so the push that shows the turn ending already carries the unread dot. Nobody looked
    // means nobody anywhere: a page of another engine showing it subscribes, which the bus counts as watching
    if idle_edge && self.viewers_on(id, None).is_empty() && !self.bus.watching(id) {
      self.mark_unread(id);
    }
    self.emit_session(&s);
    self.emit_sessions();
    self.watch_attention(&s);
    if let Some(outcome) = s.start_outcome() {
      let mut st = self.state.lock();
      if st.health_seen.get(id) != Some(&outcome) {
        st.health_seen.insert(id.to_owned(), outcome.clone());
        st.health.insert(
          s.agent.clone(),
          AgentHealth { stage: outcome.stage, at: outcome.at, error: outcome.error, source: HealthSource::Session },
        );
      }
    }
    if idle_edge {
      match (s.account_id(), &self.deps.accounts, &self.deps.local_accounts) {
        (Some(acc), Some(m), _) => {
          tokio::spawn(m.refresh_quota(&acc, true));
        }
        (None, _, Some(l)) => {
          let (l, agent) = (l.clone(), s.agent.clone());
          tokio::spawn(async move { l.refresh(Some(&agent), true).await });
        }
        _ => {}
      }
    }
    if s.status() == SessionStatus::Ready {
      self.pool.ensure(&s.agent, &s.cwd, s.account_id().as_deref());
    }
  }

  /// Announce the edges a user away from the session wants to hear about: a card waiting for them (at once), and a turn that
  /// has settled, once it stayed settled for ATTENTION_SETTLE (a queued follow-up or a hook's follow-up keeps it busy).
  /// Background work the agent reported counts as busy too: a turn that ends on "I'll report when it's done" is not done.
  /// A turn that failed is announced without waiting for it, since a long-lived monitor would hold the error back indefinitely.
  /// A turn the user cancelled is not announced. Whether the user is already looking is the shell's call
  fn watch_attention(&self, s: &Arc<AcpSession>) {
    let Some(announce) = self.deps.attention.clone() else { return };
    let id = s.id.clone();
    let waiting = s.list_state() == Some(SummaryState::Waiting);
    let held = |s: &AcpSession| s.turn_busy() || (s.background_running() && s.last_stop() != Some(TurnStop::Error));
    let busy = held(s);
    let (asked, settled) = {
      let mut st = self.state.lock();
      let asked = if waiting {
        st.was_waiting.insert(id.clone())
      } else {
        st.was_waiting.remove(&id);
        false
      };
      let settled = if busy {
        st.was_busy.insert(id.clone());
        None
      } else if st.was_busy.remove(&id) {
        let seq = st.settle_seq.entry(id.clone()).or_default();
        *seq += 1;
        Some(*seq)
      } else {
        None
      };
      (asked, settled)
    };
    let agent = self.registry().try_get(&s.agent).map(|d| d.name.clone()).unwrap_or_else(|| s.agent.clone());
    if asked {
      announce(Attention { session_id: id.clone(), title: s.title(), agent: agent.clone(), kind: AttentionKind::Waiting });
    }
    let Some(seq) = settled else { return };
    let me = self.me.clone();
    tokio::spawn(async move {
      tokio::time::sleep(ATTENTION_SETTLE).await;
      let Some(m) = me.upgrade() else { return };
      let Some(s) = m.live(&id) else { return };
      // Another turn began meanwhile (it announces itself when it settles), or the session now waits on the user
      if m.state.lock().settle_seq.get(&id) != Some(&seq)
        || held(&s)
        || s.list_state() == Some(SummaryState::Waiting)
      {
        return;
      }
      let kind = match s.last_stop() {
        Some(TurnStop::Error) => AttentionKind::Error,
        Some(TurnStop::Cancelled) | None => return,
        Some(_) => AttentionKind::Done,
      };
      announce(Attention { session_id: id, title: s.title(), agent, kind });
    });
  }

  pub(super) fn session_deps(&self) -> SessionDeps {
    let me = self.me.clone();
    let shapes_me = self.me.clone();
    let claim_me = self.me.clone();
    let toast = self.deps.toast.clone();
    let compaction = self.deps.compaction.clone();
    let plan_auto = self.deps.plan_auto_approve.clone();
    SessionDeps {
      registry: self.registry(),
      log: self.deps.log.clone(),
      on_change: Arc::new(move |id: &str, running: bool| {
        if let Some(m) = me.upgrade() {
          m.mark_dirty(id, running);
        }
      }),
      blobs: self.deps.store.clone(),
      notify: Some(Arc::new(move |text: &str| toast("info", text))),
      accounts: self.deps.accounts.clone().map(|a| Arc::new(AccountHooks(a)) as Arc<dyn SessionAccountHooks>),
      compaction: Some(Arc::new(move || compaction())),
      plan_auto_approve: Some(Arc::new(move |agent: &str| plan_auto().iter().any(|a| a == agent))),
      pool: Some(self.pool.clone()),
      host_mcp: self.deps.host_mcp.clone(),
      model_shapes: Some(Arc::new(move |agent: &str| {
        shapes_me.upgrade().and_then(|m| m.state.lock().prefs.model_shapes.as_ref().and_then(|s| s.get(agent).cloned()))
      })),
      shared_mcp: self.deps.shared_mcp.clone(),
      plugin_hooks: self.deps.plugin_hooks.clone(),
      plugin_prompts: self.deps.plugin_prompts.clone(),
      plugin_runtime: self.deps.plugin_runtime.clone(),
      claim: Some(Arc::new(move |id: &str| claim_me.upgrade().map_or(Ok(0), |m| m.claim_turn(id)))),
      remote: self.deps.remote.clone(),
    }
  }

  /// A prompt a plugin sent elsewhere (`plugin_hooks::Redirect`): a new session of that agent in the same folder gets it,
  /// and the windows that showed the source session move to it. The new session's id
  pub(super) async fn redirect_prompt(self: &Arc<Self>, r: crate::acp::session::plugin_hooks::Redirect) -> Result<String> {
    if self.registry().get(&r.agent).is_err() {
      return Err(anyhow!("no agent `{}`", r.agent));
    }
    let v = self.attach(None);
    let made = self.new_session_for(&v, Some(r.agent.clone()), None, None, Some(r.cwd.clone())).await;
    let id = v.active_id();
    self.detach(&v);
    made?;
    let id = id.ok_or_else(|| anyhow!("no session started"))?;
    let s = self.live_session(&id).ok_or_else(|| anyhow!("session {id} is gone"))?;
    self.log(&format!("plugin {}: prompt of {} sent to {} ({id})", r.plugin, r.from, r.agent));
    for w in self.viewers_on(&r.from, None) {
      self.select_session_for(&w, &id).await;
    }
    s.mark_redirected();
    tokio::spawn(async move { s.prompt(r.text, r.drafts, false, None, None).await });
    Ok(id)
  }
}
