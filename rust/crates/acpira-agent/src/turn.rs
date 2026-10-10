//! One turn: the user's prompt, then model calls and tool rounds until the model stops. The configuration, tool set,
//! system prompt and model parameters are read once at the start and stay fixed for the turn; a settings change applies
//! from the next one.
//!
//! Cancellation: `session/cancel` fires the turn's signal, the turn answers `cancelled` at once and sends nothing after
//! it. Its future is dropped, which kills a running command's process group; a model stream's reading thread notices
//! the closed channel at its next chunk and drops the connection. The history keeps what finished, plus the cut-off
//! text and a "cancelled" result for every tool call left without one, so the next request is well formed

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::RpcError;
use acpira_shared::providers::ProviderModel;

use crate::acp::{Server, Session, SessionState};
use crate::llm::{self, Endpoint, Event, Item, LlmError, Part, Request, StopReason, ToolCall, Usage};
use crate::tools::{self, Action, Ctx, Output};

/// The context size reported when a model has none configured
pub const DEFAULT_CONTEXT: u64 = 128_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
  EndTurn,
  MaxTokens,
  MaxTurnRequests,
  Refusal,
  Cancelled,
}

impl Stop {
  fn wire(self) -> &'static str {
    match self {
      Stop::EndTurn => "end_turn",
      Stop::MaxTokens => "max_tokens",
      Stop::MaxTurnRequests => "max_turn_requests",
      Stop::Refusal => "refusal",
      Stop::Cancelled => "cancelled",
    }
  }
}

/// Token totals over the turn's model calls
#[derive(Default, Clone, Copy)]
struct Stats {
  usage: Usage,
  calls: u64,
}

pub async fn run(server: Arc<Server>, params: Value, cancel: Cancel) -> Result<Value, RpcError> {
  let sid = params.get("sessionId").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "sessionId is required"))?;
  let session = server.session(sid)?;
  {
    let mut st = session.state.lock();
    if st.turn.is_some() {
      return Err(RpcError::new(-32603, "A turn is already running in this session"));
    }
    st.turn = Some(cancel.clone());
  }
  let stats = Arc::new(parking_lot::Mutex::new(Stats::default()));
  let pick = session.state.lock().model.clone();
  let result = tokio::select! {
    r = body(&server, &session, &params, &stats) => r,
    _ = cancel.cancelled() => Ok(Stop::Cancelled),
  };
  {
    let mut st = session.state.lock();
    st.turn = None;
    settle(&mut st);
  }
  let stop = result.map_err(|e| RpcError::new(-32603, e))?;
  let s = *stats.lock();
  let mut response = json!({ "stopReason": stop.wire() });
  if s.calls > 0 {
    let u = s.usage;
    response["usage"] = json!({
      "inputTokens": u.input, "outputTokens": u.output, "totalTokens": u.input + u.output,
      "thoughtTokens": u.reasoning, "cachedReadTokens": u.cache_read, "cachedWriteTokens": u.cache_write,
    });
    response["_meta"] = json!({ "modelId": pick, "usage": { "modelCalls": s.calls } });
  }
  Ok(response)
}

/// Close the history after a turn ended any way: cut-off text becomes an assistant message, and every tool call
/// without a result gets one
fn settle(st: &mut SessionState) {
  let text = std::mem::take(&mut st.partial_text);
  let reasoning = std::mem::take(&mut st.partial_reasoning);
  if !text.is_empty() {
    st.items.push(Item::Assistant { text, reasoning, tool_calls: vec![] });
  }
  let answered: std::collections::HashSet<String> = st
    .items
    .iter()
    .filter_map(|i| match i {
      Item::ToolResult { call_id, .. } => Some(call_id.clone()),
      _ => None,
    })
    .collect();
  let Some(last_call) = st.items.iter().rposition(|i| matches!(i, Item::Assistant { tool_calls, .. } if !tool_calls.is_empty())) else { return };
  let Item::Assistant { tool_calls, .. } = &st.items[last_call] else { return };
  let missing: Vec<ToolCall> = tool_calls.iter().filter(|c| !answered.contains(&c.id)).cloned().collect();
  for c in missing {
    st.items.push(Item::ToolResult { call_id: c.id, name: c.name, content: "Cancelled by the user before it finished.".into(), is_error: true });
  }
}

