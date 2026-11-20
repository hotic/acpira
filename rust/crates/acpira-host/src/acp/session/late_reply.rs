//! Antigravity's background commands (`vendors/antigravity.rs`). A `run_command` sent to the background is still
//! `in_progress` when its turn ends, and when it exits the CLI has the model write on, with no prompt running. agy acp
//! 1.5.12 streams the exit and that reply as they happen; 1.5.9 sent neither, and only two things showed them: the
//! session's `updatedAt` in `session/list` moves, and `session/load` on the live process replays the whole
//! conversation, with the row closed, its receipt, and the reply after it (the load keeps the picked model and mode).
//!
//! So a command still running at the end of a turn that ended on its own stays on its row as a background task, which
//! keeps the row open and the process alive like any other. Its exit settles that task, and the reply runs as a
//! detached turn. Streamed, the reply opens that turn itself (`late_live`); a watcher only ends it. For a CLI that
//! streams nothing, the watcher asks for the session's time and reads the replay a look after it moved: a closed row
//! settles its task, and what the replay holds past the known part goes on the detached turn. Never both: a load also
//! restarts the stream, so a replay read while the reply streams brings its steps a second time. The model writes the
//! reply step by step (thoughts, calls, words), each step moving the time, so the turn stays open and ends once, when
//! the time and the stream have stood still. The session is held as detached while a replay is read or the turn is
//! open, so a prompt sent meanwhile queues behind it

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
/// How long the session's time must stand still before an open late reply counts as over: short once the replay ends
/// on the agent's words, as a finished reply does, long while it ends on a call or a thought
const SETTLE: Duration = Duration::from_secs(10);
const SETTLE_MID: Duration = Duration::from_secs(60);
/// A watcher gives up here; a command still running keeps its row, and the next turn's end watches again
const LIMIT: Duration = Duration::from_secs(1800);
const LIST_TIMEOUT: Duration = Duration::from_secs(10);
const LIST_PAGES: usize = 5;

/// Marks an update `apply_replay` hands to `on_update`: read back from a replay, not streamed
pub(crate) const REPLAYED: &str = "acpira/replayed";

/// What the session keeps for its late replies
#[derive(Default)]
pub struct LateReply {
  /// The replay being read: every update of the session lands here instead of on the transcript
  pub capture: Option<Vec<Value>>,
  /// The watcher in charge; an older one retires
  pub watch: u64,
  /// The watcher still running, by its number
  pub watching: Option<u64>,
  /// The detached turn of a late reply still being written: the watcher's own, ended by it or by any settle
  pub open: bool,
  /// The process (by `proc_gen`) that streams what its background commands leave behind: no replay is read for it
  pub streams: Option<u64>,
  /// When the stream last added to the open late reply
  pub stirred: Option<Instant>,
  /// A stop ended the late reply: what the CLI still streams of it is dropped until the next prompt's turn
  pub stopped: bool,
}

enum Read {
  Replay(Vec<Value>),
  /// Something else has the session: look again later
  Busy,
  Failed,
}

/// The task a droid `Task` row carries while its child runs past the turn, before the call's id: a `local_agent` like
/// Claude's background agents, so the row stays running and the session lists as background work
const DELEGATION_TASK: &str = "droid-task:";

