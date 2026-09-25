//! ACP session/update → transcript blocks: pure functions over a mutable state.
//!
//! Two TS idioms are explicit here. An open user turn during replay (`_open` on the object) is `NormalizeState::open_user`.
//! An AIR async task's info object is shared between the task map and the tool row hosting it; `TaskBook::linked`
//! records which rows share it, and every change to a linked task is written through to its row

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use regex::Regex;
use serde_json::{Map, Value, json};

use acpira_shared::attachments::{base64_bytes, image_mime_of};
use acpira_shared::inventory::{AgentRuntimeInfo, McpCaps};
use acpira_shared::num::Num;
use acpira_shared::todo_tools::{is_todo_tool, todo_entries};
use acpira_shared::transcript::*;

use super::compaction_text;
use super::diff::diff_lines;
use super::plan_snapshots::{last_plan_snapshot, same_plan_entries};
use super::retry_text;
use super::session_failure::{SessionFailure, failure_of};
use super::wire::{AsyncTaskEvent, TaskEventKind};
use crate::i18n::{t, tp};
use crate::json::{basename, pretty, slice16, str_of, text_of};
use crate::limits::{MAX_OUT_IMAGE_BYTES, TOOL_OUTPUT_MAX};
use crate::util::now_ms;

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
  /// The notice of the retry run in progress (`retry_text`) and its last attempt number, until the adapter says it resumed
  pub retry: Option<(String, Option<u32>)>,
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
        apply_retry_marker(s, i, &found);
        return true;
      }
      let t = agent(s, i);
      if let Some(AgentBlock::Text(last)) = t.blocks.last_mut()
        && last.streaming == Some(true)
      {
        last.markdown.push_str(&text);
        return true;
      }
      seal(s, i);
      agent(s, i).blocks.push(AgentBlock::Text(TextBlock { id: None, phase: None, markdown: text, streaming: Some(true) }));
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
      seal(s, i);
      let id = str_of(u, "toolCallId").unwrap_or("").to_owned();
      let loc = match find_tool(&s.turns, &id) {
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
      s.usage = Some(Usage { used: Num(used), size: Num(size), cost });
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
      if let Some(title) = text_of(u, "title") {
        s.title = Some(title.to_owned());
      }
      // AIR sessionFailure rides this kind: the payload is only in _meta
      let log = s.log.clone();
      let f = failure_of(u.get("_meta"), log.as_deref().map(|l| l as &dyn Fn(&str)));
      if let Some(f) = f {
        apply_session_failure(s, &f);
      }
      true
    }
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

/// Prose compaction markers become compaction blocks. A start opens a row in place; an outcome settles the latest open
/// prose compaction, which may sit in an earlier turn (Devin / Kimi finish /compact in the background), or stands alone.
/// Detail lines are absorbed only right under a compaction row; false leaves the chunk to render as text
fn apply_compaction_markers(s: &mut NormalizeState, i: usize, found: &[compaction_text::Marker]) -> bool {
  use compaction_text::Marker;
  let under_row = matches!(agent(s, i).blocks.last(), Some(AgentBlock::Compaction(_)));
  if found.iter().all(|m| *m == Marker::Detail) {
    return under_row;
  }
  seal(s, i);
  for m in found {
    let (status, error) = match m {
      Marker::Detail => continue,
      // A repeated start while this turn's prose compaction is still open is the same compaction
      Marker::Start if agent(s, i).blocks.iter().any(open_text_compaction) => continue,
      Marker::Start => {
        s.image_seq += 1;
        let id = format!("{TEXT_COMPACTION}{}-{}", now_ms(), s.image_seq);
        agent(s, i).blocks.push(AgentBlock::Compaction(CompactionBlock { id, status: CompactionStatus::InProgress, error: None }));
        continue;
      }
      Marker::Done => (CompactionStatus::Completed, None),
      Marker::Cancelled => (CompactionStatus::Cancelled, None),
      Marker::Failed(e) => (CompactionStatus::Failed, Some(e.clone())),
    };
    let open = s.turns.iter_mut().rev().filter_map(Turn::as_agent_mut).find_map(|t| {
      t.blocks.iter_mut().rev().find(|b| open_text_compaction(b)).and_then(|b| match b {
        AgentBlock::Compaction(c) => Some(c),
        _ => None,
      })
    });
    match open {
      Some(c) => {
        c.status = status;
        c.error = error;
      }
      None => {
        s.image_seq += 1;
        let id = format!("{TEXT_COMPACTION}{}-{}", now_ms(), s.image_seq);
        agent(s, i).blocks.push(AgentBlock::Compaction(CompactionBlock { id, status, error }));
      }
    }
  }
  true
}

/// Id prefix of compaction rows synthesized from prose, so an outcome never settles a structured compaction
const TEXT_COMPACTION: &str = "text-compaction-";

fn open_text_compaction(b: &AgentBlock) -> bool {
  matches!(b, AgentBlock::Compaction(c) if c.status == CompactionStatus::InProgress && c.id.starts_with(TEXT_COMPACTION))
}

fn find_compaction<'a>(turns: &'a mut [Turn], id: &str) -> Option<&'a mut CompactionBlock> {
  turns.iter_mut().rev().filter_map(Turn::as_agent_mut).find_map(|t| {
    t.blocks.iter_mut().find_map(|b| match b {
      AgentBlock::Compaction(c) if c.id == id => Some(c),
      _ => None,
    })
  })
}

/// Prose retries become one quiet notice per retry run: each attempt rewrites it in place and the adapter's resume line
/// settles it. A resume line without a run in progress is dropped, never rendered as reply text
fn apply_retry_marker(s: &mut NormalizeState, i: usize, m: &retry_text::Marker) {
  use retry_text::Marker;
  let n = |v: u32| v.to_string();
  let finished = *m == Marker::Finished;
  let (title, attempt) = match *m {
    Marker::Attempt(Some((a, max, wait))) => (tp("host.retrying", &[("attempt", &n(a)), ("max", &n(max)), ("seconds", &n(wait))]), Some(a)),
    Marker::Attempt(None) => (t("host.retryingBare"), None),
    Marker::Finished => match s.retry.as_ref().and_then(|r| r.1) {
      Some(a) => (tp("host.retryFinished", &[("attempt", &n(a))]), Some(a)),
      None => (t("host.retryFinishedBare"), None),
    },
  };
  let open_id = s.retry.take().map(|r| r.0);
  let existing = open_id.as_deref().and_then(|id| find_notice(&mut s.turns, id));
  let id = match existing {
    Some(notice) => {
      notice.revision = Num(notice.revision.0 + 1.0);
      notice.title = title;
      notice.id.clone()
    }
    None if finished => return,
    None => {
      seal(s, i);
      s.image_seq += 1;
      let id = format!("text-retry-{}-{}", now_ms(), s.image_seq);
      agent(s, i).blocks.push(AgentBlock::Notice(NoticeBlock {
        id: id.clone(),
        revision: Num(1.0),
        category: FailureCategory::Connection,
        severity: Severity::Warning,
        title,
        details: None,
        actions: vec![],
      }));
      id
    }
  };
  if !finished {
    s.retry = Some((id, attempt));
  }
}

