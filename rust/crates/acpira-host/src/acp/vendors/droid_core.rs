//! Droid's overage swap. With the account's overage preference on Droid Core, droid 1.14.0 moves the session's model to
//! a Droid Core one by itself once Standard usage is used up (at the prompt's start, or after a 402) and only says so in
//! a system message that never reaches ACP: the wire shows nothing but a `config_option_update` naming the new model.
//! The host refuses that swap: the turn stops with the exhaustion as its error and the picked model is set back.

use acpira_shared::transcript::{AccountQuota, SessionControls, TurnError};
use chrono::{DateTime, Local};
use serde_json::Value;

use crate::i18n::tp;
use crate::util::ms_of_iso;

/// Models droid 1.14.0 bills to the Droid Core pool (`billingPool: "core"` in its model table). Only a move from another
/// model onto one of these is the overage swap; a model the user picked from this list is never swapped
const CORE_POOL: &[&str] = &[
  "beam-1.0",
  "deepseek-v4-flash-0731",
  "deepseek-v4-pro",
  "deepseek-v4.1-flash",
  "deepseek-v4.1-flash-1017",
  "glm-4.6",
  "glm-4.7",
  "glm-5",
  "glm-5.1",
  "glm-5.2",
  "glm-5.2-fast",
  "glm-5.3",
  "glm-5.3-flash",
  "kimi-k2.5",
  "kimi-k2.6",
  "kimi-k2.7-code",
  "kimi-k3",
  "kimi-k3.1-pro",
  "minimax-m2.5",
  "minimax-m2.7",
  "minimax-m3",
  "minimax-m3.1-flash",
  "mistral-large-4",
  "mistral-medium-3.5",
  "nemotron-3-ultra",
  "nemotron-3.5-super",
  "qwen3.8-max",
];

pub(crate) fn is_core(model: &str) -> bool {
  CORE_POOL.contains(&model)
}

/// A model change droid made on its own, from the picked model onto a Droid Core one: (id, name) of both
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Swap {
  pub from: (String, String),
  pub to: (String, String),
}

/// The session's swap guard: the model requests the host has in flight (their echo is the user's own pick) and the
/// swap a running turn met, settled as that turn's error
#[derive(Default)]
pub(crate) struct Guard {
  pub own_picks: u32,
  pub blocked: Option<Swap>,
}

/// The overage swap in a `config_option_update`, judged against the controls on screen before it applies
pub(crate) fn swap_of(controls: &SessionControls, update: &Value) -> Option<Swap> {
  let model = controls.options.iter().find(|o| o.category.as_deref() == Some("model"))?;
  let from = model.value.as_deref()?;
  let to = update
    .get("configOptions")?
    .as_array()?
    .iter()
    .find(|o| o.get("id").and_then(Value::as_str) == Some(model.id.as_str()))?
    .get("currentValue")?
    .as_str()?;
  if from == to || is_core(from) || !is_core(to) {
    return None;
  }
  let name = |id: &str| model.options.iter().find(|o| o.id == id).map_or_else(|| id.to_owned(), |o| o.name.clone());
  Some(Swap { from: (from.to_owned(), name(from)), to: (to.to_owned(), name(to)) })
}

/// When Standard usage comes back: the last reset among the used-up windows (each must reset before a request goes
/// through), None when the quota shows none used up or names no reset
pub(crate) fn reset_at(quota: &AccountQuota) -> Option<i64> {
  quota.windows.iter().filter(|w| w.remaining.0 <= 0.0).filter_map(|w| w.resets_at.as_deref().and_then(ms_of_iso)).max()
}

