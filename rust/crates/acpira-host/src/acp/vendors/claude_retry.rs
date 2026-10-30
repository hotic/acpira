//! Claude API retries with their error text. claude-agent-acp's AIR retry warning names only the attempt ("Retrying
//! Claude, attempt 5 of 10."); the raw SDK `api_retry` frame the host already subscribes to (`claude_workflow.rs`)
//! adds the HTTP status, the error kind and the wait, but no response body. The CLI writes the body to its own
//! session log as it retries (observed with Claude Code 2.1.x under claude-agent-acp 0.84.0):
//!
//! `<config>/projects/<encoded cwd>/<sessionId>.jsonl`, one line per failed attempt:
//! `{ type: "system", subtype: "api_error", level: "error", error: { message, status?, requestId?, formatted,
//! connection: { code, message } | null, noResponse }, retryInMs, retryAttempt, maxRetries, timestamp }`
//!
//! `message` is the SDK's error text (`529 {"type":"error","error":{"type":"overloaded_error",…},"request_id":…}`,
//! or `Connection error.` with the socket error under `connection`). A private format: anything unexpected yields
//! nothing and the frame's summary stays.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::acp::transcript::normalize::{RetryReport, RetryText};
use crate::acp::vendors::claude_workflow_log::project_dir_name;
use crate::acp::vendors::logged_failure::read_tail;

/// The log line and the SDK frame are written independently: give the line a moment to land
const POLL_ATTEMPTS: u32 = 10;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
/// A line older than this belongs to an earlier run of retries with the same attempt numbers
const FRESH_MS: i64 = 120_000;

fn u32_of(v: &Value, k: &str) -> Option<u32> {
  v.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok())
}

/// A raw `api_retry` frame (`_claude/sdkMessage` params) → the attempt it reports. None for any other frame and for
/// authentication retries: the first is the CLI's credential refresh, and `claude_auth.rs` ends the turn on the second
pub fn report_of(params: &Value) -> Option<RetryReport> {
  let msg = params.get("message")?;
  if msg.get("type").and_then(Value::as_str) != Some("system") || msg.get("subtype").and_then(Value::as_str) != Some("api_retry") {
    return None;
  }
  let error = msg.get("error").and_then(Value::as_str).filter(|e| !e.is_empty()).unwrap_or("unknown");
  if matches!(error, "authentication_failed" | "oauth_org_not_allowed") {
    return None;
  }
  let status = msg.get("error_status").and_then(Value::as_u64);
  let mut text = match status {
    Some(code) => format!("{code} {error}"),
    None => format!("connection lost ({error})"),
  };
  if let Some(n) = msg.get("no_response") {
    let secs = |k: &str| n.get(k).and_then(Value::as_u64).map(|ms| ms.div_ceil(1000)).unwrap_or(0);
    text = format!("no response after {}s; waiting up to {}s", secs("waited_ms"), secs("retry_wait_ms"));
  }
  Some(RetryReport {
    attempt: u32_of(msg, "attempt"),
    max: u32_of(msg, "max_retries"),
    text,
    quality: RetryText::Summary,
    reconnect: status.is_none(),
    wait_ms: msg.get("retry_delay_ms").and_then(Value::as_u64),
  })
}

/// `<config>/projects/<encoded cwd>/<sessionId>.jsonl`, or the file any project directory holds for the session
pub fn session_log(config: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
  let safe = !session_id.is_empty() && session_id.len() <= 128 && session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
  if !safe {
    return None;
  }
  let projects = config.join("projects");
  let name = format!("{session_id}.jsonl");
  let direct = projects.join(project_dir_name(cwd)).join(&name);
  if direct.is_file() {
    return Some(direct);
  }
  std::fs::read_dir(&projects).ok()?.filter_map(|e| e.ok()).map(|e| e.path().join(&name)).find(|p| p.is_file())
}

/// The error text of an `api_error` line: the SDK's message, then the socket error when the connection failed
fn text_of(line: &Value) -> Option<String> {
  let error = line.get("error")?;
  let mut parts: Vec<String> = vec![];
  if let Some(m) = error.get("message").and_then(Value::as_str).map(str::trim).filter(|m| !m.is_empty()) {
    parts.push(m.to_owned());
  }
  if let Some(c) = error.get("connection").filter(|c| c.is_object()) {
    let code = c.get("code").and_then(Value::as_str).unwrap_or_default();
    let message = c.get("message").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    let socket = match (code.is_empty(), message.is_empty()) {
      (false, false) if message.starts_with(code) => message.to_owned(),
      (false, false) => format!("{code}: {message}"),
      (false, true) => code.to_owned(),
      (true, false) => message.to_owned(),
      (true, true) => String::new(),
    };
    if !socket.is_empty() && !parts.contains(&socket) {
      parts.push(socket);
    }
  }
  (!parts.is_empty()).then(|| parts.join("\n"))
}

