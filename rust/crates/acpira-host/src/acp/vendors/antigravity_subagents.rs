//! Antigravity (`agy` 1.5.16) native subagent and `/swarm` metadata read from disk.
//!
//! `agy acp` forwards child tool calls on the root stream under `<child_uuid>:<inner_call_id>` without ACP subagent
//! lifecycle events, and writes every child's identity, swarm roster, plan summary and board posts under:
//!
//! `<gemini_dir>/antigravity-cli/brain/<session_id>/.system_generated/`
//! - `subagents/<child_uuid>.json`: `{ conversationId, subagentDescriptor: { typeName, role }, state }`
//! - `swarm/members.json`: `{ title, members: { <name>: { conversation_id, role, model, phase, wake_mode, killed } } }`
//! - `swarm/plan.json`: `{ goal: { title, line }, tasks: { <name>: "<summary>" } }`
//! - `swarm/board.jsonl`: `{ seq, agent, text, time, kind? }`
//!
//! Unchanged files (same length and mtime) are not re-read, and `board.jsonl` is tailed incrementally by byte offset.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::acp::vendors::antigravity::is_orchestrator;
use crate::store::data_dir::home_dir;

/// Byte cap for small JSON metadata files (`members.json`, `plan.json`, `subagents/*.json`)
const META_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// Byte cap per incremental read of `board.jsonl`
const BOARD_READ_CHUNK: u64 = 2 * 1024 * 1024;
/// Total cap on `board.jsonl` reads so a pathological board cannot grow memory unboundedly
const BOARD_MAX_BYTES: u64 = 32 * 1024 * 1024;
/// Tail window read from a finished non-swarm child's `transcript.jsonl` to recover its final reply
const CHILD_LOG_TAIL_BYTES: u64 = 256 * 1024;
/// Tail window read from the session's own `transcript.jsonl` for the latest step's model: one step's line holds its
/// whole reply and thinking
const ROOT_LOG_TAIL_BYTES: u64 = 256 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
  len: u64,
  mtime: Option<SystemTime>,
}

fn file_stamp(path: &Path) -> Option<FileStamp> {
  let m = std::fs::metadata(path).ok()?;
  m.is_file().then(|| FileStamp { len: m.len(), mtime: m.modified().ok() })
}

fn read_json_capped(path: &Path) -> Option<Value> {
  let meta = std::fs::metadata(path).ok()?;
  if !meta.is_file() || meta.len() > META_MAX_BYTES {
    return None;
  }
  let bytes = std::fs::read(path).ok()?;
  serde_json::from_slice(&bytes).ok()
}

/// Rejects ids that could leave the brain directory
fn safe_id(id: &str) -> bool {
  !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// `<gemini_dir>/antigravity-cli/brain` (or `~/.gemini/antigravity-cli/brain` for the default login)
pub fn brain_dir(gemini_dir: Option<&Path>) -> PathBuf {
  let root = match gemini_dir {
    Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
    _ => home_dir().join(".gemini"),
  };
  root.join("antigravity-cli").join("brain")
}

/// Extracts the roster name from `agy`'s swarm descriptor role (`"Swarm Worker (core)"` → `"core"`)
pub fn swarm_worker_name(role: &str) -> Option<&str> {
  let rest = role.strip_prefix("Swarm Worker (")?.strip_suffix(')')?;
  (!rest.is_empty()).then_some(rest)
}

/// Strips a leading `@<name>` mention from a swarm brief so the task card begins with the assignment itself
pub fn strip_mention_prefix<'a>(text: &'a str, name: &str) -> &'a str {
  let trimmed = text.trim_start();
  let Some(rest) = trimmed.strip_prefix('@').and_then(|s| s.strip_prefix(name)) else {
    return trimmed;
  };
  if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || matches!(c, ':' | ',' | '：' | '，')) {
    rest.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | ',' | '：' | '，'))
  } else {
    trimmed
  }
}

/// First non-empty plain text line of a post, stripped of simple Markdown markers for a one-line row preview
pub fn first_plain_line(text: &str) -> Option<String> {
  let line = text
    .lines()
    .map(str::trim)
    .find(|l| !l.is_empty() && !l.starts_with("```"))?
    .trim_start_matches(['#', '>', '-', '*', ' '])
    .replace('`', "");
  let line = line.trim();
  if line.is_empty() {
    return None;
  }
  let clipped: String = line.chars().take(140).collect();
  Some(if clipped.len() < line.len() { format!("{clipped}…") } else { clipped })
}

