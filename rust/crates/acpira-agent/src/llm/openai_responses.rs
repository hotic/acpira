//! OpenAI Responses, streamed: the format Codex speaks, and the one that carries a reasoning model's thinking from one
//! step to the next. Requests are stateless (`store: false`); each step's output items (encrypted reasoning, messages,
//! function calls) are kept as `Native` blocks and replayed unchanged to the same endpoint and model, so the model
//! does not reason its way back to where it was. Function tools go out with `strict: false` (Responses defaults to
//! strict schemas, which refuse optional parameters) and parallel calls on. Wire shapes checked against a gateway
//! serving gpt-6.1-sol, swe-2 and gemini-3.8-flash on 2026-10-11

use std::collections::BTreeMap;
use std::io::BufReader;

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use acpira_shared::providers::Thinking;

use super::sse::SseReader;
use super::{Endpoint, Event, Item, LlmError, Native, Part, Reply, Request, StopReason, ToolCall, Usage, error_message, retry_after};

/// What a replayed item must have come from: encrypted reasoning is only valid for the model that wrote it
pub fn origin(url: &str, model: &str) -> String {
  format!("{url} {model}")
}

/// The request body for the endpoint at `origin`
pub fn body(req: &Request, origin: &str) -> Value {
  let mut input: Vec<Value> = vec![];
  for item in &req.items {
    match item {
      Item::User(parts) => input.push(json!({ "type": "message", "role": "user", "content": user_content(parts) })),
      Item::Assistant { text, tool_calls, native, .. } => match native.as_ref().filter(|n| n.origin == origin) {
        Some(n) => input.extend(n.blocks.iter().filter_map(replayable)),
        // Another endpoint's (or format's) message: its text and calls, without reasoning that cannot be verified
        None => {
          if !text.is_empty() {
            input.push(json!({ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": text }] }));
          }
          for c in tool_calls {
            input.push(json!({ "type": "function_call", "call_id": c.id, "name": c.name, "arguments": c.arguments }));
          }
        }
      },
      Item::ToolResult { call_id, content, .. } => {
        input.push(json!({ "type": "function_call_output", "call_id": call_id, "output": if content.is_empty() { "(no output)" } else { content } }));
      }
    }
  }
  let mut body = Map::new();
  body.insert("model".into(), Value::from(req.model.as_str()));
  if !req.system.is_empty() {
    body.insert("instructions".into(), Value::from(req.system.as_str()));
  }
  body.insert("input".into(), Value::Array(input));
  if !req.tools.is_empty() {
    body.insert(
      "tools".into(),
      req
        .tools
        .iter()
        .map(|t| json!({ "type": "function", "name": t.name, "description": t.description, "parameters": t.parameters, "strict": false }))
        .collect(),
    );
    body.insert("tool_choice".into(), Value::from("auto"));
    body.insert("parallel_tool_calls".into(), Value::Bool(true));
  }
  match (req.thinking, req.effort.as_deref()) {
    (Thinking::Off, _) => {}
    (_, Some(e)) => {
      body.insert("reasoning".into(), json!({ "effort": e, "summary": "auto" }));
    }
    (Thinking::On, None) => {
      body.insert("reasoning".into(), json!({ "summary": "auto" }));
    }
    (Thinking::Auto, None) => {}
  }
  body.insert("store".into(), Value::Bool(false));
  body.insert("include".into(), json!(["reasoning.encrypted_content"]));
  if let Some(key) = &req.cache_key {
    body.insert("prompt_cache_key".into(), Value::from(key.as_str()));
  }
  body.insert("stream".into(), Value::Bool(true));
  if let Some(n) = req.max_tokens {
    body.insert("max_output_tokens".into(), Value::from(n));
  }
  if let Some(t) = req.sampling.temperature {
    body.insert("temperature".into(), Value::from(t));
  }
  if let Some(p) = req.sampling.top_p {
    body.insert("top_p".into(), Value::from(p));
  }
  Value::Object(body)
}

fn user_content(parts: &[Part]) -> Value {
  parts
    .iter()
    .map(|p| match p {
      Part::Text(t) => json!({ "type": "input_text", "text": t }),
      Part::Image { mime, data } => json!({ "type": "input_image", "image_url": format!("data:{mime};base64,{data}") }),
    })
    .collect()
}

/// An output item as it may go back in `input`: stream bookkeeping (`status`, a gateway's `sequence_number`) is
/// dropped, and reasoning without its encrypted content is left out, since a stateless request cannot refer to it
fn replayable(item: &Value) -> Option<Value> {
  let mut item = item.as_object()?.clone();
  item.remove("status");
  item.remove("sequence_number");
  match item.get("type").and_then(Value::as_str)? {
    "reasoning" => item.get("encrypted_content").and_then(Value::as_str).filter(|c| !c.is_empty()).map(|_| ())?,
    "message" => {
      // Ids of earlier messages are only resolvable when the server stored them
      item.remove("id");
    }
    "function_call" => {
      item.remove("id");
    }
    _ => {}
  }
  Some(Value::Object(item))
}

/// Usage of a finished response; `input_tokens` already counts the cached ones
pub fn parse_usage(u: &Value) -> Usage {
  let n = |p: &str| u.pointer(p).and_then(Value::as_u64).unwrap_or(0);
  Usage {
    input: n("/input_tokens"),
    output: n("/output_tokens"),
    cache_read: n("/input_tokens_details/cached_tokens"),
    cache_write: 0,
    reasoning: n("/output_tokens_details/reasoning_tokens"),
  }
}

#[derive(Default)]
struct Call {
  call_id: String,
  name: String,
  arguments: String,
  announced: bool,
}

/// Folds stream events into events and the final reply
pub struct Assembler {
  origin: String,
  reply: Reply,
  /// Finished output items by output_index
  items: BTreeMap<u64, Value>,
  /// Function calls still streaming, by output_index
  calls: BTreeMap<u64, Call>,
  /// The summary part being streamed, so parts are separated by a blank line
  summary_part: Option<(u64, u64)>,
  usage: Option<Usage>,
  stop: Option<StopReason>,
}

impl Assembler {
  pub fn new(origin: String) -> Assembler {
    Assembler { origin, reply: Reply::default(), items: BTreeMap::new(), calls: BTreeMap::new(), summary_part: None, usage: None, stop: None }
  }

  pub fn event(&mut self, v: &Value, emit: &mut dyn FnMut(Event)) -> Result<(), LlmError> {
    let index = v.get("output_index").and_then(Value::as_u64).unwrap_or(0);
    match v.get("type").and_then(Value::as_str).unwrap_or("") {
      "response.output_item.added" => {
        let item = &v["item"];
        if item["type"] == "function_call" {
          let call = self.calls.entry(index).or_default();
          call.call_id = item["call_id"].as_str().unwrap_or_default().to_owned();
          call.name = item["name"].as_str().unwrap_or_default().to_owned();
          if let Some(a) = item["arguments"].as_str() {
            call.arguments = a.to_owned();
          }
          if !call.name.is_empty() {
            call.announced = true;
            emit(Event::ToolCallStart { id: call.call_id.clone(), name: call.name.clone() });
          }
        }
      }
      "response.function_call_arguments.delta" => {
        if let Some(d) = v["delta"].as_str() {
          self.calls.entry(index).or_default().arguments.push_str(d);
        }
      }
      "response.function_call_arguments.done" => {
        if let Some(a) = v["arguments"].as_str() {
          self.calls.entry(index).or_default().arguments = a.to_owned();
        }
      }
      "response.output_text.delta" => {
        if let Some(d) = v["delta"].as_str().filter(|d| !d.is_empty()) {
          self.reply.text.push_str(d);
          emit(Event::Text(d.to_owned()));
        }
      }
      // Reasoning summaries (OpenAI) and raw reasoning text (open-weight models behind the same API)
      "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
        if let Some(d) = v["delta"].as_str().filter(|d| !d.is_empty()) {
          let part = (index, v.get("summary_index").or_else(|| v.get("content_index")).and_then(Value::as_u64).unwrap_or(0));
          let sep = match self.summary_part {
            Some(p) if p != part && !self.reply.reasoning.is_empty() => "\n\n",
            _ => "",
          };
          self.summary_part = Some(part);
          let chunk = format!("{sep}{d}");
          self.reply.reasoning.push_str(&chunk);
          emit(Event::Reasoning(chunk));
        }
      }
      "response.output_item.done" => {
        self.finish_item(index, v["item"].clone(), emit);
      }
      "response.completed" | "response.incomplete" => {
        let resp = &v["response"];
        // Items a server reported only here (no output_item.done)
        for (i, item) in resp["output"].as_array().into_iter().flatten().enumerate() {
          if !self.items.contains_key(&(i as u64)) {
            self.finish_item(i as u64, item.clone(), emit);
          }
        }
        if let Some(u) = resp.get("usage").filter(|u| u.is_object()) {
          self.usage = Some(parse_usage(u));
        }
        self.stop = Some(match resp.pointer("/incomplete_details/reason").and_then(Value::as_str) {
          Some("max_output_tokens") => StopReason::MaxTokens,
          Some("content_filter") => StopReason::Refusal,
          Some(other) => StopReason::Other(other.to_owned()),
          None => StopReason::EndTurn,
        });
      }
      "response.failed" => {
        let err = &v["response"]["error"];
        let message = err["message"].as_str().map(str::to_owned).unwrap_or_else(|| err.to_string());
        return Err(LlmError::Api(message));
      }
      "error" => {
        let message = v["message"].as_str().or_else(|| v.pointer("/error/message").and_then(Value::as_str)).map(str::to_owned).unwrap_or_else(|| v.to_string());
        return Err(LlmError::Api(message));
      }
      _ => {}
    }
    Ok(())
  }

  fn finish_item(&mut self, index: u64, item: Value, emit: &mut dyn FnMut(Event)) {
    if item["type"] == "function_call" {
      let call = self.calls.entry(index).or_default();
      if let Some(id) = item["call_id"].as_str() {
        call.call_id = id.to_owned();
      }
      if let Some(name) = item["name"].as_str() {
        call.name = name.to_owned();
      }
      if let Some(a) = item["arguments"].as_str() {
        call.arguments = a.to_owned();
      }
      if !call.announced && !call.name.is_empty() {
        call.announced = true;
        emit(Event::ToolCallStart { id: call.call_id.clone(), name: call.name.clone() });
      }
    }
    // Text a server sent only in the finished item
    if item["type"] == "message" && self.reply.text.is_empty() {
      let text: String = item["content"].as_array().into_iter().flatten().filter_map(|c| c["text"].as_str()).collect();
      if !text.is_empty() {
        self.reply.text.push_str(&text);
        emit(Event::Text(text));
      }
    }
    self.items.insert(index, item);
  }

  /// The stream ended; a response that never completed is a broken stream
  pub fn finish(mut self, emit: &mut dyn FnMut(Event)) -> Result<(), LlmError> {
    let Some(stop) = self.stop.take() else {
      return Err(LlmError::Protocol("the stream ended before the model finished".into()));
    };
    self.reply.tool_calls = self
      .calls
      .into_values()
      .filter(|c| !c.name.is_empty())
      .enumerate()
      .map(|(n, c)| ToolCall {
        id: if c.call_id.is_empty() { format!("call_{n}") } else { c.call_id },
        name: c.name,
        arguments: if c.arguments.trim().is_empty() { "{}".into() } else { c.arguments },
      })
      .collect();
    let stop = if !self.reply.tool_calls.is_empty() && stop == StopReason::EndTurn { StopReason::ToolUse } else { stop };
    self.reply.native = Some(Native { origin: self.origin, blocks: self.items.into_values().collect() });
    emit(Event::Done { reply: self.reply, stop, usage: self.usage });
    Ok(())
  }
}

/// One streamed call on the calling (blocking) thread
pub fn stream(http: &ureq::Agent, endpoint: &Endpoint, req: &Request, tx: &mpsc::UnboundedSender<Result<Event, LlmError>>) -> Result<(), LlmError> {
  let origin = origin(&endpoint.url, &req.model);
  let payload = body(req, &origin);
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
    if data.is_empty() || data == "[DONE]" {
      continue;
    }
    let v: Value = serde_json::from_str(data).map_err(|e| LlmError::Protocol(format!("{e}: {}", data.chars().take(200).collect::<String>())))?;
    asm.event(&v, &mut emit)?;
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
  use acpira_shared::providers::Sampling;

  const ORIGIN: &str = "https://api.example.com/v1/responses m";

  fn req(items: Vec<Item>) -> Request {
    Request {
      model: "m".into(),
      system: "sys".into(),
      items,
      tools: vec![ToolSpec { name: "read".into(), description: "Read".into(), parameters: json!({ "type": "object" }) }],
      max_tokens: None,
      output_limit: None,
      sampling: Sampling::default(),
      thinking: Thinking::Auto,
      effort: Some("high".into()),
      cache_key: Some("session-1".into()),
    }
  }

  fn run(events: &[Value]) -> (Vec<Event>, Result<(), LlmError>) {
    let mut asm = Assembler::new(ORIGIN.into());
    let mut out = vec![];
    for e in events {
      if let Err(err) = asm.event(e, &mut |ev| out.push(ev)) {
        return (out, Err(err));
      }
    }
    let r = asm.finish(&mut |ev| out.push(ev));
    (out, r)
  }

  #[test]
  fn a_streamed_step_keeps_its_items_and_replays_them_to_the_same_model() {
    let reasoning = json!({ "type": "reasoning", "id": "rs_1", "summary": [{ "type": "summary_text", "text": "Look." }], "encrypted_content": "enc", "status": "completed" });
    let call = json!({ "type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "read", "arguments": "{\"path\":\"a\"}", "status": "completed" });
    let (events, r) = run(&[
      json!({ "type": "response.output_item.added", "output_index": 0, "item": { "type": "reasoning", "id": "rs_1" } }),
      json!({ "type": "response.reasoning_summary_text.delta", "output_index": 0, "summary_index": 0, "delta": "Look." }),
      json!({ "type": "response.output_item.done", "output_index": 0, "item": reasoning }),
      json!({ "type": "response.output_item.added", "output_index": 1, "item": { "type": "function_call", "call_id": "call_1", "name": "read", "arguments": "" } }),
      json!({ "type": "response.function_call_arguments.delta", "output_index": 1, "delta": "{\"path\":" }),
      json!({ "type": "response.function_call_arguments.delta", "output_index": 1, "delta": "\"a\"}" }),
      json!({ "type": "response.output_item.done", "output_index": 1, "item": call }),
      json!({ "type": "response.completed", "response": { "output": [], "usage": { "input_tokens": 100, "output_tokens": 20, "input_tokens_details": { "cached_tokens": 80 }, "output_tokens_details": { "reasoning_tokens": 12 } } } }),
    ]);
    r.unwrap();
    assert_eq!(events[0], Event::Reasoning("Look.".into()));
    assert_eq!(events[1], Event::ToolCallStart { id: "call_1".into(), name: "read".into() });
    let Some(Event::Done { reply, stop, usage }) = events.last() else { panic!("{events:?}") };
    assert_eq!(*stop, StopReason::ToolUse);
    assert_eq!(*usage, Some(Usage { input: 100, output: 20, cache_read: 80, cache_write: 0, reasoning: 12 }));
    assert_eq!(reply.tool_calls, vec![ToolCall { id: "call_1".into(), name: "read".into(), arguments: "{\"path\":\"a\"}".into() }]);

    let item = Item::Assistant { text: String::new(), reasoning: reply.reasoning.clone(), tool_calls: reply.tool_calls.clone(), native: reply.native.clone() };
    let result = Item::ToolResult { call_id: "call_1".into(), name: "read".into(), content: "x".into(), is_error: false };
    let b = body(&req(vec![Item::User(vec![Part::Text("hi".into())]), item.clone(), result.clone()]), ORIGIN);
    let input = b["input"].as_array().unwrap();
    // Encrypted reasoning goes back with its id, the call without the item id or status
    assert_eq!(input[1], json!({ "type": "reasoning", "id": "rs_1", "summary": [{ "type": "summary_text", "text": "Look." }], "encrypted_content": "enc" }));
    assert_eq!(input[2], json!({ "type": "function_call", "call_id": "call_1", "name": "read", "arguments": "{\"path\":\"a\"}" }));
    assert_eq!(input[3], json!({ "type": "function_call_output", "call_id": "call_1", "output": "x" }));
    assert_eq!((b["instructions"].as_str(), b["store"].as_bool(), b["prompt_cache_key"].as_str()), (Some("sys"), Some(false), Some("session-1")));
    assert_eq!((b["tools"][0]["strict"].as_bool(), b["parallel_tool_calls"].as_bool()), (Some(false), Some(true)));
    assert_eq!(b["reasoning"], json!({ "effort": "high", "summary": "auto" }));

    // Another model gets the call without the reasoning it cannot decrypt
    let other = body(&req(vec![Item::User(vec![Part::Text("hi".into())]), item, result]), "https://api.example.com/v1/responses other");
    assert_eq!(other["input"].as_array().unwrap().len(), 3);
    assert_eq!(other["input"][1]["type"], "function_call");
  }

  #[test]
  fn reasoning_without_encrypted_content_text_only_in_the_final_item_and_failures() {
    // A gateway that sends neither encrypted reasoning nor text deltas (seen with gemini-3.8-flash on 2026-10-11)
    let (events, r) = run(&[
      json!({ "type": "response.output_item.done", "output_index": 0, "item": { "type": "reasoning", "id": "rs_x", "summary": [] } }),
      json!({ "type": "response.output_item.done", "output_index": 1, "item": { "type": "message", "id": "msg_1", "role": "assistant", "content": [{ "type": "output_text", "text": "Done." }] } }),
      json!({ "type": "response.completed", "response": { "output": [] } }),
    ]);
    r.unwrap();
    let Some(Event::Done { reply, stop, .. }) = events.last() else { panic!() };
    assert_eq!((reply.text.as_str(), stop), ("Done.", &StopReason::EndTurn));
    let item = Item::Assistant { text: reply.text.clone(), reasoning: String::new(), tool_calls: vec![], native: reply.native.clone() };
    let b = body(&req(vec![item]), ORIGIN);
    assert_eq!(b["input"], json!([{ "type": "message", "role": "assistant", "content": [{ "type": "output_text", "text": "Done." }] }]));

    let (_, r) = run(&[json!({ "type": "response.failed", "response": { "error": { "message": "upstream overloaded" } } })]);
    assert_eq!(r, Err(LlmError::Api("upstream overloaded".into())));
    let (_, r) = run(&[json!({ "type": "response.output_text.delta", "delta": "par" })]);
    assert!(matches!(r, Err(LlmError::Protocol(_))), "a stream without response.completed is broken");
    let (events, r) = run(&[json!({ "type": "response.incomplete", "response": { "incomplete_details": { "reason": "max_output_tokens" } } })]);
    r.unwrap();
    assert!(matches!(events.last(), Some(Event::Done { stop: StopReason::MaxTokens, .. })));
  }
}
