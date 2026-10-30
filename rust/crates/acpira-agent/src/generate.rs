//! One model call with no session, tools or history, from the same model sources the built-in agent uses
//! (`providers.json` under its home): what a plugin's `model/generate` asks for (docs/dev/plugins.md, Session API)

use std::path::Path;
use std::time::Duration;

use acpira_shared::providers::{Thinking, model_pick};

use crate::config::ConfigCache;
use crate::llm::{self, Endpoint, Event, Item, LlmError, Part, Request};

/// The most a call may answer when the caller names no limit
const DEFAULT_MAX_TOKENS: u64 = 2048;
const TIMEOUT: Duration = Duration::from_secs(120);

/// A call's answer
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
  pub text: String,
  /// The model that answered, as `provider/model`
  pub model: String,
  pub input_tokens: Option<u64>,
  pub output_tokens: Option<u64>,
}

/// Ask `pick` (or the fast model, else the first usable one) once. `home` is the built-in agent's home
pub async fn generate(home: &Path, pick: Option<&str>, system: &str, prompt: &str, max_tokens: Option<u64>) -> Result<Generated, String> {
  let config = ConfigCache::new(home.to_path_buf()).get();
  let catalog = crate::catalog::CatalogCache::new(home).get();
  let found = match pick {
    Some(p) => config.find(p).ok_or_else(|| format!("no usable model `{p}` in the model sources"))?,
    None => config.fast_model(&catalog).or_else(|| config.default_pick().and_then(|p| config.find(&p))).ok_or("no model source is set up for Acpira's built-in agent")?,
  };
  let (provider, model) = found;
  let endpoint = Endpoint::of(provider, model, config.api_key(&provider.id)).ok_or_else(|| format!("`{}` has no key or address", provider.id))?;
  let limit = max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
  let request = Request {
    model: model.id.clone(),
    system: system.to_owned(),
    items: vec![Item::User(vec![Part::Text(prompt.to_owned())])],
    tools: vec![],
    max_tokens: Some(model.output.map_or(limit, |o| o.min(limit))),
    output_limit: Some(limit),
    sampling: Default::default(),
    thinking: Thinking::Off,
    effort: None,
    cache_key: None,
    fast: false,
  };
  let name = model_pick(&provider.id, &model.id);
  let call = async {
    let mut rx = llm::stream(llm::default_http(), endpoint, request);
    while let Some(ev) = rx.recv().await {
      if let Event::Done { reply, usage, .. } = ev? {
        return Ok((reply.text, usage));
      }
    }
    Err(LlmError::Protocol("the stream ended without a result".into()))
  };
  let (text, usage) = tokio::time::timeout(TIMEOUT, call).await.map_err(|_| "the model did not answer in time".to_owned())?.map_err(|e| e.to_string())?;
  Ok(Generated { text, model: name, input_tokens: usage.map(|u| u.input), output_tokens: usage.map(|u| u.output) })
}
