//! OpenAI Chat Completions and compatible servers, streamed. Covers `reasoning_content` / `reasoning` deltas, reasoning
//! wrapped in `<think>` at the start of the content (Ollama, MiniMax), tool-call deltas keyed by index, and usage with
//! the cache fields each provider spells differently

use std::io::BufReader;

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use super::family::{Family, ReasoningEcho, apply_thinking};
use super::sse::SseReader;
use super::{Endpoint, Event, Item, LlmError, Part, Reply, Request, StopReason, ToolCall, Usage, error_message, retry_after};

/// The request body
pub fn body(req: &Request, family: &Family) -> Value {
  let mut messages = vec![json!({ "role": "system", "content": req.system })];
  // Assistant messages after the latest user message belong to the running turn
  let turn_start = req.items.iter().rposition(|i| matches!(i, Item::User(_))).unwrap_or(0);
  for (i, item) in req.items.iter().enumerate() {
    match item {
      Item::User(parts) => messages.push(json!({ "role": "user", "content": user_content(parts) })),
      Item::Assistant { text, reasoning, tool_calls, .. } => {
        let echo = !reasoning.is_empty()
          && match family.echo {
            ReasoningEcho::Never => false,
            ReasoningEcho::CurrentTurn => i > turn_start,
            ReasoningEcho::Always | ReasoningEcho::ThinkTags => true,
          };
        let content = if echo && family.echo == ReasoningEcho::ThinkTags { format!("<think>\n{reasoning}\n</think>\n\n{text}") } else { text.clone() };
        let mut m = Map::new();
        m.insert("role".into(), "assistant".into());
        // A tool-call message without text carries null content (some servers refuse an empty string next to tool_calls)
        m.insert("content".into(), if content.is_empty() && !tool_calls.is_empty() { Value::Null } else { Value::String(content) });
        if echo && family.echo != ReasoningEcho::ThinkTags {
          m.insert("reasoning_content".into(), Value::String(reasoning.clone()));
        }
        if !tool_calls.is_empty() {
          m.insert(
            "tool_calls".into(),
            tool_calls
              .iter()
              .map(|c| json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.arguments } }))
              .collect(),
          );
        }
        messages.push(Value::Object(m));
      }
      Item::ToolResult { call_id, content, .. } => {
        messages.push(json!({ "role": "tool", "tool_call_id": call_id, "content": content }));
      }
    }
  }
  let mut body = Map::new();
  body.insert("model".into(), Value::from(req.model.as_str()));
  body.insert("messages".into(), Value::Array(messages));
  if !req.tools.is_empty() {
    body.insert(
      "tools".into(),
      req
        .tools
        .iter()
        .map(|t| json!({ "type": "function", "function": { "name": t.name, "description": t.description, "parameters": t.parameters } }))
        .collect(),
    );
    if req.serial_tools {
      body.insert("parallel_tool_calls".into(), Value::Bool(false));
    }
  }
  body.insert("stream".into(), Value::Bool(true));
  body.insert("stream_options".into(), json!({ "include_usage": true }));
  if let Some(n) = req.max_tokens {
    body.insert("max_tokens".into(), Value::from(n));
  }
  if let Some(t) = req.sampling.temperature {
    body.insert("temperature".into(), Value::from(t));
  }
  if let Some(p) = req.sampling.top_p {
    body.insert("top_p".into(), Value::from(p));
  }
  if let Some(k) = req.sampling.top_k {
    body.insert("top_k".into(), Value::from(k));
  }
  apply_thinking(&mut body, family, req.thinking, req.effort.as_deref());
  Value::Object(body)
}

fn user_content(parts: &[Part]) -> Value {
  // A text-only message stays a plain string: the widest-supported shape
  if let [Part::Text(t)] = parts {
    return Value::String(t.clone());
  }
  parts
    .iter()
    .map(|p| match p {
      Part::Text(t) => json!({ "type": "text", "text": t }),
      Part::Image { mime, data } => json!({ "type": "image_url", "image_url": { "url": format!("data:{mime};base64,{data}") } }),
    })
    .collect()
}