/// The turn's error: which model ran out and, when the account's quota tells, when it comes back. The kind is the one
/// the account switch reads as quota exhaustion, so a configured fallback account carries the task on the same model
pub(crate) fn turn_error(swap: &Swap, reset: Option<i64>) -> TurnError {
  let model = swap.from.1.as_str();
  let core = swap.to.1.as_str();
  let message = match reset.and_then(DateTime::from_timestamp_millis) {
    Some(at) => {
      let at = at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string();
      tp("host.droidStandardExhaustedUntil", &[("model", model), ("core", core), ("at", &at)])
    }
    None => tp("host.droidStandardExhausted", &[("model", model), ("core", core)]),
  };
  TurnError { message, kind: Some("resource_exhausted".into()), retryable: Some(false), ..Default::default() }
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::num::Num;
  use acpira_shared::transcript::{ConfigControl, QuotaWindow, SessionOption};
  use serde_json::json;

  fn controls(value: &str) -> SessionControls {
    let opt = |id: &str, name: &str| SessionOption { id: id.into(), name: name.into(), ..Default::default() };
    SessionControls {
      options: vec![ConfigControl {
        id: "model".into(),
        name: "Model".into(),
        category: Some("model".into()),
        options: vec![opt("claude-opus-5-5", "Opus 5.5"), opt("kimi-k3", "Kimi K3"), opt("glm-5.3", "GLM-5.3")],
        value: Some(value.into()),
        ..Default::default()
      }],
      ..Default::default()
    }
  }

  fn update(model: &str) -> Value {
    json!({ "sessionUpdate": "config_option_update", "configOptions": [
      { "id": "autonomy_level", "currentValue": "auto-medium" },
      { "id": "model", "currentValue": model },
    ] })
  }

  #[test]
  fn only_a_move_onto_the_core_pool_is_a_swap() {
    let swap = swap_of(&controls("claude-opus-5-5"), &update("kimi-k3")).unwrap();
    assert_eq!(swap.from, ("claude-opus-5-5".into(), "Opus 5.5".into()));
    assert_eq!(swap.to, ("kimi-k3".into(), "Kimi K3".into()));
    // The same model, a move between core models, and droid switching back to Standard after the reset are no swap
    assert_eq!(swap_of(&controls("claude-opus-5-5"), &update("claude-opus-5-5")), None);
    assert_eq!(swap_of(&controls("glm-5.3"), &update("kimi-k3")), None);
    assert_eq!(swap_of(&controls("kimi-k3"), &update("claude-opus-5-5")), None);
    // An update without the model select leaves it alone
    assert_eq!(swap_of(&controls("claude-opus-5-5"), &json!({ "configOptions": [{ "id": "autonomy_level", "currentValue": "spec" }] })), None);
  }

  #[test]
  fn standard_returns_when_the_last_used_up_window_resets() {
    let w = |id: &str, remaining: f64, at: Option<&str>| QuotaWindow { id: id.into(), remaining: Num(remaining), resets_at: at.map(str::to_owned) };
    let quota = |windows| AccountQuota { windows, on_demand_balance_usd: None, fetched_at: String::new() };
    let q = quota(vec![
      w("5h", 0.0, Some("2026-11-20T10:00:00.000Z")),
      w("weekly", 0.0, Some("2026-11-23T00:00:00.000Z")),
      w("monthly", 0.4, Some("2026-12-01T00:00:00.000Z")),
    ]);
    assert_eq!(reset_at(&q), ms_of_iso("2026-11-23T00:00:00.000Z"));
    assert_eq!(reset_at(&quota(vec![w("5h", 0.2, Some("2026-11-20T10:00:00.000Z"))])), None);
  }

  #[test]
  fn the_error_names_the_model_and_reads_as_exhaustion() {
    let swap = Swap { from: ("claude-opus-5-5".into(), "Opus 5.5".into()), to: ("kimi-k3".into(), "Kimi K3".into()) };
    let e = turn_error(&swap, None);
    assert!(e.message.contains("Opus 5.5") && e.message.contains("Kimi K3"), "{}", e.message);
    assert!(crate::acp::session::errors::is_quota_exhausted(&e));
    let at = ms_of_iso("2026-11-23T00:00:00.000Z");
    assert_ne!(turn_error(&swap, at).message, e.message);
  }
}
