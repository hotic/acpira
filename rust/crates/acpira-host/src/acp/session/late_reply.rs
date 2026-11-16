//! Antigravity's background commands (`agy acp` 1.5.9, `vendors/antigravity.rs`). A `run_command` sent to the
//! background is still `in_progress` when its turn ends, and when it exits the CLI has the model write on: that reply
//! is stored and never sent over ACP. Two things do show it: the session's `updatedAt` in `session/list` moves, and
//! `session/load` on the live process replays the whole conversation, with the row closed, its receipt, and the reply
//! after it (the load keeps the picked model and mode).
//!
//! So a command still running at the end of a turn that ended on its own stays on its row as a background task, which
//! keeps the row open and the process alive like any other. A watcher then asks for the session's time and reads the
//! replay whenever it moved: a closed row settles its task, and what the replay holds past the known part runs as a
//! detached turn. The session is held as detached while a replay is read, so a prompt sent meanwhile queues behind it

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_shared::transcript::*;

use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::async_task_live;
use crate::acp::transport::process::AgentProcess;
use crate::acp::transport::wire::{AsyncTaskEvent, TaskEventKind};
use crate::acp::vendors::antigravity;

/// Between two looks at the session's time: often at first, when a command is most likely to exit
const POLL: Duration = Duration::from_secs(2);
const SLOW_POLL: Duration = Duration::from_secs(10);
const SLOW_AFTER: Duration = Duration::from_secs(300);
/// How long after the last command exited its reply may still land
const QUIET: Duration = Duration::from_secs(90);
/// A watcher gives up here; a command still running keeps its row, and the next turn's end watches again
const LIMIT: Duration = Duration::from_secs(1800);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const LIST_PAGES: usize = 5;

/// What the session keeps for its late replies
#[derive(Default)]
pub struct LateReply {
  /// The replay being read: every update of the session lands here instead of on the transcript
  pub capture: Option<Vec<Value>>,
  /// The watcher in charge; an older one retires
  pub watch: u64,
}

enum Read {
  Replay(Vec<Value>),
  /// Something else has the session: look again later
  Busy,
  Failed,
}

fn command_task(tool_call_id: &str, state: Option<AsyncTaskState>) -> AsyncTaskEvent {
  AsyncTaskEvent {
    event: if state.is_some() { TaskEventKind::State } else { TaskEventKind::Spawned },
    async_task_id: format!("{}{tool_call_id}", antigravity::COMMAND_TASK),
    name: None,
    task_type: Some("shell".into()),
    description: None,
    show_in_transcript: false,
    can_stop: Some(false),
    output_file_path: None,
    tool_call_id: Some(tool_call_id.to_owned()),
    summary: None,
    last_tool_name: None,
    usage: None,
    state,
  }
}

fn tool_rows(turns: &[Turn]) -> impl Iterator<Item = &ToolCallBlock> {
  turns.iter().filter_map(Turn::as_agent).flat_map(|t| &t.blocks).filter_map(|b| match b {
    AgentBlock::ToolCall(tc) => Some(tc),
    _ => None,
  })
}

/// The calls whose background command is still running, as far as the transcript knows
fn live_commands(c: &Core) -> Vec<String> {
  tool_rows(&c.state.turns)
    .filter(|tc| async_task_live(tc) && tc.async_task.as_ref().is_some_and(|a| a.id.starts_with(antigravity::COMMAND_TASK)))
    .map(|tc| tc.id.clone())
    .collect()
}

fn closed_state(row: &Value) -> AsyncTaskState {
  match row.get("status").and_then(Value::as_str) {
    Some("failed") => AsyncTaskState::Failed,
    Some("cancelled") => AsyncTaskState::Stopped,
    _ => AsyncTaskState::Completed,
  }
}

impl AcpSession {
  /// Before a turn that ended on its own is sealed: a command of it still running went to the background, and its row
  /// becomes that task's instead of a failed call
  pub(crate) fn park_background_commands(&self, c: &mut Core) {
    if !self.vendor.stores_late_replies() {
      return;
    }
    let Some(peer) = c.acp_session_id.clone() else { return };
    let Some(Turn::Agent(turn)) = c.state.turns.last() else { return };
    let ids: Vec<String> = turn
      .blocks
      .iter()
      .filter_map(|b| match b {
        AgentBlock::ToolCall(tc) if tc.kind == ToolKind::Execute && tc.status == ToolStatus::InProgress && tc.async_task.is_none() => Some(tc.id.clone()),
        _ => None,
      })
      .collect();
    for id in ids {
      self.log(&format!("background command {id} outlives its turn"));
      self.async_task_update(c, &peer, &command_task(&id, None));
    }
  }