fn find_notice<'a>(turns: &'a mut [Turn], id: &str) -> Option<&'a mut NoticeBlock> {
  turns.iter_mut().rev().filter_map(Turn::as_agent_mut).find_map(|t| {
    t.blocks.iter_mut().find_map(|b| match b {
      AgentBlock::Notice(n) if n.id == id => Some(n),
      _ => None,
    })
  })
}

/// Turn ended: seal streaming blocks, record how it ended; tools still running are marked per stop reason
pub fn end_turn(s: &mut NormalizeState, stop: TurnStop) {
  s.retry = None;
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

/// One row per failure id in the whole transcript; a higher revision rewrites it in place
pub fn apply_session_failure(s: &mut NormalizeState, f: &SessionFailure) -> bool {
  if let Some(n) = find_notice(&mut s.turns, &f.id) {
    if n.revision.0 >= f.revision {
      return false;
    }
    n.revision = Num(f.revision);
    n.category = f.category;
    n.severity = f.severity;
    n.title = f.title.clone();
    n.details = f.details.clone();
    n.actions = f.actions.clone();
    return true;
  }
  let notice = AgentBlock::Notice(NoticeBlock {
    id: f.id.clone(),
    revision: Num(f.revision),
    category: f.category,
    severity: f.severity,
    title: f.title.clone(),
    details: f.details.clone().filter(|d| !d.is_empty()),
    actions: f.actions.clone(),
  });
  push_standalone(s, notice);
  true
}

// A block that lands outside a live turn: on the last agent turn, or a new settled one
fn push_standalone(s: &mut NormalizeState, block: AgentBlock) {
  if let Some(Turn::Agent(last)) = s.turns.last_mut() {
    last.blocks.push(block);
  } else {
    s.turns.push(Turn::Agent(AgentTurn { blocks: vec![block], stop: Some(TurnStop::EndTurn), ..Default::default() }));
  }
}

fn task_terminal(st: AsyncTaskState) -> bool {
  matches!(st, AsyncTaskState::Completed | AsyncTaskState::Failed | AsyncTaskState::Stopped)
}

fn task_state_moves(from: AsyncTaskState, to: AsyncTaskState) -> bool {
  !task_terminal(from) || (from == AsyncTaskState::Stopped && matches!(to, AsyncTaskState::Completed | AsyncTaskState::Failed))
}

fn task_status(st: AsyncTaskState) -> ToolStatus {
  match st {
    AsyncTaskState::Completed => ToolStatus::Completed,
    AsyncTaskState::Failed => ToolStatus::Failed,
    AsyncTaskState::Stopped => ToolStatus::Cancelled,
    _ => ToolStatus::InProgress,
  }
}

/// The task is still observable and owns its row across turn ends
pub fn async_task_live(b: &ToolCallBlock) -> bool {
  b.async_task.as_ref().is_some_and(|a| b.observation != Some(Observation::Unknown) && !task_terminal(a.state))
}

fn block_by_task(turns: &[Turn], task_id: &str) -> Option<(usize, usize)> {
  for (ti, t) in turns.iter().enumerate() {
    let Turn::Agent(a) = t else { continue };
    if let Some(bi) =
      a.blocks.iter().position(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.async_task.as_ref().is_some_and(|x| x.id == task_id)))
    {
      return Some((ti, bi));
    }
  }
  None
}

fn tool_at(turns: &mut [Turn], (ti, bi): (usize, usize)) -> &mut ToolCallBlock {
  match &mut turns[ti].as_agent_mut().expect("agent turn").blocks[bi] {
    AgentBlock::ToolCall(tc) => tc,
    _ => unreachable!("locator points at a tool call"),
  }
}

pub fn find_tool(turns: &[Turn], id: &str) -> Option<(usize, usize)> {
  for (ti, t) in turns.iter().enumerate().rev() {
    let Turn::Agent(a) = t else { continue };
    if let Some(bi) = a.blocks.iter().position(|b| matches!(b, AgentBlock::ToolCall(tc) if tc.id == id)) {
      return Some((ti, bi));
    }
  }
  None
}

pub fn find_tool_mut<'a>(turns: &'a mut [Turn], id: &str) -> Option<&'a mut ToolCallBlock> {
  let loc = find_tool(turns, id)?;
  Some(tool_at(turns, loc))
}

/// The named row adopts the task; a synthesized placeholder hosting it first comes out. Returns the row's new locator
fn attach_async_task(s: &mut NormalizeState, mut loc: (usize, usize), task_id: &str) -> (usize, usize) {
  if let Some(prev) = block_by_task(&s.turns, task_id)
    && prev != loc
  {
    let synthesized = tool_at(&mut s.turns, prev).id == format!("async:{task_id}");
    if synthesized {
      s.turns[prev.0].as_agent_mut().unwrap().blocks.remove(prev.1);
      if prev.0 == loc.0 && prev.1 < loc.1 {
        loc.1 -= 1;
      }
    } else {
      tool_at(&mut s.turns, prev).async_task = None;
    }
  }
  let info = s.tasks.tasks.get(task_id).cloned();
  let b = tool_at(&mut s.turns, loc);
  b.async_task = info;
  b.background = Some(true);
  s.tasks.linked.insert(task_id.to_owned());
  loc
}

/// Write a linked task's info through to the row hosting it
fn sync_task(s: &mut NormalizeState, task_id: &str) {
  if !s.tasks.linked.contains(task_id) {
    return;
  }
  let Some(info) = s.tasks.tasks.get(task_id).cloned() else { return };
  if let Some(loc) = block_by_task(&s.turns, task_id) {
    tool_at(&mut s.turns, loc).async_task = Some(info);
  }
}

/// A task event named a toolCallId before the row existed; the row's arrival adopts the parked task
fn link_async_task(s: &mut NormalizeState, loc: (usize, usize)) {
  let id = tool_at(&mut s.turns, loc).id.clone();
  let Some(task_id) = s.tasks.by_tool.remove(&id) else { return };
  let Some(state) = s.tasks.tasks.get(&task_id).map(|i| i.state) else { return };
  let loc = attach_async_task(s, loc, &task_id);
  tool_at(&mut s.turns, loc).status = task_status(state);
}