async fn body(server: &Arc<Server>, session: &Arc<Session>, params: &Value, stats: &Arc<parking_lot::Mutex<Stats>>) -> Result<Stop, String> {
  let config = server.config.get();
  if let Some(e) = &config.error {
    return Err(e.clone());
  }
  let (pick, effort, system) = {
    let st = session.state.lock();
    (st.model.clone().or_else(|| config.default_pick()), st.effort.clone(), st.system.clone())
  };
  let pick = pick.ok_or("No model is configured. Add a model source in Acpira's settings (Agents → Acpira).")?;
  let (provider, model) = config.find(&pick).ok_or_else(|| format!("The model {pick} is no longer configured; pick another one."))?;
  let endpoint = Endpoint::of(provider, model, config.api_key(&provider.id))
    .ok_or_else(|| format!("{}: the API format \"{}\" is not supported", provider.display_name(), provider.format))?;
  let parts = prompt_parts(params.get("prompt").unwrap_or(&Value::Null), model.takes_images());
  session.state.lock().items.push(Item::User(parts));

  let tool_specs = tools::specs();
  let context = model.context.unwrap_or(DEFAULT_CONTEXT);
  let mut step = 0u32;
  loop {
    if model.max_steps.is_some_and(|max| step >= max) {
      return Ok(Stop::MaxTurnRequests);
    }
    step += 1;
    let request = Request {
      model: model.id.clone(),
      system: system.clone(),
      items: session.state.lock().items.clone(),
      tools: tool_specs.clone(),
      max_tokens: max_tokens(model),
      sampling: model.sampling.clone(),
      thinking: model.thinking,
      effort: effort.clone(),
    };
    let mut rx = llm::stream(server.http.clone(), endpoint.clone(), request);
    // Model tool-call id → ACP tool call id: providers reuse ids across calls, ACP needs them unique per session
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut finished = None;
    while let Some(ev) = rx.recv().await {
      match ev {
        Ok(Event::Text(t)) => {
          session.state.lock().partial_text.push_str(&t);
          server.update(&session.id, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": t } }));
        }
        Ok(Event::Reasoning(t)) => {
          session.state.lock().partial_reasoning.push_str(&t);
          server.update(&session.id, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": t } }));
        }
        Ok(Event::ToolCallStart { id, name }) => {
          let acp = session.next_tool_id();
          server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": name, "kind": kind_of(&name), "status": "pending" }));
          ids.insert(id, acp);
        }
        Ok(Event::Done { reply, stop, usage }) => {
          finished = Some((reply, stop, usage));
          break;
        }
        Err(e) => return Err(describe(provider.display_name(), &e)),
      }
    }
    let (reply, stop, usage) = finished.ok_or_else(|| describe(provider.display_name(), &LlmError::Protocol("the stream ended without a result".into())))?;
    if let Some(u) = usage {
      let mut s = stats.lock();
      s.usage.input += u.input;
      s.usage.output += u.output;
      s.usage.cache_read += u.cache_read;
      s.usage.cache_write += u.cache_write;
      s.usage.reasoning += u.reasoning;
      s.calls += 1;
      server.update(&session.id, json!({ "sessionUpdate": "usage_update", "used": u.input + u.output, "size": context }));
    } else {
      stats.lock().calls += 1;
    }
    {
      let mut st = session.state.lock();
      st.partial_text.clear();
      st.partial_reasoning.clear();
      st.items.push(Item::Assistant { text: reply.text.clone(), reasoning: reply.reasoning.clone(), tool_calls: reply.tool_calls.clone() });
    }
    match stop {
      _ if !reply.tool_calls.is_empty() && stop != StopReason::MaxTokens => {
        if !run_tools(server, session, &reply.tool_calls, &mut ids).await {
          return Ok(Stop::EndTurn);
        }
      }
      StopReason::MaxTokens => return Ok(Stop::MaxTokens),
      StopReason::Refusal => return Ok(Stop::Refusal),
      _ => return Ok(Stop::EndTurn),
    }
  }
}

/// The output limit sent with a request: only one the user (or the endpoint) gave, never a guessed one
fn max_tokens(model: &ProviderModel) -> Option<u64> {
  model.output.filter(|_| !model.estimated.iter().any(|e| e == "output"))
}

/// A failed call as the turn's error text
fn describe(source: &str, e: &LlmError) -> String {
  match e {
    LlmError::Http { status: 401 | 403, message, .. } => format!("{source} rejected the API key ({message}). Check the key in Acpira's settings."),
    LlmError::Http { status: 404, message, .. } => format!("{source}: not found ({message}). Check the base URL and the model id."),
    other => format!("{source}: {other}"),
  }
}

/// ACP ToolKind for a tool name, before its arguments are known
fn kind_of(name: &str) -> &'static str {
  match name {
    tools::READ => "read",
    tools::WRITE | tools::EDIT => "edit",
    tools::BASH => "execute",
    _ => "other",
  }
}

/// The prompt's content blocks as user message parts
pub fn prompt_parts(prompt: &Value, images: bool) -> Vec<Part> {
  let mut parts = vec![];
  for block in prompt.as_array().into_iter().flatten() {
    match block.get("type").and_then(Value::as_str) {
      Some("text") => {
        if let Some(t) = block.get("text").and_then(Value::as_str) {
          parts.push(Part::Text(t.to_owned()));
        }
      }
      Some("image") => {
        let data = block.get("data").and_then(Value::as_str).unwrap_or("");
        let mime = block.get("mimeType").and_then(Value::as_str).unwrap_or("image/png");
        if images && !data.is_empty() {
          parts.push(Part::Image { mime: mime.to_owned(), data: data.to_owned() });
        } else {
          parts.push(Part::Text("[An image was attached, but this model does not accept images.]".into()));
        }
      }
      Some("resource") => {
        let r = block.get("resource").unwrap_or(&Value::Null);
        let uri = r.get("uri").and_then(Value::as_str).unwrap_or("");
        match r.get("text").and_then(Value::as_str) {
          Some(text) => parts.push(Part::Text(format!("<attachment uri=\"{uri}\">\n{text}\n</attachment>"))),
          None => parts.push(Part::Text(format!("[Attached: {uri}]"))),
        }
      }
      Some("resource_link") => {
        let uri = block.get("uri").and_then(Value::as_str).unwrap_or("");
        let name = block.get("name").and_then(Value::as_str).unwrap_or(uri);
        parts.push(Part::Text(format!("[{name}]({uri})")));
      }
      _ => {}
    }
  }
  // Adjacent text blocks read as one message
  let mut merged: Vec<Part> = vec![];
  for p in parts {
    match (merged.last_mut(), p) {
      (Some(Part::Text(prev)), Part::Text(t)) => {
        prev.push_str("\n\n");
        prev.push_str(&t);
      }
      (_, p) => merged.push(p),
    }
  }
  if merged.is_empty() {
    merged.push(Part::Text(String::new()));
  }
  merged
}

/// A tool call after parsing and preparation
struct Prepared {
  call: ToolCall,
  acp: String,
  action: Result<Action, String>,
}

/// Run one round of tool calls; false when the user rejected one (the turn ends after the round)
async fn run_tools(server: &Arc<Server>, session: &Arc<Session>, calls: &[ToolCall], ids: &mut HashMap<String, String>) -> bool {
  let cwd = session.cwd.clone();
  let mut prepared = vec![];
  for call in calls {
    let acp = match ids.remove(&call.id) {
      Some(a) => a,
      None => {
        let a = session.next_tool_id();
        server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": a, "title": call.name, "kind": kind_of(&call.name), "status": "pending" }));
        a
      }
    };
    let args: Result<Value, String> = serde_json::from_str::<Value>(&call.arguments)
      .map_err(|e| format!("The arguments are not valid JSON ({e}). Received: {}", crate::budget::cut(&call.arguments, 2000)))
      .and_then(|v| if v.is_object() { Ok(v) } else { Err(format!("The arguments must be a JSON object. Received: {}", crate::budget::cut(&call.arguments, 2000))) });
    let action = args.clone().and_then(|a| tools::prepare(&call.name, &a, &cwd));
    let raw_input = args.unwrap_or_else(|_| Value::String(call.arguments.clone()));
    match &action {
      Ok(a) => {
        let p = a.describe(&cwd);
        server.update(
          &session.id,
          json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "title": p.title, "kind": p.kind, "rawInput": raw_input,
            "locations": p.locations.iter().map(|l| json!({ "path": l })).collect::<Vec<_>>(), "content": p.content }),
        );
      }
      Err(_) => {
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "rawInput": raw_input }));
      }
    }
    prepared.push(Prepared { call: call.clone(), acp, action });
  }

  let mut rejected = false;
  let mut i = 0;
  while i < prepared.len() {
    // Consecutive read-only calls run together
    let batch_end = (i..prepared.len()).find(|&k| !matches!(&prepared[k].action, Ok(a) if a.read_only())).unwrap_or(prepared.len());
    if batch_end > i && !rejected {
      let mut handles = vec![];
      for p in &prepared[i..batch_end] {
        let Ok(action) = p.action.clone() else { unreachable!() };
        let ctx = ctx_for(server, session, &p.acp);
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
        handles.push(tokio::task::spawn_blocking(move || action.run_sync(ctx)));
      }
      for (p, h) in prepared[i..batch_end].iter().zip(handles) {
        let out = h.await.unwrap_or_else(|e| Output::error(format!("The tool crashed: {e}")));
        finish(server, session, p, out);
      }
      i = batch_end;
      continue;
    }
    let p = &prepared[i];
    i += 1;
    if rejected {
      finish(server, session, p, Output::error("Skipped: the user rejected an earlier action in this round."));
      continue;
    }
    let action = match &p.action {
      Ok(a) => a.clone(),
      Err(e) => {
        finish(server, session, p, Output::error(e.clone()));
        continue;
      }
    };
    let pres = action.describe(&cwd);
    if pres.ask && !ask(server, session, p, &pres, &action).await {
      rejected = true;
      finish(server, session, p, Output::error("The user rejected this action. Do not retry it; wait for the user's direction."));
      continue;
    }
    server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
    let out = action.run(ctx_for(server, session, &p.acp)).await;
    finish(server, session, p, out);
  }
  !rejected
}