/// One new board message posted by a swarm worker since the last poll
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardPost {
  pub seq: u64,
  pub text: String,
}

/// Current state of one Antigravity subagent / swarm worker, as combined from disk metadata
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AntigravityChildMeta {
  pub conversation_id: String,
  /// Swarm roster handle (`"core"`, `"composer"`, …); absent for plain `invoke_subagent` children
  pub swarm_name: Option<String>,
  /// Display title: `plan.json` task summary for a swarm worker (falling back to roster name), or `subagentDescriptor.role`
  pub title: Option<String>,
  /// Faint role label beside the title: roster handle for swarm workers, or `typeName` (skipping `"self"`)
  pub role: Option<String>,
  /// Full assignment brief (from `board.jsonl` `kind == "brief"`)
  pub task: Option<String>,
  pub model: Option<String>,
  /// Swarm worker is parked on the board (`wake_mode: "mentions" | "anything"`) while still alive
  pub waiting: bool,
  /// Worker was killed, or a non-swarm child finished its turn and went idle on disk
  pub ended: bool,
  /// Transcript `step_index` at which an alive non-swarm child went idle (`None` for killed children / swarm workers),
  /// used to avoid settling a re-woken child against a stale transcript tail before its new step flushes
  pub idle_step: Option<u64>,
  /// New messages posted to `board.jsonl` by this worker since the previous snapshot
  pub new_posts: Vec<BoardPost>,
  /// Latest message posted to `board.jsonl` by this worker (or final reply from a finished non-swarm transcript)
  pub latest_report: Option<String>,
}

/// Snapshot produced by one poll of `<brain>/<session_id>/.system_generated/`
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AntigravitySnapshot {
  pub children: Vec<AntigravityChildMeta>,
  /// The model of the session's latest execution step while an orchestrator runs (`read_worker`)
  pub worker_model: Option<String>,
}

#[derive(Clone)]
struct SubagentFile {
  stamp: FileStamp,
  conversation_id: String,
  type_name: Option<String>,
  role: Option<String>,
  killed: bool,
}

#[derive(Clone)]
struct MemberEntry {
  name: String,
  conversation_id: String,
  model: Option<String>,
  waiting: bool,
  killed: bool,
}

#[derive(Clone, Default)]
struct ChildTranscript {
  stamp: Option<FileStamp>,
  /// `step_index` of the closing `PLANNER_RESPONSE` when the child finished without tool calls
  idle_step: Option<u64>,
  /// True while `<brain>/<child_id>/.system_generated/messages/undelivered/` holds a pending wake-up message
  undelivered: bool,
  /// Cached final reply (`send_message` report or closing response / content text)
  final_reply: Option<String>,
}

/// Incremental disk reader for one Antigravity session's `.system_generated/` directory
#[derive(Default)]
pub struct AntigravityMetaCursor {
  members_stamp: Option<FileStamp>,
  members: BTreeMap<String, MemberEntry>,
  plan_stamp: Option<FileStamp>,
  plan_tasks: HashMap<String, String>,
  subagent_files: HashMap<String, SubagentFile>,
  board_offset: u64,
  board_buf: String,
  board_last_seq: u64,
  /// Initial brief per swarm worker name (`kind == "brief"`)
  briefs: HashMap<String, String>,
  /// Latest post text per swarm worker name
  latest_posts: HashMap<String, String>,
  /// Per non-swarm child transcript state, refreshed only when `transcript.jsonl` or `messages/undelivered/` changes
  child_logs: HashMap<String, ChildTranscript>,
  /// The session's own `transcript.jsonl` as last read for the orchestrator's execution model
  root_log_stamp: Option<FileStamp>,
  worker_model: Option<String>,
}

impl AntigravityMetaCursor {
  pub fn new() -> Self {
    Self::default()
  }

