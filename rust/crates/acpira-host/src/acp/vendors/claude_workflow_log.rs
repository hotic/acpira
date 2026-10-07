//! The live process of a Claude Code dynamic-workflow agent, read from the CLI's own sidechain transcript.
//!
//! The `task_progress` frames `claude_workflow.rs` turns into receipt nodes carry only a label, a phase, a prompt and a
//! result preview per agent. Claude Code writes every workflow agent's whole transcript as it goes (observed 2026-10-07,
//! Claude Code 2.1.284 under claude-agent-acp, entrypoint `sdk-ts`):
//!
//! `<config>/projects/<encoded cwd>/<sessionId>/subagents/workflows/<runId>/agent-<agentId>.jsonl`
//!
//! next to `agent-<agentId>.meta.json` (`{ agentType: "workflow-subagent", description, workflowPhase, spawnDepth, … }`)
//! and the run's `journal.jsonl`.
//! - `<config>` is `CLAUDE_CONFIG_DIR` (the agent entry's env, then the process env), else `~/.claude`.
//! - `<encoded cwd>` is the SDK's `sanitizePath` (claude-agent-sdk 0.3.284): every UTF-16 unit outside `[a-zA-Z0-9]`
//!   becomes `-`; past 200 units it is cut to 200 and suffixed `-<base36 |hash|>`. The CLI may hash differently there,
//!   and resolves the cwd itself (symlinks), so a missing directory falls back to scanning `projects/*/<sessionId>/`,
//!   which is how claude-agent-acp's own `findTranscript` looks a session up.
//! - `<sessionId>` is the ACP session id (claude-agent-acp hands out the Claude session id), `<agentId>` the
//!   `workflow_agent.agentId` of the progress frame (a queued agent has none yet), `<runId>` (`wf_…`) is globbed.
//!
//! One JSON object per line, appended while the agent runs, every line `isSidechain: true` with `agentId`, `uuid`,
//! `parentUuid`, `timestamp`, `sessionId`, `version`:
//! - `user` with a string `message.content`: the task prompt (the first lines; the node already shows its task card)
//! - `assistant` with one content block per line (`thinking`, `text`, `tool_use`); several lines share `message.id`
//! - `user` with `tool_result` blocks (`tool_use_id`, `content` a string or blocks, `is_error`)
//! - `attachment` (system reminders) and anything else: skipped
//!
//! This is a private, version-dependent format: unknown types, malformed lines and a partially written last line are
//! skipped silently (the last one is kept and completed on the next read). Lines become the ACP updates claude-agent-acp
//! would have sent for the same blocks (`tool-calls/reporters`, 0.87.0: titles, kinds, locations, diffs, terminal
//! output), so the child transcript goes through the ordinary normalizer and its rows look like any Claude subagent's

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::{Map, Value, json};

use crate::store::data_dir::home_dir;

pub const CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
/// The SDK's `MAX_SANITIZED_LENGTH`
const MAX_DIR_NAME: usize = 200;
/// Bytes one read takes from a log at most: a long backlog catches up over a few polls
const READ_CHUNK: u64 = 2 * 1024 * 1024;
/// A log past this size is not read further (one node's transcript stays bounded)
const MAX_LOG_BYTES: u64 = 64 * 1024 * 1024;
/// A single line longer than this (a huge tool result) is dropped instead of buffered
const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
/// Reads that take a whole capped log, for an agent that ended (`LogCursor::read` takes one chunk at a time)
pub const MAX_READS: u64 = MAX_LOG_BYTES / READ_CHUNK + 1;

/// `CLAUDE_CONFIG_DIR` (as the agent entry or the process sets it), else `~/.claude`, like the adapter's `claudeConfigDir`
pub fn config_dir_of(value: Option<String>) -> PathBuf {
  match value.filter(|v| !v.is_empty()) {
    Some(v) => PathBuf::from(v),
    None => home_dir().join(".claude"),
  }
}