/// The newest fresh `api_error` line of `attempt` in a log tail
pub fn logged_text(tail: &str, attempt: u32, now: i64) -> Option<String> {
  tail.lines().rev().filter_map(|l| serde_json::from_str::<Value>(l).ok()).find_map(|v| {
    let fits = v.get("type").and_then(Value::as_str) == Some("system")
      && v.get("subtype").and_then(Value::as_str) == Some("api_error")
      && u32_of(&v, "retryAttempt") == Some(attempt)
      && v.get("timestamp").and_then(Value::as_str).and_then(crate::util::ms_of_iso).is_some_and(|t| now - t < FRESH_MS);
    if fits { text_of(&v) } else { None }
  })
}

/// Polls the session log until the attempt's line shows up. Blocking: run it off the async threads
pub fn read_logged(config: &Path, cwd: &str, session_id: &str, attempt: u32) -> Option<String> {
  for n in 0..POLL_ATTEMPTS {
    if n > 0 {
      std::thread::sleep(POLL_INTERVAL);
    }
    let found = session_log(config, cwd, session_id)
      .and_then(|p| read_tail(&p))
      .and_then(|tail| logged_text(&tail, attempt, crate::util::now_ms()));
    if found.is_some() {
      return found;
    }
  }
  None
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn frame(msg: Value) -> Value {
    json!({ "sessionId": "s1", "message": msg })
  }

  #[test]
  fn a_frame_reports_status_kind_and_wait() {
    let r = report_of(&frame(json!({ "type": "system", "subtype": "api_retry", "attempt": 5, "max_retries": 10,
      "retry_delay_ms": 12000, "error_status": 529, "error": "overloaded" }))).unwrap();
    assert_eq!((r.attempt, r.max, r.text.as_str(), r.wait_ms, r.reconnect), (Some(5), Some(10), "529 overloaded", Some(12000), false));
    let lost = report_of(&frame(json!({ "type": "system", "subtype": "api_retry", "attempt": 1, "max_retries": 10,
      "retry_delay_ms": 500, "error_status": null, "error": "unknown" }))).unwrap();
    assert!(lost.reconnect && lost.text.contains("connection lost"), "{lost:?}");
    let auth = frame(json!({ "type": "system", "subtype": "api_retry", "attempt": 1, "error_status": 401, "error": "authentication_failed" }));
    assert!(report_of(&auth).is_none());
    assert!(report_of(&frame(json!({ "type": "system", "subtype": "task_progress" }))).is_none());
  }

  #[test]
  fn the_log_gives_the_body_of_a_fresh_line_of_that_attempt() {
    let now = crate::util::ms_of_iso("2026-09-29T19:27:20.000Z").unwrap();
    let line = |attempt: u32, at: &str, error: Value| {
      json!({ "type": "system", "subtype": "api_error", "level": "error", "error": error, "retryAttempt": attempt, "timestamp": at }).to_string()
    };
    let body = r#"529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"},"request_id":"req_1"}"#;
    let tail = [
      line(2, "2026-09-29T10:00:00.000Z", json!({ "message": "old run" })),
      line(2, "2026-09-29T19:27:15.420Z", json!({ "message": body, "status": 529, "connection": null })),
      line(3, "2026-09-29T19:27:18.000Z", json!({ "message": "Connection error.",
        "connection": { "code": "ECONNRESET", "message": "The socket connection was closed unexpectedly" } })),
      "{not json".to_owned(),
    ]
    .join("\n");
    assert_eq!(logged_text(&tail, 2, now).as_deref(), Some(body));
    assert_eq!(logged_text(&tail, 3, now).as_deref(), Some("Connection error.\nECONNRESET: The socket connection was closed unexpectedly"));
    assert_eq!(logged_text(&tail, 4, now), None);
    let stale = line(1, "2026-09-29T10:00:00.000Z", json!({ "message": "old run" }));
    assert_eq!(logged_text(&stale, 1, now), None);
  }
}
