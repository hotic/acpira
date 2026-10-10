//! Model discovery for the settings page, called by the host: list a source's models, check that its address and key
//! work, test one model with a real call, and find local servers. Metadata is filled in three tiers: what the endpoint
//! reports, else the built-in catalogue (matched by normalized id), else conservative defaults. Every value not
//! reported by the endpoint is named in `estimated`, which the page shows as "unconfirmed" and the turn loop never
//! sends as a limit. All calls block; the host runs them on its blocking pool with its own proxy-aware agent

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_shared::model_catalog::{Catalog, model_key};
use acpira_shared::providers::{ApiFormat, Provider, ProviderModel, Thinking};

use super::{Endpoint, Event, Item, LlmError, Part, Request, StopReason, Usage, error_message};

/// The context window assumed when neither the endpoint nor the catalogue knows one: small enough that a real window
/// is rarely overstated (an overstated one overflows before anything trims the history)
pub const DEFAULT_CONTEXT: u64 = 128_000;

/// Anthropic lists models in pages
const MAX_PAGES: usize = 20;

/// What the endpoint said about one model; None where it said nothing
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Listed {
  pub id: String,
  pub name: Option<String>,
  pub context: Option<u64>,
  pub output: Option<u64>,
  pub images: Option<bool>,
  pub tools: Option<bool>,
  pub reasoning: Option<bool>,
  /// An embedding / speech / image model: no use as a chat model
  pub not_chat: bool,
}

/// The `/models` URL of a source; None for a full endpoint URL it cannot be derived from
pub fn models_url(provider: &Provider) -> Option<String> {
  let url = provider.base_url.trim().trim_end_matches('/');
  if url.is_empty() {
    return None;
  }
  if !provider.full_url {
    return Some(format!("{url}/models"));
  }
  ["/chat/completions", "/messages"].iter().find_map(|s| url.strip_suffix(s)).map(|root| format!("{root}/models"))
}

/// Request headers for a GET in the source's format: its auth, its version header, its own extra headers last
fn headers(provider: &Provider, url: &str, api_key: Option<&str>) -> Vec<(String, String)> {
  let mut out: Vec<(String, String)> = vec![("accept".into(), "application/json".into())];
  if provider.api_format() == Some(ApiFormat::Anthropic) {
    out.push(("anthropic-version".into(), super::anthropic::VERSION.into()));
    if let Some(key) = api_key {
      out.push(("x-api-key".into(), key.into()));
      if !url.contains("api.anthropic.com") {
        out.push(("authorization".into(), format!("Bearer {key}")));
      }
    }
  } else if let Some(key) = api_key {
    out.push(("authorization".into(), format!("Bearer {key}")));
  }
  for (k, v) in &provider.headers {
    out.retain(|(name, _)| !name.eq_ignore_ascii_case(k));
    out.push((k.clone(), v.clone()));
  }
  out
}

fn get(http: &ureq::Agent, url: &str, headers: &[(String, String)]) -> Result<Value, LlmError> {
  let mut call = http.get(url);
  for (k, v) in headers {
    call = call.header(k.as_str(), v.as_str());
  }
  read_json(call.call())
}

