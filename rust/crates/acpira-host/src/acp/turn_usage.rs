//! Per-prompt token accounting off the session/prompt response (mirror of src/host/acp/turnUsage.ts)

use serde_json::Value;

use acpira_shared::num::Num;
use acpira_shared::transcript::TurnUsage;

fn tokens(v: Option<&Value>) -> Option<Num> {
  v?.as_f64().filter(|n| n.is_finite() && *n >= 0.0).map(|n| Num((n + 0.5).floor()))
}

fn label(v: Option<&Value>) -> Option<String> {
  v?.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

pub fn turn_usage_of(r: &Value) -> Option<TurnUsage> {
  let mut out = TurnUsage::default();
  let fill = |slot: &mut Option<Num>, v: Option<&Value>| {
    if slot.is_none() {
      *slot = tokens(v);
    }
  };
  if let Some(u) = r.get("usage").filter(|u| u.is_object()) {
    fill(&mut out.input, u.get("inputTokens"));
    fill(&mut out.output, u.get("outputTokens"));
    fill(&mut out.total, u.get("totalTokens"));
    fill(&mut out.reasoning, u.get("thoughtTokens"));
    fill(&mut out.cached_read, u.get("cachedReadTokens"));
    fill(&mut out.cached_write, u.get("cachedWriteTokens"));
  }
  if let Some(meta) = r.get("_meta").filter(|m| m.is_object()) {
    fill(&mut out.input, meta.get("inputTokens"));
    fill(&mut out.output, meta.get("outputTokens"));
    fill(&mut out.total, meta.get("totalTokens"));
    fill(&mut out.cached_read, meta.get("cachedReadTokens"));
    fill(&mut out.reasoning, meta.get("reasoningTokens"));
    if let Some(inner) = meta.get("usage").filter(|u| u.is_object()) {
      fill(&mut out.cached_write, inner.get("cacheCreationTokens"));
      fill(&mut out.model_calls, inner.get("modelCalls"));
    }
    if out.model.is_none() {
      out.model = label(meta.get("modelId"));
    }
    if out.request_id.is_none() {
      out.request_id = label(meta.get("requestId")).or_else(|| label(meta.get("cognition.ai/userMessageId")));
    }
  }
  (out != TurnUsage::default()).then_some(out)
}
