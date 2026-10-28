//! One LLM request: the log of what its cacheable prefix is built from, the size estimate, and the retries of a
//! failed or empty reply with the notices the client sees meanwhile

use serde_json::{Value, json};

use acpira_shared::providers::ProviderModel;

use super::DEFAULT_CONTEXT;
use super::setup::Setup;
use crate::acp::Session;
use crate::llm::{self, Item, LlmError, Part, Request, StopReason, ToolSpec, Usage};

/// Record what the request's prefix is built from, and a `view` event when that changed since the last request: a
/// changed prompt, tool set or model is where a provider's prompt cache stops matching, and so is a switch between the
/// standard and the fast tier (Anthropic documents that a speed change invalidates the cache). The tool set counts as
/// changed when a definition does, not only a name: an MCP server's relisting after `tools/list_changed` (acpira's
/// `ask_agent` when the roster is edited) keeps the names and rewrites a description and an enum
pub(super) fn log_view(session: &Session, s: &Setup, request: &Request) {
  let view = json!({
    "model": s.pick,
    "prompt": { "variant": s.prompt.0, "version": s.prompt.1, "digest": s.prompt.2 },
    "tools": request.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
    "toolsDigest": tools_digest(&request.tools),
    "fast": request.fast,
  });
  let prev = session.state.lock().last_view.replace(view.clone());
  if let Some(prev) = prev
    && prev != view
  {
    let tools_changed = prev["tools"] != view["tools"] || prev["toolsDigest"] != view["toolsDigest"];
    let changed: Vec<&str> =
      ["model", "prompt", "tools", "fast"].into_iter().filter(|k| if *k == "tools" { tools_changed } else { prev[k] != view[k] }).collect();
    session.store.append(json!({ "type": "view", "changed": changed, "from": prev, "to": view }));
  }
}

/// A short digest of the tool definitions as the request sends them (name, description, parameters, in order)
fn tools_digest(tools: &[crate::llm::ToolSpec]) -> String {
  use sha2::{Digest, Sha256};
  let mut h = Sha256::new();
  for t in tools {
    for part in [t.name.as_str(), t.description.as_str(), &t.parameters.to_string()] {
      h.update(part.as_bytes());
      h.update([0]);
    }
  }
  h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
}

/// What every `request` event carries: the model, its source and the prompt and policies the call was built with, and
/// `fast` when the call asked for the fast tier
pub(super) fn request_event(s: &Setup) -> Value {
  let mut ev = json!({
    "type": "request",
    "model": s.pick,
    "source": s.provider.id,
    "format": s.provider.format,
    "prompt": { "variant": s.prompt.0, "version": s.prompt.1, "digest": s.prompt.2 },
    // Context policies in effect: the tool output budget and the compaction threshold
    "policies": {
      "outputBudget": { "lines": crate::budget::MAX_LINES, "bytes": crate::budget::MAX_BYTES },
      "compaction": { "auto": s.policy.auto, "threshold": s.policy.threshold(s.model.context.unwrap_or(DEFAULT_CONTEXT), s.model.output) },
    },
  });
  if s.fast {
    ev["fast"] = json!({});
  }
  ev
}

/// A model call still open when the turn's future is dropped (a cancel) is logged on the way out with stop
/// `Cancelled`, so the log keeps one record per call, including the ones a provider may bill without an answer
pub(super) struct Unfinished<'a> {
  pub(super) session: &'a Session,
  pub(super) event: Option<Value>,
  pub(super) started: std::time::Instant,
}

impl Drop for Unfinished<'_> {
  fn drop(&mut self) {
    if let Some(mut ev) = self.event.take() {
      ev["ms"] = json!(self.started.elapsed().as_millis() as u64);
      ev["stop"] = json!("Cancelled");
      self.session.store.append(ev);
    }
  }
}