fn time_tool_at(s: &mut NormalizeState, current: usize, loc: (usize, usize)) {
  let live = loc.0 == current && s.turns[current].as_agent().is_some_and(|t| t.started_at.is_some() && t.stop.is_none());
  // The block may have moved (a placeholder removal); find it again by position validity
  if let Some(Turn::Agent(t)) = s.turns.get_mut(loc.0)
    && let Some(AgentBlock::ToolCall(tc)) = t.blocks.get_mut(loc.1)
  {
    time_tool(tc, live);
  }
}

/// One asyncTasks event on the owning session's transcript; returns whether the transcript changed
pub fn apply_async_task(s: &mut NormalizeState, e: &AsyncTaskEvent) -> bool {
  let id = e.async_task_id.clone();
  {
    let info = s.tasks.tasks.entry(id.clone()).or_insert_with(|| AsyncTaskInfo {
      id: id.clone(),
      state: AsyncTaskState::Running,
      can_stop: false,
      task_type: None,
      name: None,
      description: None,
      summary: None,
      last_tool_name: None,
      output_file_path: None,
      usage: None,
      stop_requested: false,
    });
    if e.name.is_some() {
      info.name = e.name.clone();
    }
    if e.task_type.is_some() {
      info.task_type = e.task_type.clone();
    }
    if e.description.is_some() {
      info.description = e.description.clone();
    }
    if e.summary.is_some() {
      info.summary = e.summary.clone();
    }
    if e.last_tool_name.is_some() {
      info.last_tool_name = e.last_tool_name.clone();
    }
    if e.output_file_path.is_some() {
      info.output_file_path = e.output_file_path.clone();
    }
    if let Some(u) = &e.usage {
      let cur = info.usage.get_or_insert_with(Default::default);
      if u.total_tokens.is_some() {
        cur.total_tokens = u.total_tokens;
      }
      if u.tool_uses.is_some() {
        cur.tool_uses = u.tool_uses;
      }
      if u.duration_ms.is_some() {
        cur.duration_ms = u.duration_ms;
      }
    }
    if let Some(c) = e.can_stop {
      info.can_stop = c;
    }
    if e.event == TaskEventKind::State
      && let Some(st) = e.state
    {
      if task_state_moves(info.state, st) {
        info.state = st;
      }
      if task_terminal(info.state) {
        info.stop_requested = false;
      }
    }
  }
  sync_task(s, &id);
  let state = s.tasks.tasks[&id].state;
  let named = e.tool_call_id.as_deref().and_then(|tid| find_tool(&s.turns, tid));
  if let Some(tid) = &e.tool_call_id {
    if named.is_some() {
      s.tasks.by_tool.remove(tid);
    } else {
      s.tasks.by_tool.insert(tid.clone(), id.clone());
    }
  }
  let block = named.or_else(|| block_by_task(&s.turns, &id));
  if let Some(mut loc) = block {
    if named.is_some() {
      loc = attach_async_task(s, loc, &id);
    }
    let b = tool_at(&mut s.turns, loc);
    // A live wire event is observation itself
    b.observation = None;
    b.status = task_status(state);
    if task_terminal(state) {
      if b.ended_at.is_none() {
        b.ended_at = Some(now_ms());
      }
    } else {
      time_tool(b, true);
    }
    return true;
  }
  if e.show_in_transcript {
    let kind = if e.task_type.as_deref() == Some("shell") { ToolKind::Execute } else { ToolKind::Other };
    let row = ToolCallBlock {
      id: format!("async:{id}"),
      kind,
      verb: verb_of(kind),
      target: e.name.clone().or_else(|| e.description.clone()),
      status: task_status(state),
      background: Some(true),
      started_at: Some(now_ms()),
      async_task: Some(s.tasks.tasks[&id].clone()),
      ..empty_tool(String::new())
    };
    s.tasks.linked.insert(id);
    push_standalone(s, AgentBlock::ToolCall(row));
    return true;
  }
  s.log(&format!("async task {id}: no transcript row (no toolCallId match, showInTranscript off)"));
  false
}

/// Mark a stop request on a task (shared with its row); false when the task is unknown
pub fn set_stop_requested(s: &mut NormalizeState, task_id: &str, on: bool) -> bool {
  let Some(info) = s.tasks.tasks.get_mut(task_id) else { return false };
  info.stop_requested = on;
  sync_task(s, task_id);
  true
}

/// The process carrying the tasks is gone: live tasks lose their observer
pub fn disconnect_async_tasks(s: &mut NormalizeState) {
  let mut touched = vec![];
  for t in s.turns.iter_mut().filter_map(Turn::as_agent_mut) {
    for b in &mut t.blocks {
      let AgentBlock::ToolCall(tc) = b else { continue };
      let Some(a) = tc.async_task.as_mut() else { continue };
      if task_terminal(a.state) {
        continue;
      }
      tc.observation = Some(Observation::Unknown);
      a.can_stop = false;
      a.stop_requested = false;
      touched.push(a.id.clone());
    }
  }
  for id in touched {
    if s.tasks.linked.contains(&id)
      && let Some(info) = s.tasks.tasks.get_mut(&id)
    {
      info.can_stop = false;
      info.stop_requested = false;
    }
  }
}

