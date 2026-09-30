//! Context usage snapshots for agents that never send `usage_update`: Grok's `_x.ai/session/info` and Pi's session file
//! are polled while a prompt is on the wire; Kimi's late snapshot is awaited after end_turn

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use acpira_shared::transcript::*;

use crate::acp::session::{AcpSession, Core, num};
use crate::acp::vendors::pi_usage;

pub const USAGE_POLL_INTERVAL: Duration = Duration::from_millis(800);

impl AcpSession {
  /// Agents that never send `usage_update` and have their context snapshot polled instead: Grok over
  /// `_x.ai/session/info`, Pi from its session file (`pi_usage`)
  fn polls_usage(&self, c: &Core) -> bool {
    !c.usage_notifications && (self.agent == "pi" || (self.agent == "grok" && !c.grok_usage_unavailable))
  }

  /// Refresh before settling a turn so auto-compaction sees the current window; Grok only fills context.used after
  /// a model round and Pi's file grows per reply, so both are polled while a prompt is on the wire
  pub(crate) async fn refresh_context_usage(self: &Arc<Self>) {
    clear_usage_timer(&mut self.core.lock());
    let _serial = self.usage_lock.lock().await;
    self.core.lock().usage_inflight = true;
    match self.agent.as_str() {
      "grok" => self.read_grok_usage().await,
      "pi" => self.read_pi_usage().await,
      _ => {}
    }
    self.core.lock().usage_inflight = false;
  }

  async fn read_pi_usage(self: &Arc<Self>) {
    let (sid, revision, stamp) = {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready || !self.polls_usage(&c) {
        return;
      }
      let Some(sid) = c.acp_session_id.clone() else { return };
      c.usage_revision += 1;
      (sid, c.usage_revision, c.pi_stamp.clone())
    };
    let (cwd, id) = (self.cwd.clone(), sid.clone());
    let Ok(read) = tokio::task::spawn_blocking(move || pi_usage::read(&cwd, &id, stamp.as_ref())).await else { return };
    let pi_usage::Snapshot::Fresh(stamp, usage) = read else { return };
    let mut c = self.core.lock();
    if c.acp_session_id.as_deref() != Some(sid.as_str()) || c.status != SessionStatus::Ready || c.usage_revision != revision {
      return;
    }
    c.pi_stamp = Some(stamp);
    self.apply_context_usage(&mut c, usage);
  }

  async fn read_grok_usage(self: &Arc<Self>) {
    let (proc, sid, revision) = {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready || !self.polls_usage(&c) {
        return;
      }
      let (Some(proc), Some(sid)) = (c.proc.clone(), c.acp_session_id.clone()) else { return };
      c.usage_revision += 1;
      (proc, sid, c.usage_revision)
    };
    let r = tokio::time::timeout(Duration::from_secs(5), proc.request("_x.ai/session/info", json!({ "sessionId": sid }))).await;
    let usage = match r {
      Ok(Ok(v)) => grok_context_usage(&v, &sid),
      Ok(Err(e)) => {
        if e.code == -32601 {
          self.core.lock().grok_usage_unavailable = true;
        }
        self.log(&format!("context unavailable: {e}"));
        return;
      }
      Err(_) => {
        self.log("context unavailable: Grok context request timed out");
        return;
      }
    };
    let mut c = self.core.lock();
    if !c.proc.as_ref().is_some_and(|p| Arc::ptr_eq(p, &proc))
      || c.acp_session_id.as_deref() != Some(sid.as_str())
      || c.status != SessionStatus::Ready
      || c.usage_revision != revision
    {
      return;
    }
    self.apply_context_usage(&mut c, usage);
  }

  /// A polled snapshot becomes the session's usage and the context mark of the turn it follows
  fn apply_context_usage(&self, c: &mut Core, usage: Option<Usage>) {
    let prev = c.state.usage;
    c.state.usage = usage;
    if let (Some(u), Some(Turn::Agent(last))) = (usage, c.state.turns.last_mut()) {
      last.usage.get_or_insert_with(Default::default).context = Some(ContextUse { used: u.used, size: u.size });
    }
    if prev.map(|p| (p.used, p.size, p.cost)) != usage.map(|u| (u.used, u.size, u.cost)) {
      self.touch(c);
    }
  }

  pub(crate) fn schedule_usage_poll(&self, c: &mut Core) {
    if !self.polls_usage(c) || !c.phase.running || c.usage_timer.is_some() || c.usage_inflight {
      return;
    }
    let weak = self.me.clone();
    let handle = tokio::spawn(async move {
      tokio::time::sleep(USAGE_POLL_INTERVAL).await;
      let Some(me) = weak.upgrade() else { return };
      me.core.lock().usage_timer = None;
      me.refresh_context_usage().await;
      let mut c = me.core.lock();
      me.schedule_usage_poll(&mut c);
    });
    c.usage_timer = Some(handle.abort_handle());
  }

  /// Kimi emits its context snapshot asynchronously after end_turn: park the queue until it lands (bounded).
  /// Resolves to whether the wait was cancelled
  pub(crate) async fn wait_for_kimi_usage(self: &Arc<Self>, revision: u64) -> bool {
    let rx = {
      let mut c = self.core.lock();
      let auto = self.deps.compaction.as_ref().is_some_and(|f| f().auto);
      if self.agent != "kimi" || !auto || c.usage_revision != revision {
        return false;
      }
      let (tx, rx) = tokio::sync::oneshot::channel();
      c.finish_usage_refresh = Some(tx);
      rx
    };
    match tokio::time::timeout(Duration::from_secs(5), rx).await {
      Ok(Ok(cancelled)) => cancelled,
      Ok(Err(_)) => false,
      Err(_) => {
        self.log("context refresh unavailable after prompt");
        self.core.lock().finish_usage_refresh = None;
        false
      }
    }
  }
}

/// The session-info context snapshot of a Grok `_x.ai/session/info` answer
pub fn grok_context_usage(v: &Value, session_id: &str) -> Option<Usage> {
  let result = v.get("result")?;
  if result.get("sessionId").and_then(Value::as_str) != Some(session_id) {
    return None;
  }
  let ctx = result.get("context")?;
  let used = ctx.get("used")?.as_f64()?;
  let total = ctx.get("total")?.as_f64()?;
  let safe = |n: f64| n.fract() == 0.0 && n.abs() <= 9_007_199_254_740_991.0;
  if !safe(used) || used < 0.0 || !safe(total) || total <= 0.0 {
    return None;
  }
  Some(Usage { used: num(used), size: num(total), cost: None })
}

pub(crate) fn clear_usage_timer(c: &mut Core) {
  if let Some(h) = c.usage_timer.take() {
    h.abort();
  }
}
