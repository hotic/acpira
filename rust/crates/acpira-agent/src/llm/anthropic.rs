//! Anthropic Messages, streamed, and the servers that speak it (DeepSeek, Kimi, GLM and MiniMax offer such endpoints).
//! An assistant message's content blocks are kept as returned (`Native`) and replayed verbatim to the same endpoint
//! and model, since signed thinking blocks must come back unmodified within a tool loop. Where the family caches by
//! breakpoint, `cache_control` marks the family's breakpoints. Usage counts cached prompt tokens into `input`, as the
//! OpenAI format does. Wire shapes from Anthropic's public docs (2026-10); not yet checked against a real endpoint

use std::collections::HashMap;
use std::io::BufReader;

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use acpira_shared::providers::Thinking;

use super::family::{AnthropicThinking, Breakpoint, CacheKind, Family, ReasoningEcho};
use super::sse::SseReader;
use super::{Endpoint, Event, Item, LlmError, Native, Part, Reply, Request, StopReason, ToolCall, Usage, error_message, retry_after};

pub const VERSION: &str = "2023-06-01";
/// `max_tokens` is required; this one goes out when neither the user nor the catalogue knows the model's limit
const DEFAULT_MAX_TOKENS: u64 = 16_384;
/// The smallest thinking budget the API takes, and the room kept for the answer after it
const MIN_BUDGET: u64 = 1024;

/// What a replayed message must have come from: blocks signed by one model are not valid for another
pub fn origin(url: &str, model: &str) -> String {
  format!("{url} {model}")
}

fn is_thinking(block: &Value) -> bool {
  matches!(block.get("type").and_then(Value::as_str), Some("thinking" | "redacted_thinking"))
}

/// The request body for the endpoint at `origin`
pub fn body(req: &Request, family: &Family, origin: &str) -> Value {
  let turn_start = req.items.iter().rposition(|i| matches!(i, Item::User(_))).unwrap_or(0);
  let mut messages: Vec<Value> = vec![];
  for (i, item) in req.items.iter().enumerate() {
    match item {
      Item::User(parts) => messages.push(json!({ "role": "user", "content": user_blocks(parts) })),
      Item::Assistant { text, tool_calls, native, .. } => {
        let echo = match family.echo {
          ReasoningEcho::Never => false,
          ReasoningEcho::CurrentTurn => i > turn_start,
          ReasoningEcho::Always | ReasoningEcho::ThinkTags => true,
        };
        let blocks: Vec<Value> = match native.as_ref().filter(|n| n.origin == origin) {
          Some(n) => n
            .blocks
            .iter()
            // An empty text block is refused on the way back
            .filter(|b| !(b["type"] == "text" && b["text"].as_str().is_none_or(str::is_empty)))
            .filter(|b| echo || !is_thinking(b))
            .cloned()
            .collect(),
          // Another endpoint's (or format's) message: its text and calls, without reasoning that cannot be verified
          None => {
            let mut out = vec![];
            if !text.is_empty() {
              out.push(json!({ "type": "text", "text": text }));
            }
            for c in tool_calls {
              out.push(json!({ "type": "tool_use", "id": c.id, "name": c.name, "input": input_of(&c.arguments) }));
            }
            out
          }
        };
        // An empty reply leaves no message; the API joins the user messages around it
        if !blocks.is_empty() {
          messages.push(json!({ "role": "assistant", "content": blocks }));
        }
      }
      Item::ToolResult { call_id, content, is_error, .. } => {
        let mut block = json!({ "type": "tool_result", "tool_use_id": call_id, "content": if content.is_empty() { "(no output)" } else { content } });
        if *is_error {
          block["is_error"] = Value::Bool(true);
        }
        // Results of one round share the user message that follows the calls
        match messages.last_mut() {
          Some(m) if m["role"] == "user" && m["content"].as_array().is_some_and(|c| c.iter().all(|b| b["type"] == "tool_result")) => {
            m["content"].as_array_mut().unwrap().push(block);
          }
          _ => messages.push(json!({ "role": "user", "content": [block] })),
        }
      }
    }
  }
  let breakpoints: &[Breakpoint] = if family.cache == CacheKind::Breakpoints { family.breakpoints } else { &[] };
  let mut system = json!([{ "type": "text", "text": req.system }]);
  for bp in breakpoints {
    match bp {
      Breakpoint::System => system[0]["cache_control"] = ephemeral(),
      Breakpoint::LastMessage => mark_last_block(messages.last_mut()),
      Breakpoint::PreviousRequest => {
        // The user message before the latest assistant message ended the previous request of this turn
        let last_assistant = messages.iter().rposition(|m| m["role"] == "assistant");
        let prev = last_assistant.and_then(|a| messages[..a].iter().rposition(|m| m["role"] == "user"));
        mark_last_block(prev.map(|p| &mut messages[p]));
      }
    }
  }
  let max_tokens = req.max_tokens.or(req.output_limit).unwrap_or(DEFAULT_MAX_TOKENS);
  let mut body = Map::new();
  body.insert("model".into(), Value::from(req.model.as_str()));
  body.insert("max_tokens".into(), Value::from(max_tokens));
  body.insert("system".into(), system);
  body.insert("messages".into(), Value::Array(messages));
  if !req.tools.is_empty() {
    body.insert("tools".into(), req.tools.iter().map(|t| json!({ "name": t.name, "description": t.description, "input_schema": t.parameters })).collect());
  }
  body.insert("stream".into(), Value::Bool(true));
  let thinking = apply_thinking(&mut body, family, req.thinking, req.effort.as_deref(), max_tokens);
  // Thinking refuses a changed temperature and top_k
  if let Some(t) = req.sampling.temperature.filter(|_| !thinking) {
    body.insert("temperature".into(), Value::from(t));
  }
  if let Some(p) = req.sampling.top_p {
    body.insert("top_p".into(), Value::from(p));
  }
  if let Some(k) = req.sampling.top_k.filter(|_| !thinking) {
    body.insert("top_k".into(), Value::from(k));
  }
  Value::Object(body)
}