/// Usage from any of the shapes compatible servers send
pub fn parse_usage(u: &Value) -> Usage {
  let n = |p: &str| u.pointer(p).and_then(Value::as_u64);
  let hit = n("/prompt_cache_hit_tokens");
  let miss = n("/prompt_cache_miss_tokens");
  let input = n("/prompt_tokens").or(n("/input_tokens")).or_else(|| Some(hit? + miss?)).unwrap_or(0);
  Usage {
    input,
    output: n("/completion_tokens").or(n("/output_tokens")).unwrap_or(0),
    // OpenAI / GLM / OpenRouter: prompt_tokens_details.cached_tokens; DeepSeek: prompt_cache_hit_tokens; Kimi: cached_tokens
    cache_read: n("/prompt_tokens_details/cached_tokens").or(hit).or(n("/cached_tokens")).or(n("/cache_read_input_tokens")).unwrap_or(0),
    cache_write: n("/prompt_tokens_details/cache_write_tokens").or(n("/cache_creation_input_tokens")).unwrap_or(0),
    reasoning: n("/completion_tokens_details/reasoning_tokens").unwrap_or(0),
  }
}

/// Splits reasoning wrapped in `<think>…</think>` at the very start of the content off the visible text
#[derive(Default)]
struct ThinkSplit {
  state: ThinkState,
  held: String,
}

#[derive(Default, PartialEq)]
enum ThinkState {
  /// Nothing but whitespace seen yet
  #[default]
  Start,
  Inside,
  Text,
}

const OPEN: &str = "<think>";
const CLOSE: &str = "</think>";

impl ThinkSplit {
  /// (reasoning, text) for one content delta
  fn push(&mut self, delta: &str) -> (String, String) {
    let mut reasoning = String::new();
    let mut text = String::new();
    self.held.push_str(delta);
    loop {
      match self.state {
        ThinkState::Start => {
          let trimmed = self.held.trim_start();
          if let Some(rest) = trimmed.strip_prefix(OPEN) {
            self.held = rest.to_owned();
            self.state = ThinkState::Inside;
          } else if trimmed.is_empty() || OPEN.starts_with(trimmed) {
            return (reasoning, text);
          } else {
            self.state = ThinkState::Text;
          }
        }
        ThinkState::Inside => {
          if let Some(end) = self.held.find(CLOSE) {
            reasoning.push_str(&self.held[..end]);
            self.held = self.held[end + CLOSE.len()..].trim_start().to_owned();
            self.state = ThinkState::Text;
          } else {
            // Hold back a tail that could be the start of the closing tag
            let keep = (1..CLOSE.len()).rev().find(|k| self.held.ends_with(&CLOSE[..*k])).unwrap_or(0);
            let cut = self.held.len() - keep;
            reasoning.push_str(&self.held[..cut]);
            self.held = self.held[cut..].to_owned();
            return (reasoning, text);
          }
        }
        ThinkState::Text => {
          text.push_str(&self.held);
          self.held.clear();
          return (reasoning, text);
        }
      }
    }
  }

  /// Whatever is still held at the end of the stream
  fn finish(&mut self) -> (String, String) {
    let held = std::mem::take(&mut self.held);
    match self.state {
      ThinkState::Inside => (held, String::new()),
      _ => (String::new(), held),
    }
  }
}

#[derive(Default)]
struct Partial {
  id: String,
  name: String,
  arguments: String,
  announced: bool,
}

/// Folds chunks into events and the final reply
#[derive(Default)]
pub struct Assembler {
  reply: Reply,
  calls: Vec<(Option<u64>, Partial)>,
  finish: Option<String>,
  usage: Option<Usage>,
  think: ThinkSplit,
  done: bool,
}

