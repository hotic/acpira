//! Model sources of the built-in agent (`acpira agent`): `<ACPIRA_HOME>/providers.json`, mirrored by `packages/shared/providers.ts`.
//! The host writes the file (tmp + rename under its file lock) and keeps each source's API key in `secrets.json` under
//! `provider_secret_key(id)`; the agent re-reads both at the start of every turn. Unknown fields survive a rewrite, so an
//! older build editing the file keeps what a newer one added

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::model_catalog::Catalog;

pub const PROVIDERS_FILE: &str = "providers.json";
pub const PROVIDERS_VERSION: u32 = 1;
pub const PROVIDER_SECRET_PREFIX: &str = "acpira.provider.";

/// The vault key holding a source's API key
pub fn provider_secret_key(id: &str) -> String {
  format!("{PROVIDER_SECRET_PREFIX}{id}")
}

/// Vault keys of the web search backends (`web_secret_key`), apart from the model sources' so a source id never meets one
pub const WEB_SECRET_PREFIX: &str = "acpira.web.";

/// The vault key holding a web search backend's API key
pub fn web_secret_key(backend: &str) -> String {
  format!("{WEB_SECRET_PREFIX}{backend}")
}

/// Model sources the plugins in effect contribute (`contributes.providers` and a process's `providers/set`): written
/// by the engine on every plugin load, in `ProvidersFile`'s shape, and read by the agent beside `providers.json`. A
/// source whose id the user's file already has is skipped
pub fn plugin_providers_path(home: &std::path::Path) -> std::path::PathBuf {
  home.join("plugins").join(PROVIDERS_FILE)
}

/// A plugin source's field naming the plugin it came from
pub const PLUGIN_FIELD: &str = "plugin";

/// A plugin source's field naming the environment variable that holds its API key (a key stored under
/// `provider_secret_key` comes first)
pub const KEY_ENV_FIELD: &str = "keyEnv";

/// The search backends `ProvidersFile::web_search` takes, the default first: `default` (the free hosted endpoints, no
/// key), `jina` (Jina Search with the user's key) and `deepseek` (DeepSeek's Search API, or a gateway serving the same
/// `/v1/tools/search`, see `ProvidersFile::web_search_via`); a keyed pick has the other keyed backend and the free ones
/// behind it
pub const WEB_SEARCH_BACKENDS: [&str; 3] = ["default", "jina", "deepseek"];

/// The compaction strategies `ProvidersFile::compaction` takes, the default first: `summary` (a model writes a summary
/// of the history, `compact.rs`) and `chunk` (finished turns fold into chunks of the user's words and the final
/// replies, no model call, `chunk.rs`)
pub const COMPACTION_STRATEGIES: [&str; 2] = ["summary", "chunk"];

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersFile {
  #[serde(default)]
  pub version: u32,
  #[serde(default)]
  pub providers: Vec<Provider>,
  /// The approval level a session starts on when the host has no pick of the user's to replay, one of
  /// `APPROVAL_LEVELS`; absent (or unknown) is `ask`. The level itself belongs to the session (its `approval` select)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub approval: Option<String>,
  /// Raw mode (`acpira_agent::modes::RAW`) is offered in the mode picker; absent is off. Read when a session opens
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub raw_mode: Option<bool>,
  /// The model for quick side jobs such as naming a session (`<provider id>/<model id>`), or `FAST_MODEL_OFF`; absent,
  /// or a model no longer usable, picks one (`ProvidersFile::fast_pick`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fast_model: Option<String>,
  /// The `websearch` backend, one of `WEB_SEARCH_BACKENDS` other than `default`; absent (or unknown) is `default`
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub web_search: Option<String>,
  /// The model source whose address and key DeepSeek-shaped search goes through (a gateway serving
  /// `/v1/tools/search`); absent is DeepSeek's own API with the `deepseek` search key
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub web_search_via: Option<String>,
  /// How the agent compacts its history, one of `COMPACTION_STRATEGIES` other than `summary`; absent (or unknown) is
  /// `summary`. Read when a compaction runs
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub compaction: Option<String>,
  /// The system prompt of the agent's Raw mode (`acpira_agent::modes::RAW`); absent or blank sends none. Read at the
  /// start of every Raw turn
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub raw_system_prompt: Option<String>,
  #[serde(flatten)]
  pub extra: Map<String, Value>,
}

/// `ProvidersFile::fast_model`'s value that turns the side jobs' model off
pub const FAST_MODEL_OFF: &str = "off";

