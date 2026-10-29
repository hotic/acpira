//! Model discovery against recorded list shapes (OpenAI ids only, OpenRouter, Anthropic, Gemini, LM Studio, Ollama) and
//! against the scripted server: the three metadata tiers, `estimated`, Anthropic pages and headers, the free check, the
//! paid test call and the local probe
#![allow(clippy::unwrap_used, reason = "test code")]

use serde_json::{Value, json};

use acpira_agent::llm::discover::{self, DEFAULT_CONTEXT, Listed};
use acpira_agent::mock::{self, MockModel, Reply};
use acpira_shared::model_catalog::{Catalog, trim_models_dev};
use acpira_shared::providers::{Provider, ProviderModel};

fn catalog() -> Catalog {
  let api = json!({ "deepseek": { "models": {
    "deepseek-v4-flash": { "name": "DeepSeek V4 Flash", "reasoning": true, "tool_call": true,
      "reasoning_options": [{ "type": "effort", "values": ["low", "high", "max"] }],
      "limit": { "context": 1000000, "output": 393216 }, "modalities": { "input": ["text"] } } } },
    "google": { "models": { "gemini-3.8-flash": { "name": "Gemini 3.8 Flash", "limit": { "context": 1048576, "output": 65536 },
      "modalities": { "input": ["text", "image"] } } } } });
  Catalog::new(trim_models_dev(&api, "2026-10-10T00:00:00Z", None))
}

fn provider(format: &str, base_url: &str, preset: &str) -> Provider {
  serde_json::from_value(json!({ "id": "src", "preset": preset, "format": format, "baseUrl": base_url })).unwrap()
}

fn listed(entry: Value) -> Listed {
  discover::parse_entry(&entry).unwrap()
}

fn model(entry: Value) -> ProviderModel {
  discover::complete(&listed(entry), &catalog())
}

#[test]
fn an_id_only_listing_takes_the_catalogue_then_the_defaults_and_says_so() {
  // DeepSeek's own /models: ids only
  let known = model(json!({ "id": "deepseek-v4-flash", "object": "model", "owned_by": "deepseek" }));
  assert_eq!((known.context, known.output), (Some(1000000), Some(393216)));
  assert_eq!(known.input, ["text"]);
  assert_eq!(known.efforts, ["low", "high", "max"]);
  assert_eq!(known.estimated, ["context", "output", "input", "efforts"]);
  assert_eq!(known.name.as_deref(), Some("DeepSeek V4 Flash"));
  let unknown = model(json!({ "id": "my-finetune", "object": "model" }));
  assert_eq!((unknown.context, unknown.output), (Some(DEFAULT_CONTEXT), None));
  assert_eq!(unknown.input, ["text"], "images are off unless someone says otherwise");
  assert!(unknown.efforts.is_empty());
  assert_eq!(unknown.estimated, ["context", "input"]);
}

#[test]
fn openrouter_metadata_is_taken_as_reported() {
  let m = model(json!({
    "id": "deepseek/deepseek-v4-flash", "name": "DeepSeek: DeepSeek V4 Flash", "context_length": 163840,
    "architecture": { "input_modalities": ["text", "image"], "output_modalities": ["text"] },
    "top_provider": { "context_length": 163840, "max_completion_tokens": 65536 },
    "supported_parameters": ["tools", "tool_choice", "reasoning", "include_reasoning", "max_tokens"]
  }));
  assert_eq!((m.context, m.output), (Some(163840), Some(65536)));
  assert_eq!(m.input, ["text", "image"]);
  assert_eq!(m.name.as_deref(), Some("DeepSeek: DeepSeek V4 Flash"));
  // Only the efforts came from the catalogue (matched through the gateway prefix)
  assert_eq!(m.estimated, ["efforts"]);
  let l = listed(json!({ "id": "x/y", "supported_parameters": ["max_tokens"], "architecture": { "input_modalities": ["text"] } }));
  assert_eq!((l.tools, l.images, l.reasoning), (Some(false), Some(false), None));
}

