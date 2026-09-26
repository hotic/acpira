//! The official model catalogue (models.dev trimmed to first-party vendors) and the reasoning levels it allows. Identity is
//! resolved from opaque option ids / names by exact normalized match only; the catalogue narrows which effort options are
//! displayed, the remaining option ids stay the agent's wire values

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::composer_controls::{compact, is_reasoning_control};
use crate::model_shapes::is_model_control;
use crate::transcript::{ConfigControl, SessionOption, SourceKind};

pub const MODELS_DEV_URL: &str = "https://models.dev/api.json";

/// First-party vendors in priority order: an id listed by several keeps the first, so resellers (alibaba) come last
pub const VENDORS: &[&str] =
  &["anthropic", "openai", "google", "xai", "deepseek", "moonshotai", "zhipuai", "mistral", "minimax", "alibaba"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
  Off,
  Minimal,
  Low,
  Medium,
  High,
  XHigh,
  Max,
}

/// A reasoning level word (`none` counts as off); None for labels outside the scale (`on`, `ultra`, `standard`, …)
pub fn parse_level(value: &str) -> Option<Level> {
  let key: String = compact(value).chars().filter(char::is_ascii_alphanumeric).collect();
  Some(match key.as_str() {
    "off" | "none" => Level::Off,
    "minimal" => Level::Minimal,
    "low" => Level::Low,
    "medium" => Level::Medium,
    "high" => Level::High,
    "xhigh" | "extrahigh" => Level::XHigh,
    "max" | "maximum" => Level::Max,
    _ => return None,
  })
}

/// An effort option's level from its id, else its name (pi-acp: id `off`, name `Thinking: off`)
pub fn option_level(option: &SessionOption) -> Option<Level> {
  parse_level(&option.id).or_else(|| parse_level(&option.name))
}

static DATED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-(?:\d{8}|\d{4}-\d{2}-\d{2})$").unwrap());
static DASHES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-{2,}").unwrap());

/// Identity key: the part after the last `/`, lowercase, `_` / space / `.` as `-`, without a date or `-latest` suffix
pub fn model_key(value: &str) -> String {
  let tail = value.rsplit('/').next().unwrap_or(value).trim().to_lowercase();
  let dashed: String = tail.chars().map(|c| if matches!(c, '_' | '.') || c.is_whitespace() { '-' } else { c }).collect();
  let dashed = DASHES.replace_all(&dashed, "-");
  let key = dashed.trim_matches('-');
  let key = key.strip_suffix("-latest").unwrap_or(key);
  DATED.replace(key, "").into_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
  pub provider: String,
  pub id: String,
  pub name: String,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub reasoning: bool,
  /// models.dev `reasoning_options[type=effort].values`, vendor spelling
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub efforts: Vec<String>,
  /// `reasoning_options[type=toggle]`: thinking can be switched off
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub toggle: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub context: Option<u64>,
}

impl CatalogModel {
  /// The documented levels; None when the vendor lists no effort values (toggle / budget only, or nothing)
  pub fn levels(&self) -> Option<Vec<Level>> {
    let mut out: Vec<Level> = self.efforts.iter().filter_map(|e| parse_level(e)).collect();
    if out.is_empty() {
      return None;
    }
    if self.toggle && !out.contains(&Level::Off) {
      out.push(Level::Off);
    }
    out.sort();
    out.dedup();
    Some(out)
  }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogFile {
  pub source: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub etag: Option<String>,
  pub fetched_at: String,
  pub models: Vec<CatalogModel>,
}

impl CatalogFile {
  /// One model per line, so a snapshot refresh reviews as a line diff
  pub fn to_json(&self) -> String {
    let head = serde_json::json!({ "source": self.source, "etag": self.etag, "fetchedAt": self.fetched_at });
    let head = head.to_string();
    let lines: Vec<String> = self.models.iter().map(|m| serde_json::to_string(m).unwrap_or_default()).collect();
    format!("{},\"models\":[\n{}\n]}}\n", &head[..head.len() - 1], lines.join(",\n"))
  }
}

/// The catalogue subset of models.dev's `api.json`: first-party vendors, reasoning options, context window
pub fn trim_models_dev(api: &Value, fetched_at: &str, etag: Option<String>) -> CatalogFile {
  let mut models = vec![];
  for vendor in VENDORS {
    let Some(list) = api.get(vendor).and_then(|p| p.get("models")).and_then(Value::as_object) else { continue };
    let mut ids: Vec<&String> = list.keys().collect();
    ids.sort();
    for id in ids {
      let m = &list[id];
      let options: Vec<&Value> = m.get("reasoning_options").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
      let of = |t: &'static str| options.iter().filter(move |o| o.get("type").and_then(Value::as_str) == Some(t));
      let efforts = of("effort")
        .flat_map(|o| o.get("values").and_then(Value::as_array).into_iter().flatten())
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
      models.push(CatalogModel {
        provider: (*vendor).to_owned(),
        id: id.clone(),
        name: m.get("name").and_then(Value::as_str).unwrap_or(id).to_owned(),
        reasoning: m.get("reasoning") == Some(&Value::Bool(true)),
        efforts,
        toggle: of("toggle").next().is_some(),
        context: m.get("limit").and_then(|l| l.get("context")).and_then(Value::as_f64).filter(|c| *c >= 1.0).map(|c| c as u64),
      });
    }
  }
  CatalogFile { source: MODELS_DEV_URL.into(), etag, fetched_at: fetched_at.into(), models }
}

#[derive(Debug, Clone, Default)]
pub struct Catalog {
  pub file: CatalogFile,
  index: HashMap<String, usize>,
}

impl Catalog {
  pub fn new(file: CatalogFile) -> Catalog {
    let mut index = HashMap::new();
    for (i, m) in file.models.iter().enumerate() {
      index.entry(model_key(&m.id)).or_insert(i);
    }
    Catalog { file, index }
  }

