//! Stopping one AIR background task, one running command or one delegated subagent

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::transcript::*;

use crate::acp::session::view::visible_turns;
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

  /// The user's stop of one running command row; the turn goes on, the agent reads the command as failed. Claude Code's
  /// project tools on an SSH host are the engine's own calls, any other command is found by its text among the
  /// processes under the agent's (`platform::process_tree`). An error when nothing running that command was found
  pub async fn stop_command(self: &Arc<Self>, tool_call_id: &str) -> Result<()> {
    if self.stop_remote_call(tool_call_id) {
      self.log(&format!("stopCommand {tool_call_id}: remote call cancelled"));
      return Ok(());
    }
    let (command, pid) = {
      let c = self.core.lock();
      let command = visible_turns(&c).chain(c.tree.all_turns()).filter_map(Turn::as_agent).flat_map(|t| t.blocks.iter()).find_map(|b| match b {
        AgentBlock::ToolCall(tool)
          if tool.id == tool_call_id
            && tool.kind == ToolKind::Execute
            && matches!(tool.status, ToolStatus::Pending | ToolStatus::InProgress)
            && tool.target_mono == Some(true) =>
        {
          tool.target.clone()
        }
        _ => None,
      });
      let pid = c.proc.as_ref().filter(|p| p.alive()).and_then(|p| p.pid);
      (command, pid)
    };
    let (Some(command), Some(pid)) = (command, pid) else {
      self.log(&format!("stopCommand {tool_call_id}: no running command row"));
      return Ok(());
    };
    #[cfg(unix)]
    let found = tokio::task::spawn_blocking(move || crate::platform::process_tree::stop_command(pid, &command)).await.unwrap_or(false);
    #[cfg(not(unix))]
    let found = {
      let _ = (pid, command);
      false
    };
    self.log(&format!("stopCommand {tool_call_id}: {}", if found { "signalled" } else { "no process found" }));
    if found { Ok(()) } else { Err(anyhow!(t("command.stopNotFound"))) }
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
    let mut c = self.core.lock();
    let descendants = c.tree.descendants(id);
    // Summoned children run in processes of their own: the node and every summoned descendant get their own cancel
    let summoned: Vec<String> = descendants.iter().filter(|d| c.tree.relay_thread_of(d).is_some()).cloned().collect();
    if c.tree.relay_thread_of(id).is_some() {
      if !c.tree.relay_cancel(id) {
        return;
      }
      let ids: Vec<String> = std::iter::once(id.to_owned()).chain(descendants).collect();
      for node in &ids {
        self.cancel_permissions_for(&mut c, node);
        self.cancel_questions_for(&mut c, node);
      }
      if let Some((proc, sid)) = Self::relay_target(&c, id) {
        proc.notify("session/cancel", json!({ "sessionId": sid }));
      }
      self.relay_cancel(&mut c, &summoned);
      self.touch(&mut c);
      return;
    }
    let Some(peer) = c.tree.cancel(id) else { return };
    let ids: Vec<String> = std::iter::once(id.to_owned()).chain(descendants).collect();
    for node in &ids {
      self.cancel_permissions_for(&mut c, node);
      self.cancel_questions_for(&mut c, node);
    }
    self.relay_cancel(&mut c, &summoned);
    self.touch(&mut c);
    if let Some(p) = c.proc.clone() {
      p.notify("session/cancel", json!({ "sessionId": peer }));
    }
  }
}