  /// Poll `<brain_root>/<session_id>/.system_generated/` for new or changed child metadata, and while `orchestrating`
  /// for the model the session's execution steps run on. Returns `None` when neither exists yet or nothing on disk
  /// changed since the previous read
  pub fn poll(&mut self, brain_root: &Path, session_id: &str, orchestrating: bool) -> Option<AntigravitySnapshot> {
    if !safe_id(session_id) {
      return None;
    }
    let sys = brain_root.join(session_id).join(".system_generated");
    if !sys.is_dir() {
      return None;
    }
    let mut changed = false;
    changed |= self.read_members(&sys.join("swarm").join("members.json"));
    changed |= self.read_plan(&sys.join("swarm").join("plan.json"));
    changed |= self.read_subagents(&sys.join("subagents"));
    if orchestrating {
      changed |= self.read_worker(&sys.join("logs").join("transcript.jsonl"));
    }
    let mut posts_by_agent = self.read_board(&sys.join("swarm").join("board.jsonl"));
    if !posts_by_agent.is_empty() {
      changed = true;
    }
    // Non-swarm children (`invoke_subagent`, `image-generator`, `research`, `self`) stay `SUBAGENT_STATE_ALIVE` after
    // finishing a turn so the parent can still follow up via `send_message`. Track their transcript tail and pending
    // inbox so completed turns settle promptly and surface their final report.
    let non_swarm: Vec<(String, bool)> = self
      .subagent_files
      .values()
      .filter(|s| {
        s.role.as_deref().and_then(swarm_worker_name).is_none()
          && !self.members.values().any(|m| m.conversation_id == s.conversation_id)
      })
      .map(|s| (s.conversation_id.clone(), s.killed))
      .collect();
    for (cid, killed) in non_swarm {
      changed |= self.read_child_state(brain_root, &cid, killed);
    }
    if !changed {
      return None;
    }
    let mut by_cid: BTreeMap<String, AntigravityChildMeta> = BTreeMap::new();
    // 1. Seed from `subagents/*.json` (covers both swarm workers and `invoke_subagent`)
    for s in self.subagent_files.values() {
      let swarm_name = s.role.as_deref().and_then(swarm_worker_name).map(str::to_owned);
      let (title, role) = if let Some(name) = &swarm_name {
        let task_title = self.plan_tasks.get(name).cloned();
        (task_title.or_else(|| Some(name.clone())), Some(name.clone()))
      } else {
        let title = s.role.clone().or_else(|| s.type_name.clone());
        let role = s.type_name.clone().filter(|t| t != "self" && Some(t.as_str()) != title.as_deref());
        (title, role)
      };
      let child_log = self.child_logs.get(&s.conversation_id);
      let idle_step = (!s.killed && swarm_name.is_none())
        .then(|| child_log.filter(|l| !l.undelivered).and_then(|l| l.idle_step))
        .flatten();
      let ended = s.killed || idle_step.is_some();
      by_cid.insert(
        s.conversation_id.clone(),
        AntigravityChildMeta {
          conversation_id: s.conversation_id.clone(),
          swarm_name,
          title,
          role,
          task: None,
          model: None,
          waiting: false,
          ended,
          idle_step,
          new_posts: vec![],
          latest_report: child_log.filter(|_| ended).and_then(|l| l.final_reply.clone()),
        },
      );
    }
    // 2. Merge richer `swarm/members.json` + `plan.json` + `board.jsonl` data
    for m in self.members.values() {
      let task_title = self.plan_tasks.get(&m.name).cloned();
      let brief = self.briefs.get(&m.name).cloned();
      let latest = self.latest_posts.get(&m.name).cloned();
      let new_posts = posts_by_agent.remove(&m.name).unwrap_or_default();
      let entry = by_cid.entry(m.conversation_id.clone()).or_insert_with(|| AntigravityChildMeta {
        conversation_id: m.conversation_id.clone(),
        swarm_name: Some(m.name.clone()),
        title: None,
        role: Some(m.name.clone()),
        task: None,
        model: None,
        waiting: false,
        ended: false,
        idle_step: None,
        new_posts: vec![],
        latest_report: None,
      });
      entry.swarm_name = Some(m.name.clone());
      entry.title = task_title.or_else(|| Some(m.name.clone()));
      entry.role = Some(m.name.clone());
      if brief.is_some() {
        entry.task = brief;
      }
      if m.model.is_some() {
        entry.model = m.model.clone();
      }
      entry.waiting = m.waiting && !m.killed && !entry.ended;
      entry.ended |= m.killed;
      entry.idle_step = None;
      entry.new_posts = new_posts;
      if latest.is_some() {
        entry.latest_report = latest;
      }
    }
    if by_cid.is_empty() && self.worker_model.is_none() {
      return None;
    }
    Some(AntigravitySnapshot { children: by_cid.into_values().collect(), worker_model: self.worker_model.clone() })
  }