/// The SDK's `sanitizePath`: the project directory name of a cwd under `<config>/projects/`
pub fn project_dir_name(cwd: &str) -> String {
  let units: Vec<u16> = cwd.encode_utf16().collect();
  let sanitized: String =
    units.iter().map(|u| if (*u as u32) < 128 && (*u as u8 as char).is_ascii_alphanumeric() { *u as u8 as char } else { '-' }).collect();
  if units.len() <= MAX_DIR_NAME {
    return sanitized;
  }
  // `Math.abs(h).toString(36)` of the JS string hash `h = (h << 5) - h + unit | 0`
  let hash = units.iter().fold(0i32, |h, u| h.wrapping_shl(5).wrapping_sub(h).wrapping_add(*u as i32));
  format!("{}-{}", &sanitized[..MAX_DIR_NAME], base36((hash as i64).unsigned_abs()))
}

fn base36(mut n: u64) -> String {
  const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
  if n == 0 {
    return "0".into();
  }
  let mut out = vec![];
  while n > 0 {
    out.push(DIGITS[(n % 36) as usize]);
    n /= 36;
  }
  out.reverse();
  String::from_utf8(out).expect("ascii")
}

/// Ids that name files: nothing that could leave the directory
fn safe_id(id: &str) -> bool {
  !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The session's own directory (`projects/<encoded cwd>/<sessionId>`), or the one any project directory holds for it
pub fn session_dir(config: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
  if !safe_id(session_id) {
    return None;
  }
  let projects = config.join("projects");
  let direct = projects.join(project_dir_name(cwd)).join(session_id);
  if direct.is_dir() {
    return Some(direct);
  }
  std::fs::read_dir(&projects).ok()?.filter_map(|e| e.ok()).map(|e| e.path().join(session_id)).find(|p| p.is_dir())
}

/// `subagents/workflows/*/agent-<agentId>.jsonl` under a session directory
pub fn agent_log(session_dir: &Path, agent_id: &str) -> Option<PathBuf> {
  if !safe_id(agent_id) {
    return None;
  }
  let name = format!("agent-{agent_id}.jsonl");
  std::fs::read_dir(session_dir.join("subagents").join("workflows"))
    .ok()?
    .filter_map(|e| e.ok())
    .map(|e| e.path().join(&name))
    .find(|p| p.is_file())
}

/// Turns sidechain lines into ACP session updates; keeps what a later line needs (the tool a result answers, the block
/// before it)
#[derive(Default)]
pub struct LineParser {
  /// The session cwd: titles show paths relative to it, as the adapter's `toDisplayPath` does
  cwd: String,
  /// tool_use id → tool name, for its result
  tools: HashMap<String, String>,
  /// The last streamed block: (thinking | text, message id). The normalizer joins consecutive chunks of one kind, so a
  /// block of another message gets a paragraph break
  last: Option<(&'static str, String)>,
}

impl LineParser {
  pub fn new(cwd: &str) -> Self {
    LineParser { cwd: cwd.to_owned(), ..Default::default() }
  }

  /// One line → its updates; anything unknown or malformed yields none
  pub fn line(&mut self, line: &[u8]) -> Vec<Value> {
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return vec![] };
    let Some(msg) = v.get("message") else { return vec![] };
    let blocks = msg.get("content").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    match v.get("type").and_then(Value::as_str) {
      Some("assistant") => {
        let id = msg.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        blocks.iter().filter_map(|b| self.assistant_block(b, &id)).collect()
      }
      // A string content is the task prompt (or a harness note): the node already shows its task
      Some("user") => blocks.iter().filter_map(|b| self.tool_result(b)).collect(),
      _ => vec![],
    }
  }

  fn chunk(&mut self, kind: &'static str, text: &str, message: &str) -> Option<Value> {
    if text.trim().is_empty() {
      return None;
    }
    let joined = matches!(&self.last, Some((k, m)) if *k == kind && m != message);
    self.last = Some((kind, message.to_owned()));
    let text = if joined { format!("\n\n{text}") } else { text.to_owned() };
    let update = if kind == "thinking" { "agent_thought_chunk" } else { "agent_message_chunk" };
    Some(json!({ "sessionUpdate": update, "content": { "type": "text", "text": text } }))
  }

  fn assistant_block(&mut self, b: &Value, message: &str) -> Option<Value> {
    match b.get("type").and_then(Value::as_str)? {
      "thinking" => self.chunk("thinking", b.get("thinking").and_then(Value::as_str)?, message),
      "text" => self.chunk("text", b.get("text").and_then(Value::as_str)?, message),
      "tool_use" => {
        let id = b.get("id").and_then(Value::as_str).filter(|x| !x.is_empty())?;
        let name = b.get("name").and_then(Value::as_str).unwrap_or("");
        self.last = None;
        self.tools.insert(id.to_owned(), name.to_owned());
        Some(tool_call(id, name, b.get("input").unwrap_or(&Value::Null), &self.cwd))
      }
      _ => None,
    }
  }

  fn tool_result(&mut self, b: &Value) -> Option<Value> {
    if b.get("type").and_then(Value::as_str) != Some("tool_result") {
      return None;
    }
    let id = b.get("tool_use_id").and_then(Value::as_str).filter(|x| !x.is_empty())?;
    let name = self.tools.get(id).cloned().unwrap_or_default();
    self.last = None;
    let content = b.get("content").unwrap_or(&Value::Null);
    Some(tool_result(id, &name, content, b.get("is_error") == Some(&Value::Bool(true))))
  }
}

fn str_in<'a>(input: &'a Value, key: &str) -> Option<&'a str> {
  input.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The adapter's `toDisplayPath`: relative inside the cwd, absolute outside