fn read_json(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<Value, LlmError> {
  let mut res = res.map_err(|e| LlmError::Network(e.to_string()))?;
  let status = res.status().as_u16();
  let retry_after = super::retry_after(res.headers());
  let text = res.body_mut().with_config().limit(32 * 1024 * 1024).read_to_string().map_err(|e| LlmError::Network(e.to_string()))?;
  if !(200..300).contains(&status) {
    return Err(LlmError::Http { status, message: error_message(&text), retry_after });
  }
  serde_json::from_str(&text).map_err(|e| LlmError::Protocol(format!("the model list is not JSON: {e}")))
}

/// Every entry of the source's model list, following Anthropic's pages
pub fn list_raw(http: &ureq::Agent, provider: &Provider, api_key: Option<&str>) -> Result<Vec<Value>, LlmError> {
  let url = models_url(provider).ok_or_else(|| LlmError::Protocol("no model list URL can be derived from this endpoint URL".into()))?;
  let headers = headers(provider, &url, api_key);
  let anthropic = provider.api_format() == Some(ApiFormat::Anthropic);
  let mut out = vec![];
  let mut after: Option<String> = None;
  for _ in 0..MAX_PAGES {
    let page_url = match (&after, anthropic) {
      (Some(id), _) => format!("{url}?limit=1000&after_id={id}"),
      (None, true) => format!("{url}?limit=1000"),
      (None, false) => url.clone(),
    };
    let body = get(http, &page_url, &headers)?;
    let entries = entries(&body).ok_or_else(|| LlmError::Protocol("the answer has no model list (`data` or `models`)".into()))?;
    out.extend(entries.iter().cloned());
    after = body.get("last_id").and_then(Value::as_str).filter(|_| body.get("has_more") == Some(&Value::Bool(true))).map(str::to_owned);
    if after.is_none() {
      break;
    }
  }
  Ok(out)
}

/// `data` (OpenAI, Anthropic, LM Studio) or `models` (Gemini, Ollama's tags)
fn entries(body: &Value) -> Option<&Vec<Value>> {
  body.get("data").or_else(|| body.get("models")).and_then(Value::as_array)
}

fn uint(v: Option<&Value>) -> Option<u64> {
  let v = v?;
  v.as_u64().or_else(|| v.as_f64().filter(|f| *f >= 1.0).map(|f| f as u64)).or_else(|| v.as_str()?.trim().parse().ok()).filter(|n| *n > 0)
}

fn strings(v: Option<&Value>) -> Vec<String> {
  v.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_lowercase).collect()
}

/// The endpoint tier: every field a known server reports about one model
pub fn parse_entry(e: &Value) -> Option<Listed> {
  // Gemini's native list names models `models/<id>`; Ollama's tags call the id `model` / `name`
  let id = e
    .get("id")
    .or_else(|| e.get("model"))
    .or_else(|| e.get("name"))
    .and_then(Value::as_str)
    .map(|s| s.strip_prefix("models/").unwrap_or(s).trim().to_owned())
    .filter(|s| !s.is_empty())?;
  let first = |paths: &[&str]| paths.iter().find_map(|p| uint(e.pointer(p)));
  let context = first(&[
    "/context_length",
    "/context_window",
    "/max_context_length",
    "/max_model_len",
    "/max_input_tokens",
    "/inputTokenLimit",
    "/top_provider/context_length",
  ]);
  // LM Studio reports what the loaded instance was given, which is the real window, ahead of the model's maximum
  let context = uint(e.get("loaded_context_length")).or(context);
  let output = first(&["/top_provider/max_completion_tokens", "/max_completion_tokens", "/max_output_tokens", "/outputTokenLimit", "/max_tokens"]);
  let modalities: Vec<String> = [strings(e.pointer("/architecture/input_modalities")), strings(e.pointer("/modalities/input")), strings(e.get("input_modalities"))].concat();
  let params = strings(e.get("supported_parameters"));
  let caps = strings(e.get("capabilities"));
  let kind = e.get("type").and_then(Value::as_str).unwrap_or("");
  let images = if !modalities.is_empty() {
    Some(modalities.iter().any(|m| m == "image"))
  } else if kind == "vlm" || caps.iter().any(|c| c == "vision") {
    Some(true)
  } else {
    None
  };
  let tools = if !params.is_empty() {
    Some(params.iter().any(|p| p == "tools"))
  } else if caps.iter().any(|c| c == "tool_use" || c == "tools") {
    Some(true)
  } else {
    None
  };
  let reasoning = if params.iter().any(|p| p == "reasoning" || p == "include_reasoning") || caps.iter().any(|c| c == "thinking") { Some(true) } else { None };
  let methods = strings(e.get("supportedGenerationMethods"));
  let lower = id.to_lowercase();
  let not_chat = kind == "embeddings"
    || kind == "embedding"
    || (!methods.is_empty() && !methods.iter().any(|m| m == "generatecontent"))
    || (caps.iter().any(|c| c == "embedding") && !caps.iter().any(|c| c == "completion"))
    || ["embed", "whisper", "tts", "dall-e", "moderation", "rerank"].iter().any(|w| lower.contains(w));
  let name = ["/display_name", "/displayName", "/name"]
    .iter()
    .filter_map(|p| e.pointer(p).and_then(Value::as_str))
    .map(|n| n.strip_prefix("models/").unwrap_or(n).trim())
    .find(|n| !n.is_empty() && *n != id)
    .map(str::to_owned);
  Some(Listed { id, name, context, output, images, tools, reasoning, not_chat })
}

