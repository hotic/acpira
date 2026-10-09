//! Pi's swallowed turn failures. pi-acp 0.0.33 settles every prompt on pi's `agent_settled` with `end_turn` and never
//! forwards pi's `message_end`, so a model call that failed (HTTP 4xx / 5xx from the provider, a dropped connection)
//! reaches ACP as an empty `end_turn`; a rejected `prompt` RPC is dropped the same way (`startTurn`, `void err`). pi
//! still writes the failed reply to its session file (the one `pi_usage` reads):
//! `{"type":"message","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"502: …","timestamp":ms}}`.
//! An empty `end_turn` from Pi reads that entry back (`logged_failure`)

use std::path::Path;

use serde_json::Value;

use crate::acp::vendors::logged_failure::{Failure, LastTurn, poll, read_tail};
use crate::acp::vendors::pi_usage;
use crate::store::data_dir::home_dir;

/// The failure pi logged for the turn that started at `since_ms` (epoch ms), if that turn's reply failed. Blocking file
/// IO with a short poll: run it off the async threads
pub fn read(agent_dir: &Path, cwd: &str, session_id: &str, since_ms: i64) -> Option<Failure> {
  let home = home_dir();
  poll(|| Some(last_turn(&read_tail(&pi_usage::session_file(&home, agent_dir, cwd, session_id)?)?, since_ms)))
}

/// The last assistant reply, if it was written at or after `since_ms` (an older one is the previous turn's)
fn last_turn(jsonl: &str, since_ms: i64) -> LastTurn {
  for line in jsonl.lines().rev() {
    let Ok(e) = serde_json::from_str::<Value>(line.trim()) else { continue };
    if e.get("type").and_then(Value::as_str) != Some("message") {
      continue;
    }
    let Some(m) = e.get("message").filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant")) else { continue };
    if m.get("timestamp").and_then(Value::as_i64).is_none_or(|t| t < since_ms) {
      return LastTurn::Pending;
    }
    if m.get("stopReason").and_then(Value::as_str) != Some("error") {
      return LastTurn::Ended(None);
    }
    let message = m.get("errorMessage").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    return LastTurn::Ended(Some(Failure {
      message: message.unwrap_or("pi request failed").to_owned(),
      code: None,
      retryable: None,
    }));
  }
  LastTurn::Pending
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A failed reply as pi 0.86.0 writes it (trimmed)
  const FAILED: &str = r#"{"type":"message","id":"2487d379","parentId":"60896e1f","timestamp":"2026-09-26T05:28:07.554Z","message":{"role":"assistant","content":[],"provider":"asgard","model":"claude-opus-5.5","stopReason":"error","timestamp":2000,"errorMessage":"502: {\"type\":\"windsurf_proxy_error\",\"message\":\"The socket connection was closed unexpectedly.\"}"}}"#;
  const USER: &str = r#"{"type":"message","id":"60896e1f","parentId":null,"message":{"role":"user","content":[{"type":"text","text":"hi"}],"timestamp":1990}}"#;

  fn log(lines: &[&str]) -> String {
    lines.join("\n") + "\n"
  }

  #[test]
  fn reads_the_error_of_the_current_reply() {
    let LastTurn::Ended(Some(f)) = last_turn(&log(&[USER, FAILED]), 1500) else { panic!("no failure") };
    assert!(f.message.starts_with("502: "), "{}", f.message);
    assert_eq!((f.code, f.retryable), (None, None));
  }

  #[test]
  fn an_older_reply_or_only_the_prompt_is_not_this_turn() {
    assert_eq!(last_turn(&log(&[FAILED]), 2500), LastTurn::Pending);
    assert_eq!(last_turn(&log(&[USER]), 1500), LastTurn::Pending);
  }

  #[test]
  fn a_reply_that_did_not_fail_has_no_failure() {
    let ok = r#"{"type":"message","message":{"role":"assistant","content":[],"stopReason":"stop","timestamp":3000}}"#;
    assert_eq!(last_turn(&log(&[FAILED, ok]), 2500), LastTurn::Ended(None));
  }

  #[test]
  fn a_failure_without_a_message_still_has_one() {
    let bare = r#"{"type":"message","message":{"role":"assistant","content":[],"stopReason":"error","timestamp":3000}}"#;
    assert_eq!(last_turn(&log(&[bare]), 2500), LastTurn::Ended(Some(Failure { message: "pi request failed".into(), code: None, retryable: None })));
  }
}
