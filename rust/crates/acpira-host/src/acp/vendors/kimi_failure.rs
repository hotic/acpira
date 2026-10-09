//! Kimi's swallowed turn failures. Kimi Code 0.41.0 through 2.1.1 (`onTurnEnded` / `turnEndReasonToStopReason`) answers
//! a turn that failed with anything but an auth error (provider HTTP errors, unknown models, timeouts) with a plain
//! `end_turn` and no output, so the prompt looks like an empty success. The real error is only in Kimi's own session
//! log: `<KIMI_CODE_HOME>/sessions/<workdir>/<acp session id>/agents/main/wire.jsonl` gets a
//! `{"type":"turn.ended","reason":"failed","error":{code,message,name,details:{statusCode},retryable},"time":ms}` line.
//! An empty `end_turn` from Kimi reads that line back so the error card shows the cause instead of the generic text.
//! Upstream: MoonshotAI/kimi-code#1813 / #3107, fix PR #3161 (open as of 2026-10-09)

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::store::data_dir::home_dir;

pub const HOME_ENV: &str = "KIMI_CODE_HOME";
/// Only the end of a long session's log matters: the failed turn's lines are the last few
const TAIL_BYTES: u64 = 256 * 1024;
/// The log line and the ACP answer are written independently: give the line a moment to land
const POLL_ATTEMPTS: u32 = 10;
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// What Kimi recorded about a failed turn
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
  pub message: String,
  /// Kimi's error code (`provider.api_error`, `provider.timeout`, …)
  pub code: Option<String>,
  pub retryable: Option<bool>,
}

/// How the last main-agent turn in the log ended
#[derive(Debug, PartialEq)]
enum LastTurn {
  /// No `turn.ended` at or after the prompt yet (not flushed, or the log is missing)
  Pending,
  Ended(Option<Failure>),
}

/// `KIMI_CODE_HOME` (the agent entry's own env first, then the engine's) or `~/.kimi-code`, like Kimi's own lookup
pub fn home_of(entry_value: Option<String>) -> PathBuf {
  let value = entry_value.or_else(|| std::env::var(HOME_ENV).ok()).filter(|s| !s.trim().is_empty());
  match value {
    Some(d) if d == "~" => home_dir(),
    Some(d) => d.strip_prefix("~/").map(|rest| home_dir().join(rest)).unwrap_or_else(|| PathBuf::from(d)),
    None => home_dir().join(".kimi-code"),
  }
}

/// The failure Kimi logged for the turn that started at `since_ms` (epoch ms), if that turn failed. Blocking file IO
/// with a short poll: run it off the async threads
pub fn read(home: &Path, session_id: &str, since_ms: i64) -> Option<Failure> {
  for attempt in 0..POLL_ATTEMPTS {
    if attempt > 0 {
      std::thread::sleep(POLL_INTERVAL);
    }
    let Some(path) = wire_file(home, session_id) else { continue };
    let Some(text) = read_tail(&path) else { continue };
    if let LastTurn::Ended(failure) = last_turn(&text, since_ms) {
      return failure;
    }
  }
  None
}

/// The session's main-agent log: the workdir level is a hashed name, so every workdir is looked into
fn wire_file(home: &Path, session_id: &str) -> Option<PathBuf> {
  // The id comes from the agent: never let it walk out of the sessions directory
  if session_id.is_empty() || session_id.contains(['/', '\\']) || session_id.contains("..") {
    return None;
  }
  std::fs::read_dir(home.join("sessions"))
    .ok()?
    .filter_map(|e| e.ok())
    .map(|e| e.path().join(session_id).join("agents").join("main").join("wire.jsonl"))
    .find(|p| p.is_file())
}