  /// The model of the latest step in the session's own transcript, when an execution model ran it: each step names its
  /// model (`gemini-pro-orchestrator-neon` planning, `gemini-flash-worker-agy` executing, agy 1.5.16). A planning step
  /// last means the executor has not stepped since the hand-off, and the name read before stands
  fn read_worker(&mut self, path: &Path) -> bool {
    let Some(stamp) = file_stamp(path) else { return false };
    if self.root_log_stamp == Some(stamp) {
      return false;
    }
    self.root_log_stamp = Some(stamp);
    let Some(text) = read_tail(path, ROOT_LOG_TAIL_BYTES) else { return false };
    let latest = text.lines().rev().find_map(|line| {
      let v = serde_json::from_str::<Value>(line.trim()).ok()?;
      if v.get("type").and_then(Value::as_str) != Some("PLANNER_RESPONSE") {
        return None;
      }
      v.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()).map(str::to_owned)
    });
    match latest.filter(|m| !is_orchestrator(m)) {
      Some(model) if self.worker_model.as_deref() != Some(&model) => {
        self.worker_model = Some(model);
        true
      }
      _ => false,
    }
  }

  fn read_members(&mut self, path: &Path) -> bool {
    let Some(stamp) = file_stamp(path) else { return false };
    if self.members_stamp == Some(stamp) {
      return false;
    }
    let Some(v) = read_json_capped(path) else { return false };
    self.members_stamp = Some(stamp);
    let Some(obj) = v.get("members").and_then(Value::as_object) else { return false };
    let mut next = BTreeMap::new();
    for (name, info) in obj {
      // The coordinator is the root session itself, never a child node
      if name == "coordinator" || info.get("role").and_then(Value::as_str) == Some("coordinator") {
        continue;
      }
      let Some(cid) = info.get("conversation_id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned) else {
        continue;
      };
      let phase = info.get("phase").and_then(Value::as_str).unwrap_or("RUNNING");
      let desired = info.get("desired_state").and_then(Value::as_str).unwrap_or("");
      let killed = info.get("killed") == Some(&Value::Bool(true)) || desired == "KILLED" || phase == "KILLED";
      let wake_mode = info.get("wake_mode").and_then(Value::as_str).unwrap_or("");
      let waiting = matches!(wake_mode, "mentions" | "anything");
      let model = info
        .get("applied_model")
        .or_else(|| info.get("model"))
        .or_else(|| info.get("desired_model"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
      next.insert(name.clone(), MemberEntry { name: name.clone(), conversation_id: cid, model, waiting, killed });
    }
    self.members = next;
    true
  }

  fn read_plan(&mut self, path: &Path) -> bool {
    let Some(stamp) = file_stamp(path) else { return false };
    if self.plan_stamp == Some(stamp) {
      return false;
    }
    let Some(v) = read_json_capped(path) else { return false };
    self.plan_stamp = Some(stamp);
    let Some(tasks) = v.get("tasks").and_then(Value::as_object) else { return false };
    self.plan_tasks = tasks
      .iter()
      .filter_map(|(k, val)| val.as_str().map(str::trim).filter(|s| !s.is_empty()).map(|s| (k.clone(), s.to_owned())))
      .collect();
    true
  }

  fn read_subagents(&mut self, dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    let mut changed = false;
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().and_then(|e| e.to_str()) != Some("json") {
        continue;
      }
      let Some(key) = path.file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
      let Some(stamp) = file_stamp(&path) else { continue };
      if self.subagent_files.get(&key).is_some_and(|s| s.stamp == stamp) {
        continue;
      }
      let Some(v) = read_json_capped(&path) else { continue };
      let Some(cid) = v.get("conversationId").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned) else {
        continue;
      };
      let desc = v.get("subagentDescriptor").and_then(Value::as_object);
      let type_name = desc.and_then(|d| d.get("typeName")).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
      let role = desc.and_then(|d| d.get("role")).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
      let killed = v.get("state").and_then(Value::as_str) == Some("SUBAGENT_STATE_KILLED");
      self.subagent_files.insert(key, SubagentFile { stamp, conversation_id: cid, type_name, role, killed });
      changed = true;
    }
    changed
  }

  /// Refreshes a non-swarm child's transcript completion state and closing report when its log or inbox moved
  fn read_child_state(&mut self, brain_root: &Path, child_id: &str, killed: bool) -> bool {
    if !safe_id(child_id) {
      return false;
    }
    let sys = brain_root.join(child_id).join(".system_generated");
    let log_path = sys.join("logs").join("transcript.jsonl");
    let stamp = file_stamp(&log_path);
    let undelivered = !killed && has_undelivered_messages(&sys.join("messages").join("undelivered"));
    let entry = self.child_logs.entry(child_id.to_owned()).or_default();
    let mut changed = entry.undelivered != undelivered;
    entry.undelivered = undelivered;
    if stamp.is_none() || entry.stamp == stamp {
      return changed;
    }
    entry.stamp = stamp;
    let Some(lines) = read_tail(&log_path, CHILD_LOG_TAIL_BYTES) else { return changed };
    let next_idle = if killed { None } else { child_idle_step(&lines) };
    let next_reply = if killed || next_idle.is_some() { extract_child_final_reply(&lines) } else { None };
    if entry.idle_step != next_idle || entry.final_reply != next_reply {
      entry.idle_step = next_idle;
      entry.final_reply = next_reply;
      changed = true;
    }
    changed
  }

  fn read_board(&mut self, path: &Path) -> HashMap<String, Vec<BoardPost>> {
    let mut posts: HashMap<String, Vec<BoardPost>> = HashMap::new();
    let Some(stamp) = file_stamp(path) else { return posts };
    if stamp.len <= self.board_offset || self.board_offset >= BOARD_MAX_BYTES {
      return posts;
    }
    let Ok(mut f) = std::fs::File::open(path) else { return posts };
    if f.seek(SeekFrom::Start(self.board_offset)).is_err() {
      return posts;
    }
    let to_read = (stamp.len - self.board_offset).min(BOARD_READ_CHUNK).min(BOARD_MAX_BYTES - self.board_offset);
    let mut buf = vec![0u8; to_read as usize];
    let Ok(n) = f.read(&mut buf) else { return posts };
    if n == 0 {
      return posts;
    }
    self.board_offset += n as u64;
    self.board_buf.push_str(&String::from_utf8_lossy(&buf[..n]));
    // Keep any trailing incomplete line for the next poll
    let complete_end = match self.board_buf.rfind('\n') {
      Some(pos) => pos + 1,
      None => return posts,
    };
    let tail = self.board_buf.split_off(complete_end);
    let chunk = std::mem::replace(&mut self.board_buf, tail);
    for line in chunk.lines() {
      let line = line.trim();
      if line.is_empty() {
        continue;
      }
      let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
      let seq = v.get("seq").and_then(Value::as_u64).unwrap_or(0);
      if seq != 0 && seq <= self.board_last_seq {
        continue;
      }
      if seq > self.board_last_seq {
        self.board_last_seq = seq;
      }
      let Some(agent) = v.get("agent").and_then(Value::as_str).filter(|s| !s.is_empty()) else { continue };
      let Some(text) = v.get("text").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) else { continue };
      let kind = v.get("kind").and_then(Value::as_str).unwrap_or("post");
      if kind == "brief" {
        if let Some(target) = text.trim_start().strip_prefix('@').and_then(|s| s.split_whitespace().next()) {
          let target = target.trim_end_matches([':', ',', '：', '，']);
          if !target.is_empty() {
            let body = strip_mention_prefix(text, target);
            if !body.is_empty() {
              self.briefs.entry(target.to_owned()).or_insert_with(|| body.to_owned());
            }
          }
        }
        continue;
      }
      if agent == "coordinator" || agent == "system" {
        continue;
      }
      self.latest_posts.insert(agent.to_owned(), text.to_owned());
      posts.entry(agent.to_owned()).or_default().push(BoardPost { seq, text: text.to_owned() });
    }
    posts
  }
}