/// A stand-in for a prompt count the provider reported as 0, and the context right after a compaction: about four
/// bytes a token over the text sent, a thousand per image. Rough, and only ever used until a provider reports
pub(super) fn approx(system: &str, items: &[Item], tools: &[ToolSpec]) -> u64 {
  let (mut bytes, mut images) = (system.len(), 0u64);
  for item in items {
    match item {
      Item::User(parts) => {
        for p in parts {
          match p {
            Part::Text(t) => bytes += t.len(),
            Part::Image { .. } => images += 1,
          }
        }
      }
      Item::Assistant { text, tool_calls, .. } => bytes += text.len() + tool_calls.iter().map(|c| c.name.len() + c.arguments.len()).sum::<usize>(),
      Item::ToolResult { content, images: shown, .. } => {
        bytes += content.len();
        images += shown.len() as u64;
      }
    }
  }
  bytes += tools.iter().map(|t| t.name.len() + t.description.len() + t.parameters.to_string().len()).sum::<usize>();
  (bytes as u64).div_ceil(4) + images * 1000
}

/// How long a stream may go without an event before it counts as stalled: Codex's default stream idle timeout. A
/// gateway route was seen holding a GPT call open for six minutes without a byte (2026-10-11).
/// `ACPIRA_AGENT_STREAM_IDLE_SECS` overrides it (tests, diagnosis)
pub(super) fn stream_idle() -> std::time::Duration {
  static IDLE: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
  *IDLE.get_or_init(|| {
    let secs = std::env::var("ACPIRA_AGENT_STREAM_IDLE_SECS").ok().and_then(|v| v.parse().ok()).filter(|s| *s > 0).unwrap_or(300);
    std::time::Duration::from_secs(secs)
  })
}

/// Attempts per model call: the first and up to three retries
pub(super) const MAX_ATTEMPTS: u32 = 4;

/// Why an empty reply is retried, also the turn's error once the attempts run out
pub(super) const EMPTY_REPLY: &str = "the model ended its reply without any text or tool call";

/// A reply that ends the turn with nothing to show: no text, no tool call, at most reasoning. A gateway that converts
/// Gemini's `functionCall` buffers the whole call and, past its own timeout, sends only the reasoning and then a normal
/// `response.completed` (`gemini-4-argon` on the Responses gateway, 2026-10-22, after 130 s and 156 s while writing a
/// long file); Gemini CLI retries such a stream too (`InvalidStreamError`). An output limit or a refusal is its own
/// stop and is left alone
pub(super) fn empty_reply(reply: &llm::Reply, stop: &StopReason) -> bool {
  !matches!(stop, StopReason::MaxTokens | StopReason::Refusal) && reply.cut.is_none() && reply.tool_calls.is_empty() && reply.text.trim().is_empty()
}

/// Worth another attempt: the connection, the stream or the service failed, not the request itself. A gateway ending a
/// 200 stream with an `error` event ("response stream interrupted", seen on 2026-10-11 on a Claude route, 1 call in 40
/// or so) is the common case; a rejected key, a bad request or a context overflow would only fail again
pub(super) fn retriable(e: &LlmError) -> bool {
  match e {
    LlmError::Http { status, .. } => matches!(status, 408 | 409 | 425 | 429 | 500..=599),
    LlmError::Network(_) | LlmError::Protocol(_) => true,
    LlmError::Rejected(_) => false,
    LlmError::Api(m) => {
      let m = m.to_ascii_lowercase();
      !(m.contains("invalid_request") || m.contains("context") || m.contains("too long") || m.contains("authentication"))
    }
  }
}

/// The wait before the next attempt: the provider's Retry-After when given (at most a minute), else 1, 2, 4 s
pub(super) fn retry_delay(e: &LlmError, attempt: u32) -> std::time::Duration {
  match e {
    LlmError::Http { retry_after: Some(d), .. } => (*d).min(std::time::Duration::from_secs(60)),
    _ => std::time::Duration::from_secs(1 << (attempt - 1).min(5)),
  }
}

