//! Child sessions a delegation names on its call (droid 1.14.0 `Task`, `_meta.subagent_session_info`): the CLI streams
//! a child only after a `session/load` of its id on the parent's process, which opens a read-only view of it. Each
//! child is loaded once per process, since every load replays the child from its first message; a load that fails
//! leaves the node with its result alone. Nothing is ever sent to a child but that load (`agent-quirks.md`)

use std::time::Duration;

use serde_json::json;

use crate::acp::session::{AcpSession, Core};

/// The load answers at once (droid replays after the answer); a process that never answers must not hold a task forever
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

impl AcpSession {
  /// Ask the CLI to stream the child sessions delegations named since the last call
  pub(crate) fn load_child_sessions(&self, c: &mut Core) {
    let children = c.tree.take_child_loads();
    if children.is_empty() || !self.vendor.loads_child_sessions() || c.replaying {
      return;
    }
    let Some(proc) = c.proc.clone() else { return };
    let proc_gen = c.proc_gen;
    // The folder the parent runs in: droid refuses a cwd that does not exist where it runs
    let cwd = self.remote_folder().map(|f| f.path).unwrap_or_else(|| self.cwd.clone());
    c.child_loads.retain(|(g, _)| *g == proc_gen);
    for child in children {
      if !c.child_loads.insert((proc_gen, child.clone())) {
        continue;
      }
      let (proc, cwd, me) = (proc.clone(), cwd.clone(), self.me.clone());
      tokio::spawn(async move {
        let req = json!({ "sessionId": child, "cwd": cwd, "mcpServers": [] });
        let line = match tokio::time::timeout(LOAD_TIMEOUT, proc.request("session/load", req)).await {
          Ok(Ok(_)) => format!("subagent session {child} loaded"),
          Ok(Err(e)) => format!("subagent session {child} not loaded: {e}"),
          Err(_) => format!("subagent session {child} not loaded: no answer"),
        };
        if let Some(me) = me.upgrade() {
          me.log(&line);
        }
      });
    }
  }
}