/// The approval levels `ProvidersFile::approval` takes, the default first: `ask` (edits and commands ask), `auto-edit`
/// (edits run, commands ask), `full` (everything runs, except edits to Acpira's own configuration)
pub const APPROVAL_LEVELS: [&str; 3] = ["ask", "auto-edit", "full"];

/// The wire format a source speaks (`Provider::format` as a value; the field stays a string so a format a newer build
/// added survives this build's rewrite)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApiFormat {
  /// OpenAI Chat Completions and everything compatible with it
  OpenaiChat,
  /// OpenAI Responses (what Codex speaks; carries a reasoning model's encrypted thinking between steps)
  OpenaiResponses,
  /// Anthropic Messages
  Anthropic,
}

impl ApiFormat {
  pub fn parse(s: &str) -> Option<ApiFormat> {
    match s {
      "openai-chat" | "" => Some(ApiFormat::OpenaiChat),
      "openai-responses" => Some(ApiFormat::OpenaiResponses),
      "anthropic" => Some(ApiFormat::Anthropic),
      _ => None,
    }
  }

  pub fn as_str(self) -> &'static str {
    match self {
      ApiFormat::OpenaiChat => "openai-chat",
      ApiFormat::OpenaiResponses => "openai-responses",
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
  /// `custom`, or `ollama` / `lmstudio` for a local server found on this machine (takes no key). Files written before
  /// the vendor presets were dropped may name a vendor (`deepseek`, …), which still counts as a family hint
  #[serde(default)]
  pub preset: String,
  /// `openai-chat` | `openai-responses` | `anthropic`; anything else is listed but not used
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
  /// Whether the session offers a Fast switch (the provider's premium speed tier): None decides from the API format and
  /// the model id (`acpira_agent::llm::fast`), true / false override that
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fast: Option<bool>,
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
      fast: None,
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

  /// Whether the levels are still a guess (catalogue or defaults) rather than reported by the endpoint or saved in the
  /// settings dialog, which clears `estimated`
  pub fn efforts_guessed(&self) -> bool {
    self.estimated.iter().any(|e| e == EFFORTS_FIELD)
  }

  /// The levels the effort select offers. This is the one rule the composer, the sidekick, the summon roster and the
  /// settings dialog all follow, so they always show the same scale. None while thinking is off. A list reported by the
  /// endpoint or saved in the dialog stays as it is. A guessed list, or none at all (a model added by id, a plugin's
  /// source, a hand-written file), takes the catalogue's levels when the catalogue knows the model (by id, then name):
  /// without levels no thinking switch is sent, and Claude added by id ran without thinking
  pub fn offered_efforts(&self, catalog: &Catalog) -> Vec<String> {
    if self.thinking == Thinking::Off {
      return vec![];
    }
    match self.catalogue_efforts(catalog) {
      Some(levels) => levels.to_vec(),
      None => self.efforts.clone(),
    }
  }

  /// The catalogue's levels where they stand in for this model's own (`offered_efforts`)
  pub fn catalogue_efforts<'a>(&self, catalog: &'a Catalog) -> Option<&'a [String]> {
    (self.efforts.is_empty() || self.efforts_guessed()).then(|| catalog.efforts(&self.id, self.name.as_deref())).flatten()
  }
}

/// The `estimated` entry naming guessed effort levels
pub const EFFORTS_FIELD: &str = "efforts";

impl Provider {
  pub fn api_format(&self) -> Option<ApiFormat> {
    ApiFormat::parse(&self.format)
  }

  /// A local server found on this machine, which takes no key
  pub fn keyless(&self) -> bool {
    matches!(self.preset.as_str(), "ollama" | "lmstudio")
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

  /// The fast model in effect: none when turned off, the pick while it is usable, else `auto_fast_model`.
  /// `keyed` says whether a source has its key stored
  pub fn fast_pick(&self, catalog: &Catalog, keyed: impl Fn(&Provider) -> bool) -> Option<(&Provider, &ProviderModel)> {
    match self.fast_model.as_deref() {
      Some(FAST_MODEL_OFF) => None,
      pick => pick.and_then(|p| self.find(p)).or_else(|| self.auto_fast_model(catalog, keyed)),
    }
  }

  /// The fast model when none is picked, so sessions get named without a trip to the settings: the usable model with
  /// the lowest catalogue price among sources that can be called (a key stored, or a local server), else the first of
  /// them. A zero price is read as unlisted, since the catalogue writes 0 for models it has no price for
  pub fn auto_fast_model(&self, catalog: &Catalog, keyed: impl Fn(&Provider) -> bool) -> Option<(&Provider, &ProviderModel)> {
    let ready = || self.usable().filter(|(p, _)| keyed(p) || p.keyless());
    ready()
      .filter_map(|(p, m)| {
        let cost = catalog.lookup(&m.id, m.name.as_deref())?.cost.as_ref()?;
        let price = cost.input + cost.output;
        (price > 0.0).then_some((price, p, m))
      })
      // The first of equally priced models, in file order
      .min_by(|a, b| a.0.total_cmp(&b.0))
      .map(|(_, p, m)| (p, m))
      .or_else(|| ready().next())
  }
}

/// One source as the settings page sees it: the file's entry plus whether a key is stored. The key itself never
/// leaves the host
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
  #[serde(flatten)]
  pub provider: Provider,
  pub has_key: bool,
}

