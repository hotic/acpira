//! Model provenance and native reasoning levels read off each agent's own config files. Only source labels and levels leave
//! this module; endpoint credentials stay in the CLI config
//!   Grok      `[model.<alias>]` with a `base_url` in ~/.grok/config.toml / <cwd>/.grok/config.toml
//!   Pi        providers in models.json with a `baseUrl` or custom `models`; levels from pi's model definitions (below)
//!   OpenCode  `provider.<id>` with `options.baseURL` or an `npm` package in opencode.json(c), global then project
//!   Codex     `model_provider` other than openai in $CODEX_HOME/config.toml: every model goes through it
//!   Claude    `ANTHROPIC_BASE_URL` off api.anthropic.com (agent / process env, then settings.json `env`)

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use serde_json::Value;

use acpira_shared::model_catalog::{Level, NativeLevels, clip_efforts};
use acpira_shared::model_sources::{ALL, ModelSources, apply_model_sources};
use acpira_shared::transcript::{ConfigControl, OptionSource, SourceKind};

use super::agent_registry::AgentDef;
use super::pi_usage;
use crate::inventory::parse_json_loose;
use crate::store::data_dir::home_dir;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelFacts {
  pub sources: ModelSources,
  pub native: NativeLevels,
}

fn custom(id: &str, name: &str) -> OptionSource {
  OptionSource { id: id.to_owned(), name: name.to_owned(), kind: SourceKind::Custom }
}

fn read_json(path: &Path) -> Option<Value> {
  parse_json_loose(&std::fs::read_to_string(path).ok()?)
}

pub fn grok_model_sources(text: &str) -> ModelSources {
  let mut sources = ModelSources::new();
  let Ok(config) = text.parse::<toml::Table>() else { return sources };
  let Some(toml::Value::Table(models)) = config.get("model") else { return sources };
  for (alias, value) in models {
    let toml::Value::Table(v) = value else { continue };
    // A context-window override alone does not turn a built-in model into a custom endpoint
    if let Some(toml::Value::String(url)) = v.get("base_url")
      && !url.trim().is_empty()
    {
      sources.insert(alias.clone(), custom(alias, alias));
    }
  }
  sources
}

/// pi-ai 0.86.0 `getSupportedThinkingLevels`: nothing but off without `reasoning`; a `null` mapping disables a level, and
/// xhigh / max exist only when mapped
fn pi_levels(model: &Value) -> Vec<Level> {
  if model.get("reasoning") != Some(&Value::Bool(true)) {
    return vec![Level::Off];
  }
  let map = model.get("thinkingLevelMap");
  [
    ("off", Level::Off),
    ("minimal", Level::Minimal),
    ("low", Level::Low),
    ("medium", Level::Medium),
    ("high", Level::High),
    ("xhigh", Level::XHigh),
    ("max", Level::Max),
  ]
  .into_iter()
  .filter(|(key, level)| match map.and_then(|m| m.get(key)) {
    Some(Value::Null) => false,
    Some(_) => true,
    None => !matches!(level, Level::XHigh | Level::Max),
  })
  .map(|(_, level)| level)
  .collect()
}

/// pi 0.86.0's `applyModelOverride`: `reasoning` replaces, `thinkingLevelMap` merges key by key
fn pi_override(model: &mut Value, patch: &Value) {
  if let Some(r) = patch.get("reasoning").filter(|r| r.is_boolean()) {
    model["reasoning"] = r.clone();
  }
  if let Some(Value::Object(map)) = patch.get("thinkingLevelMap") {
    if !model.get("thinkingLevelMap").is_some_and(Value::is_object) {
      model["thinkingLevelMap"] = Value::Object(Default::default());
    }
    for (k, v) in map {
      model["thinkingLevelMap"][k] = v.clone();
    }
  }
}

/// Pi's model definitions by `provider/id`: the fetched built-in catalogue (models-store.json), then models.json's custom
/// models (`reasoning` defaults to false there) and `modelOverrides`. Providers bundled into pi without a store entry are
/// absent, and `catalog_fallback` lets the catalogue stand in for them
pub fn pi_facts(agent_dir: &Path) -> ModelFacts {
  let mut defs: BTreeMap<String, Value> = BTreeMap::new();
  if let Some(Value::Object(store)) = read_json(&agent_dir.join("models-store.json")) {
    for (provider, entry) in &store {
      for m in entry.get("models").and_then(Value::as_array).into_iter().flatten() {
        if let Some(id) = m.get("id").and_then(Value::as_str) {
          defs.insert(format!("{provider}/{id}"), m.clone());
        }
      }
    }
  }
  let mut sources = ModelSources::new();
  let config = read_json(&agent_dir.join("models.json"));
  for (provider, p) in config.as_ref().and_then(|c| c.get("providers")).and_then(Value::as_object).into_iter().flatten() {
    let models = p.get("models").and_then(Value::as_array);
    let endpoint = p.get("baseUrl").and_then(Value::as_str).is_some_and(|u| !u.trim().is_empty());
    if endpoint || models.is_some_and(|m| !m.is_empty()) {
      sources.insert(provider.clone(), custom(provider, provider));
    }
    for m in models.into_iter().flatten() {
      if let Some(id) = m.get("id").and_then(Value::as_str) {
        defs.insert(format!("{provider}/{id}"), m.clone());
      }
    }
    for (id, patch) in p.get("modelOverrides").and_then(Value::as_object).into_iter().flatten() {
      if let Some(model) = defs.get_mut(&format!("{provider}/{id}")) {
        pi_override(model, patch);
      }
    }
  }
  let levels = defs.iter().map(|(k, m)| (k.clone(), pi_levels(m))).collect();
  ModelFacts { sources, native: NativeLevels { levels, catalog_fallback: true } }
}