fn ctx_for(server: &Arc<Server>, session: &Arc<Session>, acp: &str) -> Ctx {
  let (srv, sid, id) = (server.clone(), session.id.clone(), acp.to_owned());
  Ctx {
    cwd: session.cwd.clone(),
    outputs: server.session_dir(&session.id).join("outputs"),
    call_id: acp.to_owned(),
    progress: Box::new(move |mut u: Value| {
      u["sessionUpdate"] = "tool_call_update".into();
      u["toolCallId"] = id.clone().into();
      srv.update(&sid, u);
    }),
  }
}

/// Ask the user through ACP; true when allowed
async fn ask(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, pres: &tools::Presentation, action: &Action) -> bool {
  let raw_input = match action {
    Action::Bash { command, .. } => json!({ "command": command }),
    _ => serde_json::from_str(&p.call.arguments).unwrap_or(Value::Null),
  };
  let request = json!({
    "sessionId": session.id,
    "toolCall": {
      "toolCallId": p.acp, "title": pres.title, "kind": pres.kind, "status": "pending", "rawInput": raw_input,
      "locations": pres.locations.iter().map(|l| json!({ "path": l })).collect::<Vec<_>>(), "content": pres.content,
    },
    "options": [
      { "optionId": "allow", "name": "Allow", "kind": "allow_once" },
      { "optionId": "reject", "name": "Reject", "kind": "reject_once" },
    ],
  });
  match server.conn().request("session/request_permission", request).await {
    Ok(r) => r.pointer("/outcome/outcome").and_then(Value::as_str) == Some("selected") && r.pointer("/outcome/optionId").and_then(Value::as_str) == Some("allow"),
    Err(_) => false,
  }
}