impl Assembler {
  pub fn chunk(&mut self, v: &Value, emit: &mut dyn FnMut(Event)) -> Result<(), LlmError> {
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
      return Err(LlmError::Api(error_message(&json!({ "error": err }).to_string())));
    }
    if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
      self.usage = Some(parse_usage(u));
    }
    for choice in v.get("choices").and_then(Value::as_array).into_iter().flatten() {
      let delta = choice.get("delta").or_else(|| choice.get("message")).unwrap_or(&Value::Null);
      let r = delta.get("reasoning_content").or_else(|| delta.get("reasoning")).and_then(Value::as_str).unwrap_or("");
      if !r.is_empty() {
        self.reply.reasoning.push_str(r);
        emit(Event::Reasoning(r.to_owned()));
      }
      if let Some(c) = delta.get("content").and_then(Value::as_str).filter(|c| !c.is_empty()) {
        let (r, t) = self.think.push(c);
        self.emit_split(r, t, emit);
      }
      for tc in delta.get("tool_calls").and_then(Value::as_array).into_iter().flatten() {
        self.tool_delta(tc, emit);
      }
      // Some servers repeat the usage inside the choice
      if let Some(u) = choice.get("usage").filter(|u| u.is_object()) {
        self.usage = Some(parse_usage(u));
      }
      if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
        self.finish = Some(f.to_owned());
      }
    }
    Ok(())
  }

  fn emit_split(&mut self, r: String, t: String, emit: &mut dyn FnMut(Event)) {
    if !r.is_empty() {
      self.reply.reasoning.push_str(&r);
      emit(Event::Reasoning(r));
    }
    if !t.is_empty() {
      self.reply.text.push_str(&t);
      emit(Event::Text(t));
    }
  }

  fn tool_delta(&mut self, tc: &Value, emit: &mut dyn FnMut(Event)) {
    let index = tc.get("index").and_then(Value::as_u64);
    let id = tc.get("id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let slot = match index {
      Some(i) => self.calls.iter().position(|(ix, _)| *ix == Some(i)),
      // No index: the same id continues a call, a new id (or none after a named one) starts the next
      None => match id {
        Some(id) => self.calls.iter().position(|(_, p)| p.id == id),
        None => self.calls.len().checked_sub(1),
      },
    };
    let slot = slot.unwrap_or_else(|| {
      self.calls.push((index, Partial::default()));
      self.calls.len() - 1
    });
    let n = slot;
    let p = &mut self.calls[slot].1;
    if let Some(id) = id
      && p.id.is_empty()
    {
      p.id = id.to_owned();
    }
    if let Some(name) = tc.pointer("/function/name").and_then(Value::as_str).filter(|s| !s.is_empty()) {
      // Most servers send the name once; a few repeat it whole on every delta
      if !p.name.ends_with(name) || p.name.is_empty() {
        p.name.push_str(name);
      }
    }
    if let Some(args) = tc.pointer("/function/arguments") {
      match args {
        Value::String(s) => p.arguments.push_str(s),
        // A server that sends parsed arguments sends them whole
        other if !other.is_null() => p.arguments = other.to_string(),
        _ => {}
      }
    }
    if !p.announced && !p.name.is_empty() {
      if p.id.is_empty() {
        p.id = format!("call_{n}");
      }
      p.announced = true;
      emit(Event::ToolCallStart { id: p.id.clone(), name: p.name.clone() });
    }
  }

  /// The stream ended (`[DONE]` when `done`, else the connection closed)
  pub fn finish(mut self, done: bool, emit: &mut dyn FnMut(Event)) -> Result<(), LlmError> {
    let (r, t) = self.think.finish();
    self.emit_split(r, t, emit);
    self.reply.tool_calls = self
      .calls
      .drain(..)
      .filter(|(_, p)| !p.name.is_empty())
      .enumerate()
      .map(|(n, (_, p))| ToolCall {
        id: if p.id.is_empty() { format!("call_{n}") } else { p.id },
        name: p.name,
        arguments: if p.arguments.trim().is_empty() { "{}".into() } else { p.arguments },
      })
      .collect();
    let calls = !self.reply.tool_calls.is_empty();
    let stop = match self.finish.as_deref() {
      Some("tool_calls" | "function_call") => StopReason::ToolUse,
      // Some servers finish a tool-call message with `stop`
      Some("stop" | "end_turn") if calls => StopReason::ToolUse,
      None if done && calls => StopReason::ToolUse,
      Some("stop" | "end_turn" | "eos") => StopReason::EndTurn,
      Some("length" | "max_tokens") => StopReason::MaxTokens,
      Some("content_filter" | "sensitive") => StopReason::Refusal,
      Some(other) => StopReason::Other(other.to_owned()),
      // [DONE] without a finish_reason: a lenient server; the connection closing without either is a broken stream
      None if done => StopReason::EndTurn,
      None => return Err(LlmError::Protocol("the stream ended before the model finished".into())),
    };
    self.done = true;
    emit(Event::Done { reply: self.reply, stop, usage: self.usage });
    Ok(())
  }
}