fn display_path(path: &str, cwd: &str) -> String {
  let cwd = cwd.trim_end_matches(['/', '\\']);
  if cwd.is_empty() {
    return path.to_owned();
  }
  match path.strip_prefix(cwd) {
    Some("") => ".".into(),
    Some(rest) if rest.starts_with(['/', '\\']) => rest[1..].to_owned(),
    _ => path.to_owned(),
  }
}

fn text_item(text: String) -> Value {
  json!({ "type": "content", "content": { "type": "text", "text": text } })
}

/// The adapter's `markdownEscape`: a fence longer than any fence inside
fn fenced(text: &str) -> String {
  let mut fence = "```".to_owned();
  for line in text.lines() {
    let ticks = line.chars().take_while(|c| *c == '`').count();
    while ticks >= 3 && ticks >= fence.len() {
      fence.push('`');
    }
  }
  let nl = if text.ends_with('\n') { "" } else { "\n" };
  format!("{fence}\n{text}{nl}{fence}")
}

/// A `tool_use` block as claude-agent-acp's first report of it (`AcpToolCallRenderer.toolCall`, reporters per tool)
pub fn tool_call(id: &str, name: &str, input: &Value, cwd: &str) -> Value {
  let mut locations: Option<Value> = None;
  let mut content: Vec<Value> = vec![];
  let path = str_in(input, "file_path").or_else(|| str_in(input, "path"));
  let (title, kind) = match name {
    "Bash" | "PowerShell" => (str_in(input, "command").unwrap_or("Terminal").to_owned(), "execute"),
    "Read" => {
      let offset = input.get("offset").and_then(Value::as_u64);
      let range = match (input.get("limit").and_then(Value::as_u64).filter(|l| *l > 0), offset) {
        (Some(limit), _) => format!(" ({} - {})", offset.unwrap_or(1), offset.unwrap_or(1) + limit - 1),
        (None, Some(o)) if o > 0 => format!(" (from line {o})"),
        _ => String::new(),
      };
      if let Some(p) = path {
        locations = Some(json!([{ "path": p, "line": offset.unwrap_or(1) }]));
      }
      (format!("Read {}{range}", path.map(|p| display_path(p, cwd)).unwrap_or_else(|| "File".into())), "read")
    }
    "Write" => {
      if let Some(p) = path {
        locations = Some(json!([{ "path": p }]));
        let text = ["content", "file_text", "file_content"].iter().find_map(|k| input.get(*k).filter(|v| !v.is_null()));
        content.push(json!({ "type": "diff", "path": p, "oldText": null, "newText": text }));
      }
      (path.map(|p| format!("Write {}", display_path(p, cwd))).unwrap_or_else(|| "Write".into()), "edit")
    }
    "Edit" => {
      if let Some(p) = path {
        locations = Some(json!([{ "path": p }]));
        let (old, new) = (str_in(input, "old_string"), input.get("new_string").and_then(Value::as_str));
        if old.is_some() || new.is_some_and(|n| !n.is_empty()) {
          content.push(json!({ "type": "diff", "path": p, "oldText": old, "newText": new.unwrap_or("") }));
        }
      }
      (path.map(|p| format!("Edit {}", display_path(p, cwd))).unwrap_or_else(|| "Edit".into()), "edit")
    }
    "Glob" => {
      let mut label = "Find".to_owned();
      if let Some(p) = str_in(input, "path") {
        label += &format!(" `{p}`");
        locations = Some(json!([{ "path": p }]));
      }
      if let Some(q) = str_in(input, "pattern") {
        label += &format!(" `{q}`");
      }
      (label, "search")
    }
    "Grep" => (grep_title(input), "search"),
    "WebFetch" => (str_in(input, "url").map(|u| format!("Fetch {u}")).unwrap_or_else(|| "Fetch".into()), "fetch"),
    "WebSearch" => (str_in(input, "query").map(|q| format!("Search \"{q}\"")).unwrap_or_else(|| "Web search".into()), "fetch"),
    "Agent" | "Task" => (str_in(input, "description").unwrap_or("Task").to_owned(), "think"),
    other => (if other.is_empty() { "Unknown Tool".to_owned() } else { other.to_owned() }, "other"),
  };
  let mut u = json!({
    "sessionUpdate": "tool_call",
    "toolCallId": id,
    "status": "pending",
    "title": title,
    "kind": kind,
    "rawInput": input,
    "content": content,
    "_meta": { "claudeCode": { "toolName": name } },
  });
  if let Some(l) = locations {
    u["locations"] = l;
  }
  u
}

