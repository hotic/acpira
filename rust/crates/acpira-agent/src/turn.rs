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
use crate::permission::{self, Decision, Guard, Rule};
use crate::tools::names::{self, Resolved};
use crate::tools::{self, Action, Ctx, Output};

/// Misnamed tool calls taken as the tool they map to, per turn; later ones get the error so a confused model stops
const MAX_CORRECTIONS: u32 = 2;

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
  let approval = {
    let mut st = session.state.lock();
    st.items.push(Item::User(parts));
    st.approval
  };
  // Frozen for the turn like the rest; what the user allows on a card applies at once (`SessionState::allowed`)
  let rules = Rules {
    defaults: permission::defaults(&server.session_dir(&session.id).join("outputs")),
    mode: vec![],
    approval: approval.rules(),
    guard: Guard::new(server.config.home(), &session.cwd, user_home().as_deref()),
  };
  let mut corrections = 0u32;

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
          server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": shown_name(&name), "kind": kind_of(&name), "status": "pending" }));
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
        if !run_tools(server, session, &rules, &reply.tool_calls, &mut ids, &mut corrections).await {
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

/// The tool a model-given name stands for, for display before the call is prepared
fn shown_name(name: &str) -> &str {
  match names::resolve(name) {
    Resolved::Exact(n) | Resolved::Corrected(n) => n,
    Resolved::Unknown(_) => name,
  }
}

/// ACP ToolKind for a tool name, before its arguments are known
fn kind_of(name: &str) -> &'static str {
  match shown_name(name) {
    tools::READ | tools::LIST => "read",
    tools::WRITE | tools::EDIT => "edit",
    tools::BASH => "execute",
    tools::GREP | tools::GLOB => "search",
    _ => "other",
  }
}

fn user_home() -> Option<std::path::PathBuf> {
  std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from)
}

/// The permission layers of a turn
struct Rules {
  defaults: Vec<Rule>,
  mode: Vec<Rule>,
  approval: Vec<Rule>,
  guard: Guard,
}