#[test]
fn anthropic_gemini_and_lm_studio_shapes() {
  let a = listed(json!({ "type": "model", "id": "claude-opus-5-5", "display_name": "Claude Opus 5.5", "created_at": "2026-05-01T00:00:00Z",
    "max_input_tokens": 1000000, "max_tokens": 128000 }));
  assert_eq!((a.name.as_deref(), a.context, a.output), (Some("Claude Opus 5.5"), Some(1000000), Some(128000)));
  // Gemini's native list: `models/` names, token limits, generation methods
  let g = listed(json!({ "name": "models/gemini-3.8-flash", "displayName": "Gemini 3.8 Flash", "inputTokenLimit": 1048576,
    "outputTokenLimit": 65536, "supportedGenerationMethods": ["generateContent", "countTokens"] }));
  assert_eq!((g.id.as_str(), g.context, g.output, g.not_chat), ("gemini-3.8-flash", Some(1048576), Some(65536), false));
  let embed = listed(json!({ "name": "models/text-embedding-005", "supportedGenerationMethods": ["embedContent"] }));
  assert!(embed.not_chat);
  let gm = discover::complete(&g, &catalog());
  assert_eq!(gm.input, ["text", "image"], "image input from the catalogue");
  assert_eq!(gm.estimated, ["input"]);
  // LM Studio's /api/v0/models: the loaded window wins over the maximum, `vlm` takes images, embeddings are dropped
  let lms = listed(json!({ "id": "qwen3-vl-8b", "object": "model", "type": "vlm", "max_context_length": 262144, "loaded_context_length": 32768,
    "capabilities": ["tool_use"] }));
  assert_eq!((lms.context, lms.images, lms.tools), (Some(32768), Some(true), Some(true)));
  assert!(listed(json!({ "id": "text-embedding-nomic", "type": "embeddings" })).not_chat);
}

#[test]
fn a_gateway_reasoning_flag_without_catalogue_levels_gets_the_default_levels() {
  // The asgard gateway's /v1/models entry for a model newer than the catalogue (trimmed; recorded 2026-10-14)
  let entry = json!({ "id": "gemini-4-argon", "object": "model", "owned_by": "antigravity", "name": "Gemini 4 Argon",
    "reasoning": true, "supportsReasoning": true, "supportsThinking": true,
    "capabilities": { "context_length": 1000000, "max_output_tokens": 65536, "supports_reasoning": true, "supports_vision": true },
    "modalities": { "input": ["text", "image", "video", "audio", "pdf"], "output": ["text"] },
    "limit": { "context": 1000000, "output": 65536 }, "contextLength": 1000000, "maxOutputTokens": 65536,
    "settings": { "extendParams": ["thinkingLevel"] } });
  let l = listed(entry.clone());
  assert_eq!((l.context, l.output, l.images, l.reasoning), (Some(1000000), Some(65536), Some(true), Some(true)));
  let responses = provider("openai-responses", "https://gw.example/v1", "custom");
  let m = discover::complete_for(&responses, &l, &catalog());
  assert_eq!(m.efforts, discover::DEFAULT_EFFORTS);
  assert_eq!(m.estimated, ["efforts"]);
  // Each spelling of the flag counts on its own
  for flag in [json!({ "supportsThinking": true }), json!({ "capabilities": { "supports_reasoning": true } })] {
    let mut e = flag;
    e["id"] = json!("new-model");
    assert_eq!(listed(e).reasoning, Some(true));
  }
  // Catalogue levels win over the defaults (OpenRouter's flag on a model the catalogue knows)
  let known = listed(json!({ "id": "deepseek-v4-flash", "supported_parameters": ["reasoning"] }));
  assert_eq!(discover::complete_for(&responses, &known, &catalog()).efforts, ["low", "high", "max"]);
  // On Chat a level only reaches the wire where the family's switch takes one: Kimi's does not, DeepSeek's does
  let chat = provider("openai-chat", "https://gw.example/v1", "custom");
  let kimi = listed(json!({ "id": "kimi-k9-thinking", "reasoning": true }));
  assert!(discover::complete_for(&chat, &kimi, &catalog()).efforts.is_empty());
  let deepseek = listed(json!({ "id": "deepseek-v9", "reasoning": true }));
  assert_eq!(discover::complete_for(&chat, &deepseek, &catalog()).efforts, discover::DEFAULT_EFFORTS);
  // Silence is not a flag
  let plain = listed(json!({ "id": "plain-model", "reasoning": false }));
  assert!(discover::complete_for(&responses, &plain, &catalog()).efforts.is_empty());
}

