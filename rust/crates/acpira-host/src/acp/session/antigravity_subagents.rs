//! Periodic disk metadata poller for Antigravity (`agy` 1.5.16) native `/swarm` workers and `invoke_subagent`
//! children, and for the model an orchestrator's execution steps run on (`vendors/antigravity_subagents.rs`). Runs only
//! while an Antigravity session has live child nodes or a running orchestrator round, and skips unchanged files by
//! size + mtime.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::AbortHandle;

use crate::accounts::antigravity::STORE_ENV;
use crate::acp::session::updates::current_turn_index;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::subagent_tree::RouteCtx;
use crate::acp::vendors::Vendor;
use crate::acp::vendors::antigravity::worker_name;
use crate::acp::vendors::antigravity_subagents::{AntigravityMetaCursor, brain_dir};

const POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// Fast initial tick right after a delegation or first child tool call arrives so worker names resolve promptly
const FAST_INITIAL: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(crate) struct AntigravitySubagents {
  cursor: AntigravityMetaCursor,
  /// Resolved account `--gemini-dir` (`None` = not resolved yet; `Some(None)` = default `~/.gemini`)
  gemini_dir: Option<Option<PathBuf>>,
  timer: Option<AbortHandle>,
  inflight: bool,
}

impl AntigravitySubagents {
  /// Drops the cached `--gemini-dir` when a new process attaches (e.g. after switching accounts)
  pub(crate) fn reset_dir(&mut self) {
    self.gemini_dir = None;
  }
}

impl AcpSession {
  /// Starts polling `.system_generated/{subagents,swarm}/` while any Antigravity child node is still running
  pub(crate) fn schedule_antigravity_subagents(&self, c: &mut Core, fast: bool) {
    if self.vendor != Vendor::Antigravity
      || c.acp_session_id.is_none()
      || c.agy_subagents.timer.is_some()
      || c.agy_subagents.inflight
      || !c.tree.has_running_antigravity()
    {
      return;
    }
    let delay = if fast { FAST_INITIAL } else { POLL_INTERVAL };
    let weak = self.me.clone();
    let handle = tokio::spawn(async move {
      tokio::time::sleep(delay).await;
      let Some(me) = weak.upgrade() else { return };
      me.core.lock().agy_subagents.timer = None;
      me.poll_antigravity_subagents().await;
      let mut c = me.core.lock();
      me.schedule_antigravity_subagents(&mut c, false);
    });
    c.agy_subagents.timer = Some(handle.abort_handle());
  }

  async fn poll_antigravity_subagents(self: &Arc<Self>) {
    let (sid, account_id, cached_dir, mut cursor, orchestrating) = {
      let mut c = self.core.lock();
      if self.vendor != Vendor::Antigravity || !c.tree.has_running_antigravity() {
        return;
      }
      let Some(sid) = c.acp_session_id.clone() else { return };
      c.agy_subagents.inflight = true;
      let cursor = std::mem::take(&mut c.agy_subagents.cursor);
      (sid, c.account_id.clone(), c.agy_subagents.gemini_dir.clone(), cursor, c.tree.has_running_orchestrator_round())
    };

    // Prefer the running process's actual `--gemini-dir` (`AgentProcess::own_dirs`), which covers saved accounts,
    // plugin accounts and agents.json overrides alike; fall back to account hooks when no own_dir was recorded
    let own_dir = self.own_dirs.lock().first().cloned();
    let gemini_dir = match own_dir {
      Some(dir) => Some(dir),
      None => match cached_dir {
        Some(dir) => dir,
        None => match (account_id, self.deps.accounts.as_ref()) {
          (Some(acc), Some(hooks)) => hooks
            .spawn_env(self.agent.clone(), acc)
            .await
            .and_then(|env| env.get(STORE_ENV).cloned())
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
          _ => None,
        },
      },
    };
    let brain = brain_dir(gemini_dir.as_deref());
    let polled = tokio::task::spawn_blocking(move || {
      let snap = cursor.poll(&brain, &sid, orchestrating);
      (cursor, snap)
    })
    .await;

    let mut c = self.core.lock();
    c.agy_subagents.inflight = false;
    c.agy_subagents.gemini_dir = Some(gemini_dir);
    let Ok((cursor, snap)) = polled else { return };
    c.agy_subagents.cursor = cursor;
    let Some(mut snap) = snap else { return };
    let turn_index = current_turn_index(&c);
    let changed = {
      let Core { tree, state, .. } = &mut *c;
      // The orchestrator's execution model, under the name the model select gives it
      let worker = snap.worker_model.take().is_some_and(|id| tree.set_orchestrator_worker(worker_name(&id, &state.controls.options)));
      tree.sync_antigravity(snap, &mut RouteCtx { turn_index, root_turns: &mut state.turns }) || worker
    };
    if changed {
      self.drain_terminal(&mut c);
      self.touch(&mut c);
    }
  }
}