pub fn opencode_sources(configs: &[Value]) -> ModelSources {
  let mut sources = ModelSources::new();
  for config in configs {
    for (id, p) in config.get("provider").and_then(Value::as_object).into_iter().flatten() {
      let endpoint = p.get("options").and_then(|o| o.get("baseURL")).and_then(Value::as_str).is_some_and(|u| !u.trim().is_empty());
      if endpoint || p.get("npm").and_then(Value::as_str).is_some_and(|n| !n.is_empty()) {
        let name = p.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).unwrap_or(id);
        sources.insert(id.clone(), custom(id, name));
      }
    }
  }
  sources
}

pub fn codex_sources(text: &str) -> ModelSources {
  let Ok(config) = text.parse::<toml::Table>() else { return ModelSources::new() };
  let Some(provider) = config.get("model_provider").and_then(toml::Value::as_str).filter(|p| !p.is_empty() && *p != "openai") else {
    return ModelSources::new();
  };
  let name = config
    .get("model_providers")
    .and_then(|m| m.get(provider))
    .and_then(|p| p.get("name"))
    .and_then(toml::Value::as_str)
    .filter(|n| !n.trim().is_empty())
    .unwrap_or(provider);
  ModelSources::from([(ALL.to_owned(), custom(provider, name))])
}

pub fn claude_sources(base_url: Option<&str>) -> ModelSources {
  let host = base_url.map(|u| u.trim().split("://").last().unwrap_or("").split(['/', ':']).next().unwrap_or("").to_lowercase());
  match host.filter(|h| !h.is_empty() && h != "api.anthropic.com") {
    Some(h) => ModelSources::from([(ALL.to_owned(), custom(&h, &h))]),
    None => ModelSources::new(),
  }
}

fn facts(agent: &str, env: &HashMap<String, String>, cwd: &Path) -> ModelFacts {
  let var = |k: &str| env.get(k).cloned().or_else(|| std::env::var(k).ok()).filter(|v| !v.trim().is_empty());
  let home = home_dir();
  let sources = match agent {
    "pi" => return pi_facts(&pi_usage::agent_dir_of(var(pi_usage::AGENT_DIR_ENV))),
    "grok" => {
      let texts = [home.join(".grok/config.toml"), cwd.join(".grok/config.toml")].map(|p| std::fs::read_to_string(p).unwrap_or_default());
      texts.iter().flat_map(|t| grok_model_sources(t)).collect()
    }
    "opencode" => {
      let mut paths = vec![];
      for dir in [home.join(".config/opencode"), cwd.to_path_buf()] {
        paths.extend(["opencode.json", "opencode.jsonc"].map(|f| dir.join(f)));
      }
      paths.extend(var("OPENCODE_CONFIG").map(Into::into));
      opencode_sources(&paths.iter().filter_map(|p| read_json(p)).collect::<Vec<_>>())
    }
    "codex" => {
      let dir = var("CODEX_HOME").map(Into::into).unwrap_or_else(|| home.join(".codex"));
      codex_sources(&std::fs::read_to_string(Path::new(&dir).join("config.toml")).unwrap_or_default())
    }
    "claude" => {
      let dir = var("CLAUDE_CONFIG_DIR").map(Into::into).unwrap_or_else(|| home.join(".claude"));
      let from_settings = [cwd.join(".claude/settings.local.json"), cwd.join(".claude/settings.json"), dir.join("settings.json")]
        .iter()
        .find_map(|p| read_json(p)?.get("env")?.get("ANTHROPIC_BASE_URL")?.as_str().map(str::to_owned));
      claude_sources(var("ANTHROPIC_BASE_URL").or(from_settings).as_deref())
    }
    _ => ModelSources::new(),
  };
  ModelFacts { sources, native: NativeLevels::default() }
}

