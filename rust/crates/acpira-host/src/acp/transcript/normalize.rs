//! ACP session/update → transcript blocks: pure functions over a mutable state.
//!
//! Two TS idioms are explicit here. An open user turn during replay (`_open` on the object) is `NormalizeState::open_user`.
//! An AIR async task's info object is shared between the task map and the tool row hosting it; `TaskBook::linked`
//! records which rows share it, and every change to a linked task is written through to its row

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};

use acpira_shared::inventory::{AgentRuntimeInfo, McpCaps};
use acpira_shared::num::Num;
use acpira_shared::transcript::*;

use crate::acp::session::failure::failure_of;
use crate::acp::transcript::compaction_text;
use crate::acp::transcript::plan_snapshots::{last_plan_snapshot, same_plan_entries};
use crate::acp::transcript::retry_text;
use crate::i18n::{t, tp};
use crate::json::{str_of, text_of};
use crate::util::now_ms;

mod content;
mod controls;
mod notices;
mod tasks;
mod tool_call;
mod tool_kind;
mod tool_target;

use self::content::{image_content, resolve_markdown_images};
pub use self::controls::{apply_config_options, config_option_set_value, init_controls};
use self::notices::{apply_compaction_markers, apply_notice, apply_retry_marker, clear_retry, find_compaction};
pub use self::notices::{apply_goal, apply_session_failure};
pub use self::tasks::{
  apply_async_task, async_task_live, disconnect_async_tasks, find_tool, find_tool_mut, has_live_async_task, set_stop_requested,
};
use self::tasks::{link_async_task, settled_before, time_tool_at, tool_at};
pub use self::tool_call::{empty_tool, permission_tool_update};
use self::tool_call::{merge_tool, time_tool, tool_block};
pub use self::tool_kind::{READ_SESSION_VERB, SEARCH_SESSIONS_VERB, SHOW_IMAGE_VERB};
pub use self::tool_target::command_from_raw;

pub type ImageSaver = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;
pub type FileImageSaver = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// What a tool merge may consult besides the block: background shells, parked writes, blob savers
#[derive(Default, Clone)]
pub struct ToolCtx {
  pub shells: HashMap<String, String>,
  pub pending_writes: HashMap<String, (String, String)>,
  pub save_image: Option<ImageSaver>,
  pub save_image_file: Option<FileImageSaver>,
  /// The session's working directory: relative image paths in reply markdown resolve against it
  pub cwd: Option<String>,
}

#[derive(Default, Clone)]
pub struct TaskBook {
  pub tasks: HashMap<String, AsyncTaskInfo>,
  pub by_tool: HashMap<String, String>,
  pub linked: HashSet<String>,
}

#[derive(Default, Clone)]
pub struct NormalizeState {
  pub turns: Vec<Turn>,
  pub controls: SessionControls,
  pub usage: Option<Usage>,
  pub commands: Vec<SlashCommand>,
  pub title: Option<String>,
  pub thought_started_at: Option<i64>,
  pub open_user: Option<usize>,
  pub image_seq: u64,
  pub ctx: ToolCtx,
  pub tasks: TaskBook,
  pub log: Option<Log>,
  /// The agent id, for adapter strings that stand for structured events (`compaction_text`)
  pub agent: Option<String>,
  /// The agent's current goal (`vendors::goal`), persisted with the record
  pub goal: Option<SessionGoal>,
}

impl NormalizeState {
  pub fn new(turns: Vec<Turn>) -> Self {
    NormalizeState { turns, ..Default::default() }
  }

  fn log(&self, line: &str) {
    if let Some(l) = &self.log {
      l(line);
    }
  }
}

/// What the agent told us in initialize: name / version and MCP transports
pub fn runtime_info_of(init: &Value) -> AgentRuntimeInfo {
  let info = init.get("agentInfo");
  let mcp = init.get("agentCapabilities").and_then(|c| c.get("mcpCapabilities")).filter(|m| !m.is_null());
  AgentRuntimeInfo {
    name: info.and_then(|i| str_of(i, "name")).map(str::to_owned),
    version: info.and_then(|i| str_of(i, "version")).map(str::to_owned),
    mcp: mcp.map(|m| McpCaps { http: crate::json::truthy(m.get("http")), sse: crate::json::truthy(m.get("sse")) }),
  }
}