/// The adapter's `GrepReporter` title: the equivalent grep command line
fn grep_title(input: &Value) -> String {
  let mut label = "grep".to_owned();
  let flag = |k: &str| input.get(k).and_then(Value::as_bool) == Some(true);
  if flag("-i") {
    label += " -i";
  }
  if flag("-n") {
    label += " -n";
  }
  for k in ["-A", "-B", "-C"] {
    if let Some(n) = input.get(k).filter(|v| !v.is_null()) {
      label += &format!(" {k} {}", n.as_str().map(str::to_owned).unwrap_or_else(|| n.to_string()));
    }
  }
  match str_in(input, "output_mode") {
    Some("files_with_matches") => label += " -l",
    Some("count") => label += " -c",
    _ => {}
  }
  if let Some(n) = input.get("head_limit").filter(|v| !v.is_null()) {
    label += &format!(" | head -{n}");
  }
  if let Some(g) = str_in(input, "glob") {
    label += &format!(" --include=\"{g}\"");
  }
  if let Some(t) = str_in(input, "type") {
    label += &format!(" --type={t}");
  }
  if flag("multiline") {
    label += " -P";
  }
  if let Some(p) = str_in(input, "pattern") {
    label += &format!(" \"{p}\"");
  }
  if let Some(p) = str_in(input, "path") {
    label += &format!(" {p}");
  }
  label
}

/// The text blocks of a tool_result content (a string, or text / image blocks)
fn result_texts(content: &Value) -> Vec<String> {
  match content {
    Value::String(s) => vec![s.clone()],
    Value::Array(items) => items.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).map(str::to_owned).collect(),
    _ => vec![],
  }
}

/// The adapter's `toAcpContentUpdate`: text (fenced when it is an error), base64 images as images
fn content_items(content: &Value, fence: impl Fn(&str) -> String) -> Vec<Value> {
  let one = |b: &Value| -> Option<Value> {
    match b.get("type").and_then(Value::as_str) {
      Some("text") => Some(text_item(fence(b.get("text").and_then(Value::as_str)?))),
      Some("image") if b.get("source").and_then(|s| s.get("type")).and_then(Value::as_str) == Some("base64") => {
        let s = b.get("source")?;
        Some(json!({ "type": "content", "content": { "type": "image", "data": s.get("data")?, "mimeType": s.get("media_type")? } }))
      }
      Some(_) => Some(text_item(fence(&b.to_string()))),
      None => None,
    }
  };
  match content {
    Value::String(s) if !s.is_empty() => vec![text_item(fence(s))],
    Value::Array(items) => items.iter().filter_map(one).collect(),
    _ => vec![],
  }
}

