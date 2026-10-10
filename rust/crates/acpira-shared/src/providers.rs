//! Model sources of the built-in agent (`acpira agent`): `<ACPIRA_HOME>/providers.json`, mirrored by `src/shared/providers.ts`.
//! The host writes the file (tmp + rename under its file lock) and keeps each source's API key in `secrets.json` under
//! `provider_secret_key(id)`; the agent re-reads both at the start of every turn. Unknown fields survive a rewrite, so an
//! older build editing the file keeps what a newer one added

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const PROVIDERS_FILE: &str = "providers.json";
pub const PROVIDERS_VERSION: u32 = 1;
pub const PROVIDER_SECRET_PREFIX: &str = "acpira.provider.";

/// The vault key holding a source's API key
pub fn provider_secret_key(id: &str) -> String {
  format!("{PROVIDER_SECRET_PREFIX}{id}")
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersFile {
  #[serde(default)]
  pub version: u32,
  #[serde(default)]
  pub providers: Vec<Provider>,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

/// The wire format a source speaks (`Provider::format` as a value; the field stays a string so a format a newer build
/// added survives this build's rewrite)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiFormat {
  /// OpenAI Chat Completions and everything compatible with it
  OpenaiChat,
  /// Anthropic Messages
  Anthropic,
}

impl ApiFormat {
  pub fn parse(s: &str) -> Option<ApiFormat> {
    match s {
      "openai-chat" | "" => Some(ApiFormat::OpenaiChat),
      "anthropic" => Some(ApiFormat::Anthropic),
      _ => None,
    }
  }

  pub fn as_str(self) -> &'static str {
    match self {
      ApiFormat::OpenaiChat => "openai-chat",
      ApiFormat::Anthropic => "anthropic",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
  /// Unique, no `/` (model picks are `<provider id>/<model id>`)
  pub id: String,
  #[serde(default)]
  pub name: String,
  /// The preset it was created from (`deepseek`, `ollama`, … or `custom`)
  #[serde(default)]
  pub preset: String,
  /// `openai-chat` | `anthropic`; anything else is listed but not used
  #[serde(default = "openai_chat")]
  pub format: String,
  /// The API root (`https://api.deepseek.com/v1`), or the full endpoint when `full_url` is on
  #[serde(default)]
  pub base_url: String,
  /// Use `base_url` exactly as given instead of appending `/chat/completions` / `/messages`
  #[serde(default)]
  pub full_url: bool,
  #[serde(default = "yes")]
  pub enabled: bool,
  /// Extra request headers sent with every call
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub headers: BTreeMap<String, String>,
  #[serde(default)]
  pub models: Vec<ProviderModel>,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

/// Whether a model reasons: `Auto` leaves the provider's default, `On` / `Off` send the family's switch
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Thinking {
  #[default]
  Auto,
  On,
  Off,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sampling {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub temperature: Option<f64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub top_p: Option<f64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub top_k: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModel {
  /// The id the API takes
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default = "yes")]
  pub enabled: bool,
  /// Context window in tokens
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub context: Option<u64>,
  /// Output limit in tokens
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub output: Option<u64>,
  /// Accepted inputs: `text` always, `image` when the model takes images
  #[serde(default = "text_only")]
  pub input: Vec<String>,
  #[serde(default)]
  pub thinking: Thinking,
  /// Reasoning levels offered as the effort select (empty = no select)
  #[serde(default)]
  pub efforts: Vec<String>,
  /// The prompt / request family pinned for this model; None matches automatically
  #[serde(default)]
  pub family: Option<String>,
  #[serde(default)]
  pub sampling: Sampling,
  /// Tool-call rounds per turn before the agent pauses; None = no cap
  #[serde(default)]
  pub max_steps: Option<u32>,
  /// Field names whose values were guessed (catalogue or defaults) rather than reported by the endpoint
  #[serde(default)]
  pub estimated: Vec<String>,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

fn openai_chat() -> String {
  ApiFormat::OpenaiChat.as_str().to_owned()
}

fn yes() -> bool {
  true
}

fn text_only() -> Vec<String> {
  vec!["text".to_owned()]
}

impl ProviderModel {
  pub fn new(id: impl Into<String>) -> Self {
    ProviderModel {
      id: id.into(),
      name: None,
      enabled: true,
      context: None,
      output: None,
      input: text_only(),
      thinking: Thinking::Auto,
      efforts: vec![],
      family: None,
      sampling: Sampling::default(),
      max_steps: None,
      estimated: vec![],
      extra: Map::new(),
    }
  }

  pub fn display_name(&self) -> &str {
    self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&self.id)
  }

  pub fn takes_images(&self) -> bool {
    self.input.iter().any(|i| i == "image")
  }
}

impl Provider {
  pub fn api_format(&self) -> Option<ApiFormat> {
    ApiFormat::parse(&self.format)
  }

  pub fn display_name(&self) -> &str {
    if self.name.trim().is_empty() { &self.id } else { &self.name }
  }
}

/// A model pick on the wire: `<provider id>/<model id>`; the model id may itself contain `/`
pub fn model_pick(provider: &str, model: &str) -> String {
  format!("{provider}/{model}")
}

pub fn split_pick(pick: &str) -> Option<(&str, &str)> {
  pick.split_once('/').filter(|(p, m)| !p.is_empty() && !m.is_empty())
}

impl ProvidersFile {
  pub fn parse(text: &str) -> serde_json::Result<ProvidersFile> {
    serde_json::from_str(text)
  }

  /// Every usable model, in file order: enabled source with a known format, enabled model
  pub fn usable(&self) -> impl Iterator<Item = (&Provider, &ProviderModel)> {
    self
      .providers
      .iter()
      .filter(|p| p.enabled && p.api_format().is_some())
      .flat_map(|p| p.models.iter().filter(|m| m.enabled).map(move |m| (p, m)))
  }

  pub fn find(&self, pick: &str) -> Option<(&Provider, &ProviderModel)> {
    let (p, m) = split_pick(pick)?;
    self.usable().find(|(pp, mm)| pp.id == p && mm.id == m)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn defaults_fill_a_minimal_file_and_unknown_fields_survive() {
    let f = ProvidersFile::parse(
      &json!({ "providers": [{ "id": "ds", "baseUrl": "https://api.deepseek.com/v1", "future": 1,
        "models": [{ "id": "deepseek-chat", "later": true }, { "id": "off", "enabled": false }] }] })
      .to_string(),
    )
    .unwrap();
    let p = &f.providers[0];
    assert!(p.enabled && p.api_format() == Some(ApiFormat::OpenaiChat));
    assert_eq!(p.models[0].input, ["text"]);
    assert_eq!(f.usable().count(), 1);
    assert_eq!(f.find("ds/deepseek-chat").unwrap().1.id, "deepseek-chat");
    let back = serde_json::to_value(&f).unwrap();
    assert_eq!(back["providers"][0]["future"], 1);
    assert_eq!(back["providers"][0]["models"][0]["later"], true);
  }

  #[test]
  fn an_unknown_format_is_listed_but_not_usable() {
    let f = ProvidersFile::parse(r#"{"providers":[{"id":"x","format":"gemini","models":[{"id":"m"}]}]}"#).unwrap();
    assert_eq!(f.providers[0].api_format(), None);
    assert_eq!(f.usable().count(), 0);
    assert_eq!(serde_json::to_value(&f).unwrap()["providers"][0]["format"], "gemini");
  }

  #[test]
  fn picks_split_on_the_first_slash() {
    assert_eq!(split_pick("or/deepseek/deepseek-chat"), Some(("or", "deepseek/deepseek-chat")));
    assert_eq!(split_pick("nope"), None);
  }
}