fn current_agent_turn(s: &mut NormalizeState) -> usize {
  if matches!(s.turns.last(), Some(Turn::Agent(_))) {
    return s.turns.len() - 1;
  }
  s.turns.push(Turn::Agent(AgentTurn::default()));
  s.turns.len() - 1
}

#[expect(clippy::expect_used, reason = "callers pass the index current_agent_turn just returned")]
fn agent(s: &mut NormalizeState, i: usize) -> &mut AgentTurn {
  s.turns[i].as_agent_mut().expect("agent turn")
}

/// Seal the streaming body / thought when the block changes
fn seal_streaming(thought_started_at: &mut Option<i64>, t: &mut AgentTurn) {
  for b in &mut t.blocks {
    match b {
      AgentBlock::Thought(th) if th.streaming == Some(true) => {
        th.streaming = Some(false);
        if let Some(from) = th.started_at.or(*thought_started_at) {
          th.duration_sec = Some(Num(((now_ms() - from) as f64 / 1000.0).round().max(1.0)));
        }
        *thought_started_at = None;
      }
      AgentBlock::Text(tx) if tx.streaming == Some(true) => tx.streaming = Some(false),
      _ => {}
    }
  }
}

fn seal(s: &mut NormalizeState, i: usize) {
  let NormalizeState { turns, thought_started_at, .. } = s;
  if let Some(t) = turns[i].as_agent_mut() {
    seal_streaming(thought_started_at, t);
  }
}

/// A queued prompt steered into the running turn: the streaming body / thought before it is sealed, so the output that
/// answers the steer opens blocks of its own after it
pub fn push_steer(s: &mut NormalizeState, block: SteerBlock) {
  close_user_turn(s);
  let i = current_agent_turn(s);
  seal(s, i);
  agent(s, i).blocks.push(AgentBlock::Steer(block));
}

pub fn text_of_content(c: &Value) -> String {
  match str_of(c, "type") {
    Some("text") => str_of(c, "text").unwrap_or("").to_owned(),
    Some("resource_link") => str_of(c, "uri").unwrap_or("").to_owned(),
    Some("resource") => {
      let r = c.get("resource").unwrap_or(&Value::Null);
      if r.get("text").is_some() { str_of(r, "text").unwrap_or("").to_owned() } else { str_of(r, "uri").unwrap_or("").to_owned() }
    }
    other => format!("[{}]", other.unwrap_or("undefined")),
  }
}

fn close_user_turn(s: &mut NormalizeState) {
  s.open_user = None;
}

fn user_open(s: &NormalizeState) -> bool {
  s.open_user.is_some_and(|i| i + 1 == s.turns.len() && matches!(s.turns.last(), Some(Turn::User(_))))
}

/// One update comes in, mutate state. Returns whether there is a UI-visible change
pub fn apply_update(s: &mut NormalizeState, u: &Value) -> bool {
  let changed = apply_update_kind(s, u);
  if changed {
    mark_responded(s);
  }
  changed
}

/// Stamp the live prompt's first output: the first update that leaves a block in the turn (a thought, text, tool,
/// plan …). Usage, command and config updates add no block and keep the clock waiting; replayed turns have no
/// `started_at` and are never stamped
fn mark_responded(s: &mut NormalizeState) {
  if let Some(Turn::Agent(t)) = s.turns.last_mut()
    && t.started_at.is_some()
    && t.responded_at.is_none()
    && t.stop.is_none()
    && t.ended_at.is_none()
    && !t.blocks.is_empty()
  {
    t.responded_at = Some(now_ms());
  }
}

