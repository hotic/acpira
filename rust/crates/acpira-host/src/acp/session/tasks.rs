//! Stopping one AIR background task or one delegated subagent

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::transcript::*;

use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::set_stop_requested;
use crate::i18n::t;

impl AcpSession {
  /// Ask the adapter to stop one background task on the session that owns it; stopRequested is only optimistic
  pub async fn stop_async_task(self: &Arc<Self>, task_id: &str) -> Result<()> {
    let (proc, peer) = {
      let mut c = self.core.lock();
      let peer = c.task_peer.get(task_id).cloned();
      let root = peer.is_some() && peer == c.acp_session_id;
      let info = match &peer {
        Some(p) if root => {
          let _ = p;
          c.state.tasks.tasks.get(task_id).cloned()
        }
        Some(p) => c.tree.task_state(p).and_then(|(st, _)| st.tasks.tasks.get(task_id).cloned()),
        None => None,
      };
      let (Some(info), Some(peer)) = (info, peer) else {
        drop(c);
        self.log(&format!("stopAsyncTask {task_id}: unknown task"));
        return Ok(());
      };
      if info.stop_requested || !matches!(info.state, AsyncTaskState::Running | AsyncTaskState::Paused) {
        return Ok(());
      }
      if !info.can_stop {
        drop(c);
        self.log(&format!("stopAsyncTask {task_id}: the adapter says it cannot be stopped"));
        return Ok(());
      }
      let Some(proc) = c.proc.clone().filter(|p| p.alive()) else { return Ok(()) };
      self.mark_stop(&mut c, &peer, task_id, true);
      self.touch(&mut c);
      (proc, peer)
    };
    let r = proc.request("_session/async_task/stop", json!({ "sessionId": peer, "asyncTaskId": task_id })).await;
    let refused = matches!(&r, Ok(v) if v.get("stopped") == Some(&Value::Bool(false)));
    if r.is_err() || refused {
      let mut c = self.core.lock();
      self.mark_stop(&mut c, &peer, task_id, false);
      self.touch(&mut c);
    }
    match r {
      Err(e) => Err(anyhow::Error::new(e)),
      Ok(_) if refused => Err(anyhow!(t("asyncTask.stopRefused"))),
      Ok(_) => Ok(()),
    }
  }

  fn mark_stop(&self, c: &mut Core, peer: &str, task_id: &str, on: bool) {
    if c.acp_session_id.as_deref() == Some(peer) {
      set_stop_requested(&mut c.state, task_id, on);
    } else if let Some((st, node)) = c.tree.task_state(peer) {
      set_stop_requested(st, task_id, on);
      c.tree.bump(&node);
    }
  }

  /// Ask the agent to cancel one delegated child; its pending cards (and its descendants') close first
  pub async fn cancel_subagent(self: &Arc<Self>, id: &str) {
    let (proc, peer) = {
      let mut c = self.core.lock();
      let Some(peer) = c.tree.cancel(id) else { return };
      let mut ids = vec![id.to_owned()];
      ids.extend(c.tree.descendants(id));
      for node in ids {
        self.cancel_permissions_for(&mut c, &node);
        self.cancel_questions_for(&mut c, &node);
      }
      self.touch(&mut c);
      (c.proc.clone(), peer)
    };
    if let Some(p) = proc {
      p.notify("session/cancel", json!({ "sessionId": peer }));
    }
  }
}