impl Rules {
  fn decide(&self, session: &Session, action: &Action) -> Decision {
    let (key, target) = action.permission(&session.cwd);
    let allowed = session.state.lock().allowed.clone();
    let d = permission::evaluate(&[&self.defaults, &self.mode, &self.approval, &allowed], key, &target);
    match action {
      Action::Write { path, .. } if d == Decision::Allow && self.guard.protects(path) => Decision::Ask,
      _ => d,
    }
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
  /// Put before the result: the misnamed call was taken as another tool
  note: Option<String>,
}

/// Run one round of tool calls; false when the user rejected one (the turn ends after the round)
async fn run_tools(
  server: &Arc<Server>,
  session: &Arc<Session>,
  rules: &Rules,
  calls: &[ToolCall],
  ids: &mut HashMap<String, String>,
  corrections: &mut u32,
) -> bool {
  let cwd = session.cwd.clone();
  let mut prepared = vec![];
  for call in calls {
    let acp = match ids.remove(&call.id) {
      Some(a) => a,
      None => {
        let a = session.next_tool_id();
        server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": a, "title": shown_name(&call.name), "kind": kind_of(&call.name), "status": "pending" }));
        a
      }
    };
    let args: Result<Value, String> = serde_json::from_str::<Value>(&call.arguments)
      .map_err(|e| format!("The arguments are not valid JSON ({e}). Received: {}", crate::budget::cut(&call.arguments, 2000)))
      .and_then(|v| if v.is_object() { Ok(v) } else { Err(format!("The arguments must be a JSON object. Received: {}", crate::budget::cut(&call.arguments, 2000))) });
    let (name, note) = match names::resolve(&call.name) {
      Resolved::Exact(n) => (Ok(n), None),
      Resolved::Corrected(n) if *corrections < MAX_CORRECTIONS => {
        *corrections += 1;
        (Ok(n), Some(format!("(\"{}\" was taken as the {n} tool; call tools by their exact names.)", call.name)))
      }
      Resolved::Corrected(n) => (Err(format!("Unknown tool \"{}\" (did you mean \"{n}\"?). Tool names must match exactly: {}.", call.name, names::ALL.join(", "))), None),
      Resolved::Unknown(message) => (Err(message), None),
    };
    // A bad argument goes back with what was received, so the model can see its own mistake
    let action = name.and_then(|n| {
      let a = args.clone()?;
      tools::prepare(n, &a, &cwd).map_err(|e| format!("{e}. Received arguments: {}", crate::budget::cut(&a.to_string(), 2000)))
    });
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
    prepared.push(Prepared { call: call.clone(), acp, action, note });
  }

  let mut rejected = false;
  let mut i = 0;
  while i < prepared.len() {
    // Consecutive read-only calls the rules allow run together
    let start = i;
    while i < prepared.len() && !rejected && matches!(&prepared[i].action, Ok(a) if a.read_only() && rules.decide(session, a) == Decision::Allow) {
      i += 1;
    }
    if i > start {
      let mut handles = vec![];
      for p in &prepared[start..i] {
        let Ok(action) = p.action.clone() else { unreachable!() };
        let ctx = ctx_for(server, session, &p.acp);
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
        handles.push(tokio::task::spawn_blocking(move || action.run_sync(ctx)));
      }
      for (p, h) in prepared[start..i].iter().zip(handles) {
        let out = h.await.unwrap_or_else(|e| Output::error(format!("The tool crashed: {e}")));
        finish(server, session, p, out);
      }
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
    match rules.decide(session, &action) {
      Decision::Allow => {}
      Decision::Deny => {
        let (key, target) = action.permission(&cwd);
        finish(server, session, p, Output::error(format!("Not allowed: the permission rules deny {key} on {target}. Do not retry it.")));
        continue;
      }
      Decision::Ask => match ask(server, session, p, &action).await {
        Answer::Once => {}
        Answer::Always(pattern) => {
          let (key, _) = action.permission(&cwd);
          session.state.lock().allowed.push(Rule::new(key, &pattern, Decision::Allow));
        }
        Answer::Reject => {
          rejected = true;
          finish(server, session, p, Output::error("The user rejected this action. Do not retry it; wait for the user's direction."));
          continue;
        }
      },
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

enum Answer {
  Once,
  /// Allowed for the rest of the session: the rule pattern
  Always(String),
  Reject,
}

/// Ask the user through ACP
async fn ask(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, action: &Action) -> Answer {
  let pres = action.describe(&session.cwd);
  let raw_input = match action {
    Action::Bash { command, .. } => json!({ "command": command }),
    _ => serde_json::from_str(&p.call.arguments).unwrap_or(Value::Null),
  };
  let always = action.always();
  let mut options = vec![json!({ "optionId": "allow", "name": "Allow", "kind": "allow_once" })];
  if let Some((_, label)) = &always {
    options.push(json!({ "optionId": "always", "name": label, "kind": "allow_always" }));
  }
  options.push(json!({ "optionId": "reject", "name": "Reject", "kind": "reject_once" }));
  let request = json!({
    "sessionId": session.id,
    "toolCall": {
      "toolCallId": p.acp, "title": pres.title, "kind": pres.kind, "status": "pending", "rawInput": raw_input,
      "locations": pres.locations.iter().map(|l| json!({ "path": l })).collect::<Vec<_>>(), "content": pres.content,
    },
    "options": options,
  });
  let Ok(r) = server.conn().request("session/request_permission", request).await else { return Answer::Reject };
  if r.pointer("/outcome/outcome").and_then(Value::as_str) != Some("selected") {
    return Answer::Reject;
  }
  match (r.pointer("/outcome/optionId").and_then(Value::as_str), always) {
    (Some("allow"), _) => Answer::Once,
    (Some("always"), Some((pattern, _))) => Answer::Always(pattern),
    _ => Answer::Reject,
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
  let content = match &p.note {
    Some(note) => format!("{note}\n{}", out.model),
    None => out.model,
  };
  session.state.lock().items.push(Item::ToolResult { call_id: p.call.id.clone(), name: p.call.name.clone(), content, is_error: out.is_error });
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