fn apply_update_kind(s: &mut NormalizeState, u: &Value) -> bool {
  let kind = str_of(u, "sessionUpdate").unwrap_or("");
  match kind {
    "user_message_chunk" => {
      // Appears only during load / resume replay; consecutive chunks merge into the same entry
      let text = text_of_content(u.get("content").unwrap_or(&Value::Null));
      if user_open(s) {
        if let Some(Turn::User(last)) = s.turns.last_mut() {
          last.text.push_str(&text);
        }
      } else {
        s.turns.push(Turn::User(UserTurn { text, ..Default::default() }));
        s.open_user = Some(s.turns.len() - 1);
      }
      true
    }
    "agent_message_chunk" => {
      close_user_turn(s);
      let i = current_agent_turn(s);
      let content = u.get("content").cloned().unwrap_or(Value::Null);
      let img = image_content(&content, Some(&s.ctx));
      if let Some(ToolContent::Image(r)) = img {
        clear_retry(s);
        seal(s, i);
        s.image_seq += 1;
        let id = format!("img-{}", s.image_seq);
        agent(s, i).blocks.push(AgentBlock::Image(ImageBlock { id, blob: r.blob, mime_type: r.mime_type, uri: r.uri }));
        return true;
      }
      let text = match img {
        Some(ToolContent::Text { text }) => text,
        _ => text_of_content(&content),
      };
      if let Some(found) = s.agent.as_deref().and_then(|a| compaction_text::markers(a, &text))
        && apply_compaction_markers(s, i, &found)
      {
        return true;
      }
      if let Some(found) = s.agent.as_deref().and_then(|a| retry_text::marker(a, &text)) {
        apply_retry_marker(s, &found);
        return true;
      }
      if !text.trim().is_empty() {
        clear_retry(s);
      }
      let NormalizeState { turns, ctx, .. } = s;
      if let Some(AgentBlock::Text(last)) = turns[i].as_agent_mut().and_then(|t| t.blocks.last_mut())
        && last.streaming == Some(true)
      {
        let from = last.markdown.len();
        last.markdown.push_str(&text);
        resolve_markdown_images(last, from, ctx);
        return true;
      }
      seal(s, i);
      let mut block = TextBlock { id: None, phase: None, markdown: text, streaming: Some(true), images: None };
      resolve_markdown_images(&mut block, 0, &s.ctx);
      agent(s, i).blocks.push(AgentBlock::Text(block));
      true
    }
    "agent_thought_chunk" => {
      let text = text_of_content(u.get("content").unwrap_or(&Value::Null));
      // Empty deltas carry no reasoning body and must not create a timed disclosure
      if text.is_empty() {
        return false;
      }
      close_user_turn(s);
      let i = current_agent_turn(s);
      if !text.trim().is_empty() {
        clear_retry(s);
      }
      if let Some(AgentBlock::Thought(last)) = agent(s, i).blocks.last_mut()
        && last.streaming == Some(true)
      {
        last.text.push_str(&text);
        return true;
      }
      if text.trim().is_empty() {
        return false;
      }
      seal(s, i);
      let now = now_ms();
      s.thought_started_at = Some(now);
      agent(s, i).blocks.push(AgentBlock::Thought(ThoughtBlock { text, started_at: Some(now), duration_sec: None, streaming: Some(true) }));
      true
    }
    "tool_call" => {
      close_user_turn(s);
      let i = current_agent_turn(s);
      clear_retry(s);
      seal(s, i);
      let id = str_of(u, "toolCallId").unwrap_or("").to_owned();
      // An announcement never revives a finished call of an earlier turn: that id came from another agent session
      // (`settled_before`), so the call gets its own row here and its updates find that row first
      let loc = match find_tool(&s.turns, &id).filter(|&loc| !settled_before(&s.turns, loc, i)) {
        Some(loc) => {
          let NormalizeState { turns, ctx, .. } = s;
          merge_tool(tool_at(turns, loc), u, Some(ctx));
          loc
        }
        None => {
          let block = tool_block(u, Some(&mut s.ctx));
          let t = agent(s, i);
          t.blocks.push(AgentBlock::ToolCall(block));
          (i, t.blocks.len() - 1)
        }
      };
      link_async_task(s, loc);
      time_tool_at(s, i, loc);
      true
    }
    "tool_call_update" => {
      let i = current_agent_turn(s);
      let id = str_of(u, "toolCallId").unwrap_or("").to_owned();
      let loc = match find_tool(&s.turns, &id) {
        Some(loc) => {
          let NormalizeState { turns, ctx, .. } = s;
          merge_tool(tool_at(turns, loc), u, Some(ctx));
          loc
        }
        None => {
          // A first sighting through an update: only the call's own fields, rawOutput waits for the next update
          let mut seed = Map::new();
          seed.insert("toolCallId".into(), Value::from(id.clone()));
          seed.insert("title".into(), u.get("title").filter(|v| !v.is_null()).cloned().unwrap_or(Value::from("")));
          for k in ["kind", "status", "content", "locations", "rawInput", "_meta"] {
            if let Some(v) = u.get(k).filter(|v| !v.is_null()) {
              seed.insert(k.into(), v.clone());
            }
          }
          let block = tool_block(&Value::Object(seed), Some(&mut s.ctx));
          seal(s, i);
          let t = agent(s, i);
          t.blocks.push(AgentBlock::ToolCall(block));
          (i, t.blocks.len() - 1)
        }
      };
      link_async_task(s, loc);
      time_tool_at(s, i, loc);
      true
    }
    "plan" => {
      let entries: Vec<PlanEntry> = u
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|e| PlanEntry {
          title: str_of(e, "content").unwrap_or("").to_owned(),
          status: e.get("status").and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or(PlanStatus::Pending),
          priority: e.get("priority").and_then(|v| serde_json::from_value(v.clone()).ok()),
        })
        .collect();
      // Legacy plan notifications are session snapshots; a repeat must not open a history row
      if last_plan_snapshot(&s.turns).is_some_and(|p| same_plan_entries(&p.entries, &entries)) {
        return false;
      }
      close_user_turn(s);
      let i = current_agent_turn(s);
      let has_plan = agent(s, i).blocks.iter().any(|b| matches!(b, AgentBlock::Plan(_)));
      if has_plan {
        for b in &mut agent(s, i).blocks {
          if let AgentBlock::Plan(p) = b {
            p.entries = entries;
            p.changed = true;
            break;
          }
        }
      } else {
        seal(s, i);
        agent(s, i).blocks.push(AgentBlock::Plan(PlanBlock { entries, changed: true }));
      }
      true
    }
    "plan_update" | "plan_removed" => false,
    "usage_update" => {
      let used = u.get("used").and_then(Value::as_f64).unwrap_or(0.0);
      let size = u.get("size").and_then(Value::as_f64).unwrap_or(0.0);
      let cost = u.get("cost").and_then(|c| c.get("amount")).and_then(Value::as_f64).map(Num);
      s.usage = Some(Usage { used: Num(used), size: Num(size), cost, compacted_from: None });
      // The context snapshot belongs to the turn it followed
      if let Some(Turn::Agent(last)) = s.turns.last_mut() {
        last.usage.get_or_insert_with(Default::default).context = Some(ContextUse { used: Num(used), size: Num(size) });
      }
      true
    }
    "available_commands_update" => {
      // The list replaces the previous one wholesale: an empty update clears the menu
      s.commands = u
        .get("availableCommands")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| SlashCommand {
          name: str_of(c, "name").unwrap_or("").to_owned(),
          description: str_of(c, "description").unwrap_or("").to_owned(),
          input: c.get("input").and_then(|i| text_of(i, "hint")).map(|h| CommandInput { hint: h.to_owned() }),
        })
        .collect();
      true
    }
    "current_mode_update" => {
      s.controls.mode_id = str_of(u, "currentModeId").map(str::to_owned);
      true
    }
    "config_option_update" => {
      apply_config_options(&mut s.controls, u.get("configOptions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]));
      true
    }
    "session_info_update" => {
      // Agents title after the raw prompt (paths and all) with no length cap; hold them to the host's own title rules
      if let Some(title) = text_of(u, "title").map(|t| crate::util::tidy_title(t, crate::limits::TITLE_MAX)).filter(|t| !t.is_empty()) {
        s.title = Some(title);
      }
      // AIR sessionFailure rides this kind: the payload is only in _meta
      let log = s.log.clone();
      let f = failure_of(u.get("_meta"), log.as_deref().map(|l| l as &dyn Fn(&str)));
      if let Some(f) = f {
        apply_session_failure(s, &f);
      }
      true
    }
    "notice" => apply_notice(s, u),
    "compaction_update" => {
      close_user_turn(s);
      let i = current_agent_turn(s);
      let status = match str_of(u, "status") {
        Some("completed") => CompactionStatus::Completed,
        Some("failed") => CompactionStatus::Failed,
        Some("cancelled") => CompactionStatus::Cancelled,
        _ => CompactionStatus::InProgress,
      };
      let id = str_of(u, "compactionId").unwrap_or("").to_owned();
      let error = if status == CompactionStatus::Failed { text_of(u, "error").map(str::to_owned) } else { None };
      if let Some(b) = find_compaction(&mut s.turns, &id) {
        b.status = status;
        b.error = error.or(b.error.take());
      } else {
        seal(s, i);
        agent(s, i).blocks.push(AgentBlock::Compaction(CompactionBlock { id, status, error }));
      }
      true
    }
    _ => false,
  }
}