/// A session/load replay leaves every block looking mid-stream: seal it without inventing timestamps
pub fn seal_replay(s: &mut NormalizeState) {
  close_user_turn(s);
  for t in s.turns.iter_mut().filter_map(Turn::as_agent_mut) {
    t.activity = None;
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

fn kind_of(meta: Option<&Value>) -> Option<String> {
  meta.and_then(|m| text_of(m, "kind")).map(str::to_owned)
}

/// Build the controls from a session/new / resume / load response
pub fn init_controls(controls: &mut SessionControls, modes: Option<&Value>, config_options: Option<&Value>) {
  let modes = modes.filter(|m| !m.is_null());
  controls.modes = modes
    .and_then(|m| m.get("availableModes"))
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .map(|m| SessionOption {
      id: str_of(m, "id").unwrap_or("").to_owned(),
      name: str_of(m, "name").unwrap_or("").to_owned(),
      description: str_of(m, "description").map(str::to_owned),
      kind: kind_of(m.get("_meta")),
      ..Default::default()
    })
    .collect();
  controls.mode_id = modes.and_then(|m| str_of(m, "currentModeId")).map(str::to_owned);
  if let Some(opts) = config_options.and_then(Value::as_array) {
    apply_config_options(controls, opts);
  }
}

const CATEGORY_ORDER: [&str; 3] = ["model", "thought_level", "model_config"];

fn bool_options() -> Vec<SessionOption> {
  vec![
    SessionOption { id: "false".into(), name: "Off".into(), ..Default::default() },
    SessionOption { id: "true".into(), name: "On".into(), ..Default::default() },
  ]
}

/// A category=mode select is treated purely as modes; the remaining selects / booleans are ordered model →
/// thought_level → model_config → others, keeping the agent's order within each class
pub fn apply_config_options(controls: &mut SessionControls, options: &[Value]) {
  let mode = options.iter().find(|o| str_of(o, "type") == Some("select") && str_of(o, "category") == Some("mode"));
  if let Some(mode) = mode {
    let id = str_of(mode, "id").unwrap_or("");
    if controls.modes.is_empty() || controls.mode_config_id.as_deref() == Some(id) {
      controls.modes = flatten_select(mode.get("options"));
      controls.mode_id = str_of(mode, "currentValue").map(str::to_owned);
      controls.mode_config_id = Some(id.to_owned());
    }
  }
  let rank = |c: &ConfigControl| CATEGORY_ORDER.iter().position(|x| Some(*x) == c.category.as_deref()).unwrap_or(CATEGORY_ORDER.len());
  let mut list: Vec<ConfigControl> = options
    .iter()
    .filter(|o| matches!(str_of(o, "type"), Some("select" | "boolean")) && str_of(o, "category") != Some("mode"))
    .map(|o| {
      let category = str_of(o, "category").map(str::to_owned);
      let (id, name) = (str_of(o, "id").unwrap_or("").to_owned(), str_of(o, "name").unwrap_or("").to_owned());
      if str_of(o, "type") == Some("boolean") {
        let value = match o.get("currentValue") {
          Some(Value::Bool(b)) => b.to_string(),
          Some(Value::String(s)) => s.clone(),
          Some(Value::Null) => "null".into(),
          Some(other) => other.to_string(),
          None => "undefined".into(),
        };
        ConfigControl { id, name, category, kind: Some(ControlType::Boolean), options: bool_options(), value: Some(value) }
      } else {
        ConfigControl {
          id,
          name,
          category,
          kind: None,
          options: flatten_select(o.get("options")),
          value: str_of(o, "currentValue").map(str::to_owned),
        }
      }
    })
    .collect();
  list.sort_by_key(|c| rank(c));
  controls.options = list;
}

/// A user pick → the session/set_config_option payload fields; booleans travel as `type: 'boolean'` + a real boolean
pub fn config_option_set_value(control: Option<&ConfigControl>, value: &str) -> Map<String, Value> {
  let mut m = Map::new();
  if control.is_some_and(|c| c.kind == Some(ControlType::Boolean)) {
    m.insert("type".into(), Value::from("boolean"));
    m.insert("value".into(), Value::Bool(value == "true"));
  } else {
    m.insert("value".into(), Value::from(value));
  }
  m
}

fn flatten_select(opts: Option<&Value>) -> Vec<SessionOption> {
  let mut out = vec![];
  for o in opts.and_then(Value::as_array).into_iter().flatten() {
    if o.get("group").is_some() {
      let gname = str_of(o, "name").unwrap_or("").to_owned();
      let gid = o.get("group").and_then(Value::as_str).unwrap_or("").to_owned();
      for x in o.get("options").and_then(Value::as_array).into_iter().flatten() {
        out.push(SessionOption {
          id: str_of(x, "value").unwrap_or("").to_owned(),
          name: str_of(x, "name").unwrap_or("").to_owned(),
          description: Some(str_of(x, "description").map(str::to_owned).unwrap_or_else(|| gname.clone())),
          group: Some(OptionGroup { id: gid.clone(), name: gname.clone() }),
          source: None,
          kind: kind_of(x.get("_meta")),
        });
      }
    } else {
      out.push(SessionOption {
        id: str_of(o, "value").unwrap_or("").to_owned(),
        name: str_of(o, "name").unwrap_or("").to_owned(),
        description: str_of(o, "description").map(str::to_owned),
        group: None,
        source: None,
        kind: kind_of(o.get("_meta")),
      });
    }
  }
  out
}

fn verb_key(kind: ToolKind) -> &'static str {
  match kind {
    ToolKind::Read => "verb.read",
    ToolKind::Edit => "verb.edit",
    ToolKind::Delete => "verb.delete",
    ToolKind::Move => "verb.move",
    ToolKind::Search => "verb.search",
    ToolKind::Execute => "verb.execute",
    ToolKind::Think => "verb.think",
    ToolKind::Fetch => "verb.fetch",
    ToolKind::SwitchMode => "verb.switch_mode",
    ToolKind::Other => "verb.other",
  }
}

pub fn verb_of(kind: ToolKind) -> String {
  t(verb_key(kind))
}

static TODO_TITLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^todo([_\s-]?(write|update|read|list))?$").unwrap());
static ASK_TITLE: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"(?i)^(ask_?user_?questions?|ask(ed|ing)?\s+(the\s+)?(user\s+)?(\d+\s+)?questions?\b)").unwrap());
static SHELL_WAIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(get_output|read(ing)?\s+shell(\s+output)?)$").unwrap());
static SHELL_KILL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(kill_shell|kill(ing)?\s+shell)$").unwrap());
static KIND_BY_TITLE: LazyLock<Vec<(Regex, ToolKind)>> = LazyLock::new(|| {
  [
    (r"(?i)^(read|open|view|cat)(_[a-z]+)*$", ToolKind::Read),
    (r"(?i)^(write|edit|create|patch|apply_?patch|str_?replace|insert)(_[a-z]+)*$", ToolKind::Edit),
    (r"(?i)^(list|ls|dir|glob|grep|find|search)(_[a-z]+)*$", ToolKind::Search),
    (r"(?i)^(web_?search|google|bing)(_[a-z]+)*$", ToolKind::Search),
    (r"(?i)^(bash|shell|terminal|exec|execute|run|command)(_[a-z]+)*$", ToolKind::Execute),
    (r"(?i)^(web_?fetch|fetch|browse|curl)(_[a-z]+)*$", ToolKind::Fetch),
  ]
  .into_iter()
  .map(|(r, k)| (Regex::new(r).unwrap(), k))
  .collect()
});
static STRIP_VERB: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?i)^(read(ing)?|edit(ing)?|write|writing|search(ing)?|run(ning)?|execute|executing|fetch(ing)?|delete|deleting|move|moving|list(ing)?)\s+(file|files|directory|command)?\s*").unwrap()
});
static DATA_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)^data:([\w.+-]+/[\w.+-]+)?;base64,(.+)$").unwrap());