  /// After a prompt's turn: watch for what the background commands leave behind
  pub(crate) fn watch_late_replies(self: &Arc<Self>) {
    if !self.vendor.stores_late_replies() {
      return;
    }
    let (watch, proc_gen) = {
      let mut c = self.core.lock();
      if live_commands(&c).is_empty() {
        return;
      }
      c.late.watch += 1;
      (c.late.watch, c.proc_gen)
    };
    let me = self.clone();
    tokio::spawn(async move { me.late_replies(watch, proc_gen).await });
  }

  async fn late_replies(self: Arc<Self>, watch: u64, proc_gen: u64) {
    let (proc, acp_id) = {
      let c = self.core.lock();
      let (Some(proc), Some(acp_id)) = (c.proc.clone(), c.acp_session_id.clone()) else { return };
      (proc, acp_id)
    };
    // One request for every read: the same MCP entry each time, so the CLI has no reason to restart a server
    let req = self.session_request(&proc, Some(&acp_id)).await;
    let cwd = req.get("cwd").and_then(Value::as_str).unwrap_or(&self.cwd).to_owned();
    let started = Instant::now();
    // The replay updates the transcript holds; None until the first read
    let mut known: Option<usize> = None;
    let mut seen: Option<String> = None;
    let mut quiet: Option<Instant> = None;
    self.log("watching for late replies");
    loop {
      let (free, live) = {
        let c = self.core.lock();
        // A prompt's turn has the session: its end starts a watcher of its own, with the replay as it is then
        if !self.late_current(&c, watch, proc_gen, &acp_id) || c.phase.running {
          return;
        }
        (self.late_free(&c), !live_commands(&c).is_empty())
      };
      let moved = if free {
        let Some(now) = self.listed_time(&proc, &acp_id, &cwd).await else {
          self.log("late replies: the session is not in session/list, giving up");
          return;
        };
        let moved = known.is_none() || seen.as_deref() != Some(now.as_str());
        seen = Some(now);
        moved
      } else {
        false
      };
      if moved {
        match self.read_replay(&proc, req.clone(), watch, proc_gen, &acp_id).await {
          // The next look waits like any other: a CLI whose load moved the time must not be read in a loop
          Read::Replay(replay) => {
            let from = known.unwrap_or_else(|| self.replay_known(&replay));
            self.apply_replay(&acp_id, &replay, from);
            known = Some(replay.len());
            quiet = None;
          }
          // The time is asked again, and differs again, on the next round
          Read::Busy => seen = None,
          Read::Failed => return,
        }
      }
      if live {
        quiet = None;
      } else if quiet.get_or_insert_with(Instant::now).elapsed() > QUIET {
        self.log("late replies: nothing more followed");
        return;
      }
      if started.elapsed() > LIMIT {
        self.log("late replies: gave up watching");
        return;
      }
      tokio::time::sleep(if started.elapsed() > SLOW_AFTER { SLOW_POLL } else { POLL }).await;
    }
  }

  /// This watcher still has a session to watch: no newer one, and the process and native session it started with (an
  /// edit continues the transcript on a new one)
  fn late_current(&self, c: &Core, watch: u64, proc_gen: u64, acp_id: &str) -> bool {
    c.late.watch == watch && c.proc_gen == proc_gen && c.status == SessionStatus::Ready && c.acp_session_id.as_deref() == Some(acp_id)
  }

  /// Nothing else is using the session or about to: a replay may be read
  fn late_free(&self, c: &Core) -> bool {
    !(c.phase.running
      || c.phase.editing
      || c.phase.edit_waiting
      || c.replaying
      || c.switching
      || c.hooks.gating
      || c.ultracode.rebuilding
      || c.peer.detached
      || c.pending_prompt.is_some()
      || c.late.capture.is_some())
  }