/// One streamed call on the calling (blocking) thread
pub fn stream(http: &ureq::Agent, endpoint: &Endpoint, req: &Request, tx: &mpsc::UnboundedSender<Result<Event, LlmError>>) -> Result<(), LlmError> {
  let payload = body(req, &endpoint.family);
  let mut call = http.post(&endpoint.url).header("content-type", "application/json").header("accept", "text/event-stream");
  if let Some(key) = &endpoint.api_key {
    call = call.header("authorization", format!("Bearer {key}"));
  }
  for (k, v) in &endpoint.headers {
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
  let mut asm = Assembler::default();
  let mut emit = |e: Event| {
    let _ = tx.send(Ok(e));
  };
  let mut done = false;
  loop {
    let ev = match events.next_event() {
      Ok(Some(ev)) => ev,
      Ok(None) => break,
      Err(e) => return Err(LlmError::Network(e.to_string())),
    };
    let data = ev.data.trim();
    if data == "[DONE]" {
      done = true;
      break;
    }
    if data.is_empty() {
      continue;
    }
    let v: Value = serde_json::from_str(data).map_err(|e| LlmError::Protocol(format!("{e}: {}", data.chars().take(200).collect::<String>())))?;
    asm.chunk(&v, &mut emit)?;
    // A closed receiver means the turn is gone: stop reading, which drops the connection
    if tx.is_closed() {
      return Ok(());
    }
  }
  asm.finish(done, &mut emit)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::llm::family::{GENERIC, by_name};
  use acpira_shared::providers::{Sampling, Thinking};

  fn req(items: Vec<Item>) -> Request {
    Request {
      model: "m".into(),
      system: "sys".into(),
      items,
      tools: vec![],
      max_tokens: None,
      output_limit: None,
      sampling: Sampling::default(),
      thinking: Thinking::Auto,
      effort: None,
      serial_tools: false,
    }
  }

  fn assistant(text: &str, reasoning: &str, calls: &[(&str, &str)]) -> Item {
    Item::Assistant {
      text: text.into(),
      reasoning: reasoning.into(),
      tool_calls: calls.iter().map(|(id, name)| ToolCall { id: (*id).into(), name: (*name).into(), arguments: "{}".into() }).collect(),
      native: None,
    }
  }

  #[test]
  fn reasoning_goes_back_only_where_the_family_wants_it() {
    let items = vec![
      Item::User(vec![Part::Text("one".into())]),
      assistant("done", "old thought", &[]),
      Item::User(vec![Part::Text("two".into())]),
      assistant("", "new thought", &[("c1", "read")]),
      Item::ToolResult { call_id: "c1".into(), name: "read".into(), content: "x".into(), is_error: false },
    ];
    let ds = body(&req(items.clone()), &by_name("deepseek").unwrap());
    let msgs = ds["messages"].as_array().unwrap();
    assert!(msgs[2].get("reasoning_content").is_none(), "an earlier turn's reasoning is dropped");
    assert_eq!(msgs[4]["reasoning_content"], "new thought");
    assert_eq!(msgs[4]["content"], Value::Null);
    assert_eq!(msgs[5], json!({ "role": "tool", "tool_call_id": "c1", "content": "x" }));
    let generic = body(&req(items.clone()), &GENERIC);
    assert!(generic["messages"].as_array().unwrap().iter().all(|m| m.get("reasoning_content").is_none()));
    let mm = body(&req(items), &by_name("minimax").unwrap());
    assert_eq!(mm["messages"][2]["content"], "<think>\nold thought\n</think>\n\ndone");
  }

  #[test]
  fn images_make_a_part_list() {
    let b = body(&req(vec![Item::User(vec![Part::Text("see".into()), Part::Image { mime: "image/png".into(), data: "AAA".into() }])]), &GENERIC);
    assert_eq!(b["messages"][1]["content"][1]["image_url"]["url"], "data:image/png;base64,AAA");
    assert_eq!(b["stream_options"]["include_usage"], true);
  }

  #[test]
  fn usage_cache_fields_per_provider() {
    let openai = parse_usage(&json!({ "prompt_tokens": 100, "completion_tokens": 7, "prompt_tokens_details": { "cached_tokens": 64 },
      "completion_tokens_details": { "reasoning_tokens": 3 } }));
    assert_eq!(openai, Usage { input: 100, output: 7, cache_read: 64, cache_write: 0, reasoning: 3 });
    let deepseek = parse_usage(&json!({ "prompt_cache_hit_tokens": 80, "prompt_cache_miss_tokens": 20, "completion_tokens": 5 }));
    assert_eq!((deepseek.input, deepseek.cache_read), (100, 80));
    let kimi = parse_usage(&json!({ "prompt_tokens": 50, "completion_tokens": 1, "cached_tokens": 40 }));
    assert_eq!(kimi.cache_read, 40);
    let openrouter = parse_usage(&json!({ "prompt_tokens": 50, "completion_tokens": 1,
      "prompt_tokens_details": { "cached_tokens": 10, "cache_write_tokens": 30 } }));
    assert_eq!((openrouter.cache_read, openrouter.cache_write), (10, 30));
  }

  fn run(chunks: &[Value], done: bool) -> (Vec<Event>, Result<(), LlmError>) {
    let mut asm = Assembler::default();
    let mut out = vec![];
    for c in chunks {
      if let Err(e) = asm.chunk(c, &mut |e| out.push(e)) {
        return (out, Err(e));
      }
    }
    let r = asm.finish(done, &mut |e| out.push(e));
    (out, r)
  }

  fn delta(d: Value) -> Value {
    json!({ "choices": [{ "index": 0, "delta": d }] })
  }

  #[test]
  fn tool_calls_assemble_across_deltas() {
    let (events, r) = run(
      &[
        delta(json!({ "reasoning_content": "hmm" })),
        delta(json!({ "content": "Let me look." })),
        delta(json!({ "tool_calls": [{ "index": 0, "id": "call_a", "type": "function", "function": { "name": "read", "arguments": "" } }] })),
        delta(json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "{\"path\":" } }] })),
        delta(json!({ "tool_calls": [{ "index": 0, "function": { "arguments": "\"a.rs\"}" } }, { "index": 1, "id": "call_b", "function": { "name": "bash", "arguments": "{}" } }] })),
        json!({ "choices": [{ "index": 0, "delta": {}, "finish_reason": "tool_calls" }] }),
        json!({ "choices": [], "usage": { "prompt_tokens": 10, "completion_tokens": 2 } }),
      ],
      true,
    );
    r.unwrap();
    assert!(events.contains(&Event::ToolCallStart { id: "call_a".into(), name: "read".into() }));
    let Some(Event::Done { reply, stop, usage }) = events.last() else { panic!("no Done") };
    assert_eq!(*stop, StopReason::ToolUse);
    assert_eq!(reply.reasoning, "hmm");
    assert_eq!(reply.tool_calls[0].arguments, "{\"path\":\"a.rs\"}");
    assert_eq!(reply.tool_calls[1].name, "bash");
    assert_eq!(usage.unwrap().input, 10);
  }

  #[test]
  fn think_tags_split_even_across_chunk_boundaries() {
    let (events, r) =
      run(&[delta(json!({ "content": " <thi" })), delta(json!({ "content": "nk>plan it</th" })), delta(json!({ "content": "ink>\n\nAnswer" }))], true);
    r.unwrap();
    let Some(Event::Done { reply, .. }) = events.last() else { panic!() };
    assert_eq!((reply.reasoning.as_str(), reply.text.as_str()), ("plan it", "Answer"));
    let (events, _) = run(&[delta(json!({ "content": "a < b" }))], true);
    let Some(Event::Done { reply, .. }) = events.last() else { panic!() };
    assert_eq!(reply.text, "a < b");
  }

  #[test]
  fn a_stream_cut_before_the_finish_is_an_error_and_in_band_errors_surface() {
    let (_, r) = run(&[delta(json!({ "content": "par" }))], false);
    assert!(matches!(r, Err(LlmError::Protocol(_))));
    let (_, r) = run(&[json!({ "error": { "message": "overloaded" } })], false);
    assert_eq!(r, Err(LlmError::Api("overloaded".into())));
  }
}