fn shell_verb(meta: Option<&Map<String, Value>>, title: Option<&str>) -> Option<&'static str> {
  match meta.and_then(|m| m.get("cognition.ai/inferenceToolName")).and_then(Value::as_str) {
    Some("get_output") => return Some("verb.wait"),
    Some("kill_shell") => return Some("verb.kill"),
    _ => {}
  }
  let title = title.filter(|t| !t.is_empty())?.trim();
  if SHELL_WAIT.is_match(title) {
    Some("verb.wait")
  } else if SHELL_KILL.is_match(title) {
    Some("verb.kill")
  } else {
    None
  }
}

fn infer_kind(title: Option<&str>) -> Option<ToolKind> {
  let name = title.filter(|t| !t.is_empty())?.trim();
  KIND_BY_TITLE.iter().find(|(re, _)| re.is_match(name)).map(|(_, k)| *k)
}

/// Sparse updates retain the first observed start and the first terminal timestamp
pub fn time_tool(b: &mut ToolCallBlock, live: bool) {
  if live && b.status == ToolStatus::InProgress && b.ended_at.is_none() && b.started_at.is_none() {
    b.started_at = Some(now_ms());
  }
  if b.started_at.is_some() && !b.status.is_open() && b.ended_at.is_none() {
    b.ended_at = Some(now_ms());
  }
}

pub fn empty_tool(id: String) -> ToolCallBlock {
  ToolCallBlock {
    observation: None,
    id,
    kind: ToolKind::Other,
    verb: String::new(),
    verb_key: None,
    target: None,
    target_mono: None,
    locations: None,
    read_range: None,
    status: ToolStatus::Pending,
    background: None,
    async_task: None,
    started_at: None,
    ended_at: None,
    meta: None,
    diff_stat: None,
    todo_entries: None,
    content: None,
    contents: None,
    subagent_id: None,
  }
}

fn kind_field(u: &Value) -> Option<ToolKind> {
  str_of(u, "kind").filter(|k| !k.is_empty()).and_then(ToolKind::parse)
}

pub fn tool_block(tc: &Value, ctx: Option<&mut ToolCtx>) -> ToolCallBlock {
  let kind = kind_field(tc).unwrap_or(ToolKind::Other);
  let mut b = ToolCallBlock {
    kind,
    verb: verb_of(kind),
    status: str_of(tc, "status").and_then(ToolStatus::parse).unwrap_or(ToolStatus::Pending),
    ..empty_tool(str_of(tc, "toolCallId").unwrap_or("").to_owned())
  };
  merge_tool(&mut b, tc, ctx);
  b
}

struct Target {
  text: String,
  mono: bool,
  from_title: bool,
}

