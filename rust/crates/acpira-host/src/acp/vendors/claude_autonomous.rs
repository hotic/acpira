//! claude-agent-acp 0.84.0 (`dist/acp-agent.js`, `AUTONOMOUS_RESULT_ORIGINS`) runs "autonomous cycles" with no prompt on
//! the wire: a background Bash / task finishing after `end_turn` hands the CLI a task notification, the model answers it
//! on its own and its prose and tool calls stream as ordinary `session/update`s. The CLI's `session_state_changed`
//! running / idle is consumed inside the adapter and never forwarded, so the only end marker a client sees is the
//! cycle's result, relayed as a `usage_update` (sent whenever the cycle produced an assistant message). It carries
//! `cost`, and `_meta["_claude/origin"]` only when the result names an origin (`task-notification`, `peer`, …): a cycle a
//! scheduled wakeup started ended without one and stayed running. The other `usage_update`s (mid-stream context
//! snapshots, compaction, rate limits) carry neither

use serde_json::Value;

use crate::acp::vendors::steering::version_triple;

/// Updates that mean the model is working: one arriving with nothing on the wire opens an autonomous cycle. A bare
/// `tool_call_update` is left out, since a background task's own row can still settle after the turn
pub const CYCLE_START_KINDS: [&str; 4] = ["agent_message_chunk", "agent_thought_chunk", "tool_call", "plan"];

/// A result's `usage_update`: its `cost`, or its origin. With no prompt on the wire every result is the autonomous
/// cycle's, whatever its origin kind or none
pub fn cycle_ended(update: &Value) -> bool {
  update.get("sessionUpdate").and_then(Value::as_str) == Some("usage_update")
    && (update.get("cost").is_some_and(Value::is_object) || update.pointer("/_meta/_claude~1origin").is_some_and(Value::is_object))
}

/// A `session/prompt` sent during a cycle is answered by that cycle: from claude-agent-acp 0.95.0 a prompt the CLI folds
/// into an autonomous turn owns it, and the cycle's result settles the prompt (source read in 0.95.0 and 0.99.1). Earlier
/// adapters book that result as autonomous and absorb its idle, so the prompt never returns (seen on 0.84.0)
pub fn prompt_joins_cycle(init: &Value) -> bool {
  init.pointer("/agentInfo/version").and_then(Value::as_str).and_then(version_triple).is_some_and(|v| v >= (0, 95, 0))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn only_a_results_usage_update_ends_the_cycle() {
    let usage = |kind: &str| json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2, "_meta": { "_claude/origin": { "kind": kind } } });
    assert!(cycle_ended(&usage("task-notification")));
    assert!(cycle_ended(&usage("peer")));
    // a result with no origin (a scheduled wakeup's cycle) still carries its cost
    assert!(cycle_ended(&json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2, "cost": { "amount": 0.1, "currency": "USD" } })));
    // the mid-stream and rate-limit snapshots carry no origin
    assert!(!cycle_ended(&json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2 })));
    assert!(!cycle_ended(&json!({ "sessionUpdate": "usage_update", "used": 1, "size": 2, "_meta": { "_claude/rateLimit": {} } })));
    assert!(!cycle_ended(&json!({ "sessionUpdate": "agent_message_chunk", "_meta": { "_claude/origin": { "kind": "peer" } } })));
  }

  #[test]
  fn only_adapters_from_0_95_let_a_prompt_join_a_cycle() {
    let at = |v: &str| prompt_joins_cycle(&json!({ "agentInfo": { "version": v } }));
    assert!(!at("0.84.0"));
    assert!(!at("0.94.1-preview.13"));
    assert!(at("0.95.0"));
    assert!(at("0.99.1"));
    assert!(!prompt_joins_cycle(&json!({})));
  }
}
