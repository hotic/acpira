//! What Claude Code's own session log (`<config>/projects/<encoded cwd>/<sessionId>.jsonl`, claude-agent-acp 0.84.0,
//! Claude Code 2.1.284) tells of its context, for the report's estimate (`context_estimate.rs`): every request's usage
//! (`message.usage`: `input_tokens` + `cache_read_input_tokens` + `cache_creation_input_tokens` is the whole prompt),
//! the summary a compaction left (a user line with `isCompactSummary`), and where the compaction cut
//! (`system`/`compact_boundary`). The first request after the cut carries the system prompt, the tool definitions and
//! the injected files plus the messages before it, so less those messages it is what the agent keeps of its own.
//! Subagent lines (`isSidechain`) are another context and are skipped. The log is read on from where the last read
//! stopped: it reaches tens of megabytes and the report asks every few seconds while a turn runs.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use acpira_shared::context::ContextCategory;
use serde_json::Value;

use crate::acp::session::context_estimate::{Known, approx_tokens};
use crate::acp::vendors::claude_workflow_log as wlog;

/// A line longer than this (a huge tool result) is counted by its length instead of parsed
const MAX_PARSE: usize = 4 * 1024 * 1024;

#[derive(Default, Clone)]
struct Since {
  summary: Option<String>,
  /// Tokens of what went into the first request after the cut besides the agent's own (summary, prompt, attachments)
  before_first: u64,
  first_prompt: Option<u64>,
  last_prompt: Option<u64>,
  last_cached: Option<u64>,
}

#[derive(Default)]
struct Cursor {
  offset: u64,
  /// The unfinished last line
  tail: Vec<u8>,
  since: Since,
}

static CURSORS: LazyLock<Mutex<HashMap<PathBuf, Cursor>>> = LazyLock::new(Default::default);

/// The session's log, under the config directory the agent runs with
pub fn log_path(config_env: Option<String>, cwd: &str, session_id: &str) -> Option<PathBuf> {
  if session_id.is_empty() || !session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
    return None;
  }
  let projects = wlog::config_dir_of(config_env.or_else(|| std::env::var(wlog::CONFIG_DIR_ENV).ok())).join("projects");
  let file = format!("{session_id}.jsonl");
  let direct = projects.join(wlog::project_dir_name(cwd)).join(&file);
  if direct.is_file() {
    return Some(direct);
  }
  std::fs::read_dir(&projects).ok()?.filter_map(|e| e.ok()).map(|e| e.path().join(&file)).find(|p| p.is_file())
}

/// The log's split as far as it is written now
pub fn known(path: &Path) -> Option<Known> {
  let mut cursors = CURSORS.lock().ok()?;
  let cur = cursors.entry(path.to_owned()).or_default();
  let mut f = std::fs::File::open(path).ok()?;
  let len = f.metadata().ok()?.len();
  // Rewritten shorter (a rewind): read it again from the start
  if len < cur.offset {
    *cur = Cursor::default();
  }
  if len > cur.offset {
    f.seek(SeekFrom::Start(cur.offset)).ok()?;
    let mut buf = Vec::with_capacity((len - cur.offset) as usize);
    f.take(len - cur.offset).read_to_end(&mut buf).ok()?;
    cur.offset += buf.len() as u64;
    let mut data = std::mem::take(&mut cur.tail);
    data.extend_from_slice(&buf);
    let mut start = 0;
    while let Some(n) = data[start..].iter().position(|b| *b == b'\n') {
      line(&mut cur.since, &data[start..start + n]);
      start += n + 1;
    }
    cur.tail = data[start..].to_vec();
  }
  of(&cur.since)
}

fn of(s: &Since) -> Option<Known> {
  let first = s.first_prompt?;
  let last = s.last_prompt.unwrap_or(first);
  let own = first.saturating_sub(s.before_first);
  Some(Known {
    parts: vec![(ContextCategory::Agent, own)],
    chat: last.saturating_sub(own),
    summary: s.summary.clone(),
    cached: s.last_cached,
  })
}