/// The last `max` bytes of a log, from its first whole line
fn read_tail(path: &Path, max: u64) -> Option<String> {
  let meta = std::fs::metadata(path).ok()?;
  if !meta.is_file() || meta.len() == 0 {
    return None;
  }
  let mut f = std::fs::File::open(path).ok()?;
  let start = meta.len().saturating_sub(max);
  f.seek(SeekFrom::Start(start)).ok()?;
  let mut buf = Vec::new();
  f.read_to_end(&mut buf).ok()?;
  let text = String::from_utf8_lossy(&buf);
  Some(if start > 0 { text.split_once('\n').map_or("", |(_, rest)| rest).to_owned() } else { text.into_owned() })
}

/// Whether a child's `messages/undelivered/` folder holds a queued message waiting to wake it
fn has_undelivered_messages(dir: &Path) -> bool {
  std::fs::read_dir(dir).ok().is_some_and(|mut rd| rd.any(|e| e.is_ok()))
}

/// Returns the `step_index` when a non-swarm child's transcript ends on a settled `PLANNER_RESPONSE` with no tool calls,
/// indicating that the child finished its turn and went idle while remaining `SUBAGENT_STATE_ALIVE`
fn child_idle_step(lines: &str) -> Option<u64> {
  let last = lines.lines().rev().find_map(|line| {
    let trimmed = line.trim();
    (!trimmed.is_empty()).then(|| serde_json::from_str::<Value>(trimmed).ok()).flatten()
  })?;
  if last.get("type").and_then(Value::as_str) != Some("PLANNER_RESPONSE") {
    return None;
  }
  if let Some(status) = last.get("status").and_then(Value::as_str)
    && status != "DONE"
  {
    return None;
  }
  let has_tools = last.get("tool_calls").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
  if has_tools {
    return None;
  }
  Some(last.get("step_index").and_then(Value::as_u64).unwrap_or(0))
}