/// A `tool_result` block as the adapter's final report of the call (`AcpToolCallRenderer.result`)
pub fn tool_result(id: &str, name: &str, content: &Value, is_error: bool) -> Value {
  let mut u = Map::new();
  u.insert("sessionUpdate".into(), json!("tool_call_update"));
  u.insert("toolCallId".into(), json!(id));
  u.insert("status".into(), json!(if is_error { "failed" } else { "completed" }));
  let mut meta = json!({ "claudeCode": { "toolName": name } });
  let plain = |t: &str| t.to_owned();
  let error_fence = |t: &str| if is_error { format!("```\n{t}\n```") } else { t.to_owned() };
  let has_images = content.as_array().is_some_and(|a| a.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("image")));
  match name {
    // A command's output goes the terminal channel the adapter streams it on (Acpira advertises `terminal_output_delta`)
    "Bash" | "PowerShell" if !has_images => {
      let output = result_texts(content).join("\n");
      let code = if is_error { exit_code_of(&output).unwrap_or(1) } else { 0 };
      meta["terminal_output_delta"] = json!({ "terminal_id": id, "data": output });
      meta["terminal_exit"] = json!({ "terminal_id": id, "exit_code": code, "signal": null });
    }
    // The diff of the call is the result to show; the result text is only a confirmation
    "Edit" | "Write" if !is_error => {}
    "Read" if !is_error => {
      u.insert("content".into(), Value::Array(content_items(content, fenced)));
    }
    _ => {
      let items = content_items(content, if is_error { &error_fence as &dyn Fn(&str) -> String } else { &plain });
      if !items.is_empty() {
        u.insert("content".into(), Value::Array(items));
      }
    }
  }
  u.insert("_meta".into(), meta);
  Value::Object(u)
}

/// Claude Code starts a failed command's text with `Exit code N`
fn exit_code_of(text: &str) -> Option<i64> {
  let rest = text.strip_prefix("Exit code ")?;
  let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
  digits.parse().ok()
}

/// Where one agent's log has been read up to. `read` takes what was appended since; a partly written last line waits
/// for the next read
pub struct LogCursor {
  path: Option<PathBuf>,
  offset: u64,
  partial: Vec<u8>,
  /// A line over `MAX_LINE_BYTES` is being skipped up to its newline
  skipping: bool,
  /// (size, mtime) the last read reached the end at: an unchanged file is not opened again
  stamp: Option<(u64, Option<SystemTime>)>,
  parser: LineParser,
  /// The log outgrew `MAX_LOG_BYTES`, or was truncated under us: nothing more is read
  pub stopped: bool,
}

impl LogCursor {
  pub fn new(cwd: &str) -> Self {
    LogCursor { path: None, offset: 0, partial: vec![], skipping: false, stamp: None, parser: LineParser::new(cwd), stopped: false }
  }

  /// Whether the log file has been found
  pub fn located(&self) -> bool {
    self.path.is_some()
  }

  /// The last read reached the end of the file
  pub fn at_end(&self) -> bool {
    self.stamp.is_some_and(|(len, _)| len == self.offset)
  }