fn prompt_of(usage: &Value) -> Option<(u64, u64)> {
  let n = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
  let read = n("cache_read_input_tokens");
  let total = n("input_tokens") + read + n("cache_creation_input_tokens");
  (total > 0).then_some((total, read))
}

fn text_of(content: &Value) -> String {
  match content {
    Value::String(s) => s.clone(),
    Value::Array(parts) => parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("\n"),
    _ => String::new(),
  }
}

fn line(s: &mut Since, raw: &[u8]) {
  if raw.is_empty() {
    return;
  }
  if raw.len() > MAX_PARSE {
    // Too big to parse every time; still part of what the first request carried when it comes before it
    if s.first_prompt.is_none() {
      s.before_first += (raw.len() as u64).div_ceil(4);
    }
    return;
  }
  let Ok(v) = serde_json::from_slice::<Value>(raw) else { return };
  if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
    return;
  }
  match v.get("type").and_then(Value::as_str) {
    Some("system") if v.get("subtype").and_then(Value::as_str) == Some("compact_boundary") => *s = Since::default(),
    Some("assistant") => {
      if let Some((total, read)) = v.pointer("/message/usage").and_then(prompt_of) {
        s.first_prompt.get_or_insert(total);
        s.last_prompt = Some(total);
        s.last_cached = Some(read);
      }
    }
    Some("user") => {
      let content = v.pointer("/message/content").map(text_of).unwrap_or_default();
      if v.get("isCompactSummary").and_then(Value::as_bool) == Some(true) {
        s.summary = Some(content.clone());
      }
      if s.first_prompt.is_none() {
        s.before_first += approx_tokens(&content);
      }
    }
    Some("attachment") if s.first_prompt.is_none() => {
      s.before_first += v.get("attachment").map_or(0, |a| approx_tokens(&a.to_string()));
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  #![allow(clippy::unwrap_used, reason = "test code")]
  use super::*;
  use serde_json::json;

  fn write(dir: &Path, lines: &[Value]) -> PathBuf {
    let p = dir.join("s.jsonl");
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&p, text).unwrap();
    p
  }

  fn assistant(input: u64, read: u64, created: u64) -> Value {
    json!({ "type": "assistant", "message": { "usage": { "input_tokens": input, "cache_read_input_tokens": read, "cache_creation_input_tokens": created } } })
  }

  #[test]
  fn the_first_request_after_a_compaction_less_its_messages_is_the_agents_own() {
    let dir = tempfile::tempdir().unwrap();
    let summary = "s".repeat(4000);
    let lines = vec![
      json!({ "type": "user", "message": { "content": "old" } }),
      assistant(10, 0, 50_000),
      json!({ "type": "system", "subtype": "compact_boundary" }),
      json!({ "type": "user", "isCompactSummary": true, "message": { "content": summary } }),
      json!({ "type": "assistant", "isSidechain": true, "message": { "usage": { "input_tokens": 999_999 } } }),
      assistant(5, 0, 21_000),
      json!({ "type": "user", "message": { "content": [{ "type": "tool_result", "content": "x" }] } }),
      assistant(5, 20_000, 3_000),
    ];
    let p = write(dir.path(), &lines);
    let k = known(&p).unwrap();
    // 21_005 less the summary's 1000 tokens
    assert_eq!(k.parts, vec![(ContextCategory::Agent, 20_005)]);
    assert_eq!(k.chat, 23_005 - 20_005);
    assert_eq!(k.summary.as_deref().map(str::len), Some(4000));
    assert_eq!(k.cached, Some(20_000));

    // Appended lines are read on from where the last read stopped
    let mut more = std::fs::OpenOptions::new().append(true).open(&p).unwrap();
    std::io::Write::write_all(&mut more, format!("{}\n", assistant(5, 23_000, 1_000)).as_bytes()).unwrap();
    let k = known(&p).unwrap();
    assert_eq!((k.chat, k.cached), (24_005 - 20_005, Some(23_000)));
  }

  #[test]
  fn a_log_without_a_request_knows_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), &[json!({ "type": "user", "message": { "content": "hi" } })]);
    assert!(known(&p).is_none());
  }
}