/// The last `TAIL_BYTES` of the file; a line cut at the start fails to parse and is skipped
fn read_tail(path: &Path) -> Option<String> {
  let mut file = std::fs::File::open(path).ok()?;
  let len = file.metadata().ok()?.len();
  file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES))).ok()?;
  let mut bytes = Vec::new();
  file.read_to_end(&mut bytes).ok()?;
  Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The last main-agent `turn.ended` line, if it was written at or after `since_ms` (an older one is the previous turn's)
fn last_turn(jsonl: &str, since_ms: i64) -> LastTurn {
  for line in jsonl.lines().rev() {
    let Ok(e) = serde_json::from_str::<Value>(line.trim()) else { continue };
    if e.get("type").and_then(Value::as_str) != Some("turn.ended") {
      continue;
    }
    if e.get("agentId").and_then(Value::as_str).is_some_and(|a| a != "main") {
      continue;
    }
    if e.get("time").and_then(Value::as_i64).is_none_or(|t| t < since_ms) {
      return LastTurn::Pending;
    }
    if e.get("reason").and_then(Value::as_str) != Some("failed") {
      return LastTurn::Ended(None);
    }
    return LastTurn::Ended(Some(failure_of(e.get("error"))));
  }
  LastTurn::Pending
}

/// The error object of a failed `turn.ended`; a failure without one still gets a message
fn failure_of(error: Option<&Value>) -> Failure {
  let str_of = |key: &str| error.and_then(|e| e.get(key)).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
  let code = str_of("code").map(str::to_owned);
  let message = str_of("message").or_else(|| str_of("name")).or(code.as_deref()).unwrap_or("Agent turn failed").to_owned();
  Failure { message, code, retryable: error.and_then(|e| e.get("retryable")).and_then(Value::as_bool) }
}

#[cfg(test)]
mod tests {
  use super::*;

  const FAILED: &str = r#"{"type":"turn.ended","agentId":"main","turnId":0,"reason":"failed","error":{"code":"provider.api_error","message":"400 unsupported Ollama model: kimi-k2.7-highspeed","name":"APIStatusError","details":{"statusCode":400,"requestId":null,"traceId":null},"retryable":false},"durationMs":96,"time":2000}"#;

  fn log(lines: &[&str]) -> String {
    lines.join("\n") + "\n"
  }

  #[test]
  fn reads_the_failure_of_the_current_turn() {
    let text = log(&[
      r#"{"type":"turn.started","agentId":"main","turnId":0,"time":1990}"#,
      FAILED,
      r#"{"type":"prompt.completed","agentId":"main","reason":"failed","time":2001}"#,
    ]);
    let want = Failure {
      message: "400 unsupported Ollama model: kimi-k2.7-highspeed".into(),
      code: Some("provider.api_error".into()),
      retryable: Some(false),
    };
    assert_eq!(last_turn(&text, 1500), LastTurn::Ended(Some(want)));
  }

  #[test]
  fn an_older_turn_is_not_this_one() {
    assert_eq!(last_turn(&log(&[FAILED]), 2500), LastTurn::Pending);
    assert_eq!(last_turn("", 0), LastTurn::Pending);
  }

  #[test]
  fn a_completed_turn_has_no_failure() {
    let text = log(&[FAILED, r#"{"type":"turn.ended","agentId":"main","turnId":1,"reason":"completed","time":3000}"#]);
    assert_eq!(last_turn(&text, 2500), LastTurn::Ended(None));
  }

  #[test]
  fn subagent_turns_and_cut_lines_are_skipped() {
    let text = log(&[
      FAILED,
      r#"{"type":"turn.ended","agentId":"sub-1","reason":"completed","time":2100}"#,
      r#"{"type":"turn.en"#,
    ]);
    assert!(matches!(last_turn(&text, 1500), LastTurn::Ended(Some(_))));
  }

  #[test]
  fn a_failure_without_details_still_has_a_message() {
    assert_eq!(failure_of(None).message, "Agent turn failed");
    let only_code = serde_json::json!({ "code": "provider.timeout" });
    assert_eq!(failure_of(Some(&only_code)).message, "provider.timeout");
  }

  #[test]
  fn finds_the_log_under_any_workdir_and_rejects_paths() {
    let home = std::env::temp_dir().join(format!("acpira-kimi-failure-{}", crate::util::random_uuid()));
    let dir = home.join("sessions").join("wd_proj_abc123").join("session_x").join("agents").join("main");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("wire.jsonl"), log(&[FAILED])).unwrap();
    std::fs::create_dir_all(home.join("sessions").join("wd_other_def456")).unwrap();
    assert_eq!(read(&home, "session_x", 1500).map(|f| f.code), Some(Some("provider.api_error".into())));
    assert_eq!(wire_file(&home, "../session_x"), None);
    assert_eq!(wire_file(&home, "session_y"), None);
    std::fs::remove_dir_all(&home).unwrap();
  }
}