/// A retry in progress, in the shape the host already labels for other agents (an AIR `sessionFailure` warning
/// titled "attempt N of M", see `retry_of_failure` on the host): the working label shows it until the stream resumes
pub(super) fn retry_notice(source: &str, e: &LlmError, next: u32) -> Value {
  let category = match e {
    LlmError::Http { status: 429, .. } => "limit",
    LlmError::Http { .. } | LlmError::Api(_) | LlmError::Rejected(_) => "service",
    LlmError::Network(_) | LlmError::Protocol(_) => "connection",
  };
  json!({
    "sessionUpdate": "session_info_update",
    "_meta": { "jetbrains": { "air": { "sessionFailure": {
      "id": format!("acpira-retry-{}", uuid::Uuid::new_v4()),
      "revision": 1,
      "severity": "warning",
      "category": category,
      "title": format!("Retrying {source}, attempt {next} of {MAX_ATTEMPTS}"),
      "details": e.to_string(),
      "actions": [],
    } } } },
  })
}

/// One `request` event per model call (a retried call has one per attempt, `attempt` from 2 on)
#[allow(clippy::too_many_arguments, reason = "one argument per field of the request event, gathered from different points of the call")]
pub(super) fn log_request(
  session: &Session,
  s: &Setup,
  started: std::time::Instant,
  attempt: u32,
  stop: Option<&StopReason>,
  usage: Option<&Usage>,
  input_estimated: bool,
  error: Option<&str>,
  reply: Option<&llm::Reply>,
) {
  let mut ev = request_event(s);
  // A fast request is billed at the fast tier unless the provider says it served the standard one
  let fast_billed = s.fast && reply.and_then(|r| llm::fast::served_fast(r.served.as_deref())).unwrap_or(true);
  if let Some(served) = reply.and_then(|r| r.served.as_deref()).filter(|_| s.fast) {
    ev["fast"]["served"] = Value::from(served);
  }
  ev["ms"] = json!(started.elapsed().as_millis() as u64);
  if attempt > 1 {
    ev["attempt"] = json!(attempt);
  }
  if let Some(u) = usage {
    ev["usage"] = json!({ "input": u.input, "output": u.output, "cacheRead": u.cache_read, "cacheWrite": u.cache_write, "reasoning": u.reasoning });
    if input_estimated {
      ev["usage"]["inputEstimated"] = Value::Bool(true);
    }
    // At list prices: an estimate, not the bill (discounts, tiers and gateways' markups are not known here)
    if let Some(c) = &s.cost {
      let factor = if fast_billed { llm::fast::PRICE_FACTOR } else { 1.0 };
      ev["cost"] = json!(c.estimate(u.input, u.output, u.cache_read, u.cache_write) * factor);
    }
  }
  if let Some(stop) = stop {
    ev["stop"] = Value::String(format!("{stop:?}"));
  }
  if let Some(e) = error {
    ev["error"] = Value::String(e.to_owned());
  }
  // A reply kept up to the call that was streaming when the provider broke it off (`Assembler::salvage`)
  if let Some(c) = reply.and_then(|r| r.cut.as_deref()) {
    ev["cut"] = Value::String(c.to_owned());
  }
  session.store.append(ev);
}

/// The output limit sent with a request: only one the user (or the endpoint) gave, never a guessed one
pub(super) fn max_tokens(model: &ProviderModel) -> Option<u64> {
  model.output.filter(|_| !model.estimated.iter().any(|e| e == "output"))
}

/// A failed call as the turn's error text
pub(super) fn describe(source: &str, e: &LlmError) -> String {
  match e {
    LlmError::Http { status: 401 | 403, message, .. } => format!("{source} rejected the API key ({message}). Check the key in Acpira's settings."),
    LlmError::Http { status: 404, message, .. } => format!("{source}: not found ({message}). Check the base URL and the model id."),
    other => format!("{source}: {other}"),
  }
}