/// The catalogue and default tiers over what the endpoint said
pub fn complete(listed: &Listed, catalog: &Catalog) -> ProviderModel {
  let mut m = ProviderModel::new(listed.id.clone());
  m.name = listed.name.clone();
  let entry = catalog.get(&model_key(&listed.id));
  let mut estimated: Vec<&str> = vec![];
  m.context = listed.context.or_else(|| {
    estimated.push("context");
    Some(entry.and_then(|c| c.context).unwrap_or(DEFAULT_CONTEXT))
  });
  m.output = listed.output.or_else(|| {
    let o = entry.and_then(|c| c.output);
    if o.is_some() {
      estimated.push("output");
    }
    o
  });
  let images = listed.images.unwrap_or_else(|| {
    estimated.push("input");
    entry.is_some_and(|c| c.images)
  });
  if images {
    m.input.push("image".into());
  }
  // Effort levels come only from the catalogue (no listing reports them); reasoning stays the provider's default
  if let Some(c) = entry.filter(|c| c.reasoning && !c.efforts.is_empty()) {
    m.efforts = c.efforts.clone();
    estimated.push("efforts");
  }
  if m.name.is_none() {
    m.name = entry.map(|c| c.name.clone()).filter(|n| n != &listed.id);
  }
  m.estimated = estimated.into_iter().map(str::to_owned).collect();
  m
}