  /// The updates of every complete line appended since the last read (blocking file IO). `locate` finds the file the
  /// first time
  pub fn read(&mut self, locate: impl FnOnce() -> Option<PathBuf>) -> Vec<Value> {
    if self.stopped {
      return vec![];
    }
    if self.path.is_none() {
      self.path = locate();
    }
    let Some(path) = self.path.clone() else { return vec![] };
    let Ok(meta) = std::fs::metadata(&path) else { return vec![] };
    let stamp = (meta.len(), meta.modified().ok());
    if self.stamp == Some(stamp) {
      return vec![];
    }
    if meta.len() < self.offset {
      // Rewritten rather than appended: what was read no longer lines up with the file
      self.stopped = true;
      return vec![];
    }
    let want = (meta.len() - self.offset).min(READ_CHUNK).min(MAX_LOG_BYTES.saturating_sub(self.offset));
    if want == 0 {
      if self.offset >= MAX_LOG_BYTES {
        self.stopped = true;
      }
      self.stamp = Some(stamp);
      return vec![];
    }
    let mut buf = Vec::with_capacity(want as usize);
    let read = std::fs::File::open(&path).and_then(|mut f| {
      f.seek(SeekFrom::Start(self.offset))?;
      f.take(want).read_to_end(&mut buf)
    });
    if read.is_err() {
      return vec![];
    }
    self.offset += buf.len() as u64;
    if self.offset == meta.len() {
      self.stamp = Some(stamp);
    }
    self.feed(&buf)
  }

