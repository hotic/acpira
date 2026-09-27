//! claude-agent-acp 0.81.0 runs Claude Code in stream-json mode, where no `--thinking-display` is passed unless the
//! session asks for one. Recent models (Opus 4.7+, Opus 5.5) then default the API's `thinking.display` to "omitted":
//! thinking blocks arrive signature-only, the adapter drops the empty text, and no `agent_thought_chunk` ever reaches
//! the host. `_meta.claudeCode.options.thinking` is spread into the SDK options, so asking for "summarized" there
//! brings the thought rows back. The SDK only forwards `display` together with `--thinking adaptive`, which is Claude
//! Code's own default, so the reasoning itself is unchanged.

use serde_json::{Value, json};

/// The SDK `thinking` option for a session, honouring the adapter's legacy `MAX_THINKING_TOKENS` knob: `0` keeps
/// thinking disabled (nothing is sent, the adapter's own mapping applies), a positive budget keeps the budget, anything
/// else falls back to adaptive
pub fn thinking_option(max_thinking_tokens: Option<&str>) -> Option<Value> {
  match max_thinking_tokens.map(|v| v.trim().parse::<i64>()) {
    Some(Ok(0)) => None,
    Some(Ok(n)) if n > 0 => Some(json!({ "type": "enabled", "budgetTokens": n, "display": "summarized" })),
    _ => Some(json!({ "type": "adaptive", "display": "summarized" })),
  }
}

/// Attach the thinking option to a session/new, resume or load request
pub fn with_thinking(mut req: Value, max_thinking_tokens: Option<&str>) -> Value {
  if let Some(thinking) = thinking_option(max_thinking_tokens) {
    req["_meta"] = json!({ "claudeCode": { "options": { "thinking": thinking } } });
  }
  req
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn unset_asks_for_adaptive_summaries() {
    let req = with_thinking(json!({ "cwd": "/w", "mcpServers": [] }), None);
    assert_eq!(req["_meta"]["claudeCode"]["options"]["thinking"], json!({ "type": "adaptive", "display": "summarized" }));
    assert_eq!(req["cwd"], "/w");
  }

  #[test]
  fn legacy_budget_is_respected() {
    assert_eq!(thinking_option(Some("0")), None);
    assert_eq!(thinking_option(Some("8000")), Some(json!({ "type": "enabled", "budgetTokens": 8000, "display": "summarized" })));
    assert_eq!(thinking_option(Some("junk")), Some(json!({ "type": "adaptive", "display": "summarized" })));
    assert!(with_thinking(json!({ "cwd": "/w" }), Some("0")).get("_meta").is_none());
  }
}