/// The settings page's copy of providers.json; `error` when the file exists but cannot be read (it is then left alone)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvidersView {
  pub providers: Vec<ProviderView>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  /// The families a model can be pinned to (`acpira_agent::llm::family::names`)
  #[serde(default)]
  pub families: Vec<String>,
  /// The file's approval level as written (absent reads as `ask`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub approval: Option<String>,
  /// Raw mode is offered in the mode picker (`ProvidersFile::raw_mode`)
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub raw_mode: bool,
  /// The file's fast model pick as written (`ProvidersFile::fast_model`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fast_model: Option<String>,
  /// The model the automatic pick lands on now (`ProvidersFile::auto_fast_model`), whatever the file holds
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub fast_model_auto: Option<String>,
  /// The file's search backend as written (`ProvidersFile::web_search`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub web_search: Option<String>,
  /// The file's search source as written (`ProvidersFile::web_search_via`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub web_search_via: Option<String>,
  /// The search backends with a stored key (never the key itself)
  #[serde(default)]
  pub web_keys: Vec<String>,
  /// The file's compaction strategy as written (`ProvidersFile::compaction`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub compaction: Option<String>,
  /// The Raw mode's system prompt as written (`ProvidersFile::raw_system_prompt`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub raw_system_prompt: Option<String>,
}

