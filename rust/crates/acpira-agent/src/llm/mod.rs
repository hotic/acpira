//! Model clients. One provider-neutral conversation model (`Item`), one streaming entry point (`stream`), and one module
//! per wire format. HTTP is blocking ureq on the blocking pool: the caller hands in the `ureq::Agent` (the agent process
//! uses the default, which reads the proxy variables the engine injects; the host passes its own proxy-aware one). A
//! dropped receiver ends the reading thread at its next chunk

pub mod anthropic;
pub mod discover;
pub mod family;
pub mod openai_chat;
pub mod presets;
pub mod sse;

use std::time::Duration;

use serde_json::Value;
use tokio::sync::mpsc;

use acpira_shared::providers::{ApiFormat, Provider, ProviderModel, Sampling, Thinking};

/// A user message part
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
  Text(String),
  /// Base64 data and its mime type
  Image { mime: String, data: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
  pub id: String,
  pub name: String,
  /// The arguments as the model wrote them (JSON text, possibly malformed)
  pub arguments: String,
}

/// An assistant message as a wire format returned it, replayed verbatim to the same endpoint and model (Anthropic's
/// signed thinking blocks must come back unmodified within a tool loop, and are not valid for another model)
#[derive(Debug, Clone, PartialEq)]
pub struct Native {
  /// The endpoint URL and model id that produced the blocks
  pub origin: String,
  pub blocks: Vec<Value>,
}

/// One conversation entry, in the order the model saw it
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
  User(Vec<Part>),
  Assistant {
    text: String,
    /// Visible reasoning, kept so families that need it back get it
    reasoning: String,
    tool_calls: Vec<ToolCall>,
    /// The provider's own content blocks, when its format has any to replay
    native: Option<Native>,
  },
  ToolResult {
    call_id: String,
    name: String,
    content: String,
    is_error: bool,
  },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
  pub name: String,
  pub description: String,
  pub parameters: Value,
}

/// Everything one model call needs, provider-neutral
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
  pub model: String,
  pub system: String,
  pub items: Vec<Item>,
  pub tools: Vec<ToolSpec>,
  /// Sent only when the user (or the endpoint) gave a real limit, never a guessed one
  pub max_tokens: Option<u64>,
  /// The model's output limit, estimated or not: a format that requires a limit (Anthropic) sends it when `max_tokens`
  /// is None
  pub output_limit: Option<u64>,
  pub sampling: Sampling,
  pub thinking: Thinking,
  pub effort: Option<String>,
  /// At most one tool call per reply: for a route that breaks a reply holding several (`turn.rs`, the retry)
  pub serial_tools: bool,
}

/// Token usage of one call. `input` counts every prompt token, cached ones included
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
  pub input: u64,
  pub output: u64,
  pub cache_read: u64,
  pub cache_write: u64,
  pub reasoning: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
  EndTurn,
  ToolUse,
  MaxTokens,
  /// Refused or filtered by the provider
  Refusal,
  Other(String),
}

/// The finished assistant message of one call
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Reply {
  pub text: String,
  pub reasoning: String,
  pub tool_calls: Vec<ToolCall>,
  pub native: Option<Native>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
  Text(String),
  Reasoning(String),
  /// A tool call's name is known (arguments still streaming)
  ToolCallStart { id: String, name: String },
  Done { reply: Reply, stop: StopReason, usage: Option<Usage> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum LlmError {
  /// A non-2xx answer
  Http { status: u16, message: String, retry_after: Option<Duration> },
  /// Connecting, sending or reading failed
  Network(String),
  /// The stream broke the protocol (bad JSON, ended without a finish)
  Protocol(String),
  /// The provider reported an error inside a 200 stream
  Api(String),
}

impl std::fmt::Display for LlmError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    match self {
      LlmError::Http { status, message, .. } if message.is_empty() => write!(f, "HTTP {status}"),
      LlmError::Http { status, message, .. } => write!(f, "HTTP {status}: {message}"),
      LlmError::Network(m) => write!(f, "network error: {m}"),
      LlmError::Protocol(m) => write!(f, "bad stream: {m}"),
      LlmError::Api(m) => f.write_str(m),
    }
  }
}

impl std::error::Error for LlmError {}

/// Where and how to call a model
#[derive(Debug, Clone, PartialEq)]
pub struct Endpoint {
  pub format: ApiFormat,
  pub url: String,
  pub api_key: Option<String>,
  pub headers: Vec<(String, String)>,
  pub family: family::Family,
}

impl Endpoint {
  /// The endpoint of a configured model; None for a format this build does not speak
  pub fn of(provider: &Provider, model: &ProviderModel, api_key: Option<&str>) -> Option<Endpoint> {
    let format = provider.api_format()?;
    let base = provider.base_url.trim().trim_end_matches('/');
    let url = if provider.full_url {
      provider.base_url.trim().to_owned()
    } else {
      match format {
        ApiFormat::OpenaiChat => format!("{base}/chat/completions"),
        ApiFormat::Anthropic => format!("{base}/messages"),
      }
    };
    Some(Endpoint {
      format,
      url,
      api_key: api_key.map(str::to_owned),
      headers: provider.headers.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
      family: family::resolve(provider, model),
    })
  }
}

/// The HTTP client the agent process uses: proxy from the environment (the engine injects it), no overall timeout (a
/// long answer streams for minutes), bounded connect and time to first byte
pub fn default_http() -> ureq::Agent {
  ureq::Agent::config_builder()
    .http_status_as_error(false)
    .timeout_connect(Some(Duration::from_secs(30)))
    .timeout_recv_response(Some(Duration::from_secs(600)))
    .user_agent(format!("acpira/{}", env!("CARGO_PKG_VERSION")))
    .build()
    .into()
}

/// Start one streamed call; events arrive on the receiver, the last one is `Done` or an error. Dropping the receiver
/// stops the reading thread at its next chunk and drops the connection
pub fn stream(http: ureq::Agent, endpoint: Endpoint, request: Request) -> mpsc::UnboundedReceiver<Result<Event, LlmError>> {
  let (tx, rx) = mpsc::unbounded_channel();
  tokio::task::spawn_blocking(move || {
    let result = match endpoint.format {
      ApiFormat::OpenaiChat => openai_chat::stream(&http, &endpoint, &request, &tx),
      ApiFormat::Anthropic => anthropic::stream(&http, &endpoint, &request, &tx),
    };
    if let Err(e) = result {
      let _ = tx.send(Err(e));
    }
  });
  rx
}

/// Read an error body (bounded) into a message: the provider's `error.message` when it is JSON
pub(crate) fn error_message(body: &str) -> String {
  let parsed: Option<Value> = serde_json::from_str(body).ok();
  let from_json = parsed.as_ref().and_then(|v| {
    v.pointer("/error/message")
      .or_else(|| v.get("message"))
      .or_else(|| v.get("error").filter(|e| e.is_string()))
      .or_else(|| v.get("detail"))
      .and_then(Value::as_str)
      .map(str::to_owned)
  });
  from_json.unwrap_or_else(|| body.chars().take(500).collect::<String>().trim().to_owned())
}

pub(crate) fn retry_after(headers: &ureq::http::HeaderMap) -> Option<Duration> {
  headers.get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs)
}