/// Turn ended: seal streaming blocks, record how it ended; tools still running are marked per stop reason
pub fn end_turn(s: &mut NormalizeState, stop: TurnStop) {
  clear_retry(s);
  let NormalizeState { turns, thought_started_at, .. } = s;
  let Some(Turn::Agent(t)) = turns.last_mut() else { return };
  seal_streaming(thought_started_at, t);
  // Replay-only turns have no live start time; never invent a duration for them
  if t.started_at.is_some() && t.ended_at.is_none() {
    t.ended_at = Some(now_ms());
  }
  t.activity = None;
  t.stop = Some(stop);
  for b in &mut t.blocks {
    if let AgentBlock::ToolCall(tc) = b
      && !async_task_live(tc)
      && tc.status.is_open()
    {
      tc.status = if stop == TurnStop::Cancelled { ToolStatus::Cancelled } else { ToolStatus::Failed };
      time_tool(tc, false);
    }
  }
}

/// session/prompt itself failed: wrap up like a cancellation and keep the error on the turn
pub fn fail_turn(s: &mut NormalizeState, error: TurnError) {
  end_turn(s, TurnStop::Cancelled);
  if let Some(Turn::Agent(t)) = s.turns.last_mut() {
    t.stop = Some(TurnStop::Error);
    t.error = Some(error);
  }
}