/// The source's chat models with their metadata, sorted by id
pub fn discover(http: &ureq::Agent, provider: &Provider, api_key: Option<&str>, catalog: &Catalog) -> Result<Vec<ProviderModel>, LlmError> {
  let local = local_kind(provider);
  // LM Studio's own list carries context lengths and model types; its OpenAI one only ids
  let raw = match local {
    Some(Local::LmStudio) => get(http, &format!("{}/api/v0/models", server_root(provider)), &[])
      .ok()
      .and_then(|b| entries(&b).cloned())
      .map(Ok)
      .unwrap_or_else(|| list_raw(http, provider, api_key))?,
    _ => list_raw(http, provider, api_key)?,
  };
  let mut listed: Vec<Listed> = raw.iter().filter_map(parse_entry).filter(|l| !l.not_chat).collect();
  if local == Some(Local::Ollama) {
    for l in &mut listed {
      ollama_show(http, &server_root(provider), l);
    }
    listed.retain(|l| !l.not_chat);
  }
  listed.sort_by(|a, b| a.id.cmp(&b.id));
  listed.dedup_by(|a, b| a.id == b.id);
  let mut models: Vec<ProviderModel> = listed.iter().map(|l| complete(l, catalog)).collect();
  // Ollama reports the model's trained window, not the `num_ctx` it serves with: still an estimate
  if local == Some(Local::Ollama) {
    for m in &mut models {
      if !m.estimated.iter().any(|e| e == "context") {
        m.estimated.push("context".into());
      }
    }
  }
  Ok(models)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Local {
  Ollama,
  LmStudio,
}

fn local_kind(provider: &Provider) -> Option<Local> {
  match provider.preset.as_str() {
    "ollama" => return Some(Local::Ollama),
    "lmstudio" => return Some(Local::LmStudio),
    _ => {}
  }
  let url = provider.base_url.as_str();
  if url.contains(":11434") {
    Some(Local::Ollama)
  } else if url.contains(":1234") {
    Some(Local::LmStudio)
  } else {
    None
  }
}

/// The server root of an OpenAI-compatible local base URL (`http://127.0.0.1:11434/v1` → `http://127.0.0.1:11434`)
fn server_root(provider: &Provider) -> String {
  let url = provider.base_url.trim().trim_end_matches('/');
  url.strip_suffix("/v1").unwrap_or(url).to_owned()
}

/// Ollama's per-model details: the trained context length and the capability list
fn ollama_show(http: &ureq::Agent, root: &str, l: &mut Listed) {
  let res = http.post(&format!("{root}/api/show")).header("content-type", "application/json").send(json!({ "model": l.id }).to_string());
  let Ok(info) = read_json(res) else { return };
  apply_ollama_show(&info, l);
}

pub fn apply_ollama_show(info: &Value, l: &mut Listed) {
  if l.context.is_none() {
    l.context = info.get("model_info").and_then(Value::as_object).and_then(|m| m.iter().find(|(k, _)| k.ends_with(".context_length")).and_then(|(_, v)| uint(Some(v))));
  }
  let caps = strings(info.get("capabilities"));
  if !caps.is_empty() {
    l.images = Some(caps.iter().any(|c| c == "vision"));
    l.tools = Some(caps.iter().any(|c| c == "tools"));
    if caps.iter().any(|c| c == "thinking") {
      l.reasoning = Some(true);
    }
    if !caps.iter().any(|c| c == "completion") {
      l.not_chat = true;
    }
  }
}

/// A free check of the address and key: the model list answers. Returns how many models it lists
pub fn check(http: &ureq::Agent, provider: &Provider, api_key: Option<&str>) -> Result<usize, LlmError> {
  list_raw(http, provider, api_key).map(|l| l.len())
}

/// What a test call returned
#[derive(Debug, Clone, PartialEq)]
pub struct Tested {
  pub ms: u64,
  pub text: String,
  pub usage: Option<Usage>,
  pub stop: StopReason,
}

/// One real, tiny call to a model (it spends a few tokens): proves the model id, the format and the key together
pub async fn test(http: ureq::Agent, provider: &Provider, model: &ProviderModel, api_key: Option<&str>) -> Result<Tested, LlmError> {
  let endpoint = Endpoint::of(provider, model, api_key).ok_or_else(|| LlmError::Protocol(format!("the API format \"{}\" is not supported", provider.format)))?;
  let request = Request {
    model: model.id.clone(),
    system: String::new(),
    items: vec![Item::User(vec![Part::Text("Reply with the single word OK.".into())])],
    tools: vec![],
    // A few tokens rather than one: some servers refuse a limit below their minimum
    max_tokens: Some(16),
    output_limit: Some(16),
    sampling: Default::default(),
    thinking: Thinking::Off,
    effort: None,
    serial_tools: false,
  };
  let started = Instant::now();
  let mut rx = super::stream(http, endpoint, request);
  while let Some(ev) = rx.recv().await {
    if let Event::Done { reply, stop, usage } = ev? {
      return Ok(Tested { ms: started.elapsed().as_millis() as u64, text: reply.text, usage, stop });
    }
  }
  Err(LlmError::Protocol("the stream ended without a result".into()))
}

/// A model server running on this machine
#[derive(Debug, Clone, PartialEq)]
pub struct LocalServer {
  /// The preset id (`ollama`, `lmstudio`)
  pub preset: &'static str,
  pub name: &'static str,
  /// The OpenAI-compatible base URL to save
  pub base_url: String,
  pub models: Vec<ProviderModel>,
}

/// Look for Ollama (11434) and LM Studio (1234) on the loopback address. Direct connections with short timeouts: a
/// proxy must not see loopback traffic, and a closed port answers at once
pub fn probe_local(catalog: &Catalog) -> Vec<LocalServer> {
  let http: ureq::Agent = ureq::Agent::config_builder()
    .http_status_as_error(false)
    .proxy(None)
    .timeout_connect(Some(Duration::from_millis(500)))
    .timeout_global(Some(Duration::from_secs(10)))
    .build()
    .into();
  probe_at(&http, catalog, &[("ollama", "Ollama", "http://127.0.0.1:11434"), ("lmstudio", "LM Studio", "http://127.0.0.1:1234")])
}

/// `probe_local` against given roots (tests point it at a mock server)
pub fn probe_at(http: &ureq::Agent, catalog: &Catalog, servers: &[(&'static str, &'static str, &str)]) -> Vec<LocalServer> {
  let mut out = vec![];
  for &(preset, name, root) in servers {
    let base_url = format!("{root}/v1");
    let provider: Provider = serde_json::from_value(json!({ "id": preset, "preset": preset, "baseUrl": base_url })).expect("a minimal provider parses");
    if let Ok(models) = discover(http, &provider, None, catalog) {
      out.push(LocalServer { preset, name, base_url, models });
    }
  }
  out
}
