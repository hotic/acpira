//! The live process of Claude workflow agents: while any workflow receipt node of the session runs, its sidechain log
//! (`vendors/claude_workflow_log.rs`) is polled, and what was appended lands in the node's own transcript. A node that
//! ends gets one more read for its last lines; a restored node without a process is read once when a viewer opens it.
//! Nothing is polled while no workflow agent runs, and an unchanged log (size + mtime) is not opened

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::task::AbortHandle;

use crate::acp::session::{AcpSession, Core};
use crate::acp::vendors::claude_workflow_log::{self as wlog, LogCursor};

pub const WORKFLOW_LOG_INTERVAL: Duration = Duration::from_millis(1000);
/// A session directory not found yet is looked for again (a scan of every project directory) at most this often
const RESCAN_INTERVAL: Duration = Duration::from_secs(5);

/// Per-session poll state: one cursor per node, the resolved session directory, and which nodes are finished with
#[derive(Default)]
pub(crate) struct WorkflowLogs {
  cursors: HashMap<String, LogCursor>,
  /// `<config>/projects/<encoded cwd>/<sessionId>`, once found
  session_dir: Option<PathBuf>,
  last_scan: Option<Instant>,
  /// Nodes whose final read ran
  done: HashSet<String>,
  /// Restored nodes a viewer opened: read once if their record holds no process
  wanted: HashSet<String>,
  timer: Option<AbortHandle>,
  inflight: bool,
}

/// One node's read, done off the async threads
struct Job {
  node: String,
  agent_id: String,
  running: bool,
  cursor: LogCursor,
}

impl AcpSession {
  /// Start polling when some workflow agent has a log to read; a no-op while a poll is already scheduled or running
  pub(crate) fn schedule_workflow_logs(&self, c: &mut Core) {
    let w = &c.workflow_logs;
    if w.timer.is_some() || w.inflight || c.acp_session_id.is_none() || c.tree.workflow_log_targets(&w.wanted, &w.done).is_empty() {
      return;
    }
    let weak = self.me.clone();
    let handle = tokio::spawn(async move {
      tokio::time::sleep(WORKFLOW_LOG_INTERVAL).await;
      let Some(me) = weak.upgrade() else { return };
      me.core.lock().workflow_logs.timer = None;
      me.poll_workflow_logs().await;
      let mut c = me.core.lock();
      me.schedule_workflow_logs(&mut c);
    });
    c.workflow_logs.timer = Some(handle.abort_handle());
  }

  /// A viewer opened a node: a restored workflow agent without a process gets its log read once
  pub fn observe_subagent(&self, id: &str) {
    let mut c = self.core.lock();
    if c.workflow_logs.wanted.insert(id.to_owned()) {
      self.schedule_workflow_logs(&mut c);
    }
  }

  async fn poll_workflow_logs(self: &Arc<Self>) {
    let (jobs, sid, dir, rescan) = {
      let mut c = self.core.lock();
      let Some(sid) = c.acp_session_id.clone() else { return };
      let Core { tree, workflow_logs: w, .. } = &mut *c;
      let targets = tree.workflow_log_targets(&w.wanted, &w.done);
      // Cursors of nodes that are gone (an edit removed their turn) go with them
      w.cursors.retain(|id, _| targets.iter().any(|(n, _, _)| n == id));
      if targets.is_empty() {
        return;
      }
      w.inflight = true;
      let jobs: Vec<Job> = targets
        .into_iter()
        .map(|(node, agent_id, running)| {
          let cursor = w.cursors.remove(&node).unwrap_or_else(|| LogCursor::new(&self.cwd));
          Job { node, agent_id, running, cursor }
        })
        .collect();
      let rescan = w.session_dir.is_none() && w.last_scan.is_none_or(|t| t.elapsed() >= RESCAN_INTERVAL);
      if rescan {
        w.last_scan = Some(Instant::now());
      }
      (jobs, sid, w.session_dir.clone(), rescan)
    };
    let config = wlog::config_dir_of(self.def().env.as_ref().and_then(|e| e.get(wlog::CONFIG_DIR_ENV).cloned()).or_else(|| std::env::var(wlog::CONFIG_DIR_ENV).ok()));
    let cwd = self.cwd.clone();
    let read = tokio::task::spawn_blocking(move || {
      let dir = dir.or_else(|| if rescan { wlog::session_dir(&config, &cwd, &sid) } else { None });
      let jobs: Vec<(Job, Vec<Value>)> = jobs
        .into_iter()
        .map(|mut j| {
          let mut updates = vec![];
          if let Some(d) = &dir {
            // A running agent's backlog catches up over a few polls; an ended one is read to its end now (the cursor
            // caps the whole log, so this stays bounded)
            for _ in 0..wlog::MAX_READS {
              updates.extend(j.cursor.read(|| wlog::agent_log(d, &j.agent_id)));
              if j.running || j.cursor.stopped || !j.cursor.located() || j.cursor.at_end() {
                break;
              }
            }
          }
          (j, updates)
        })
        .collect();
      (dir, jobs)
    })
    .await;
    let mut c = self.core.lock();
    c.workflow_logs.inflight = false;
    let Ok((dir, jobs)) = read else { return };
    // Whether this poll could look for the logs at all: a throttled rescan leaves ended nodes for a later poll
    let searched = dir.is_some() || rescan;
    if c.workflow_logs.session_dir.is_none() {
      c.workflow_logs.session_dir = dir;
    }
    let mut changed = false;
    for (j, updates) in jobs {
      if !updates.is_empty() && c.tree.workflow_log(&j.node, &updates) {
        changed = true;
      }
      // A node that was not running when this read started has had its last lines read
      if !j.running && searched {
        c.workflow_logs.done.insert(j.node.clone());
        if !j.cursor.located() {
          self.log(&format!("workflow agent {} has no sidechain log; its node keeps the receipt only", j.agent_id));
        }
      } else {
        c.workflow_logs.cursors.insert(j.node, j.cursor);
      }
    }
    if changed {
      self.touch(&mut c);
    }
  }
}
