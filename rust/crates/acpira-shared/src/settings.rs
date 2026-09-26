//! Settings view and sanitation (mirror of src/shared/settings.ts)

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::{Language, Locale};
use crate::transcript::AgentId;

pub type HiddenMap = BTreeMap<AgentId, BTreeMap<String, Vec<String>>>;

pub const UI_FONT_SIZE: (i64, i64, i64) = (10, 20, 13);
pub const CODE_FONT_SIZE: (i64, i64, i64) = (9, 20, 12);
pub const MIN_COMPACT_AT_TOKENS: i64 = 10_000;

pub const SETTING_KEYS: [&str; 15] = [
  "language",
  "defaultAgent",
  "agentOrder",
  "disabledAgents",
  "sessionScope",
  "sessionListPosition",
  "autoCompact",
  "compactAtTokens",
  "hiddenOptions",
  "accountSwitch",
  "theme",
  "uiFontSize",
  "codeFontSize",
  "diffMarkers",
  "fontSmoothing",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
  pub language: Language,
  pub locale: Locale,
  pub default_agent: AgentId,
  pub agent_order: Vec<AgentId>,
  pub disabled_agents: Vec<AgentId>,
  pub session_scope: String,
  pub session_list_position: String,
  pub auto_compact: bool,
  pub compact_at_tokens: i64,
  pub hidden_options: HiddenMap,
  /// Strategy of the automatic account switch, shared by every agent (`off` by default)
  pub account_switch: String,
  pub theme: String,
  pub ui_font_size: i64,
  pub code_font_size: i64,
  pub diff_markers: String,
  pub font_smoothing: bool,
}

pub const ACCOUNT_SWITCH_STRATEGIES: [&str; 4] = ["off", "earliestReset", "mostRemaining", "listOrder"];
pub const DEFAULT_ACCOUNT_SWITCH: &str = "off";

pub fn is_setting_key(k: &str) -> bool {
  SETTING_KEYS.contains(&k)
}

fn one_of(v: &Value, list: &[&str], fallback: &str) -> String {
  v.as_str().filter(|s| list.contains(s)).unwrap_or(fallback).to_owned()
}

fn clamp_size(v: &Value, (min, max, default): (i64, i64, i64)) -> i64 {
  match v.as_f64() {
    Some(n) if n.is_finite() => (js_round(n) as i64).clamp(min, max),
    _ => default,
  }
}

/// Math.round: halves go up
fn js_round(n: f64) -> f64 {
  (n + 0.5).floor()
}

/// Trimmed, non-empty, first occurrence wins; anything that is not an array reads as empty
pub fn id_list(v: &Value) -> Vec<String> {
  let mut out: Vec<String> = vec![];
  for s in v.as_array().into_iter().flatten().filter_map(Value::as_str) {
    let s = s.trim();
    if !s.is_empty() && !out.iter().any(|x| x == s) {
      out.push(s.to_owned());
    }
  }
  out
}

pub fn is_hidden_map(v: &Value) -> bool {
  let Some(m) = v.as_object() else { return false };
  m.values().all(|families| {
    families.as_object().is_some_and(|f| f.values().all(|names| names.as_array().is_some_and(|a| a.iter().all(Value::is_string))))
  })
}

/// A sanitized value per key, as JSON (the webview receives it inside SettingsView)
pub fn sanitize_setting(key: &str, value: &Value) -> Value {
  match key {
    "language" => Value::from(one_of(value, &["auto", "zh-CN", "en"], "auto")),
    "defaultAgent" => Value::from(value.as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("grok")),
    "autoCompact" => Value::from(value.as_bool().unwrap_or(true)),
    "fontSmoothing" => Value::from(value.as_bool().unwrap_or(false)),
    "compactAtTokens" => {
      let n = value.as_f64().filter(|n| n.is_finite()).map(|n| js_round(n) as i64).unwrap_or(300_000);
      Value::from(n.max(MIN_COMPACT_AT_TOKENS))
    }
    "uiFontSize" => Value::from(clamp_size(value, UI_FONT_SIZE)),
    "codeFontSize" => Value::from(clamp_size(value, CODE_FONT_SIZE)),
    "theme" => Value::from(one_of(value, &["auto", "light", "dark"], "auto")),
    "diffMarkers" => Value::from(one_of(value, &["color", "signs"], "color")),
    "sessionScope" => Value::from(one_of(value, &["workspace", "all"], "workspace")),
    "sessionListPosition" => Value::from(one_of(value, &["hidden", "left", "right"], "hidden")),
    "hiddenOptions" => {
      if is_hidden_map(value) {
        value.clone()
      } else {
        Value::Object(Default::default())
      }
    }
    "accountSwitch" => Value::from(one_of(value, &ACCOUNT_SWITCH_STRATEGIES, DEFAULT_ACCOUNT_SWITCH)),
    "agentOrder" | "disabledAgents" => Value::from(id_list(value)),
    _ => Value::Null,
  }
}

/// Whether a session belongs to the workspace shown
pub fn in_workspace(cwd: &str, workspace: &str) -> bool {
  cwd == workspace
}