/// A network question from the settings page; none of them changes providers.json. `key`: a key typed into the form
/// and not saved yet; absent uses the stored key of the source with this id
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProviderProbe {
  /// The source's models with their metadata (free: reads the model list)
  Models {
    provider: Provider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
  },
  /// Whether the address and key work (free: reads the model list)
  Check {
    provider: Provider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
  },
  /// One tiny real call to a model (spends a few tokens)
  Test {
    provider: Provider,
    model: ProviderModel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
  },
  /// Model servers running on this machine
  Local,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProbeOutcome {
  Models { models: Vec<ProviderModel> },
  Check { count: usize },
  Test { ms: u64, text: String },
  Local { servers: Vec<LocalSource> },
  Failed { error: String },
}

/// A local model server found by `ProviderProbe::Local`, ready to save as a source
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSource {
  pub preset: String,
  pub name: String,
  pub base_url: String,
  pub models: Vec<ProviderModel>,
}

/// An edit from the settings page; the reply is the fresh view
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProviderAction {
  /// Insert, or replace the entry with the same id; an empty id gets one made from the name. `key`: absent keeps the
  /// stored key, empty removes it
  Save {
    provider: Provider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
  },
  /// Remove the source and its key
  Delete { id: String },
  /// Set the approval level new sessions start on (one of `APPROVAL_LEVELS`)
  Approval { level: String },
  /// Offer Raw mode in the mode picker, or stop offering it
  RawMode { on: bool },
  /// Set the fast model (a usable `<provider id>/<model id>`, or `FAST_MODEL_OFF`); absent or empty picks one
  FastModel {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pick: Option<String>,
  },
  /// Set the search backend (one of `WEB_SEARCH_BACKENDS`). `key` is that backend's: absent keeps the stored key, empty
  /// removes it. `via` names the model source DeepSeek-shaped search goes through: absent keeps it, empty goes back to
  /// DeepSeek's own API
  WebSearch {
    backend: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    via: Option<String>,
  },
  /// Set the compaction strategy (one of `COMPACTION_STRATEGIES`)
  Compaction { strategy: String },
  /// Set the Raw mode's system prompt; absent or blank removes it
  RawSystemPrompt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text: Option<String>,
  },
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn the_fast_model_is_picked_when_none_is_set_and_stays_off_when_turned_off() {
    use crate::model_catalog::{Catalog, trim_models_dev};
    let api = json!({ "deepseek": { "models": {
      "deepseek-big": { "name": "DeepSeek Big", "cost": { "input": 5.0, "output": 25.0 } },
      "deepseek-small": { "name": "DeepSeek Small", "cost": { "input": 0.1, "output": 0.4 } },
      "deepseek-unpriced": { "name": "DeepSeek Unpriced", "cost": { "input": 0.0, "output": 0.0 } } } } });
    let catalog = Catalog::new(trim_models_dev(&api, "2026-10-28T00:00:00Z", None));
    let mut file: ProvidersFile = serde_json::from_value(json!({ "providers": [
      { "id": "nokey", "format": "openai-chat", "baseUrl": "https://a", "models": [{ "id": "deepseek-small" }] },
      { "id": "gw", "format": "openai-chat", "baseUrl": "https://b", "models": [{ "id": "deepseek-big" }, { "id": "deepseek-unpriced" }, { "id": "deepseek-small" }] },
    ] })).unwrap();
    let keyed = |p: &Provider| p.id == "gw";
    let pick = |f: &ProvidersFile| f.fast_pick(&catalog, keyed).map(|(p, m)| model_pick(&p.id, &m.id));
    // Unset: the cheapest priced model of a source with a key; a source without one is passed over
    assert_eq!(pick(&file).as_deref(), Some("gw/deepseek-small"));
    // A usable pick wins; one edited away falls back to the automatic pick
    file.fast_model = Some("gw/deepseek-big".into());
    assert_eq!(pick(&file).as_deref(), Some("gw/deepseek-big"));
    file.fast_model = Some("gw/gone".into());
    assert_eq!(pick(&file).as_deref(), Some("gw/deepseek-small"));
    file.fast_model = Some(FAST_MODEL_OFF.into());
    assert_eq!(pick(&file), None);
    // Nothing priced: the first callable model
    file.fast_model = None;
    file.providers[1].models.retain(|m| m.id == "deepseek-unpriced");
    assert_eq!(pick(&file).as_deref(), Some("gw/deepseek-unpriced"));
  }

  #[test]
  fn offered_efforts_keep_a_confirmed_scale_and_replace_a_guess() {
    use crate::model_catalog::{Catalog, trim_models_dev};
    let api = json!({ "moonshotai": { "models": { "kimi-k3.1-pro": { "name": "Kimi K3.1 Pro", "reasoning": true,
      "reasoning_options": [{ "type": "effort", "values": ["low", "high", "max"] }] } } } });
    let catalog = Catalog::new(trim_models_dev(&api, "2026-10-28T00:00:00Z", None));
    let model = |v: serde_json::Value| -> ProviderModel { serde_json::from_value(v).unwrap() };
    // Guessed defaults on a gateway id the catalogue knows by name: the catalogue's scale
    let guessed = model(json!({ "id": "k3.1-pro", "name": "Kimi K3.1 Pro", "efforts": ["low", "medium", "high"], "estimated": ["efforts"] }));
    assert_eq!(guessed.offered_efforts(&catalog), ["low", "high", "max"]);
    // Saved in the dialog (or reported by the endpoint): as it is
    let saved = model(json!({ "id": "kimi-k3.1-pro", "efforts": ["low", "medium", "high"] }));
    assert_eq!(saved.offered_efforts(&catalog), ["low", "medium", "high"]);
    // None at all: the catalogue's; unknown to it: none
    assert_eq!(model(json!({ "id": "kimi-k3.1-pro" })).offered_efforts(&catalog), ["low", "high", "max"]);
    assert!(model(json!({ "id": "mystery" })).offered_efforts(&catalog).is_empty());
    // Thinking off: no scale whatever the list holds
    assert!(model(json!({ "id": "kimi-k3.1-pro", "thinking": "off", "efforts": ["low"] })).offered_efforts(&catalog).is_empty());
  }

  #[test]
  fn the_view_flattens_the_entry_and_actions_are_tagged() {
    let v = serde_json::to_value(ProviderView {
      provider: ProvidersFile::parse(r#"{"providers":[{"id":"ds"}]}"#).unwrap().providers.remove(0),
      has_key: true,
    })
    .unwrap();
    assert_eq!((v["id"].as_str(), v["hasKey"].as_bool()), (Some("ds"), Some(true)));
    let a: ProviderAction = serde_json::from_value(json!({ "kind": "delete", "id": "ds" })).unwrap();
    assert_eq!(a, ProviderAction::Delete { id: "ds".into() });
    let a: ProviderAction = serde_json::from_value(json!({ "kind": "save", "provider": { "id": "" } })).unwrap();
    assert!(matches!(a, ProviderAction::Save { key: None, .. }));
  }

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
