//! Claude Code's `ScheduleWakeup` (claude-agent-acp 0.84.0): a timer inside the CLI that, when it fires, starts an
//! autonomous cycle (`claude_autonomous`). Nothing reports a pending wakeup on the wire (no AIR `asyncTasks`, no
//! `task_started`), so the session would read as idle until then, and an idle release would kill the process and the
//! wakeup with it. The engine reads it off the tool's own row instead: the result names the delay ("Next wakeup scheduled
//! for 02:22:00 (in 1860s)"), the input's `delaySeconds` stands in when it does not, and `stop: true` cancels

use std::collections::HashMap;

use serde_json::Value;

/// The tool's name in `_meta.claudeCode.toolName`
pub const TOOL: &str = "ScheduleWakeup";
/// How long past its time a wakeup still counts as pending: the CLI fires it on its own clock
pub const GRACE_MS: i64 = 120_000;
/// How early a cycle may start and still be the wakeup: the timer runs on this machine's clock, so only rounding.
/// A wider window would take another task's cycle shortly before the time for the wakeup and drop the keep-alive
const EARLY_MS: i64 = 5_000;
/// The CLI clamps the requested delay to this range (the tool's own schema)
const DELAY_RANGE: (u64, u64) = (60, 3600);

/// The session's pending wakeup, and the inputs of the wakeup calls still in flight
#[derive(Default)]
pub struct Wake {
  /// When the CLI will resume the session (ms since the epoch)
  pub at: Option<i64>,
  inputs: HashMap<String, Input>,
}

#[derive(Default, Clone, Copy)]
struct Input {
  delay: Option<u64>,
  stop: bool,
}

impl Wake {
  /// Still pending at `now`: set, and not so far past its time that it evidently will not fire
  pub fn pending(&self, now: i64) -> Option<i64> {
    self.at.filter(|at| at + GRACE_MS > now)
  }

  /// One packet of a wakeup call's row. Returns true when the pending wakeup changed
  pub fn observe(&mut self, u: &Value, now: i64) -> bool {
    let Some(id) = u.get("toolCallId").and_then(Value::as_str) else { return false };
    if let Some(raw) = u.get("rawInput").and_then(Value::as_object).filter(|r| !r.is_empty()) {
      let input = self.inputs.entry(id.to_owned()).or_default();
      if let Some(d) = raw.get("delaySeconds").and_then(Value::as_f64) {
        input.delay = Some((d.max(0.0) as u64).clamp(DELAY_RANGE.0, DELAY_RANGE.1));
      }
      input.stop = raw.get("stop").and_then(Value::as_bool).unwrap_or(false);
    }
    match u.get("status").and_then(Value::as_str) {
      Some("completed") => {}
      Some("failed") => {
        self.inputs.remove(id);
        return false;
      }
      _ => return false,
    }
    let input = self.inputs.remove(id).unwrap_or_default();
    let before = self.at;
    self.at = if input.stop {
      None
    } else {
      delay_in_result(u).or(input.delay).map(|s| now + (s as i64) * 1000)
    };
    self.at != before
  }

  /// A cycle started: at or past its time, it is the wakeup firing
  pub fn fired(&mut self, now: i64) -> bool {
    let fired = self.at.is_some_and(|at| now + EARLY_MS >= at);
    if fired {
      self.at = None;
    }
    fired
  }

  /// The process went away, and its timer with it
  pub fn clear(&mut self) {
    self.at = None;
    self.inputs.clear();
  }
}

/// "(in 1860s)" in the result's text, wherever the adapter put it (`rawOutput` as a string or content blocks, `content`)
fn delay_in_result(u: &Value) -> Option<u64> {
  let mut texts = Vec::new();
  collect_text(u.get("rawOutput"), &mut texts);
  collect_text(u.get("content"), &mut texts);
  texts.iter().find_map(|t| {
    let (_, rest) = t.split_once("(in ")?;
    let (digits, tail) = rest.split_once('s')?;
    tail.starts_with(')').then(|| digits.parse().ok()).flatten()
  })
}

fn collect_text<'a>(v: Option<&'a Value>, out: &mut Vec<&'a str>) {
  match v {
    Some(Value::String(s)) => out.push(s),
    Some(Value::Array(items)) => items.iter().for_each(|i| collect_text(Some(i), out)),
    Some(Value::Object(o)) => {
      collect_text(o.get("text"), out);
      collect_text(o.get("content"), out);
    }
    _ => {}
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  const NOW: i64 = 1_000_000;

  #[test]
  fn the_result_names_the_delay_and_the_input_stands_in() {
    let mut w = Wake::default();
    assert!(!w.observe(&json!({ "toolCallId": "t1", "rawInput": { "delaySeconds": 1800, "prompt": "go" } }), NOW));
    let done = json!({ "toolCallId": "t1", "status": "completed", "rawOutput": [{ "type": "text", "text": "Next wakeup scheduled for 02:22:00 (in 1860s). If you owe…" }] });
    assert!(w.observe(&done, NOW));
    assert_eq!(w.at, Some(NOW + 1_860_000));
    // No delay in the result: the clamped input
    let mut w = Wake::default();
    w.observe(&json!({ "toolCallId": "t2", "rawInput": { "delaySeconds": 5 } }), NOW);
    w.observe(&json!({ "toolCallId": "t2", "status": "completed", "rawOutput": "scheduled" }), NOW);
    assert_eq!(w.at, Some(NOW + 60_000));
  }

  #[test]
  fn stop_failure_firing_and_expiry_end_it() {
    let mut w = Wake { at: Some(NOW + 600_000), ..Wake::default() };
    w.observe(&json!({ "toolCallId": "t3", "rawInput": { "stop": true } }), NOW);
    assert!(w.observe(&json!({ "toolCallId": "t3", "status": "completed", "rawOutput": "Loop stopped" }), NOW));
    assert_eq!(w.at, None);
    let mut w = Wake { at: Some(NOW + 600_000), ..Wake::default() };
    w.observe(&json!({ "toolCallId": "t4", "rawInput": { "delaySeconds": 900 } }), NOW);
    assert!(!w.observe(&json!({ "toolCallId": "t4", "status": "failed" }), NOW));
    assert_eq!(w.at, Some(NOW + 600_000));
    // A cycle long before the time is something else (a background task's notification); at the time it is the wakeup
    assert!(!w.fired(NOW));
    assert!(!w.fired(NOW + 600_000 - 60_000));
    assert!(w.fired(NOW + 600_000));
    let w = Wake { at: Some(NOW), ..Wake::default() };
    assert_eq!(w.pending(NOW + GRACE_MS - 1), Some(NOW));
    assert_eq!(w.pending(NOW + GRACE_MS), None);
  }
}