/// Fields of tool_call and tool_call_update are all optional; overwrite only the ones provided
pub fn merge_tool(b: &mut ToolCallBlock, u: &Value, mut ctx: Option<&mut ToolCtx>) {
  let meta = u.get("_meta").and_then(Value::as_object);
  let raw = u.get("rawInput").and_then(Value::as_object);
  let raw_present = crate::json::truthy(u.get("rawInput"));
  let title = str_of(u, "title");
  if let Some(k) = kind_field(u) {
    b.kind = k;
    b.verb = verb_of(k);
  }
  let set_verb = |b: &mut ToolCallBlock, key: &str| {
    b.verb_key = Some(key.to_owned());
    b.verb = t(key);
  };
  if title.is_some_and(|x| !x.is_empty() && TODO_TITLE.is_match(x.trim())) {
    set_verb(b, "verb.todo");
  }
  let tool_name = meta
    .and_then(|m| m.get("x.ai/tool"))
    .and_then(|x| x.get("name"))
    .filter(|v| !v.is_null())
    .or_else(|| meta.and_then(|m| m.get("cognition.ai/inferenceToolName")));
  if let Some(name) = tool_name.and_then(Value::as_str)
    && TODO_TITLE.is_match(name)
  {
    set_verb(b, "verb.todo");
  }
  if title.is_some_and(|x| !x.is_empty() && ASK_TITLE.is_match(x.trim())) {
    set_verb(b, "verb.ask");
  }
  if let Some(shell) = shell_verb(meta, title) {
    set_verb(b, shell);
  }
  if b.verb_key.is_none()
    && matches!(b.kind, ToolKind::Other | ToolKind::Think)
    && let Some(inferred) = infer_kind(title)
  {
    b.kind = inferred;
    b.verb = verb_of(inferred);
  }
  // A row an async task owns takes its status from the task's state updates
  if let Some(st) = str_of(u, "status").filter(|x| !x.is_empty()).and_then(ToolStatus::parse)
    && b.async_task.is_none()
  {
    b.status = st;
  }
  if meta.and_then(|m| m.get("cognition.ai/background")) == Some(&Value::Bool(true)) {
    b.background = Some(true);
    let m = meta.unwrap();
    if let (Some(ctx), Some(shell_id)) = (ctx.as_deref_mut(), m.get("cognition.ai/backgroundShellId").and_then(Value::as_str)) {
      let command = m.get("cognition.ai/backgroundCommand").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
      ctx.shells.insert(shell_id.to_owned(), command.or_else(|| b.target.clone()).unwrap_or_else(|| shell_id.to_owned()));
    }
  }
  if meta.and_then(|m| m.get("jetbrains")).and_then(|j| j.get("air")).and_then(|a| a.get("asyncTasks")).and_then(|t| t.get("backgrounded"))
    == Some(&Value::Bool(true))
  {
    b.background = Some(true);
  }
  if let Some(locs) = u.get("locations").and_then(Value::as_array) {
    b.locations = Some(
      locs
        .iter()
        .map(|l| Location {
          path: str_of(l, "path").unwrap_or("").to_owned(),
          line: l.get("line").and_then(Value::as_f64).map(|n| n.max(0.0) as u64),
        })
        .collect(),
    );
  }
  if b.kind == ToolKind::Read
    && raw_present
    && let Some(range) = raw.and_then(|r| read_range_from_raw(r, b.locations.as_deref()))
  {
    b.read_range = Some(range);
  }
  let file_kind = matches!(b.kind, ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move);
  if b.locations.as_ref().is_none_or(Vec::is_empty)
    && file_kind
    && let Some(path) = path_from_raw(raw)
  {
    b.locations = Some(vec![Location { path, line: None }]);
  }
  // OpenCode's write sends the contents only in rawInput.content: park them for a new-file completion
  if let Some(ctx) = ctx.as_deref_mut()
    && b.kind == ToolKind::Edit
    && let Some(content) = raw.and_then(|r| r.get("content")).and_then(Value::as_str)
    && let Some(path) = path_from_raw(raw).or_else(|| b.locations.as_ref().and_then(|l| l.first()).map(|l| l.path.clone()))
  {
    ctx.pending_writes.insert(b.id.clone(), (path, content.to_owned()));
  }
  let named = matches!(b.verb_key.as_deref(), Some("verb.todo" | "verb.ask" | "verb.wait" | "verb.kill"));
  let target = if matches!(b.verb_key.as_deref(), Some("verb.wait" | "verb.kill")) {
    shell_target(raw, ctx.as_deref())
  } else {
    pick_target(u, b.kind)
  };
  if let Some(tg) = target
    && !(named && tg.from_title)
    && (!tg.from_title || b.target.is_none())
  {
    b.target = Some(tg.text);
    b.target_mono = Some(tg.mono);
  }
  let status_completed = str_of(u, "status") == Some("completed");
  let pending_write = match ctx.as_deref_mut() {
    Some(ctx) if status_completed && !matches!(b.content, Some(ToolContent::Diff { .. })) => ctx.pending_writes.remove(&b.id),
    _ => None,
  };
  if let Some((path, content)) = pending_write
    && u.get("rawOutput").and_then(|o| o.get("metadata")).and_then(|m| m.get("exists")) == Some(&Value::Bool(false))
  {
    let lines = diff_lines("", &content);
    let add = lines.iter().filter(|l| l.kind == DiffKind::Add).count() as u64;
    let diff = ToolContent::Diff { lines, source: Some(DiffSource { path, old_text: String::new(), new_text: content }) };
    let items =
      u.get("content").and_then(Value::as_array).filter(|a| !a.is_empty()).map(|a| tool_contents(a, ctx.as_deref())).unwrap_or_default();
    let receipts: Vec<ToolContent> = items.into_iter().filter(|x| matches!(x, ToolContent::Text { text } if !text.is_empty())).collect();
    b.contents = if receipts.is_empty() {
      None
    } else {
      let mut all = vec![diff.clone()];
      all.extend(receipts);
      Some(all)
    };
    b.content = Some(diff);
    b.diff_stat = Some(DiffStat { add, del: 0 });
  }
  if let Some(items) = u.get("content").and_then(Value::as_array).filter(|a| !a.is_empty()) {
    let list = tool_contents(items, ctx.as_deref());
    let primary = list.iter().find(|x| matches!(x, ToolContent::Diff { .. })).or(list.first()).cloned();
    // Kimi sends the edit diff before execution, then a plain success receipt: keep the diff
    let keep_diff = b.kind == ToolKind::Edit
      && b.status == ToolStatus::Completed
      && matches!(b.content, Some(ToolContent::Diff { .. }))
      && list.len() == 1
      && matches!(primary, Some(ToolContent::Text { .. }));
    if let Some(c) = primary
      && !keep_diff
    {
      b.content = Some(c);
      let diffs: Vec<&Vec<DiffLine>> = list
        .iter()
        .filter_map(|x| match x {
          ToolContent::Diff { lines, .. } => Some(lines),
          _ => None,
        })
        .collect();
      b.diff_stat = if diffs.is_empty() {
        None
      } else {
        Some(DiffStat {
          add: diffs.iter().map(|d| d.iter().filter(|l| l.kind == DiffKind::Add).count() as u64).sum(),
          del: diffs.iter().map(|d| d.iter().filter(|l| l.kind == DiffKind::Del).count() as u64).sum(),
        })
      };
      b.contents = if list.len() > 1 { Some(list) } else { None };
    }
  }
  // pi-acp and the claude / codex adapters stream terminal output through _meta
  let term_out = meta.and_then(|m| m.get("terminal_output").filter(|v| !v.is_null()).or_else(|| m.get("terminal_output_delta")));
  if let Some(data) = term_out.and_then(|o| o.get("data")).and_then(Value::as_str).filter(|d| !d.is_empty()) {
    let prev = match &b.content {
      Some(ToolContent::Text { text }) => text.clone(),
      _ => String::new(),
    };
    let note = term_out.and_then(|o| o.get("terminal_id")).and_then(Value::as_str).map(|id| tp("host.terminalNotWired", &[("id", id)]));
    let base = if note.as_deref() == Some(prev.as_str()) { String::new() } else { prev };
    b.content = Some(ToolContent::Text { text: slice16(&(base + data), TOOL_OUTPUT_MAX) });
  }
  if let Some(code) =
    meta.and_then(|m| m.get("terminal_exit")).and_then(|e| e.get("exit_code")).and_then(Value::as_f64).filter(|c| *c != 0.0)
  {
    let prev = match &b.content {
      Some(ToolContent::Text { text }) => text.clone(),
      _ => String::new(),
    };
    let code_text = js_num(code);
    if !prev.ends_with(&format!("exit code {code_text}")) {
      b.content = Some(ToolContent::Text { text: slice16(&append_exit_code(&prev, &code_text), TOOL_OUTPUT_MAX) });
    }
  }
  // A terminal item the agent cannot stream through _meta expects a client-side terminal, which is not provided
  let agent_wired =
    meta.is_some_and(|m| ["terminal_info", "terminal_output", "terminal_output_delta", "terminal_exit"].iter().any(|k| m.contains_key(*k)));
  if b.content.is_none()
    && !agent_wired
    && let Some(term) = u.get("content").and_then(Value::as_array).and_then(|a| a.iter().find(|c| str_of(c, "type") == Some("terminal")))
  {
    b.content = Some(ToolContent::Text { text: tp("host.terminalNotWired", &[("id", str_of(term, "terminalId").unwrap_or("undefined"))]) });
  }
  if b.content.is_none()
    && let Some(out) = u.get("rawOutput").filter(|v| !v.is_null())
  {
    let text = match formatted_output(out) {
      Some((text, Some(code))) => append_exit_code(&text, &code),
      Some((text, None)) => text,
      None => match out {
        Value::String(s) => s.clone(),
        other => pretty(other),
      },
    };
    if !text.trim().is_empty() {
      b.content = Some(ToolContent::Text { text: slice16(&text, TOOL_OUTPUT_MAX) });
    }
  }
  if is_todo_tool(b) && b.status == ToolStatus::Completed {
    // Parse the full wire result before the preview's size limit
    let entries = todo_entries(u.get("rawOutput").unwrap_or(&Value::Null)).or_else(|| match &b.content {
      Some(ToolContent::Text { text }) => todo_entries(&Value::String(text.clone())),
      _ => None,
    });
    if entries.is_some() {
      b.todo_entries = entries;
    }
  }
}