/// Extracts a finished non-swarm child's closing report from its transcript tail: prefers the latest `send_message`
/// payload (which carries the full report to the parent) and falls back to the last `PLANNER_RESPONSE`'s `response` or
/// `content` text
fn extract_child_final_reply(lines: &str) -> Option<String> {
  let mut fallback: Option<String> = None;
  for line in lines.lines().rev() {
    let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
    if v.get("type").and_then(Value::as_str) != Some("PLANNER_RESPONSE") {
      continue;
    }
    if let Some(calls) = v.get("tool_calls").and_then(Value::as_array) {
      for tc in calls.iter().rev() {
        if tc.get("name").and_then(Value::as_str) == Some("send_message")
          && let Some(msg) = tc.pointer("/args/Message").or_else(|| tc.pointer("/args/message")).and_then(Value::as_str)
        {
          // `transcript.jsonl` stores tool call arg values JSON-encoded as strings
          let unquoted = serde_json::from_str::<String>(msg).unwrap_or_else(|_| msg.to_owned());
          if !unquoted.trim().is_empty() {
            return Some(unquoted);
          }
        }
      }
    }
    if fallback.is_none()
      && let Some(reply) = v
        .get("response")
        .or_else(|| v.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
      fallback = Some(reply.to_owned());
    }
  }
  fallback
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn reads_swarm_members_plan_board_and_incremental_updates() {
    let dir = tempfile::tempdir().unwrap();
    let brain = dir.path();
    let sid = "10000000-0000-4000-8000-000000000001";
    let sys = brain.join(sid).join(".system_generated");
    std::fs::create_dir_all(sys.join("swarm")).unwrap();
    std::fs::create_dir_all(sys.join("subagents")).unwrap();

    let cid = "30000000-0000-4000-8000-000000000003";
    std::fs::write(
      sys.join("subagents").join(format!("{cid}.json")),
      format!(r#"{{"conversationId":"{cid}","subagentDescriptor":{{"typeName":"worker","role":"Swarm Worker (ui)"}},"state":"SUBAGENT_STATE_ALIVE"}}"#),
    )
    .unwrap();
    std::fs::write(
      sys.join("swarm/members.json"),
      format!(
        r#"{{"title":"Example project","members":{{"coordinator":{{"conversation_id":"{sid}","role":"coordinator","phase":"RUNNING"}},"ui":{{"conversation_id":"{cid}","role":"worker","model":"gemini-4-neon-xhigh","phase":"RUNNING","wake_mode":"working","killed":false}}}}}}"#
      ),
    )
    .unwrap();
    std::fs::write(sys.join("swarm/plan.json"), r#"{"tasks":{"ui":"Settings form"}}"#).unwrap();
    std::fs::write(
      sys.join("swarm/board.jsonl"),
      "{\"seq\":1,\"agent\":\"coordinator\",\"kind\":\"brief\",\"text\":\"@ui Implement the settings form\"}\n",
    )
    .unwrap();

    let mut cur = AntigravityMetaCursor::new();
    let snap = cur.poll(brain, sid, false).unwrap();
    assert_eq!(snap.children.len(), 1);
    let c = &snap.children[0];
    assert_eq!(c.conversation_id, cid);
    assert_eq!(c.swarm_name.as_deref(), Some("ui"));
    assert_eq!(c.title.as_deref(), Some("Settings form"));
    assert_eq!(c.role.as_deref(), Some("ui"));
    assert_eq!(c.task.as_deref(), Some("Implement the settings form"));
    assert_eq!(c.model.as_deref(), Some("gemini-4-neon-xhigh"));
    assert!(!c.waiting && !c.ended);
    assert!(c.new_posts.is_empty());

    // Unchanged files return None
    assert!(cur.poll(brain, sid, false).is_none());

    // Append a board post and mark killed
    let mut board = std::fs::OpenOptions::new().append(true).open(sys.join("swarm/board.jsonl")).unwrap();
    use std::io::Write;
    writeln!(board, "{{\"seq\":2,\"agent\":\"ui\",\"text\":\"@core The form is ready\"}}").unwrap();
    std::fs::write(
      sys.join("swarm/members.json"),
      format!(
        r#"{{"title":"Example project","members":{{"ui":{{"conversation_id":"{cid}","role":"worker","model":"gemini-4-neon-xhigh","phase":"KILLED","wake_mode":"mentions","killed":true}}}}}}"#
      ),
    )
    .unwrap();

    let snap2 = cur.poll(brain, sid, false).unwrap();
    let c2 = &snap2.children[0];
    assert!(c2.ended && !c2.waiting);
    assert_eq!(c2.new_posts, vec![BoardPost { seq: 2, text: "@core The form is ready".into() }]);
    assert_eq!(c2.latest_report.as_deref(), Some("@core The form is ready"));
  }

  #[test]
  fn reads_finished_non_swarm_subagent_final_reply() {
    let dir = tempfile::tempdir().unwrap();
    let brain = dir.path();
    let sid = "50000000-0000-4000-8000-000000000005";
    let cid = "60000000-0000-4000-8000-000000000006";
    let sys = brain.join(sid).join(".system_generated");
    std::fs::create_dir_all(sys.join("subagents")).unwrap();
    std::fs::write(
      sys.join("subagents").join(format!("{cid}.json")),
      format!(r#"{{"conversationId":"{cid}","subagentDescriptor":{{"typeName":"self","role":"Example reviewer"}},"state":"SUBAGENT_STATE_KILLED"}}"#),
    )
    .unwrap();
    let logs = brain.join(cid).join(".system_generated/logs");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::write(
      logs.join("transcript.jsonl"),
      "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"tool_calls\":[{\"name\":\"send_message\",\"args\":{\"Message\":\"\\\"Reviewed the sample\\\"\"}}]}\n",
    )
    .unwrap();

    let mut cur = AntigravityMetaCursor::new();
    let snap = cur.poll(brain, sid, false).unwrap();
    let c = &snap.children[0];
    assert_eq!(c.title.as_deref(), Some("Example reviewer"));
    assert_eq!(c.role, None);
    assert!(c.ended);
    assert_eq!(c.idle_step, None);
    assert_eq!(c.latest_report.as_deref(), Some("Reviewed the sample"));
  }

  #[test]
  fn detects_alive_non_swarm_subagent_completion_from_transcript_and_respects_undelivered_inbox() {
    let dir = tempfile::tempdir().unwrap();
    let brain = dir.path();
    let sid = "50000000-0000-4000-8000-000000000015";
    let cid = "60000000-0000-4000-8000-000000000016";
    let sys = brain.join(sid).join(".system_generated");
    std::fs::create_dir_all(sys.join("subagents")).unwrap();
    std::fs::write(
      sys.join("subagents").join(format!("{cid}.json")),
      format!(r#"{{"conversationId":"{cid}","subagentDescriptor":{{"typeName":"image-generator","role":"Generate icon"}},"state":"SUBAGENT_STATE_ALIVE"}}"#),
    )
    .unwrap();
    let child_sys = brain.join(cid).join(".system_generated");
    let logs = child_sys.join("logs");
    let undelivered = child_sys.join("messages/undelivered");
    std::fs::create_dir_all(&logs).unwrap();
    std::fs::create_dir_all(&undelivered).unwrap();
    let log = logs.join("transcript.jsonl");

    // 1. Mid-turn (last step calls a tool): still running
    std::fs::write(
      &log,
      "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"generate_image\",\"args\":{}}]}\n",
    )
    .unwrap();
    let mut cur = AntigravityMetaCursor::new();
    let snap = cur.poll(brain, sid, false).unwrap();
    assert!(!snap.children[0].ended);
    assert_eq!(snap.children[0].idle_step, None);

    // 2. Child sends its report and finishes with a tool-less DONE PLANNER_RESPONSE -> settled with send_message text
    let done = concat!(
      "{\"step_index\":1,\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"generate_image\",\"args\":{}}]}\n",
      "{\"step_index\":2,\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"tool_calls\":[{\"name\":\"send_message\",\"args\":{\"Message\":\"\\\"Saved icon.png\\\"\"}}]}\n",
      "{\"step_index\":3,\"type\":\"GENERIC\",\"status\":\"DONE\"}\n",
      "{\"step_index\":4,\"type\":\"PLANNER_RESPONSE\",\"status\":\"DONE\",\"content\":\"Done, sent via send_message.\"}\n"
    );
    std::fs::write(&log, done).unwrap();
    let snap = cur.poll(brain, sid, false).unwrap();
    let c = &snap.children[0];
    assert!(c.ended);
    assert_eq!(c.idle_step, Some(4));
    assert_eq!(c.latest_report.as_deref(), Some("Saved icon.png"));

    // 3. A pending message in `messages/undelivered/` means the child is waking up again -> not ended
    let pending = undelivered.join("msg1.json");
    std::fs::write(&pending, "{}").unwrap();
    let snap = cur.poll(brain, sid, false).unwrap();
    assert!(!snap.children[0].ended);
    assert_eq!(snap.children[0].idle_step, None);

    // Once delivered (inbox cleared) and no new steps written yet, idle resumes
    std::fs::remove_file(&pending).unwrap();
    let snap = cur.poll(brain, sid, false).unwrap();
    assert!(snap.children[0].ended);
    assert_eq!(snap.children[0].idle_step, Some(4));
  }

  #[test]
  fn helpers_strip_mentions_and_parse_worker_roles() {
    assert_eq!(swarm_worker_name("Swarm Worker (core)"), Some("core"));
    assert_eq!(swarm_worker_name("Example reviewer"), None);
    assert_eq!(strip_mention_prefix("@core Update settings.rs", "core"), "Update settings.rs");
    assert_eq!(strip_mention_prefix("@core: Update settings.rs", "core"), "Update settings.rs");
    assert_eq!(strip_mention_prefix("@core2 Own something", "core"), "@core2 Own something");
    assert_eq!(first_plain_line("\n## `@core` Ready!\nMore lines"), Some("@core Ready!".into()));
  }

  #[test]
  fn reads_the_orchestrators_execution_model_from_the_sessions_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let brain = dir.path();
    let sid = "70000000-0000-4000-8000-000000000007";
    let logs = brain.join(sid).join(".system_generated").join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let log = logs.join("transcript.jsonl");
    // Steps as agy 1.5.16 logs them: the planner's model, then the executor's after the hand-off
    let step = |i: u32, ty: &str, model: Option<&str>| {
      let mut v = serde_json::json!({ "step_index": i, "type": ty, "source": "MODEL" });
      if let Some(m) = model {
        v["model"] = m.into();
      }
      format!("{v}\n")
    };
    std::fs::write(&log, [step(0, "USER_INPUT", None), step(1, "PLANNER_RESPONSE", Some("gemini-pro-orchestrator-neon")), step(2, "GENERIC", None)].concat()).unwrap();
    let mut cur = AntigravityMetaCursor::new();
    // Only the planner has stepped: nothing to tell yet, and nothing is read unless an orchestrator round runs
    assert!(cur.poll(brain, sid, true).is_none());
    let more = [step(3, "PLANNER_RESPONSE", Some("gemini-flash-worker-agy")), step(4, "GENERIC", None)].concat();
    std::fs::write(&log, std::fs::read_to_string(&log).unwrap() + &more).unwrap();
    assert!(cur.poll(brain, sid, false).is_none());
    let snap = cur.poll(brain, sid, true).unwrap();
    assert_eq!(snap.worker_model.as_deref(), Some("gemini-flash-worker-agy"));
    assert!(snap.children.is_empty());
    // The planner reviewing keeps the name; an unchanged file is not read again
    assert!(cur.poll(brain, sid, true).is_none());
    std::fs::write(&log, std::fs::read_to_string(&log).unwrap() + &step(5, "PLANNER_RESPONSE", Some("gemini-pro-orchestrator-neon"))).unwrap();
    assert!(cur.poll(brain, sid, true).is_none());
  }
}