/// The synthetic task a parked row carries (`park_background_commands`): spawned, or moved to `state`
fn parked_task(task: &str, task_type: Option<&str>, tool_call_id: &str, state: Option<AsyncTaskState>) -> AsyncTaskEvent {
  AsyncTaskEvent {
    event: if state.is_some() { TaskEventKind::State } else { TaskEventKind::Spawned },
    async_task_id: task.to_owned(),
    name: None,
    task_type: task_type.map(str::to_owned),
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

/// The calls an open late reply was still making when it was last read
fn open_calls(c: &Core) -> Vec<String> {
  let Some(Turn::Agent(turn)) = c.state.turns.last() else { return vec![] };
  let open = |b: &AgentBlock| match b {
    AgentBlock::ToolCall(tc) if matches!(tc.status, ToolStatus::InProgress | ToolStatus::Pending) && tc.async_task.is_none() => Some(tc.id.clone()),
    _ => None,
  };
  turn.blocks.iter().filter_map(open).collect()
}

/// Whether the open late reply waits on a call no quiet spell may end it under: only a command goes to the background
/// (past its `WaitMsBeforeAsync` the model moves on, and `settle` parks its row), any other call is one the model waits for
fn waits_on_call(c: &Core) -> bool {
  let Some(Turn::Agent(turn)) = c.state.turns.last() else { return false };
  turn.blocks.iter().any(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.kind != ToolKind::Execute && matches!(tc.status, ToolStatus::InProgress | ToolStatus::Pending) && tc.async_task.is_none()))
}

/// A row a turn left running (`park_background_commands`), by the synthetic task it carries
struct Parked {
  call: String,
  task: String,
  task_type: Option<String>,
}

/// The parked rows whose command or delegation still runs, as far as the transcript knows
fn parked(c: &Core) -> Vec<Parked> {
  tool_rows(&c.state.turns)
    .filter_map(|tc| {
      let a = tc.async_task.as_ref().filter(|a| a.id.starts_with(antigravity::COMMAND_TASK) || a.id.starts_with(DELEGATION_TASK))?;
      async_task_live(tc).then(|| Parked { call: tc.id.clone(), task: a.id.clone(), task_type: a.task_type.clone() })
    })
    .collect()
}

/// Whether a command or delegation sent to the background still runs, so its late reply is still owed (`after_prompt`
/// holds the auto /compact for it)
pub(crate) fn has_background_commands(c: &Core) -> bool {
  !parked(c).is_empty()
}

fn closed_state(row: &Value) -> AsyncTaskState {
  match row.get("status").and_then(Value::as_str) {
    Some("failed") => AsyncTaskState::Failed,
    Some("cancelled") => AsyncTaskState::Stopped,
    _ => AsyncTaskState::Completed,
  }
}

/// Whether the open late reply ends on the agent's words, as a finished reply does: one still being written ends on a
/// call or a thought
fn ends_on_reply(c: &Core) -> bool {
  matches!(c.state.turns.last(), Some(Turn::Agent(t)) if matches!(t.blocks.last(), Some(AgentBlock::Text(_))))
}

impl AcpSession {
  /// Before a turn that ended on its own is sealed: a command of it still running went to the background (agy), or a
  /// delegation still runs its child (droid's asynchronous `Task`), and its row becomes that task's instead of a failed
  /// call
  pub(crate) fn park_background_commands(&self, c: &mut Core) {
    if !self.vendor.late_replies() {
      return;
    }
    let Some(peer) = c.acp_session_id.clone() else { return };
    let delegations = if self.vendor.parks_delegations() { c.tree.outliving_delegations() } else { vec![] };
    let Some(Turn::Agent(turn)) = c.state.turns.last() else { return };
    let commands = self.vendor.stores_late_replies();
    let rows: Vec<(String, &str)> = turn
      .blocks
      .iter()
      .filter_map(|b| match b {
        AgentBlock::ToolCall(tc) if tc.async_task.is_some() => None,
        AgentBlock::ToolCall(tc) if commands && tc.kind == ToolKind::Execute && tc.status == ToolStatus::InProgress => Some((tc.id.clone(), "command")),
        AgentBlock::ToolCall(tc) if tc.status.is_open() && delegations.contains(&tc.id) => Some((tc.id.clone(), "delegation")),
        _ => None,
      })
      .collect();
    for (id, what) in rows {
      self.log(&format!("background {what} {id} outlives its turn"));
      let task = if what == "command" {
        parked_task(&format!("{}{id}", antigravity::COMMAND_TASK), Some("shell"), &id, None)
      } else {
        // droid streams what follows the delegation: no replay is ever read for it
        c.late.streams = Some(c.proc_gen);
        parked_task(&format!("{DELEGATION_TASK}{id}"), Some("local_agent"), &id, None)
      };
      self.async_task_update(c, &peer, &task);
    }
  }

  /// After a prompt's turn: watch for what the background commands leave behind
  pub(crate) fn watch_late_replies(self: &Arc<Self>) {
    if !self.vendor.late_replies() {
      return;
    }
    let mut c = self.core.lock();
    // A prompt's turn came after the stop
    c.late.stopped = false;
    if !parked(&c).is_empty() {
      self.start_watch(&mut c);
    }
  }

  /// A new watcher takes over; an older one retires at its next look
  fn start_watch(self: &Arc<Self>, c: &mut Core) {
    c.late.watch += 1;
    let (watch, proc_gen) = (c.late.watch, c.proc_gen);
    c.late.watching = Some(watch);
    let me = self.clone();
    tokio::spawn(async move {
      me.clone().late_replies(watch, proc_gen).await;
      let mut c = me.core.lock();
      if c.late.watching == Some(watch) {
        c.late.watching = None;
      }
    });
  }

  /// An update of the root session on its way to the transcript, as a CLI that streams its late replies (agy 1.5.12)
  /// sends them: a parked command's exit settles its task, even during a prompt's turn, and with no prompt running the
  /// reply it wakes the model for opens the late reply's detached turn, with a watcher to end it. False when the update
  /// is dropped: what still streams of a late reply a stop ended
  pub(crate) fn late_live(self: &Arc<Self>, c: &mut Core, session_id: &str, kind: &str, u: &Value) -> bool {
    if !self.vendor.late_replies() || c.replaying || c.acp_session_id.as_deref() != Some(session_id) || !antigravity::REPLAY_AGENT_KINDS.contains(&kind) {
      return true;
    }
    if kind == "tool_call_update"
      && matches!(u.get("status").and_then(Value::as_str), Some("completed" | "failed" | "cancelled"))
      && let Some(id) = u.get("toolCallId").and_then(Value::as_str)
      && let Some(p) = parked(c).into_iter().find(|p| p.call == id)
    {
      // The task first: its row then takes the update like any closed call
      self.async_task_update(c, session_id, &parked_task(&p.task, p.task_type.as_deref(), id, Some(closed_state(u))));
      self.log(&format!("background task {id} ended"));
      c.late.streams = Some(c.proc_gen);
      return true;
    }
    // A prompt's own turn
    if c.phase.running && !c.late.open {
      return true;
    }
    c.late.streams = Some(c.proc_gen);
    if c.late.stopped {
      self.log(&format!("late reply {kind} after a stop dropped"));
      return false;
    }
    if !c.late.open {
      if c.status != SessionStatus::Ready || !matches!(c.state.turns.last(), Some(Turn::Agent(_))) {
        return true;
      }
      c.peer.detached = true;
      self.reopen_for_detached(c);
      c.late.open = true;
      self.log("late reply streams");
      if c.late.watching != Some(c.late.watch) {
        self.start_watch(c);
      }
    }
    c.late.stirred = Some(Instant::now());
    true
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
    // The session's time at the last look; the first look only takes it
    let mut seen: Option<String> = None;
    // The time moved and no replay has been read since
    let mut unread = false;
    let mut quiet: Option<Instant> = None;
    // When the time last moved, or a replay was read
    let mut grew: Option<Instant> = None;
    self.log("watching for late replies");
    loop {
      let (free, live) = {
        let mut c = self.core.lock();
        if !self.late_current(&c, watch, proc_gen, &acp_id) {
          // Nobody else ends a turn this watcher left open
          if c.late.watch == watch {
            self.release_late(&mut c, proc_gen);
          }
          return;
        }
        // A prompt's turn has the session: its end starts a watcher of its own, with the replay as it is then
        if c.phase.running && !c.late.open {
          return;
        }
        (self.late_free(&c), !parked(&c).is_empty())
      };
      let moved = if free {
        match self.listed_time(&proc, &acp_id, &cwd).await {
          Some(now) => {
            let moved = seen.as_ref().is_some_and(|s| *s != now);
            seen = Some(now);
            moved
          }
          // A process that streams its late replies still ends one once nothing streamed for a while
          None if self.core.lock().late.streams == Some(proc_gen) => false,
          None => {
            self.log("late replies: the session is not in session/list, giving up");
            self.close_late();
            return;
          }
        }
      } else {
        false
      };
      if moved {
        grew = Some(Instant::now());
      }
      // A process that streams its late replies is never read: the load would stream their steps a second time. One
      // that does not is read a look after its time moved, so an update streamed meanwhile is seen first
      let streams = self.core.lock().late.streams == Some(proc_gen);
      let mut read = false;
      if streams {
        unread = false;
      } else if unread && free {
        match self.read_replay(&proc, req.clone(), watch, proc_gen, &acp_id).await {
          // The next look waits like any other: a CLI whose load moved the time must not be read in a loop
          Read::Replay(replay) => {
            let from = known.unwrap_or_else(|| self.replay_known(&replay));
            if !self.apply_replay(&acp_id, &replay, from, watch, proc_gen) {
              return;
            }
            known = Some(replay.len());
            quiet = None;
            grew = Some(Instant::now());
            unread = false;
            read = true;
          }
          // Read on the next round
          Read::Busy => {}
          Read::Failed => return,
        }
      }
      // A read on this round already holds what this look found
      if moved && !streams && !read {
        unread = true;
      }
      let (open, waiting, on_reply, stirred) = {
        let c = self.core.lock();
        (c.late.open, c.late.open && waits_on_call(&c), ends_on_reply(&c), c.late.stirred)
      };
      if open {
        quiet = None;
        // Only a look that found the time unmoved, with nothing left to read and nothing streamed lately, says the
        // reply stopped growing
        let last = grew.max(stirred);
        let still = free && !moved && !unread && !waiting && last.is_none_or(|at| at.elapsed() > if on_reply { SETTLE } else { SETTLE_MID });
        if still {
          self.close_late();
        }
      } else if live {
        quiet = None;
      } else if quiet.get_or_insert_with(Instant::now).elapsed() > QUIET {
        self.log("late replies: nothing more followed");
        return;
      }
      if started.elapsed() > LIMIT {
        self.log("late replies: gave up watching");
        self.close_late();
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

  /// The late reply stopped growing (or can no longer be followed): its detached turn ends, once
  fn close_late(self: &Arc<Self>) {
    let mut c = self.core.lock();
    if c.late.open {
      self.end_detached(&mut c, TurnStop::EndTurn);
      self.touch(&mut c);
    }
  }

  /// Nothing else is using the session or about to: a replay may be read. An open late reply is the watcher's own turn
  fn late_free(&self, c: &Core) -> bool {
    !((!c.late.open && (c.phase.running || c.peer.detached))
      || c.phase.editing
      || c.phase.edit_waiting
      || c.replaying
      || c.switching
      || c.hooks.gating
      || c.ultracode.rebuilding
      || c.pending_prompt.is_some()
      || c.late.capture.is_some())
  }

  /// The session's `updatedAt` as `session/list` has it. droid 1.14.0 files a session under its folder's resolved path
  /// and lists by exact match, so a folder reached through a symlink (`/var` → `/private/var`) is asked for both ways
  async fn listed_time(&self, proc: &AgentProcess, acp_id: &str, cwd: &str) -> Option<String> {
    if let Some(at) = self.listed_time_in(proc, acp_id, cwd).await {
      return Some(at);
    }
    let resolved = std::fs::canonicalize(cwd).ok()?.to_string_lossy().into_owned();
    if resolved == cwd {
      return None;
    }
    self.listed_time_in(proc, acp_id, &resolved).await
  }

  async fn listed_time_in(&self, proc: &AgentProcess, acp_id: &str, cwd: &str) -> Option<String> {
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
    // A stop retired this watcher while the load ran, or the session moved on: the replay is nobody's
    let current = self.late_current(&c, watch, proc_gen, acp_id);
    if let (Ok(_), true) = (&result, current) {
      drop(c);
      // Only now may the updates after the response go on: they are live ones
      drop(result);
      return Read::Replay(replay);
    }
    self.release_late(&mut c, proc_gen);
    drop(c);
    match result {
      Err(e) => self.log(&format!("late replies: session/load failed: {e}")),
      Ok(_) => self.log("late replies: a replay read for a session that moved on is dropped"),
    }
    // A prompt held back while the replay was captured may go
    self.flush_queue();
    Read::Failed
  }

  /// This watcher lets go of the session: an open late reply ends, a hold taken for a read is released
  fn release_late(self: &Arc<Self>, c: &mut Core, proc_gen: u64) {
    if c.proc_gen != proc_gen {
      return;
    }
    if c.late.open {
      self.end_detached(c, TurnStop::EndTurn);
      self.touch(c);
    } else if !c.phase.running {
      c.peer.detached = false;
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

  /// Settle the commands a replay shows closed, then add what it holds past `known` to the late reply's detached turn,
  /// opening it at the first of it. The turn stays open: `close_late` ends it. Releases the hold `read_replay` took
  /// when there is no such turn. False when the watcher lost the session meanwhile (a stop): nothing is added
  fn apply_replay(self: &Arc<Self>, acp_id: &str, replay: &[Value], known: usize, watch: u64, proc_gen: u64) -> bool {
    let (live, calls) = {
      let mut c = self.core.lock();
      if !self.late_current(&c, watch, proc_gen, acp_id) {
        self.release_late(&mut c, proc_gen);
        drop(c);
        self.flush_queue();
        return false;
      }
      (parked(&c), if c.late.open { open_calls(&c) } else { vec![] })
    };
    // The receipt first, as the update the wire never sent
    let close = |id: &str| {
      let row = antigravity::replay_closed(replay, id)?;
      let mut update = row.clone();
      update["sessionUpdate"] = Value::from("tool_call_update");
      self.on_update(json!({ "sessionId": acp_id, "update": update, REPLAYED: true }));
      Some(closed_state(row))
    };
    for p in live {
      // The row's status is its task's
      let Some(state) = close(&p.call) else { continue };
      let mut c = self.core.lock();
      self.async_task_update(&mut c, acp_id, &parked_task(&p.task, p.task_type.as_deref(), &p.call, Some(state)));
      self.touch(&mut c);
      self.log(&format!("background task {} ended", p.call));
    }
    // A call read while it ran may be closed in place, before `known`, instead of by an update after it
    for id in calls {
      close(&id);
    }
    let tail = antigravity::replay_tail(replay, known);
    {
      let mut c = self.core.lock();
      // Checked again where the turn opens: the calls above ran without the lock
      if !self.late_current(&c, watch, proc_gen, acp_id) {
        self.release_late(&mut c, proc_gen);
        drop(c);
        self.flush_queue();
        return false;
      }
      if tail.is_empty() || !matches!(c.state.turns.last(), Some(Turn::Agent(_))) {
        if !c.late.open {
          c.peer.detached = false;
          drop(c);
          self.flush_queue();
        }
        return true;
      }
      if !c.late.open {
        self.reopen_for_detached(&mut c);
        c.late.open = true;
      }
      self.touch(&mut c);
    }
    self.log(&format!("late reply: {} update(s)", tail.len()));
    for update in tail {
      self.on_update(json!({ "sessionId": acp_id, "update": update, REPLAYED: true }));
    }
    true
  }
}
