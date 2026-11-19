//! Per-prompt token accounting off the session/prompt response, or Droid's local counters when ACP has none

use serde_json::Value;

use acpira_shared::num::Num;
use acpira_shared::transcript::TurnUsage;

use crate::acp::vendors::droid_usage::Baseline;

/// An actual wire tally always wins; mixing fields from different sources can pair a last-call count with a whole turn.
pub(crate) async fn prompt_usage_of(r: &Value, baseline: Option<Baseline>) -> Option<TurnUsage> {
  let wire = turn_usage_of(r);
  if r.get("stopReason").and_then(Value::as_str) != Some("end_turn")
    || wire.as_ref().is_some_and(|u| {
      u.output.or(u.input).or(u.total).or(u.reasoning).or(u.cached_read).or(u.cached_write).or(u.model_calls).or(u.decode_ms).is_some()
    })
  {
    return wire;
  }
  let Some(baseline) = baseline else { return wire };
  let Some(mut local) = baseline.usage().await else { return wire };
  if let Some(wire) = wire {
    local.model = wire.model;
    local.request_id = wire.request_id;
  }
  Some(local)
}

/// Adds one `session/prompt` answer's `usage` counts into another's: a turn that went on under a steer's own prompt
/// (`Vendor::steers_by_prompt`) has each part reported by the answer that closed it
pub(crate) fn add_wire_usage(into: &mut Value, from: &Value) {
  let Some(add) = from.get("usage").and_then(Value::as_object).filter(|u| !u.is_empty()) else { return };
  if !into.get("usage").is_some_and(Value::is_object) {
    into["usage"] = Value::Object(serde_json::Map::new());
  }
  let Some(sum) = into["usage"].as_object_mut() else { return };
  for (key, v) in add {
    let total = match (sum.get(key), v) {
      (None, v) => v.clone(),
      (Some(a), b) => match (a.as_u64(), b.as_u64()) {
        (Some(a), Some(b)) => Value::from(a + b),
        _ => match (a.as_f64(), b.as_f64()) {
          (Some(a), Some(b)) => Value::from(a + b),
          _ => continue,
        },
      },
    };
    sum.insert(key.clone(), total);
  }
}

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
      fill(&mut out.decode_ms, inner.get("decodeMs"));
    }
    if out.model.is_none() {
      out.model = label(meta.get("modelId"));
    }
    if out.request_id.is_none() {
      out.request_id = label(meta.get("requestId")).or_else(|| label(meta.get("cognition.ai/userMessageId")));
    }
  }
  out.fold_cache_into_input();
  (out != TurnUsage::default()).then_some(out)
}