fn js_num(n: f64) -> String {
  if n.fract() == 0.0 && n.abs() < 1e21 { format!("{}", n as i64) } else { format!("{n}") }
}

/// OpenCode's permission request embeds a low-fidelity copy of the call: with a block already there, take only what
/// the request can improve. Without a block the request's toolCall applies whole
pub fn permission_tool_update(existing: Option<&ToolCallBlock>, tc: &Value) -> Value {
  let mut u = Map::new();
  u.insert("sessionUpdate".into(), Value::from("tool_call_update"));
  let Some(existing) = existing else {
    if let Some(o) = tc.as_object() {
      for (k, v) in o {
        u.insert(k.clone(), v.clone());
      }
    }
    return Value::Object(u);
  };
  u.insert("toolCallId".into(), tc.get("toolCallId").cloned().unwrap_or(Value::Null));
  if let Some(st) = text_of(tc, "status") {
    u.insert("status".into(), Value::from(st));
  }
  if let Some(k) = text_of(tc, "kind")
    && k != "other"
    && matches!(existing.kind, ToolKind::Other | ToolKind::Think)
  {
    u.insert("kind".into(), Value::from(k));
  }
  let file_kind = matches!(existing.kind, ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move);
  if existing.target.is_none() || (file_kind && existing.locations.as_ref().is_none_or(Vec::is_empty)) {
    if let Some(title) = tc.get("title") {
      u.insert("title".into(), title.clone());
    }
    if let Some(raw) = tc.get("rawInput") {
      u.insert("rawInput".into(), raw.clone());
    }
  }
  let raw_for_path = u.get("rawInput").or(tc.get("rawInput")).and_then(Value::as_object);
  if let Some(locs) = tc.get("locations").and_then(Value::as_array)
    && !locs.is_empty()
    && existing.locations.as_ref().is_none_or(Vec::is_empty)
    && path_from_raw(raw_for_path).is_none()
  {
    u.insert("locations".into(), Value::Array(locs.clone()));
  }
  Value::Object(u)
}

pub fn path_from_raw(raw: Option<&Map<String, Value>>) -> Option<String> {
  let raw = raw?;
  ["path", "file_path", "filePath", "filepath"]
    .iter()
    .find_map(|k| raw.get(*k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_owned))
}

fn positive(v: Option<&Value>) -> Option<u64> {
  let n = v?.as_f64()?;
  (n.fract() == 0.0 && n > 0.0 && n <= 9_007_199_254_740_991.0).then_some(n as u64)
}

// JS `a ?? b`: the first operand that is neither absent nor null
fn first<'a>(raw: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
  keys.iter().find_map(|k| raw.get(*k).filter(|v| !v.is_null()))
}

fn read_range_from_raw(raw: &Map<String, Value>, locations: Option<&[Location]>) -> Option<ReadRange> {
  let path = path_from_raw(Some(raw)).or_else(|| locations.filter(|l| l.len() == 1).map(|l| l[0].path.clone()))?;
  let start = positive(first(raw, &["line_offset", "start_line", "startLine", "offset"]))?;
  let count = positive(first(raw, &["n_lines", "limit", "line_count"]));
  let end = positive(first(raw, &["end_line", "endLine"])).or_else(|| count.map(|c| start + c - 1));
  Some(ReadRange { path, start, end: end.filter(|e| *e >= start) })
}

pub fn command_from_raw(raw: Option<&Map<String, Value>>) -> Option<String> {
  let raw = raw?;
  raw.get("command").and_then(Value::as_str).or_else(|| raw.get("cmd").and_then(Value::as_str)).map(str::to_owned)
}

fn pick_target(u: &Value, kind: ToolKind) -> Option<Target> {
  let raw = u.get("rawInput").and_then(Value::as_object);
  let mono = |text: String| Some(Target { text, mono: true, from_title: false });
  if kind == ToolKind::Execute
    && let Some(cmd) = command_from_raw(raw).filter(|c| !c.is_empty())
  {
    return mono(cmd);
  }
  if kind == ToolKind::Search {
    let q = raw.and_then(|r| r.get("pattern").and_then(Value::as_str).or_else(|| r.get("query").and_then(Value::as_str)));
    if let Some(q) = q.filter(|q| !q.is_empty()) {
      return mono(q.to_owned());
    }
  }
  if kind == ToolKind::Fetch
    && let Some(url) = raw.and_then(|r| r.get("url")).and_then(Value::as_str).filter(|x| !x.is_empty())
  {
    return mono(url.to_owned());
  }
  if let Some(loc) = u.get("locations").and_then(Value::as_array).and_then(|l| l.first()).and_then(|l| text_of(l, "path")) {
    return Some(Target { text: basename(loc), mono: false, from_title: false });
  }
  if matches!(kind, ToolKind::Read | ToolKind::Edit | ToolKind::Delete | ToolKind::Move)
    && let Some(path) = path_from_raw(raw)
  {
    return Some(Target { text: basename(&path), mono: false, from_title: false });
  }
  text_of(u, "title").map(|title| Target { text: strip_verb(title), mono: false, from_title: true })
}

fn shell_target(raw: Option<&Map<String, Value>>, ctx: Option<&ToolCtx>) -> Option<Target> {
  let raw = raw?;
  let shell_id = ["shell_id", "shellId", "id"].iter().find_map(|k| raw.get(*k).and_then(Value::as_str).filter(|v| !v.is_empty()))?;
  let text = ctx.and_then(|c| c.shells.get(shell_id)).cloned().unwrap_or_else(|| shell_id.to_owned());
  Some(Target { text, mono: true, from_title: false })
}

/// An agent's title is often like "Read file foo.ts"; the verb is ours, so strip the English one
fn strip_verb(title: &str) -> String {
  let s = STRIP_VERB.replace(title, "");
  let s = s.strip_prefix('`').unwrap_or(&s);
  let s = s.strip_suffix('`').unwrap_or(s);
  let s = s.trim();
  if s.is_empty() { title.to_owned() } else { s.to_owned() }
}

fn append_exit_code(prev: &str, code: &str) -> String {
  let base = prev.trim_end_matches('\n');
  if base.is_empty() { format!("exit code {code}") } else { format!("{base}\nexit code {code}") }
}

/// codex-acp's { formatted_output, exit_code } receipt
fn formatted_output(raw: &Value) -> Option<(String, Option<String>)> {
  let o = raw.as_object()?;
  let text = o.get("formatted_output")?.as_str()?.to_owned();
  let code = o.get("exit_code").and_then(Value::as_f64).filter(|c| *c != 0.0).map(js_num);
  Some((text, code))
}