/// Report a finished call and add its result to the history
fn finish(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, out: Output) {
  let mut update = json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": if out.is_error { "failed" } else { "completed" } });
  if !out.content.is_empty() {
    update["content"] = Value::Array(out.content);
  }
  if let Some(raw) = out.raw_output {
    update["rawOutput"] = raw;
  }
  // A failed command's exit code shows on its card
  if let Some(code) = update.pointer("/rawOutput/exitCode").and_then(Value::as_i64).filter(|c| *c != 0) {
    update["_meta"] = json!({ "terminal_exit": { "exit_code": code } });
  }
  server.update(&session.id, update);
  session.state.lock().items.push(Item::ToolResult { call_id: p.call.id.clone(), name: p.call.name.clone(), content: out.model, is_error: out.is_error });
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn settle_answers_every_open_call_and_keeps_cut_text() {
    let mut st = SessionState::default();
    st.items.push(Item::User(vec![Part::Text("go".into())]));
    st.items.push(Item::Assistant {
      text: String::new(),
      reasoning: String::new(),
      tool_calls: vec![
        ToolCall { id: "a".into(), name: "read".into(), arguments: "{}".into() },
        ToolCall { id: "b".into(), name: "bash".into(), arguments: "{}".into() },
      ],
    });
    st.items.push(Item::ToolResult { call_id: "a".into(), name: "read".into(), content: "ok".into(), is_error: false });
    settle(&mut st);
    assert!(matches!(&st.items[3], Item::ToolResult { call_id, is_error: true, .. } if call_id == "b"));
    st.partial_text = "half an ans".into();
    settle(&mut st);
    assert!(matches!(st.items.last(), Some(Item::Assistant { text, .. }) if text == "half an ans"));
  }

  #[test]
  fn prompt_blocks_become_parts() {
    let parts = prompt_parts(
      &json!([
        { "type": "text", "text": "look" },
        { "type": "resource", "resource": { "uri": "file:///a.rs", "text": "fn a() {}" } },
        { "type": "image", "data": "AAA", "mimeType": "image/png" },
      ]),
      false,
    );
    let [Part::Text(t)] = &parts[..] else { panic!("{parts:?}") };
    assert!(t.contains("look") && t.contains("<attachment uri=\"file:///a.rs\">") && t.contains("does not accept images"));
  }
}