#[test]
fn levels_the_endpoint_lists_are_taken_as_reported() {
  // The asgard gateway's /v1/models entry for Kimi K3.1 Pro (trimmed; recorded 2026-10-29): both spellings of the scale
  let entry = json!({ "id": "kimi-k3.1-pro", "name": "Kimi K3.1 Pro", "reasoning": true, "context_length": 1048576, "max_output_tokens": 131072,
    "capabilities": { "supports_reasoning": true, "reasoning": { "supported": true, "effort": ["low", "high", "max"], "default": "max" } },
    "supported_reasoning_levels": [{ "effort": "low", "description": "Fast responses with lighter reasoning" }, { "effort": "high" }, { "effort": "max" }],
    "default_reasoning_level": "max" });
  let l = listed(entry);
  assert_eq!(l.efforts, Some(vec!["low".to_owned(), "high".into(), "max".into()]));
  let responses = provider("openai-responses", "https://gw.example/v1", "custom");
  let m = discover::complete_for(&responses, &l, &catalog());
  assert_eq!(m.efforts, ["low", "high", "max"]);
  assert!(!m.estimated.iter().any(|e| e == "efforts"), "{:?}", m.estimated);
  // Codex's shape on its own flags the model as reasoning, and levels outside the usual scale stay
  let codex = listed(json!({ "id": "gpt-x", "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "ULTRA" }] }));
  assert_eq!((codex.efforts, codex.reasoning), (Some(vec!["low".to_owned(), "ultra".into()]), Some(true)));
  // The endpoint wins over the catalogue's low / high / max
  let ds = listed(json!({ "id": "deepseek-v4-flash", "capabilities": { "reasoning": { "effort": ["high", "max"] } } }));
  assert_eq!(discover::complete(&ds, &catalog()).efforts, ["high", "max"]);
  // An empty list is no list
  assert_eq!(listed(json!({ "id": "y", "supported_reasoning_levels": [] })).efforts, None);
}

#[test]
fn a_gateway_id_finds_the_catalogue_through_the_listed_name() {
  let m = model(json!({ "id": "ds-flash", "name": "DeepSeek V4 Flash" }));
  assert_eq!((m.context, m.efforts.clone()), (Some(1000000), vec!["low".to_owned(), "high".into(), "max".into()]));
  assert!(m.estimated.iter().any(|e| e == "efforts"));
}

#[test]
fn ollama_show_fills_the_window_and_capabilities() {
  let mut l = listed(json!({ "id": "qwen3:8b", "object": "model", "owned_by": "library" }));
  discover::apply_ollama_show(
    &json!({ "model_info": { "general.architecture": "qwen3", "qwen3.context_length": 40960 }, "capabilities": ["completion", "tools", "thinking"] }),
    &mut l,
  );
  assert_eq!((l.context, l.images, l.tools, l.reasoning), (Some(40960), Some(false), Some(true), Some(true)));
  let mut e = listed(json!({ "id": "nomic-embed-text:latest" }));
  assert!(e.not_chat);
  e.not_chat = false;
  discover::apply_ollama_show(&json!({ "capabilities": ["embedding"] }), &mut e);
  assert!(e.not_chat);
}

#[test]
fn the_models_url_follows_the_base_or_the_full_endpoint() {
  let mut p = provider("openai-chat", "https://api.example.com/v1/", "custom");
  assert_eq!(discover::models_url(&p).as_deref(), Some("https://api.example.com/v1/models"));
  p.full_url = true;
  p.base_url = "https://gw.example.com/v2/chat/completions".into();
  assert_eq!(discover::models_url(&p).as_deref(), Some("https://gw.example.com/v2/models"));
  // Every wire format's own endpoint
  for (format, path) in [("openai-responses", "responses"), ("anthropic", "messages")] {
    let q = Provider { format: format.into(), base_url: format!("https://gw.example.com/v2/{path}"), ..p.clone() };
    assert_eq!(discover::models_url(&q).as_deref(), Some("https://gw.example.com/v2/models"), "{format}");
  }
  p.base_url = "https://gw.example.com/invoke".into();
  assert_eq!(discover::models_url(&p), None);
}

fn ok(body: Value) -> Reply {
  Reply::Status(200, body.to_string())
}

#[tokio::test]
async fn anthropic_lists_follow_pages_with_its_headers() {
  let server = MockModel::start();
  server.push(ok(json!({ "data": [{ "type": "model", "id": "m-a" }], "has_more": true, "first_id": "m-a", "last_id": "m-a" })));
  server.push(ok(json!({ "data": [{ "type": "model", "id": "m-b", "display_name": "B" }], "has_more": false, "last_id": "m-b" })));
  let p = provider("anthropic", &server.base_url(), "custom");
  let http = acpira_agent::llm::default_http();
  let models = tokio::task::spawn_blocking(move || discover::discover(&http, &p, Some("sk-ant"), &catalog())).await.unwrap().unwrap();
  assert_eq!(models.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m-a", "m-b"]);
  let reqs = server.requests();
  assert_eq!(reqs[0].path, "/v1/models?limit=1000");
  assert_eq!(reqs[1].path, "/v1/models?limit=1000&after_id=m-a");
  assert_eq!(reqs[0].header("x-api-key"), Some("sk-ant"));
  assert_eq!(reqs[0].header("anthropic-version"), Some("2023-06-01"));
  // Not api.anthropic.com: gateways that take Anthropic's format usually want a bearer token too
  assert_eq!(reqs[0].header("authorization"), Some("Bearer sk-ant"));
}

#[tokio::test]
async fn the_check_is_the_model_list_and_a_rejected_key_says_why() {
  let server = MockModel::start();
  server.push(ok(json!({ "object": "list", "data": [{ "id": "a" }, { "id": "b" }, { "id": "text-embedding-3" }] })));
  server.push(Reply::Status(401, r#"{"error":{"message":"Invalid API key"}}"#.into()));
  let p = provider("openai-chat", &server.base_url(), "custom");
  let http = acpira_agent::llm::default_http();
  let (n, err) = tokio::task::spawn_blocking(move || (discover::check(&http, &p, Some("sk")), discover::check(&http, &p, Some("bad")))).await.unwrap();
  assert_eq!(n.unwrap(), 3);
  assert_eq!(err.unwrap_err().to_string(), "HTTP 401: Invalid API key");
  assert_eq!(server.requests()[0].header("authorization"), Some("Bearer sk"));
}

#[tokio::test]
async fn a_test_call_is_one_tiny_request() {
  let server = MockModel::start();
  server.push(mock::text("OK"));
  let p = provider("openai-chat", &server.base_url(), "custom");
  let t = discover::test(acpira_agent::llm::default_http(), &p, &ProviderModel::new("m1"), Some("sk")).await.unwrap();
  assert_eq!(t.text, "OK");
  assert!(t.usage.is_some());
  let body = &server.requests()[0].body;
  assert_eq!(body["max_tokens"], 16);
  assert!(body.get("tools").is_none_or(|t| t.as_array().is_some_and(|a| a.is_empty())));
}

#[tokio::test]
async fn the_local_probe_lists_an_ollama_server_with_its_details() {
  let ollama = MockModel::start();
  ollama.push(ok(json!({ "object": "list", "data": [{ "id": "qwen3:8b", "object": "model" }, { "id": "nomic-embed-text:latest" }] })));
  ollama.push(ok(json!({ "model_info": { "qwen3.context_length": 40960 }, "capabilities": ["completion", "tools"] })));
  let root = ollama.base_url().trim_end_matches("/v1").to_owned();
  let closed = {
    // A port nothing listens on
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    format!("http://127.0.0.1:{}", l.local_addr().unwrap().port())
  };
  let http = acpira_agent::llm::default_http();
  let found = tokio::task::spawn_blocking(move || discover::probe_at(&http, &catalog(), &[("ollama", "Ollama", &root), ("lmstudio", "LM Studio", &closed)]))
    .await
    .unwrap();
  assert_eq!(found.len(), 1, "only the server that answered");
  assert_eq!(found[0].preset, "ollama");
  assert!(found[0].base_url.ends_with("/v1"));
  let m = &found[0].models;
  assert_eq!(m.len(), 1, "the embedding model is left out");
  assert_eq!(m[0].context, Some(40960));
  assert_eq!(m[0].estimated, ["context"], "the trained window is not the served num_ctx");
  let paths: Vec<String> = ollama.requests().iter().map(|r| r.path.clone()).collect();
  assert_eq!(paths, ["/v1/models", "/api/show"]);
}