fn ephemeral() -> Value {
  json!({ "type": "ephemeral" })
}

fn mark_last_block(message: Option<&mut Value>) {
  if let Some(last) = message.and_then(|m| m["content"].as_array_mut()).and_then(|c| c.last_mut()).filter(|b| !is_thinking(b)) {
    last["cache_control"] = ephemeral();
  }
}

/// A call's arguments as the `input` object; text that is not an object (a malformed call) goes back as an empty one
fn input_of(arguments: &str) -> Value {
  serde_json::from_str::<Value>(arguments).ok().filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn user_blocks(parts: &[Part]) -> Vec<Value> {
  let blocks: Vec<Value> = parts
    .iter()
    .filter_map(|p| match p {
      // Empty text blocks are refused
      Part::Text(t) if t.trim().is_empty() => None,
      Part::Text(t) => Some(json!({ "type": "text", "text": t })),
      Part::Image { mime, data } => Some(json!({ "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } })),
    })
    .collect();
  if blocks.is_empty() { vec![json!({ "type": "text", "text": "(empty message)" })] } else { blocks }
}

/// The budget for an effort level; levels outside the usual four take the largest
fn budget_for(effort: Option<&str>) -> u64 {
  match effort {
    Some("minimal" | "low") => 4096,
    None | Some("medium") => 10_240,
    Some("high") => 24_576,
    Some(_) => 32_768,
  }
}

/// Add the thinking switch; true when thinking is on
pub fn apply_thinking(body: &mut Map<String, Value>, family: &Family, thinking: Thinking, effort: Option<&str>, max_tokens: u64) -> bool {
  match (thinking, effort) {
    (Thinking::Off, _) => {
      body.insert("thinking".into(), json!({ "type": "disabled" }));
      false
    }
    // No switch and no level: the provider's default
    (Thinking::Auto, None) => false,
    _ => match family.anthropic {
      AnthropicThinking::Adaptive => {
        body.insert("thinking".into(), json!({ "type": "adaptive" }));
        if let Some(e) = effort {
          body.insert("output_config".into(), json!({ "effort": e }));
        }
        true
      }
      AnthropicThinking::Budget => {
        // The budget counts toward max_tokens and must leave room for the answer
        let budget = budget_for(effort).min(max_tokens.saturating_sub(MIN_BUDGET));
        if budget < MIN_BUDGET {
          return false;
        }
        body.insert("thinking".into(), json!({ "type": "enabled", "budget_tokens": budget }));
        true
      }
    },
  }
}

/// Folds stream events into events for the turn and the final reply
pub struct Assembler {
  origin: String,
  reply: Reply,
  /// Content blocks by index, as they will be replayed
  blocks: Vec<Value>,
  /// Tool input JSON being streamed, by block index
  inputs: HashMap<usize, String>,
  usage: Map<String, Value>,
  stop: Option<String>,
  done: bool,
}

impl Assembler {
  pub fn new(origin: String) -> Assembler {
    Assembler { origin, reply: Reply::default(), blocks: vec![], inputs: HashMap::new(), usage: Map::new(), stop: None, done: false }
  }

  fn merge_usage(&mut self, u: Option<&Value>) {
    for (k, v) in u.and_then(Value::as_object).into_iter().flatten() {
      if v.is_u64() {
        self.usage.insert(k.clone(), v.clone());
      }
    }
  }

  pub fn event(&mut self, v: &Value, emit: &mut dyn FnMut(Event)) -> Result<(), LlmError> {
    let index = v.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
      "message_start" => self.merge_usage(v.pointer("/message/usage")),
      "content_block_start" => {
        let block = v.get("content_block").cloned().unwrap_or_else(|| json!({}));
        match block["type"].as_str() {
          Some("tool_use") => {
            let (id, name) = (block["id"].as_str().unwrap_or("").to_owned(), block["name"].as_str().unwrap_or("").to_owned());
            emit(Event::ToolCallStart { id, name });
            self.inputs.insert(index, String::new());
          }
          Some("text") => self.text(block["text"].as_str().unwrap_or(""), emit),
          Some("thinking") => self.reasoning(block["thinking"].as_str().unwrap_or(""), emit),
          _ => {}
        }
        if self.blocks.len() <= index {
          self.blocks.resize(index + 1, Value::Null);
        }
        self.blocks[index] = block;
      }
      "content_block_delta" => {
        let d = v.get("delta").unwrap_or(&Value::Null);
        let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("");
        let Some(block) = self.blocks.get_mut(index).filter(|b| !b.is_null()) else {
          return Err(LlmError::Protocol(format!("a delta for content block {index}, which never started")));
        };
        match d.get("type").and_then(Value::as_str).unwrap_or("") {
          "text_delta" => {
            append(block, "text", s("text"));
            self.text(s("text"), emit);
          }
          "thinking_delta" => {
            append(block, "thinking", s("thinking"));
            self.reasoning(s("thinking"), emit);
          }
          "signature_delta" => append(block, "signature", s("signature")),
          "input_json_delta" => self.inputs.entry(index).or_default().push_str(s("partial_json")),
          _ => {}
        }
      }
      "content_block_stop" => {
        if let Some(raw) = self.inputs.get(&index)
          && let Some(block) = self.blocks.get_mut(index)
          && !raw.trim().is_empty()
        {
          block["input"] = input_of(raw);
        }
      }
      "message_delta" => {
        if let Some(r) = v.pointer("/delta/stop_reason").and_then(Value::as_str) {
          self.stop = Some(r.to_owned());
        }
        self.merge_usage(v.get("usage"));
      }
      "message_stop" => self.done = true,
      "error" => {
        let message = v.pointer("/error/message").and_then(Value::as_str).unwrap_or("the provider reported an error");
        if self.salvage(message) {
          return Ok(());
        }
        return Err(LlmError::Api(message.to_owned()));
      }
      // ping and event types added later
      _ => {}
    }
    Ok(())
  }

  /// Keep a reply cut while a later tool call was streaming: blocks stream strictly in order, so every block before the
  /// last started one is complete, and running the finished calls moves the turn on where a retry would likely make the
  /// same calls and break the same way (a Claude gateway route ends the stream at a tool call with empty input,
  /// 2026-10-11). Only when at least one finished tool call precedes an unfinished one
  fn salvage(&mut self, message: &str) -> bool {
    let Some(last) = self.blocks.iter().rposition(|b| !b.is_null()) else { return false };
    if self.blocks[last]["type"] != "tool_use" || !self.blocks[..last].iter().any(|b| b["type"] == "tool_use") {
      return false;
    }
    self.blocks.truncate(last);
    self.inputs.remove(&last);
    // The finished blocks never got their stop: their inputs go into the replayed blocks here
    for (i, raw) in &self.inputs {
      if let Some(block) = self.blocks.get_mut(*i)
        && !raw.trim().is_empty()
      {
        block["input"] = input_of(raw);
      }
    }
    self.reply.cut = Some(message.to_owned());
    self.stop = Some("tool_use".into());
    self.done = true;
    true
  }

  fn text(&mut self, t: &str, emit: &mut dyn FnMut(Event)) {
    if !t.is_empty() {
      self.reply.text.push_str(t);
      emit(Event::Text(t.to_owned()));
    }
  }

  fn reasoning(&mut self, t: &str, emit: &mut dyn FnMut(Event)) {
    if !t.is_empty() {
      self.reply.reasoning.push_str(t);
      emit(Event::Reasoning(t.to_owned()));
    }
  }

  pub fn finish(mut self, emit: &mut dyn FnMut(Event)) -> Result<(), LlmError> {
    if !self.done && self.stop.is_none() {
      return Err(LlmError::Protocol("the stream ended before the model finished".into()));
    }
    for (i, b) in self.blocks.iter().enumerate() {
      if b["type"] != "tool_use" {
        continue;
      }
      // The arguments as streamed (a malformed call reaches the turn as written); a server that sent the input whole
      // at the start has no deltas
      let streamed = self.inputs.get(&i).cloned().filter(|s| !s.trim().is_empty());
      let arguments = streamed.unwrap_or_else(|| b.get("input").filter(|v| v.is_object()).map(Value::to_string).unwrap_or_else(|| "{}".into()));
      self.reply.tool_calls.push(ToolCall { id: b["id"].as_str().unwrap_or("").to_owned(), name: b["name"].as_str().unwrap_or("").to_owned(), arguments });
    }
    let calls = !self.reply.tool_calls.is_empty();
    let stop = match self.stop.as_deref() {
      Some("tool_use") => StopReason::ToolUse,
      Some("end_turn" | "stop_sequence") | None if calls => StopReason::ToolUse,
      Some("end_turn" | "stop_sequence") | None => StopReason::EndTurn,
      Some("max_tokens" | "model_context_window_exceeded") => StopReason::MaxTokens,
      Some("refusal") => StopReason::Refusal,
      Some(other) => StopReason::Other(other.to_owned()),
    };
    let n = |k: &str| self.usage.get(k).and_then(Value::as_u64).unwrap_or(0);
    let usage = (!self.usage.is_empty()).then(|| {
      let (read, write) = (n("cache_read_input_tokens"), n("cache_creation_input_tokens"));
      Usage { input: n("input_tokens") + read + write, output: n("output_tokens"), cache_read: read, cache_write: write, reasoning: 0 }
    });
    self.blocks.retain(|b| !b.is_null());
    self.reply.native = Some(Native { origin: self.origin, blocks: self.blocks });
    emit(Event::Done { reply: self.reply, stop, usage });
    Ok(())
  }
}

fn append(block: &mut Value, key: &str, text: &str) {
  let prev = block.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
  block[key] = Value::String(prev + text);
}

/// The headers of a call. Anthropic takes the key as `x-api-key`; the compatible endpoints document a bearer token, so
/// they get both. The source's own headers replace these by name
fn headers(endpoint: &Endpoint) -> Vec<(String, String)> {
  let mut out: Vec<(String, String)> = vec![
    ("content-type".into(), "application/json".into()),
    ("accept".into(), "text/event-stream".into()),
    ("anthropic-version".into(), VERSION.into()),
  ];
  if let Some(key) = &endpoint.api_key {
    out.push(("x-api-key".into(), key.clone()));
    if !endpoint.url.contains("api.anthropic.com") {
      out.push(("authorization".into(), format!("Bearer {key}")));
    }
  }
  for (k, v) in &endpoint.headers {
    out.retain(|(name, _)| !name.eq_ignore_ascii_case(k));
    out.push((k.clone(), v.clone()));
  }
  out
}

/// One streamed call on the calling (blocking) thread
pub fn stream(http: &ureq::Agent, endpoint: &Endpoint, req: &Request, tx: &mpsc::UnboundedSender<Result<Event, LlmError>>) -> Result<(), LlmError> {
  let origin = origin(&endpoint.url, &req.model);
  let payload = body(req, &endpoint.family, &origin);
  let mut call = http.post(&endpoint.url);
  for (k, v) in headers(endpoint) {
    call = call.header(k.as_str(), v.as_str());
  }
  let res = call.send(payload.to_string()).map_err(|e| LlmError::Network(e.to_string()))?;
  let status = res.status().as_u16();
  if !(200..300).contains(&status) {
    let wait = retry_after(res.headers());
    let mut body = res.into_body();
    let text = body.with_config().limit(64 * 1024).read_to_string().unwrap_or_default();
    return Err(LlmError::Http { status, message: error_message(&text), retry_after: wait });
  }
  let mut events = SseReader::new(BufReader::new(res.into_body().into_reader()));
  let mut asm = Assembler::new(origin);
  let mut emit = |e: Event| {
    let _ = tx.send(Ok(e));
  };
  loop {
    let ev = match events.next_event() {
      Ok(Some(ev)) => ev,
      Ok(None) => break,
      Err(e) => return Err(LlmError::Network(e.to_string())),
    };
    let data = ev.data.trim();
    // Some compatible servers end with OpenAI's marker
    if data == "[DONE]" {
      break;
    }
    if data.is_empty() {
      continue;
    }
    let v: Value = serde_json::from_str(data).map_err(|e| LlmError::Protocol(format!("{e}: {}", data.chars().take(200).collect::<String>())))?;
    asm.event(&v, &mut emit)?;
    if asm.done {
      break;
    }
    // A closed receiver means the turn is gone: stop reading, which drops the connection
    if tx.is_closed() {
      return Ok(());
    }
  }
  asm.finish(&mut emit)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::llm::ToolSpec;
  use crate::llm::family::{GENERIC, by_name};
  use acpira_shared::providers::Sampling;

  const ORIGIN: &str = "https://api.anthropic.com/v1/messages claude-x";

  fn claude() -> Family {
    by_name("claude").unwrap()
  }

  fn req(items: Vec<Item>) -> Request {
    Request {
      model: "claude-x".into(),
      system: "sys".into(),
      items,
      tools: vec![ToolSpec { name: "read".into(), description: "Read".into(), parameters: json!({ "type": "object" }) }],
      max_tokens: None,
      output_limit: None,
      sampling: Sampling::default(),
      thinking: Thinking::Auto,
      effort: None,
      cache_key: None,
    }
  }

  fn user(t: &str) -> Item {
    Item::User(vec![Part::Text(t.into())])
  }

  fn result(id: &str, content: &str) -> Item {
    Item::ToolResult { call_id: id.into(), name: "read".into(), content: content.into(), is_error: false }
  }

  fn signed_call(id: &str, origin: &str) -> Item {
    Item::Assistant {
      text: String::new(),
      reasoning: "think".into(),
      tool_calls: vec![ToolCall { id: id.into(), name: "read".into(), arguments: "{\"path\":\"a\"}".into() }],
      native: Some(Native {
        origin: origin.into(),
        blocks: vec![
          json!({ "type": "thinking", "thinking": "think", "signature": "sig" }),
          json!({ "type": "text", "text": "" }),
          json!({ "type": "tool_use", "id": id, "name": "read", "input": { "path": "a" } }),
        ],
      }),
    }
  }

  #[test]
  fn signed_blocks_replay_to_their_own_model_and_results_share_a_message() {
    let items = vec![user("one"), signed_call("t1", ORIGIN), result("t1", "x"), result("t1b", "")];
    let b = body(&req(items.clone()), &claude(), ORIGIN);
    let msgs = b["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[1]["content"][0], json!({ "type": "thinking", "thinking": "think", "signature": "sig" }));
    assert_eq!(msgs[1]["content"].as_array().unwrap().len(), 2, "the empty text block is dropped");
    assert_eq!(msgs[2]["content"][1]["content"], "(no output)");
    assert_eq!(b["tools"][0], json!({ "name": "read", "description": "Read", "input_schema": { "type": "object" } }));
    assert_eq!(b["max_tokens"], DEFAULT_MAX_TOKENS);
    // Another model: no thinking, the call rebuilt from its arguments
    let other = body(&req(items), &claude(), "https://other/v1/messages m");
    assert_eq!(other["messages"][1]["content"], json!([{ "type": "tool_use", "id": "t1", "name": "read", "input": { "path": "a" } }]));
  }

  #[test]
  fn breakpoints_mark_the_system_and_the_last_two_request_ends() {
    let items = vec![user("one"), signed_call("t1", ORIGIN), result("t1", "x"), signed_call("t2", ORIGIN), result("t2", "y")];
    let b = body(&req(items.clone()), &claude(), ORIGIN);
    assert_eq!(b["system"][0]["cache_control"], json!({ "type": "ephemeral" }));
    let msgs = b["messages"].as_array().unwrap();
    let marked: Vec<usize> = msgs.iter().enumerate().filter(|(_, m)| m["content"].as_array().unwrap().iter().any(|c| c.get("cache_control").is_some())).map(|(i, _)| i).collect();
    assert_eq!(marked, [2, 4], "the previous request's end and this one's");
    // A family with implicit caching gets no markers
    let ds = body(&req(items), &by_name("deepseek").unwrap(), ORIGIN);
    assert!(!ds.to_string().contains("cache_control"));
  }

  #[test]
  fn thinking_per_family_style_and_sampling_it_refuses() {
    let mut r = req(vec![user("hi")]);
    r.effort = Some("high".into());
    r.sampling.temperature = Some(0.2);
    r.sampling.top_k = Some(40);
    let b = body(&r, &claude(), ORIGIN);
    assert_eq!((b["thinking"].clone(), b["output_config"].clone()), (json!({ "type": "adaptive" }), json!({ "effort": "high" })));
    assert!(b.get("temperature").is_none() && b.get("top_k").is_none());
    r.output_limit = Some(8000);
    let b = body(&r, &GENERIC, ORIGIN);
    assert_eq!((b["thinking"].clone(), b["max_tokens"].clone()), (json!({ "type": "enabled", "budget_tokens": 8000 - MIN_BUDGET }), json!(8000)));
    r.thinking = Thinking::Off;
    let b = body(&r, &GENERIC, ORIGIN);
    assert_eq!((b["thinking"].clone(), b["temperature"].clone()), (json!({ "type": "disabled" }), json!(0.2)));
    r.thinking = Thinking::Auto;
    r.effort = None;
    assert!(body(&r, &claude(), ORIGIN).get("thinking").is_none());
  }

  fn run(events: &[Value]) -> (Vec<Event>, Result<(), LlmError>) {
    let mut asm = Assembler::new(ORIGIN.into());
    let mut out = vec![];
    for e in events {
      if let Err(err) = asm.event(e, &mut |x| out.push(x)) {
        return (out, Err(err));
      }
    }
    let r = asm.finish(&mut |x| out.push(x));
    (out, r)
  }

  #[test]
  fn a_stream_with_thinking_text_and_a_tool_call() {
    let (events, r) = run(&[
      json!({ "type": "message_start", "message": { "usage": { "input_tokens": 10, "cache_read_input_tokens": 80, "cache_creation_input_tokens": 5, "output_tokens": 1 } } }),
      json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "" } }),
      json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "Look first." } }),
      json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "signature_delta", "signature": "abc" } }),
      json!({ "type": "content_block_stop", "index": 0 }),
      json!({ "type": "content_block_start", "index": 1, "content_block": { "type": "text", "text": "" } }),
      json!({ "type": "content_block_delta", "index": 1, "delta": { "type": "text_delta", "text": "Reading." } }),
      json!({ "type": "content_block_stop", "index": 1 }),
      json!({ "type": "content_block_start", "index": 2, "content_block": { "type": "tool_use", "id": "toolu_1", "name": "read", "input": {} } }),
      json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "{\"path\":" } }),
      json!({ "type": "content_block_delta", "index": 2, "delta": { "type": "input_json_delta", "partial_json": "\"a.rs\"}" } }),
      json!({ "type": "content_block_stop", "index": 2 }),
      json!({ "type": "ping" }),
      json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" }, "usage": { "output_tokens": 42 } }),
      json!({ "type": "message_stop" }),
    ]);
    r.unwrap();
    assert!(events.contains(&Event::ToolCallStart { id: "toolu_1".into(), name: "read".into() }));
    assert!(events.contains(&Event::Reasoning("Look first.".into())) && events.contains(&Event::Text("Reading.".into())));
    let Some(Event::Done { reply, stop, usage }) = events.last() else { panic!() };
    assert_eq!(*stop, StopReason::ToolUse);
    assert_eq!(*usage, Some(Usage { input: 95, output: 42, cache_read: 80, cache_write: 5, reasoning: 0 }));
    assert_eq!(reply.tool_calls, vec![ToolCall { id: "toolu_1".into(), name: "read".into(), arguments: "{\"path\":\"a.rs\"}".into() }]);
    let native = reply.native.as_ref().unwrap();
    assert_eq!(native.blocks[0], json!({ "type": "thinking", "thinking": "Look first.", "signature": "abc" }));
    assert_eq!(native.blocks[2]["input"], json!({ "path": "a.rs" }));
    // What came back replays as it was
    let item = Item::Assistant { text: reply.text.clone(), reasoning: reply.reasoning.clone(), tool_calls: reply.tool_calls.clone(), native: reply.native.clone() };
    let b = body(&req(vec![user("go"), item, result("toolu_1", "x")]), &claude(), ORIGIN);
    assert_eq!(b["messages"][1]["content"].as_array().unwrap(), &native.blocks);
  }

  #[test]
  fn errors_cuts_and_stop_reasons() {
    let (_, r) = run(&[json!({ "type": "error", "error": { "type": "overloaded_error", "message": "Overloaded" } })]);
    assert_eq!(r, Err(LlmError::Api("Overloaded".into())));
    let (_, r) = run(&[json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "par" } })]);
    assert!(matches!(r, Err(LlmError::Protocol(_))));
    let (events, _) = run(&[json!({ "type": "message_delta", "delta": { "stop_reason": "max_tokens" } }), json!({ "type": "message_stop" })]);
    assert!(matches!(events.last(), Some(Event::Done { stop: StopReason::MaxTokens, usage: None, .. })));
  }

  #[test]
  fn compatible_endpoints_get_a_bearer_token_too_and_source_headers_win() {
    let mut e = Endpoint {
      format: acpira_shared::providers::ApiFormat::Anthropic,
      url: "https://api.anthropic.com/v1/messages".into(),
      api_key: Some("k".into()),
      headers: vec![],
      family: claude(),
    };
    let names = |e: &Endpoint| headers(e).into_iter().map(|(k, _)| k).collect::<Vec<_>>();
    assert!(!names(&e).contains(&"authorization".to_owned()));
    e.url = "https://api.deepseek.com/anthropic/v1/messages".into();
    e.headers = vec![("Anthropic-Version".into(), "2024-01-01".into())];
    let h = headers(&e);
    assert!(h.contains(&("authorization".into(), "Bearer k".into())));
    assert_eq!(h.iter().filter(|(k, _)| k.eq_ignore_ascii_case("anthropic-version")).count(), 1);
  }
}