/// A session/load replay leaves every block looking mid-stream: seal it without inventing timestamps
pub fn seal_replay(s: &mut NormalizeState) {
  close_user_turn(s);
  for t in s.turns.iter_mut().filter_map(Turn::as_agent_mut) {
    t.activity = None;
    t.retry = None;
    if t.stop.is_none() {
      t.stop = Some(TurnStop::EndTurn);
    }
    for b in &mut t.blocks {
      match b {
        AgentBlock::Text(x) => x.streaming = None,
        AgentBlock::Thought(x) => x.streaming = None,
        AgentBlock::ToolCall(x) if x.status.is_open() => x.status = ToolStatus::Cancelled,
        _ => {}
      }
    }
  }
}

pub use crate::platform::file_url::file_url_to_path;

/// What's happening right now: the Activity line
pub fn activity_of(turns: &[Turn]) -> Option<Activity> {
  live_activity(turns).or_else(|| Some(Activity { kind: ToolKind::Think, label: t("host.working") }))
}

/// The specific action in flight (an open tool call, a pending card, streamed reply text); `None` when the
/// only honest label is the generic "Working".
///
/// An open tool call followed by later model output (a thought or reply text) no longer names the activity:
/// the model has moved past it. codex-acp can leave a call `pending` for good (a summoned Codex consult,
/// 2026-10-15: `tool_call` pending, never updated, the command never ran, the model retried another way and
/// went on for half an hour), and scanning for any open call pinned that dead command on the row. Parallel
/// calls of one step sit next to each other with no output between, so a sibling still running keeps counting.
pub fn live_activity(turns: &[Turn]) -> Option<Activity> {
  let Some(Turn::Agent(turn)) = turns.last() else { return None };
  let mut later_output = false;
  for b in turn.blocks.iter().rev() {
    match b {
      AgentBlock::Thought(_) | AgentBlock::Text(_) => later_output = true,
      AgentBlock::ToolCall(tc) if !later_output && tc.background != Some(true) && tc.status.is_open() => {
        return Some(tool_activity(tc));
      }
      AgentBlock::Permission(_) => return Some(Activity { kind: ToolKind::Other, label: t("host.awaitingApproval") }),
      AgentBlock::Question(q) if q.outcome.is_none() => return Some(Activity { kind: ToolKind::Other, label: t("host.awaitingAnswers") }),
      _ => {}
    }
  }
  match turn.blocks.last() {
    Some(AgentBlock::Text(tx)) if tx.streaming == Some(true) => Some(Activity { kind: ToolKind::Other, label: t("host.replying") }),
    _ => None,
  }
}