  /// The session's `updatedAt` as `session/list` has it
  async fn listed_time(&self, proc: &AgentProcess, acp_id: &str, cwd: &str) -> Option<String> {
    let mut cursor: Option<String> = None;
    for _ in 0..LIST_PAGES {
      let mut params = json!({ "cwd": cwd });
      if let Some(c) = &cursor {
        params["cursor"] = Value::from(c.clone());
      }
      let r = tokio::time::timeout(LIST_TIMEOUT, proc.request("session/list", params)).await.ok()?.ok()?;
      let found = r.get("sessions").and_then(Value::as_array).and_then(|l| l.iter().find(|s| s.get("sessionId").and_then(Value::as_str) == Some(acp_id)));
      if let Some(s) = found {
        return Some(s.get("updatedAt").and_then(Value::as_str).unwrap_or("").to_owned());
      }
      cursor = r.get("nextCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
      cursor.as_ref()?;
    }
    None
  }

  /// `session/load` on the live process, its replay kept off the transcript. The session stays held as detached on a
  /// replay, for `apply_replay` to release. The load is not timed: its replay must not land on the transcript after
  /// the capture is gone, and a process that never answers fails the request when it goes
  async fn read_replay(self: &Arc<Self>, proc: &AgentProcess, req: Value, watch: u64, proc_gen: u64, acp_id: &str) -> Read {
    {
      let mut c = self.core.lock();
      if !self.late_current(&c, watch, proc_gen, acp_id) || !self.late_free(&c) {
        return Read::Busy;
      }
      c.peer.detached = true;
      c.late.capture = Some(vec![]);
    }
    let result = proc.request_ordered("session/load", req).await;
    let mut c = self.core.lock();
    let replay = c.late.capture.take().unwrap_or_default();
    match result {
      Ok((_, handoff)) => {
        drop(c);
        // Only now may the updates after the response go on: they are live ones
        drop(handoff);
        Read::Replay(replay)
      }
      Err(e) => {
        if c.proc_gen == proc_gen {
          c.peer.detached = false;
        }
        drop(c);
        self.log(&format!("late replies: session/load failed: {e}"));
        self.flush_queue();
        Read::Failed
      }
    }
  }

  /// Where the transcript ends in the first replay read
  fn replay_known(&self, replay: &[Value]) -> usize {
    let c = self.core.lock();
    let Some(Turn::Agent(turn)) = c.state.turns.last() else { return replay.len() };
    let reply = turn.blocks.iter().rev().find_map(|b| match b {
      AgentBlock::Text(t) => Some(t.markdown.as_str()),
      _ => None,
    });
    antigravity::replay_known(replay, reply, |id| turn.blocks.iter().any(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.id == id)))
  }

  /// Settle the commands a replay shows closed, then run what it holds past `known` as a detached turn. Releases the
  /// hold `read_replay` took
  fn apply_replay(self: &Arc<Self>, acp_id: &str, replay: &[Value], known: usize) {
    let live = live_commands(&self.core.lock());
    for id in live {
      let Some(row) = antigravity::replay_closed(replay, &id) else { continue };
      // The receipt first, as the update the wire never sent; the row's status is its task's
      let mut update = row.clone();
      update["sessionUpdate"] = Value::from("tool_call_update");
      self.on_update(json!({ "sessionId": acp_id, "update": update }));
      let mut c = self.core.lock();
      self.async_task_update(&mut c, acp_id, &command_task(&id, Some(closed_state(row))));
      self.touch(&mut c);
      self.log(&format!("background command {id} ended"));
    }
    let tail = antigravity::replay_tail(replay, known);
    let opened = {
      let mut c = self.core.lock();
      let open = !tail.is_empty() && matches!(c.state.turns.last(), Some(Turn::Agent(_)));
      if open {
        self.reopen_for_detached(&mut c);
        self.touch(&mut c);
      } else {
        c.peer.detached = false;
      }
      open
    };
    if !opened {
      self.flush_queue();
      return;
    }
    self.log(&format!("late reply: {} update(s)", tail.len()));
    for update in tail {
      self.on_update(json!({ "sessionId": acp_id, "update": update }));
    }
    let mut c = self.core.lock();
    self.end_detached(&mut c, TurnStop::EndTurn);
    self.touch(&mut c);
  }
}