  /// Complete lines of `bytes` (after what an earlier read left) → updates
  pub fn feed(&mut self, bytes: &[u8]) -> Vec<Value> {
    let mut out = vec![];
    let mut rest = bytes;
    while let Some(nl) = rest.iter().position(|b| *b == b'\n') {
      let (head, tail) = (&rest[..nl], &rest[nl + 1..]);
      rest = tail;
      if self.skipping {
        self.skipping = false;
        continue;
      }
      if self.partial.is_empty() {
        out.extend(self.parser.line(head));
      } else {
        self.partial.extend_from_slice(head);
        let line = std::mem::take(&mut self.partial);
        out.extend(self.parser.line(&line));
      }
    }
    if !self.skipping {
      self.partial.extend_from_slice(rest);
      if self.partial.len() > MAX_LINE_BYTES {
        self.partial.clear();
        self.skipping = true;
      }
    }
    out
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  const FIXTURE: &str = include_str!("../../../../../../test/fixtures/claude-workflow-agent.jsonl");

  fn kinds(updates: &[Value]) -> Vec<String> {
    updates.iter().map(|u| u["sessionUpdate"].as_str().unwrap_or("").to_owned()).collect()
  }

  #[test]
  fn the_fixture_becomes_thoughts_tool_calls_results_and_the_reply() {
    let mut cursor = LogCursor::new("/Volumes/Nano/Projects/acpira");
    let updates = cursor.feed(FIXTURE.as_bytes());
    assert_eq!(
      kinds(&updates),
      [
        "agent_thought_chunk",
        "tool_call",
        "tool_call_update",
        "agent_thought_chunk",
        "tool_call",
        "tool_call_update",
        "tool_call",
        "tool_call_update",
        "tool_call",
        "tool_call_update",
        "agent_message_chunk",
      ]
    );
    // The task prompt (a string user line), the attachment and the unknown `progress` line add nothing
    let bash = &updates[1];
    assert_eq!(bash["toolCallId"], "toolu_018HXFqz5mwbZVxd931JXxaA");
    assert_eq!(bash["kind"], "execute");
    assert_eq!(bash["_meta"]["claudeCode"]["toolName"], "Bash");
    assert!(bash["title"].as_str().unwrap().starts_with("git status --short"));
    assert_eq!(bash["rawInput"]["description"], "Check repo status and doc sections");
    let out = &updates[2];
    assert_eq!(out["status"], "completed");
    assert!(out["_meta"]["terminal_output_delta"]["data"].as_str().unwrap().starts_with("39392d4"));
    assert_eq!(out["_meta"]["terminal_exit"]["exit_code"], 0);
    // A failed Read: fenced error text, the row fails
    let read = &updates[8];
    assert_eq!(read["title"], "Read missing.md (3 - 12)");
    assert_eq!(read["kind"], "read");
    assert_eq!(read["locations"][0]["line"], 3);
    expect_text(&updates[9], "```\nFile does not exist.\n```");
    assert_eq!(updates[9]["status"], "failed");
    assert_eq!(updates[10]["content"]["text"], "The ultra toggle is wired end to end.");
  }

  fn expect_text(u: &Value, text: &str) {
    assert_eq!(u["content"][0]["content"]["text"], text, "{u}");
  }

  #[test]
  fn a_partial_last_line_waits_for_its_newline() {
    let mut cursor = LogCursor::new("/w");
    let all = FIXTURE.as_bytes();
    // Cut inside the first thinking line: nothing of it is emitted until the rest arrives
    let thinking_at = FIXTURE.find("\"thinking\"").unwrap();
    let first = cursor.feed(&all[..thinking_at]);
    assert!(first.is_empty());
    let rest = cursor.feed(&all[thinking_at..]);
    assert_eq!(rest.len(), 11);
    assert_eq!(rest[0]["sessionUpdate"], "agent_thought_chunk");
    // A line still being written at the end is held back
    let mut cursor = LogCursor::new("/w");
    let cut = all.len() - 20;
    assert_eq!(cursor.feed(&all[..cut]).len(), 10);
    assert_eq!(kinds(&cursor.feed(&all[cut..])), ["agent_message_chunk"]);
  }

  #[test]
  fn unknown_and_malformed_lines_are_skipped() {
    let mut p = LineParser::new("/w");
    assert!(p.line(b"not json").is_empty());
    assert!(p.line(br#"{"type":"summary","summary":"x"}"#).is_empty());
    assert!(p.line(br#"{"type":"assistant","message":{"id":"m","content":[{"type":"redacted_thinking","data":"x"},{"type":"thinking","thinking":""}]}}"#).is_empty());
    assert!(p.line(br#"{"type":"user","message":{"role":"user","content":"the task"}}"#).is_empty());
    assert!(p.line(br#"{"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","name":"Bash"}]}}"#).is_empty());
  }

  #[test]
  fn several_blocks_of_one_line_and_a_new_message_keep_their_order_and_breaks() {
    let mut p = LineParser::new("/w");
    let out = p.line(
      br#"{"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"one"},{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/w/src/a.rs","old_string":"a","new_string":"b"}},{"type":"text","text":"two"}]}}"#,
    );
    assert_eq!(kinds(&out), ["agent_message_chunk", "tool_call", "agent_message_chunk"]);
    assert_eq!(out[1]["title"], "Edit src/a.rs");
    assert_eq!(out[1]["content"][0]["type"], "diff");
    assert_eq!(out[1]["content"][0]["oldText"], "a");
    // Another message's text right after joins the streaming block with a paragraph break; the same message's does not
    let next = p.line(br#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"three"}]}}"#);
    assert_eq!(next[0]["content"]["text"], "\n\nthree");
    let same = p.line(br#"{"type":"assistant","message":{"id":"m2","content":[{"type":"text","text":"four"}]}}"#);
    assert_eq!(same[0]["content"]["text"], "four");
    // An Edit result shows the call's diff, not the confirmation text
    let done = p.line(br#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"The file has been updated."}]}}"#);
    assert_eq!(done[0]["status"], "completed");
    assert!(done[0].get("content").is_none());
    // A failed command keeps its exit code
    p.line(br#"{"type":"assistant","message":{"id":"m3","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"false"}}]}}"#);
    let failed = p.line(br#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"Exit code 2\nboom"}],"is_error":true}]}}"#);
    assert_eq!(failed[0]["status"], "failed");
    assert_eq!(failed[0]["_meta"]["terminal_exit"]["exit_code"], 2);
  }

  #[test]
  fn tool_titles_follow_the_adapter() {
    let t = |name: &str, input: Value| tool_call("x", name, &input, "/w")["title"].as_str().unwrap().to_owned();
    assert_eq!(t("Read", json!({ "file_path": "/w/a.md" })), "Read a.md");
    assert_eq!(t("Read", json!({ "file_path": "/elsewhere/a.md", "offset": 5 })), "Read /elsewhere/a.md (from line 5)");
    assert_eq!(t("Glob", json!({ "pattern": "**/*.rs", "path": "src" })), "Find `src` `**/*.rs`");
    assert_eq!(t("Grep", json!({ "pattern": "fn main", "-n": true, "output_mode": "files_with_matches", "path": "src" })), "grep -n -l \"fn main\" src");
    assert_eq!(t("WebFetch", json!({ "url": "https://example.com" })), "Fetch https://example.com");
    assert_eq!(t("WebSearch", json!({ "query": "acp" })), "Search \"acp\"");
    assert_eq!(t("Agent", json!({ "description": "look around" })), "look around");
    assert_eq!(t("mcp__x__y", json!({})), "mcp__x__y");
    assert_eq!(tool_call("x", "Write", &json!({ "file_path": "/w/n.txt", "content": "hi" }), "/w")["content"][0]["newText"], "hi");
  }

  #[test]
  fn the_project_directory_follows_the_sdk_sanitizer() {
    assert_eq!(project_dir_name("/Volumes/Nano/Projects/acpira"), "-Volumes-Nano-Projects-acpira");
    assert_eq!(project_dir_name("C:\\Users\\me\\my.repo"), "C--Users-me-my-repo");
    // One dash per UTF-16 unit: a CJK character is one, an emoji two
    assert_eq!(project_dir_name("/tmp/项目"), "-tmp---");
    assert_eq!(project_dir_name("/a😀"), "-a--");
    // Past 200 units: cut, then the base36 JS string hash (`Math.abs(h).toString(36)`)
    let long = format!("/{}", "a".repeat(250));
    let name = project_dir_name(&long);
    assert!(name.starts_with(&format!("-{}-", "a".repeat(199))));
    let units: Vec<u16> = long.encode_utf16().collect();
    let mut h: i64 = 0;
    for u in units {
      h = ((h << 5) - h + u as i64) as i32 as i64;
    }
    assert_eq!(name, format!("{}-{}", &format!("-{}", "a".repeat(250))[..200], base36(h.unsigned_abs())));
  }

  #[test]
  fn the_config_dir_and_the_log_resolve_with_a_fallback_scan() {
    assert_eq!(config_dir_of(Some("/custom".into())), PathBuf::from("/custom"));
    assert_eq!(config_dir_of(Some(String::new())), home_dir().join(".claude"));
    assert_eq!(config_dir_of(None), home_dir().join(".claude"));
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path();
    let run = config.join("projects/-w/sid-1/subagents/workflows/wf_1");
    std::fs::create_dir_all(&run).unwrap();
    std::fs::write(run.join("agent-ab12.jsonl"), "").unwrap();
    let dir = session_dir(config, "/w", "sid-1").unwrap();
    assert_eq!(dir, config.join("projects/-w/sid-1"));
    assert_eq!(agent_log(&dir, "ab12"), Some(run.join("agent-ab12.jsonl")));
    assert_eq!(agent_log(&dir, "zz"), None);
    assert_eq!(agent_log(&dir, "../x"), None);
    // The CLI resolved the cwd differently (a symlink, a hashed long path): any project directory holding the session
    assert_eq!(session_dir(config, "/somewhere/else", "sid-1"), Some(dir));
    assert_eq!(session_dir(config, "/w", "sid-2"), None);
    assert_eq!(session_dir(config, "/w", "../sid-1"), None);
  }

  #[test]
  fn a_cursor_reads_only_what_was_appended() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("agent-a.jsonl");
    let lines: Vec<&str> = FIXTURE.lines().collect();
    std::fs::write(&path, format!("{}\n{}\n{}", lines[0], lines[2], &lines[3][..40])).unwrap();
    let mut cursor = LogCursor::new("/w");
    assert!(cursor.read(|| None).is_empty());
    assert!(!cursor.located());
    let first = cursor.read(|| Some(path.clone()));
    assert_eq!(kinds(&first), ["agent_thought_chunk"]);
    // Unchanged: nothing
    assert!(cursor.read(|| unreachable!()).is_empty());
    let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
    std::io::Write::write_all(&mut f, format!("{}\n{}\n", &lines[3][40..], lines[4]).as_bytes()).unwrap();
    assert_eq!(kinds(&cursor.read(|| unreachable!())), ["tool_call", "tool_call_update"]);
    // Rewritten shorter: the cursor stops rather than re-reading from a wrong offset
    std::fs::write(&path, "{}\n").unwrap();
    assert!(cursor.read(|| unreachable!()).is_empty());
    assert!(cursor.stopped);
  }
}