const SHOWABLE_IMAGE: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// ACP image content → a transcript image whose pixels go to the blob store; degrades to a note otherwise
fn image_content(c: &Value, ctx: Option<&ToolCtx>) -> Option<ToolContent> {
  if str_of(c, "type") != Some("image") {
    return None;
  }
  let mut mime = str_of(c, "mimeType").unwrap_or("image/png").to_owned();
  let mut uri = text_of(c, "uri").map(str::to_owned);
  let mut data = text_of(c, "data").map(str::to_owned);
  let inline = data
    .as_deref()
    .or(uri.as_deref())
    .and_then(|x| DATA_URL.captures(x))
    .map(|m| (m.get(1).map(|x| x.as_str().to_owned()), m[2].to_owned()));
  if let Some((m, payload)) = inline {
    data = Some(payload);
    if let Some(m) = m {
      mime = m;
    }
    if uri.as_deref().is_some_and(|u| u.starts_with("data:")) {
      uri = None;
    }
  }
  let saver = ctx.and_then(|c| c.save_image.as_ref());
  let blob = match (&data, saver) {
    (Some(d), Some(save)) if SHOWABLE_IMAGE.contains(&mime.as_str()) && base64_bytes(d) as u64 <= MAX_OUT_IMAGE_BYTES => save(d, &mime),
    _ => None,
  };
  if blob.is_some() || (uri.is_some() && data.is_none()) {
    return Some(ToolContent::Image(ImageRef { blob, mime_type: mime, uri }));
  }
  Some(ToolContent::Text { text: if saver.is_some() { format!("[image: {mime}, not shown]") } else { "[image]".into() } })
}

/// A local path behind a resource_link's uri
fn local_path_of(uri: &str) -> Option<String> {
  if uri.len() >= 5 && uri[..5].eq_ignore_ascii_case("file:") {
    return file_url_to_path(uri);
  }
  std::path::Path::new(uri).is_absolute().then(|| uri.to_owned())
}

/// fileURLToPath for the shapes agents send (file:///abs, percent-encoded)
pub fn file_url_to_path(uri: &str) -> Option<String> {
  let rest = uri.get(5..)?;
  let rest = rest.strip_prefix("//")?;
  let (host, path) = {
    let i = rest.find('/')?;
    (&rest[..i], &rest[i..])
  };
  if !host.is_empty() && host != "localhost" {
    return None;
  }
  let decoded = percent_decode(path)?;
  if cfg!(windows) {
    let p = decoded.trim_start_matches('/');
    return Some(p.replace('/', "\\"));
  }
  Some(decoded)
}

fn percent_decode(s: &str) -> Option<String> {
  let bytes = s.as_bytes();
  let mut out = Vec::with_capacity(bytes.len());
  let mut i = 0;
  while i < bytes.len() {
    if bytes[i] == b'%' {
      let h = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
      let v = u8::from_str_radix(h, 16).ok()?;
      // An encoded path separator is refused, as fileURLToPath does
      if v == b'/' {
        return None;
      }
      out.push(v);
      i += 3;
    } else {
      out.push(bytes[i]);
      i += 1;
    }
  }
  String::from_utf8(out).ok()
}

fn file_image_content(c: &Value, ctx: Option<&ToolCtx>) -> Option<ToolContent> {
  if str_of(c, "type") != Some("resource_link") {
    return None;
  }
  let uri = str_of(c, "uri")?;
  let path = local_path_of(uri)?;
  let mime = image_mime_of(&path)?;
  let blob = ctx.and_then(|x| x.save_image_file.as_ref()).and_then(|save| save(&path))?;
  Some(ToolContent::Image(ImageRef { blob: Some(blob), mime_type: mime.to_owned(), uri: Some(uri.to_owned()) }))
}

/// Every renderable content item, in wire order
fn tool_contents(items: &[Value], ctx: Option<&ToolCtx>) -> Vec<ToolContent> {
  let mut out: Vec<ToolContent> = vec![];
  fn push_text(out: &mut Vec<ToolContent>, text: &str) {
    if text.is_empty() {
      return;
    }
    if let Some(ToolContent::Text { text: last }) = out.last_mut() {
      *last = slice16(&format!("{last}\n{text}"), TOOL_OUTPUT_MAX);
    } else {
      out.push(ToolContent::Text { text: slice16(text, TOOL_OUTPUT_MAX) });
    }
  }
  for item in items {
    match str_of(item, "type") {
      Some("diff") => {
        let old = str_of(item, "oldText").unwrap_or("").to_owned();
        let new = str_of(item, "newText").unwrap_or("").to_owned();
        out.push(ToolContent::Diff {
          lines: diff_lines(&old, &new),
          source: Some(DiffSource { path: str_of(item, "path").unwrap_or("").to_owned(), old_text: old, new_text: new }),
        });
      }
      Some("content") => {
        let content = item.get("content").unwrap_or(&Value::Null);
        match image_content(content, ctx).or_else(|| file_image_content(content, ctx)) {
          Some(ToolContent::Text { text }) => push_text(&mut out, &text),
          Some(img) => out.push(img),
          None => push_text(&mut out, &text_of_content(content)),
        }
      }
      _ => {}
    }
  }
  out
}

/// What's happening right now: the Activity line
pub fn activity_of(turns: &[Turn]) -> Option<Activity> {
  let working = || Some(Activity { kind: ToolKind::Think, label: t("host.working") });
  let Some(Turn::Agent(turn)) = turns.last() else { return working() };
  for b in turn.blocks.iter().rev() {
    match b {
      AgentBlock::ToolCall(tc) if tc.background != Some(true) && tc.status.is_open() => {
        return Some(Activity {
          kind: tc.kind,
          label: tp("host.doing", &[("verb", &tc.verb), ("target", tc.target.as_deref().unwrap_or(""))]).trim().to_owned(),
        });
      }
      AgentBlock::Permission(_) => return Some(Activity { kind: ToolKind::Other, label: t("host.awaitingApproval") }),
      AgentBlock::Question(q) if q.outcome.is_none() => return Some(Activity { kind: ToolKind::Other, label: t("host.awaitingAnswers") }),
      _ => {}
    }
  }
  match turn.blocks.last() {
    Some(AgentBlock::Thought(th)) if th.streaming == Some(true) => working(),
    Some(AgentBlock::Text(tx)) if tx.streaming == Some(true) => Some(Activity { kind: ToolKind::Other, label: t("host.replying") }),
    _ => working(),
  }
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