  pub fn get(&self, key: &str) -> Option<&CatalogModel> {
    self.index.get(key).map(|&i| &self.file.models[i])
  }

  /// The catalogue entry an option stands for: its id first, then its display name
  pub fn resolve(&self, option: &SessionOption) -> Option<&CatalogModel> {
    self.get(&model_key(&option.id)).or_else(|| self.get(&model_key(&option.name)))
  }
}

/// Levels an agent's own model data allows beyond what its adapter advertises (Pi: `thinkingLevelMap`), by model option id
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NativeLevels {
  pub levels: BTreeMap<String, Vec<Level>>,
  /// Official models missing from `levels` are narrowed by the catalogue too: the adapter's list is not the agent's own
  pub catalog_fallback: bool,
}

/// Narrow the reasoning selects to what the current model really takes: the agent's own levels when it has them, and the
/// catalogue's for a custom-source model (or an unlisted one under `catalog_fallback`). Labels outside the scale stay;
/// a narrowing that would leave no level on the scale is skipped, and the current value is left for `thought_correction`
pub fn clip_efforts(controls: &mut [ConfigControl], catalog: &Catalog, native: &NativeLevels) {
  let Some(option) = controls
    .iter()
    .find(|c| is_model_control(c))
    .and_then(|m| m.value.as_ref().and_then(|v| m.options.iter().find(|o| &o.id == v)))
    .cloned()
  else {
    return;
  };
  let own = native.levels.get(&option.id);
  let custom = option.source.as_ref().is_some_and(|s| s.kind == SourceKind::Custom);
  let official =
    if custom || (own.is_none() && native.catalog_fallback) { catalog.resolve(&option).and_then(CatalogModel::levels) } else { None };
  if own.is_none() && official.is_none() {
    return;
  }
  let allowed = |l: Level| own.is_none_or(|o| o.contains(&l)) && official.as_ref().is_none_or(|o| o.contains(&l));
  for control in controls.iter_mut().filter(|c| is_reasoning_control(c)) {
    let kept: Vec<SessionOption> = control.options.iter().filter(|o| option_level(o).is_none_or(allowed)).cloned().collect();
    if kept.len() < control.options.len() && kept.iter().any(|o| option_level(o).is_some()) {
      control.options = kept;
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::transcript::OptionSource;
  use serde_json::json;

  fn catalog() -> Catalog {
    let api = json!({
      "anthropic": { "models": {
        "claude-opus-5-5": { "name": "Claude Opus 5.5", "reasoning": true, "limit": { "context": 1000000 },
          "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high", "xhigh", "max"] }] },
        "claude-opus-4-6": { "name": "Claude Opus 4.6", "reasoning": true,
          "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high", "max"] }, { "type": "budget_tokens", "min": 1024 }] } } },
      "openai": { "models": {
        "gpt-6-sol": { "name": "GPT-6 Sol", "reasoning": true,
          "reasoning_options": [{ "type": "effort", "values": ["none", "low", "medium", "high", "xhigh", "max"] }] } } },
      "google": { "models": {
        "gemini-3.8-flash": { "name": "Gemini 3.8 Flash", "reasoning": true,
          "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high"] }] } } },
      "moonshotai": { "models": {
        "kimi-k3": { "name": "Kimi K3", "reasoning": true,
          "reasoning_options": [{ "type": "toggle" }, { "type": "effort", "values": ["low", "high", "max"] }] } } },
      "alibaba": { "models": {
        "kimi-k3": { "name": "Kimi K3", "reasoning": true, "reasoning_options": [{ "type": "effort", "values": ["low", "high", "max"] }] } } },
      "deepseek": { "models": {
        "deepseek-v4-flash": { "name": "DeepSeek V4 Flash", "reasoning": true,
          "reasoning_options": [{ "type": "toggle" }, { "type": "effort", "values": ["low", "high", "max"] }] } } },
      "openrouter": { "models": { "gemini-3.8-flash": { "name": "x", "reasoning_options": [{ "type": "effort", "values": ["max"] }] } } }
    });
    Catalog::new(trim_models_dev(&api, "2026-09-26T00:00:00Z", None))
  }

  fn opt(id: &str, name: &str) -> SessionOption {
    SessionOption { id: id.into(), name: name.into(), ..Default::default() }
  }

  fn sourced(id: &str, name: &str, kind: SourceKind) -> SessionOption {
    SessionOption { source: Some(OptionSource { id: "asgard".into(), name: "asgard".into(), kind }), ..opt(id, name) }
  }

  /// pi-acp 0.0.33's controls: the model select plus its fixed `off…xhigh` thought_level select
  fn pi_controls(model: SessionOption) -> Vec<ConfigControl> {
    let efforts = ["off", "minimal", "low", "medium", "high", "xhigh"].map(|l| opt(l, &format!("Thinking: {l}")));
    vec![
      ConfigControl {
        id: "model".into(),
        name: "Model".into(),
        category: Some("model".into()),
        kind: None,
        value: Some(model.id.clone()),
        options: vec![model],
      },
      ConfigControl {
        id: "thought_level".into(),
        name: "Thinking".into(),
        category: Some("thought_level".into()),
        kind: None,
        value: Some("medium".into()),
        options: efforts.to_vec(),
      },
    ]
  }

  fn efforts(controls: &[ConfigControl]) -> Vec<&str> {
    controls[1].options.iter().map(|o| o.id.as_str()).collect()
  }

  const PI_OFF_TO_HIGH: [Level; 5] = [Level::Off, Level::Minimal, Level::Low, Level::Medium, Level::High];

  #[test]
  fn keys_normalize_vendor_spelling() {
    for (raw, key) in [
      ("asgard/claude-opus-5.5", "claude-opus-5-5"),
      ("asgard/Claude Opus 5.5", "claude-opus-5-5"),
      ("claude-opus-4-6-20260115", "claude-opus-4-6"),
      ("gemini-3.8-flash-latest", "gemini-3-8-flash"),
      ("Gemini 3.8 Flash", "gemini-3-8-flash"),
      ("GPT_6  Sol", "gpt-6-sol"),
      ("openrouter/moonshotai/kimi-k3", "kimi-k3"),
    ] {
      assert_eq!(model_key(raw), key, "{raw}");
    }
  }

  #[test]
  fn levels_read_ids_then_names() {
    assert_eq!(option_level(&opt("off", "Thinking: off")), Some(Level::Off));
    assert_eq!(option_level(&opt("xhigh", "Extra High")), Some(Level::XHigh));
    assert_eq!(option_level(&opt("effort-2", "Extra High")), Some(Level::XHigh));
    assert_eq!(option_level(&opt("low", "Thinking Low")), Some(Level::Low));
    assert_eq!(option_level(&opt("on", "Thinking")), None);
    assert_eq!(option_level(&opt("ultra", "Ultra")), None);
  }

  #[test]
  fn resolve_takes_first_party_vendor_and_exact_keys_only() {
    let c = catalog();
    assert_eq!(c.resolve(&opt("asgard/kimi-k3", "asgard/Kimi K3")).map(|m| m.provider.as_str()), Some("moonshotai"));
    assert_eq!(c.resolve(&opt("opus", "claude-opus-5.5")).map(|m| m.id.as_str()), Some("claude-opus-5-5"));
    assert_eq!(c.resolve(&opt("asgard", "gemini-3.8-flash")).map(|m| m.provider.as_str()), Some("google"));
    assert!(c.resolve(&opt("asgard/deepseek-v4.1-flash", "asgard/DeepSeek V4.1 Flash")).is_none());
    assert!(c.resolve(&opt("default", "Default (recommended)")).is_none());
  }

  #[test]
  fn catalogue_levels_add_off_only_for_toggle_or_none() {
    let c = catalog();
    assert_eq!(c.get("claude-opus-5-5").unwrap().levels().unwrap(), [Level::Low, Level::Medium, Level::High, Level::XHigh, Level::Max]);
    assert_eq!(c.get("kimi-k3").unwrap().levels().unwrap(), [Level::Off, Level::Low, Level::High, Level::Max]);
    assert_eq!(c.get("gpt-6-sol").unwrap().levels().unwrap()[0], Level::Off);
  }

  #[test]
  fn pi_custom_models_meet_pi_and_catalogue_levels() {
    let c = catalog();
    let mut native = NativeLevels { catalog_fallback: true, ..Default::default() };
    for id in ["asgard/claude-opus-5.5", "asgard/gemini-3.8-flash", "asgard/kimi-k3", "asgard/deepseek-v4.1-flash"] {
      native.levels.insert(id.into(), PI_OFF_TO_HIGH.to_vec());
    }
    let cases = [
      ("asgard/claude-opus-5.5", "asgard/Claude Opus 5.5", vec!["low", "medium", "high"]),
      ("asgard/gemini-3.8-flash", "asgard/Gemini 3.8 Flash", vec!["low", "medium", "high"]),
      ("asgard/kimi-k3", "asgard/Kimi K3", vec!["off", "low", "high"]),
      // Not in the catalogue: pi's own levels alone (pi would clamp xhigh down to high)
      ("asgard/deepseek-v4.1-flash", "asgard/DeepSeek V4.1 Flash", vec!["off", "minimal", "low", "medium", "high"]),
    ];
    for (id, name, want) in cases {
      let mut controls = pi_controls(sourced(id, name, SourceKind::Custom));
      clip_efforts(&mut controls, &c, &native);
      assert_eq!(efforts(&controls), want, "{id}");
      assert_eq!(controls[1].value.as_deref(), Some("medium"), "the value is left for thought_correction");
    }
  }

  #[test]
  fn pi_official_models_trust_pi_then_the_catalogue() {
    let c = catalog();
    let mut native = NativeLevels { catalog_fallback: true, ..Default::default() };
    native
      .levels
      .insert("anthropic/claude-opus-5".into(), vec![Level::Minimal, Level::Low, Level::Medium, Level::High, Level::XHigh, Level::Max]);
    let mut controls = pi_controls(sourced("anthropic/claude-opus-5", "anthropic/Claude Opus 5", SourceKind::Official));
    clip_efforts(&mut controls, &c, &native);
    assert_eq!(efforts(&controls), ["minimal", "low", "medium", "high", "xhigh"]);
    // Unlisted in pi's data (a bundled provider): the catalogue stands in
    let mut controls = pi_controls(sourced("google/gemini-3.8-flash", "google/Gemini 3.8 Flash", SourceKind::Official));
    clip_efforts(&mut controls, &c, &native);
    assert_eq!(efforts(&controls), ["low", "medium", "high"]);
  }

  #[test]
  fn official_and_unsourced_options_stay_untouched_without_native_data() {
    let c = catalog();
    for option in
      [sourced("gemini-3.8-flash", "Gemini 3.8 Flash", SourceKind::Official), opt("gemini-3-8-flash-medium", "Gemini 3.8 Flash")]
    {
      let mut controls = pi_controls(option);
      clip_efforts(&mut controls, &c, &NativeLevels::default());
      assert_eq!(efforts(&controls).len(), 6);
    }
  }

  #[test]
  fn off_scale_labels_survive_and_empty_narrowing_is_skipped() {
    let c = catalog();
    let mut controls = pi_controls(sourced("asgard/kimi-k3", "K3", SourceKind::Custom));
    controls[1].options = vec![opt("on", "Thinking"), opt("medium", "Medium"), opt("ultra", "Ultra")];
    let before = controls.clone();
    clip_efforts(&mut controls, &c, &NativeLevels::default());
    assert_eq!(controls, before, "only medium is on the scale and kimi-k3 has no medium");
    controls[1].options.push(opt("high", "High"));
    clip_efforts(&mut controls, &c, &NativeLevels::default());
    assert_eq!(efforts(&controls), ["on", "ultra", "high"]);
  }

  #[test]
  fn snapshot_json_round_trips_one_model_per_line() {
    let file = catalog().file;
    let text = file.to_json();
    assert_eq!(text.lines().count(), file.models.len() + 2);
    assert_eq!(serde_json::from_str::<CatalogFile>(&text).unwrap(), file);
  }
}