/// The latest foreground tool call of the last agent turn, open or finished
pub fn latest_step(turns: &[Turn]) -> Option<Activity> {
  let Some(Turn::Agent(turn)) = turns.last() else { return None };
  turn.blocks.iter().rev().find_map(|b| match b {
    AgentBlock::ToolCall(tc) if tc.background != Some(true) => Some(tool_activity(tc)),
    _ => None,
  })
}

fn tool_activity(tc: &ToolCallBlock) -> Activity {
  let label = tp("host.doing", &[("verb", &tc.verb), ("target", tc.target.as_deref().unwrap_or(""))]);
  Activity { kind: tc.kind, label: label.trim().to_owned() }
}

/// A tool content JSON value, for callers that build updates by hand
pub fn tool_call_update(id: &str, fields: Value) -> Value {
  let mut v = json!({ "sessionUpdate": "tool_call_update", "toolCallId": id });
  if let (Some(o), Some(f)) = (v.as_object_mut(), fields.as_object()) {
    for (k, x) in f {
      o.insert(k.clone(), x.clone());
    }
  }
  v
}

#[cfg(test)]
mod retitled_row_tests {
  use super::*;
  use serde_json::json;

  // The built-in agent turns the row a stream opened for an edit into the row of the to-do list the edit carried
  // (acpira-agent `turn/riders.rs`, `ride_todo`), so the list sits above the edit
  #[test]
  fn a_streamed_row_retitled_todo_becomes_the_to_do_row() {
    let mut s = NormalizeState::new(vec![]);
    let todos = json!({ "todos": [{ "content": "fix it", "status": "in_progress" }] });
    apply_update(&mut s, &json!({ "sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "Edit", "kind": "edit", "status": "pending" }));
    apply_update(&mut s, &json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "title": "todo", "kind": "other", "status": "pending", "rawInput": todos }));
    apply_update(&mut s, &json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed", "rawOutput": todos }));
    let loc = find_tool(&s.turns, "call-1").unwrap();
    let AgentBlock::ToolCall(b) = &agent(&mut s, loc.0).blocks[loc.1] else { panic!("not a tool row") };
    assert!(acpira_shared::todo_tools::is_todo_tool(b), "{:?} {:?}", b.verb_key, b.kind);
    assert_eq!(b.todo_entries.as_ref().expect("the to-do bar reads the list")[0].title, "fix it");
  }
}

#[cfg(test)]
mod notice_tests {
  use super::*;
  use serde_json::json;

  // claude-agent-acp 0.93.0 acknowledges a stopped background task with an info notice after the turn ended
  #[test]
  fn a_notice_after_the_reply_becomes_its_own_row_on_the_settled_turn() {
    let mut s = NormalizeState::new(vec![]);
    apply_update(&mut s, &json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "waiting on [08:" } }));
    end_turn(&mut s, TurnStop::EndTurn);
    let task = json!({ "sessionUpdate": "notice", "severity": "info", "title": "Task stopped by user", "description": "grep \"^[08:(0[0-9]\" log." });
    assert!(apply_update(&mut s, &task));
    assert!(apply_update(&mut s, &json!({ "sessionUpdate": "notice", "severity": "warning", "title": "Model fallback" })));
    assert!(!apply_update(&mut s, &json!({ "sessionUpdate": "notice", "severity": "info", "title": " " })));
    assert_eq!(s.turns.len(), 1);
    let Some(Turn::Agent(t)) = s.turns.last() else { panic!("no agent turn") };
    assert_eq!(t.stop, Some(TurnStop::EndTurn));
    let AgentBlock::Text(reply) = &t.blocks[0] else { panic!("reply first") };
    assert_eq!(reply.markdown, "waiting on [08:");
    let AgentBlock::Notice(n) = &t.blocks[1] else { panic!("notice row") };
    assert_eq!((n.id.as_str(), n.severity, n.info, n.title.as_str()), ("notice:1", Severity::Warning, true, "Task stopped by user"));
    assert_eq!(n.details.as_deref(), Some("grep \"^[08:(0[0-9]\" log."));
    let AgentBlock::Notice(n) = &t.blocks[2] else { panic!("second notice row") };
    assert_eq!((n.id.as_str(), n.severity, n.info), ("notice:2", Severity::Warning, false));
  }
}