/// Label the model options by source, then narrow the reasoning selects to what the current model takes; idempotent, so
/// every publish may run it
pub fn refine_controls(agent: &str, options: &mut [ConfigControl], facts: &ModelFacts) {
  apply_model_sources(agent, options, &facts.sources);
  clip_efforts(options, &crate::model_catalog::current(), &facts.native);
}

/// Everything the engine needs to label and narrow one agent's model controls; blocking reads, off the async threads
pub async fn read_model_facts(def: &AgentDef, cwd: &str) -> ModelFacts {
  // No config to read: answer without a trip through the blocking pool
  if !matches!(def.id.as_str(), "grok" | "pi" | "opencode" | "codex" | "claude") {
    return ModelFacts::default();
  }
  let (agent, env, cwd) = (def.id.clone(), def.env.clone().unwrap_or_default(), cwd.to_owned());
  tokio::task::spawn_blocking(move || facts(&agent, &env.into_iter().collect(), Path::new(&cwd))).await.unwrap_or_default()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn only_endpoints_count_as_custom() {
    let s = grok_model_sources("[model.mine]\nbase_url = \"https://x\"\n[model.grok-4]\ncontext_window = 1\n");
    assert_eq!(s.keys().collect::<Vec<_>>(), ["mine"]);
  }

  #[test]
  fn pi_levels_follow_pi_ai() {
    use Level::*;
    assert_eq!(pi_levels(&json!({ "reasoning": false })), [Off]);
    assert_eq!(pi_levels(&json!({ "reasoning": true })), [Off, Minimal, Low, Medium, High]);
    assert_eq!(
      pi_levels(&json!({ "reasoning": true, "thinkingLevelMap": { "off": null, "xhigh": "xhigh", "max": "max" } })),
      [Minimal, Low, Medium, High, XHigh, Max]
    );
    assert_eq!(pi_levels(&json!({ "reasoning": true, "thinkingLevelMap": { "minimal": null, "low": null, "high": null } })), [Off, Medium]);
  }

  #[test]
  fn pi_reads_the_store_then_custom_models_and_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let store = json!({ "anthropic": { "models": [
      { "id": "claude-opus-5", "reasoning": true, "thinkingLevelMap": { "off": null, "xhigh": "xhigh", "max": "max" } },
      { "id": "claude-haiku-4-5", "reasoning": true }
    ] } });
    let models = json!({ "providers": {
      "asgard": { "baseUrl": "https://gw.example/v1", "apiKey": "test-only", "models": [{ "id": "kimi-k3", "reasoning": true }, { "id": "plain" }] },
      "anthropic": { "modelOverrides": { "claude-haiku-4-5": { "thinkingLevelMap": { "max": "max" } } } }
    } });
    std::fs::write(dir.path().join("models-store.json"), store.to_string()).unwrap();
    std::fs::write(dir.path().join("models.json"), models.to_string()).unwrap();
    let f = pi_facts(dir.path());
    assert_eq!(f.sources.keys().collect::<Vec<_>>(), ["asgard"]);
    assert!(f.native.catalog_fallback);
    let l = |k: &str| f.native.levels.get(k).cloned().unwrap_or_default();
    assert_eq!(l("anthropic/claude-opus-5").last(), Some(&Level::Max));
    assert!(!l("anthropic/claude-opus-5").contains(&Level::Off));
    assert_eq!(l("anthropic/claude-haiku-4-5"), [Level::Off, Level::Minimal, Level::Low, Level::Medium, Level::High, Level::Max]);
    assert_eq!(l("asgard/kimi-k3").len(), 5);
    assert_eq!(l("asgard/plain"), [Level::Off]);
  }

  #[test]
  fn opencode_codex_and_claude_endpoints() {
    let oc = opencode_sources(&[json!({ "provider": {
      "asgard": { "npm": "@ai-sdk/openai-compatible", "name": "Asgard", "options": { "baseURL": "https://gw.example/v1" } },
      "anthropic": { "options": { "apiKey": "test-only" } }
    } })]);
    assert_eq!(oc.get("asgard").map(|s| s.name.as_str()), Some("Asgard"));
    assert!(!oc.contains_key("anthropic"));
    let cx = codex_sources("model_provider = \"asgard\"\n[model_providers.asgard]\nname = \"Asgard\"\nbase_url = \"https://gw\"\n");
    assert_eq!(cx.get(ALL).map(|s| (s.id.as_str(), s.name.as_str())), Some(("asgard", "Asgard")));
    assert!(codex_sources("model_provider = \"openai\"\n").is_empty());
    assert!(codex_sources("model = \"gpt-6-astra\"\n").is_empty());
    assert_eq!(claude_sources(Some("https://gw.example:8443/v1")).get(ALL).map(|s| s.id.as_str()), Some("gw.example"));
    assert!(claude_sources(Some("https://api.anthropic.com")).is_empty());
    assert!(claude_sources(None).is_empty());
  }
}
