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
use acpira_shared::model_catalog::Cost;
use acpira_shared::providers::{Provider, ProviderModel, Sampling, Thinking};

use crate::acp::{Server, Session, SessionState};
use crate::llm::{self, Endpoint, Event, Item, LlmError, Part, Request, StopReason, ToolCall, ToolSpec, Usage};
use crate::{modes, prompt};
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
#[derive(Default, Clone)]
struct Stats {
  usage: Usage,
  calls: u64,
  /// The model of the last call
  pick: Option<String>,
  /// (variant, version, digest) of the last call's system prompt
  prompt: Option<(String, String, String)>,
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
  let result = tokio::select! {
    r = body(&server, &session, &params, &stats) => r,
    _ = cancel.cancelled() => Ok(Stop::Cancelled),
  };
  {
    let mut st = session.state.lock();
    st.turn = None;
    let before = st.items.len();
    settle(&mut st);
    for item in &st.items[before..] {
      session.store.item(item);
    }
  }
  session.store.flush();
  let stop = result.map_err(|e| RpcError::new(-32603, e))?;
  let s = stats.lock().clone();
  let mut response = json!({ "stopReason": stop.wire() });
  if s.calls > 0 {
    let u = s.usage;
    response["usage"] = json!({
      "inputTokens": u.input, "outputTokens": u.output, "totalTokens": u.input + u.output,
      "thoughtTokens": u.reasoning, "cachedReadTokens": u.cache_read, "cachedWriteTokens": u.cache_write,
    });
    response["_meta"] = json!({ "modelId": s.pick, "usage": { "modelCalls": s.calls } });
    if let Some((variant, version, digest)) = &s.prompt {
      response["_meta"]["acpira/prompt"] = json!({ "variant": variant, "version": version, "digest": digest });
    }
  }
  Ok(response)
}

/// Close the history after a turn ended any way: cut-off text becomes an assistant message, and every tool call
/// without a result gets one
pub(crate) fn settle(st: &mut SessionState) {
  let text = std::mem::take(&mut st.partial_text);
  let reasoning = std::mem::take(&mut st.partial_reasoning);
  if !text.is_empty() {
    st.items.push(Item::Assistant { text, reasoning, tool_calls: vec![], native: None });
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

/// What the model calls of a turn use: read from the session at the start, and again after an approved plan switched
/// the mode (the user may have picked the model to build with on the plan card)
struct Setup {
  pick: String,
  provider: Provider,
  model: ProviderModel,
  endpoint: Endpoint,
  /// List prices from the catalogue, when it knows the model
  cost: Option<Cost>,
  effort: Option<String>,
  /// The model's sampling over its prompt variant's defaults
  sampling: Sampling,
  thinking: Thinking,
  system: String,
  /// (variant, version, digest) of the system prompt
  prompt: (String, String, String),
  tools: &'static [&'static str],
  specs: Vec<ToolSpec>,
  rules: Rules,
}

fn setup(server: &Server, session: &Session) -> Result<Setup, String> {
  let config = server.config.get();
  if let Some(e) = &config.error {
    return Err(e.clone());
  }
  let (pick, effort, mode, approval) = {
    let st = session.state.lock();
    (st.model.clone().or_else(|| config.default_pick()), st.effort.clone(), modes::get(&st.mode), st.approval)
  };
  let pick = pick.ok_or("No model is configured. Add a model source in Acpira's settings (Agents → Acpira).")?;
  let (provider, model) = config.find(&pick).ok_or_else(|| format!("The model {pick} is no longer configured; pick another one."))?;
  let endpoint = Endpoint::of(provider, model, config.api_key(&provider.id))
    .ok_or_else(|| format!("{}: the API format \"{}\" is not supported", provider.display_name(), provider.format))?;
  // The prompt changes only when the model now matches another variant (a view change at a turn boundary)
  let places = prompt::Places::new(&session.cwd, user_home().as_deref());
  let composed = {
    let variant = prompt::variant_name(&places, Some((provider, model)));
    let mut st = session.state.lock();
    if st.prompt.variant != variant || st.prompt.text.is_empty() {
      st.prompt = prompt::compose(&places, Some((provider, model)));
      session.save_prompt(&st.prompt);
    }
    st.prompt.clone()
  };
  let defaults = &composed.sampling;
  let sampling = Sampling {
    temperature: model.sampling.temperature.or(defaults.temperature),
    top_p: model.sampling.top_p.or(defaults.top_p),
    top_k: model.sampling.top_k.or(defaults.top_k),
  };
  let thinking = if model.thinking == Thinking::Auto { composed.thinking.unwrap_or(Thinking::Auto) } else { model.thinking };
  // Frozen for the turn like the rest; what the user allows on a card applies at once (`SessionState::allowed`)
  let rules = Rules {
    defaults: permission::defaults(&server.session_dir(&session.id).join("outputs")),
    approval: approval.rules(),
    mode: mode.rules(&server.plan_file(&session.id), &session.cwd),
    guard: Guard::new(server.config.home(), &session.cwd, user_home().as_deref()),
  };
  // Limits the configuration leaves open come from the catalogue, marked estimated (never sent as max_tokens)
  let mut model = model.clone();
  let cost = crate::catalog::complete(&mut model, &server.catalog.get());
  Ok(Setup {
    pick,
    provider: provider.clone(),
    model,
    endpoint,
    cost,
    effort,
    sampling,
    thinking,
    system: composed.text,
    prompt: (composed.variant, composed.version, composed.digest),
    tools: mode.tools,
    specs: tools::specs(mode.tools),
    rules,
  })
}

async fn body(server: &Arc<Server>, session: &Arc<Session>, params: &Value, stats: &Arc<parking_lot::Mutex<Stats>>) -> Result<Stop, String> {
  let mut s = setup(server, session)?;
  let mut parts = prompt_parts(params.get("prompt").unwrap_or(&Value::Null), s.model.takes_images());
  {
    let mut st = session.state.lock();
    // A mode entered since the model last heard about it is announced in the user message: the system prompt stays fixed
    if st.mode != st.announced {
      let overlay = modes::get(&st.mode).overlay(&server.plan_file(&session.id));
      st.announced = st.mode.clone();
      match parts.first_mut() {
        Some(Part::Text(t)) => *t = format!("<mode>\n{overlay}\n</mode>\n\n{t}"),
        _ => parts.insert(0, Part::Text(format!("<mode>\n{overlay}\n</mode>"))),
      }
    }
    let item = Item::User(parts);
    session.store.item(&item);
    st.items.push(item);
  }
  // The replay shows the prompt as the user sent it, without the mode note
  for block in params.get("prompt").and_then(Value::as_array).into_iter().flatten() {
    session.store.record_update(&json!({ "sessionUpdate": "user_message_chunk", "content": block }));
  }
  let mut corrections = 0u32;
  let mut step = 0u32;
  loop {
    if s.model.max_steps.is_some_and(|max| step >= max) {
      return Ok(Stop::MaxTurnRequests);
    }
    step += 1;
    {
      let mut st = stats.lock();
      st.pick = Some(s.pick.clone());
      st.prompt = Some(s.prompt.clone());
    }
    let context = s.model.context.unwrap_or(DEFAULT_CONTEXT);
    let request = Request {
      model: s.model.id.clone(),
      system: s.system.clone(),
      items: session.state.lock().items.clone(),
      tools: s.specs.clone(),
      max_tokens: max_tokens(&s.model),
      output_limit: s.model.output,
      sampling: s.sampling.clone(),
      thinking: s.thinking,
      effort: s.effort.clone(),
    };
    log_view(session, &s, &request);
    let started = std::time::Instant::now();
    let mut unfinished = Unfinished { session, event: Some(request_event(&s)), started };
    let mut rx = llm::stream(server.http.clone(), s.endpoint.clone(), request);
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
          server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": call_title(&name, s.tools), "kind": kind_of(&name, s.tools), "status": "pending" }));
          ids.insert(id, acp);
        }
        Ok(Event::Done { reply, stop, usage }) => {
          finished = Some((reply, stop, usage));
          break;
        }
        Err(e) => {
          unfinished.event = None;
          log_request(session, &s, started, None, None, Some(&e.to_string()));
          return Err(describe(s.provider.display_name(), &e));
        }
      }
    }
    let Some((reply, stop, usage)) = finished else {
      let e = LlmError::Protocol("the stream ended without a result".into());
      unfinished.event = None;
      log_request(session, &s, started, None, None, Some(&e.to_string()));
      return Err(describe(s.provider.display_name(), &e));
    };
    unfinished.event = None;
    log_request(session, &s, started, Some(&stop), usage.as_ref(), None);
    if let Some(u) = usage {
      let mut st = stats.lock();
      st.usage.input += u.input;
      st.usage.output += u.output;
      st.usage.cache_read += u.cache_read;
      st.usage.cache_write += u.cache_write;
      st.usage.reasoning += u.reasoning;
      st.calls += 1;
      server.update(&session.id, json!({ "sessionUpdate": "usage_update", "used": u.input + u.output, "size": context }));
    } else {
      stats.lock().calls += 1;
    }
    {
      let mut st = session.state.lock();
      st.partial_text.clear();
      st.partial_reasoning.clear();
      let item = Item::Assistant {
        text: reply.text.clone(),
        reasoning: reply.reasoning.clone(),
        tool_calls: reply.tool_calls.clone(),
        native: reply.native.clone(),
      };
      session.store.item(&item);
      st.items.push(item);
    }
    match stop {
      _ if !reply.tool_calls.is_empty() && stop != StopReason::MaxTokens => match run_tools(server, session, &s, &reply.tool_calls, &mut ids, &mut corrections).await {
        Flow::Continue => {}
        Flow::Stop => return Ok(Stop::EndTurn),
        // The approved plan is carried out in this turn, with Agent mode's tools and rules
        Flow::Switched => s = setup(server, session)?,
      },
      StopReason::MaxTokens => return Ok(Stop::MaxTokens),
      StopReason::Refusal => return Ok(Stop::Refusal),
      _ => return Ok(Stop::EndTurn),
    }
  }
}

/// Record what the request's prefix is built from, and a `view` event when that changed since the last request: a
/// changed prompt, tool set or model is where a provider's prompt cache stops matching
fn log_view(session: &Session, s: &Setup, request: &Request) {
  let view = json!({
    "model": s.pick,
    "prompt": { "variant": s.prompt.0, "version": s.prompt.1, "digest": s.prompt.2 },
    "tools": request.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
  });
  let prev = session.state.lock().last_view.replace(view.clone());
  if let Some(prev) = prev
    && prev != view
  {
    let changed: Vec<&str> = ["model", "prompt", "tools"].into_iter().filter(|k| prev[k] != view[k]).collect();
    session.store.append(json!({ "type": "view", "changed": changed, "from": prev, "to": view }));
  }
}

/// What every `request` event carries: the model, its source and the prompt and policies the call was built with
fn request_event(s: &Setup) -> Value {
  json!({
    "type": "request",
    "model": s.pick,
    "source": s.provider.id,
    "format": s.provider.format,
    "prompt": { "variant": s.prompt.0, "version": s.prompt.1, "digest": s.prompt.2 },
    // Context policies in effect; only the tool output budget exists so far
    "policies": { "outputBudget": { "lines": crate::budget::MAX_LINES, "bytes": crate::budget::MAX_BYTES } },
  })
}

/// A model call still open when the turn's future is dropped (a cancel) is logged on the way out with stop
/// `Cancelled`, so the log keeps one record per call, including the ones a provider may bill without an answer
struct Unfinished<'a> {
  session: &'a Session,
  event: Option<Value>,
  started: std::time::Instant,
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

/// One `request` event per model call
fn log_request(session: &Session, s: &Setup, started: std::time::Instant, stop: Option<&StopReason>, usage: Option<&Usage>, error: Option<&str>) {
  let mut ev = request_event(s);
  ev["ms"] = json!(started.elapsed().as_millis() as u64);
  if let Some(u) = usage {
    ev["usage"] = json!({ "input": u.input, "output": u.output, "cacheRead": u.cache_read, "cacheWrite": u.cache_write, "reasoning": u.reasoning });
    // At list prices: an estimate, not the bill (discounts, tiers and gateways' markups are not known here)
    if let Some(c) = &s.cost {
      ev["cost"] = json!(c.estimate(u.input, u.output, u.cache_read, u.cache_write));
    }
  }
  if let Some(stop) = stop {
    ev["stop"] = Value::String(format!("{stop:?}"));
  }
  if let Some(e) = error {
    ev["error"] = Value::String(e.to_owned());
  }
  session.store.append(ev);
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
fn shown_name<'a>(name: &'a str, tools: &[&'static str]) -> &'a str {
  match names::resolve(name, tools) {
    Resolved::Exact(n) | Resolved::Corrected(n) => n,
    Resolved::Unknown(_) => name,
  }
}

/// The title a tool call starts with: the tool's name, except Plan mode's exit, which shows as the plan step the
/// webview already knows (`Exit plan mode`, localized there) from the first update on
fn call_title<'a>(name: &'a str, tools: &[&'static str]) -> &'a str {
  match shown_name(name, tools) {
    tools::EXIT_PLAN => EXIT_PLAN_TITLE,
    n => n,
  }
}

const EXIT_PLAN_TITLE: &str = "Exit plan mode";

/// ACP ToolKind for a tool name, before its arguments are known
fn kind_of(name: &str, tools: &[&'static str]) -> &'static str {
  match shown_name(name, tools) {
    tools::READ | tools::LIST => "read",
    tools::WRITE | tools::EDIT => "edit",
    tools::BASH => "execute",
    tools::GREP | tools::GLOB => "search",
    tools::EXIT_PLAN => "switch_mode",
    _ => "other",
  }
}

pub fn user_home() -> Option<std::path::PathBuf> {
  std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from)
}

/// The permission layers of a turn, evaluated in this order: the mode last, so its restrictions hold under any approval
/// level and over what the user allowed earlier
struct Rules {
  defaults: Vec<Rule>,
  approval: Vec<Rule>,
  mode: Vec<Rule>,
  guard: Guard,
}

impl Rules {
  fn decide(&self, session: &Session, action: &Action) -> Decision {
    self.decide_with(session, action, None)
  }

  /// The decision with one more session rule: an "always allow" answer is offered only when it would take effect
  fn decide_with(&self, session: &Session, action: &Action, extra: Option<&Rule>) -> Decision {
    let (key, target) = action.permission(&session.cwd);
    let mut allowed = session.state.lock().allowed.clone();
    allowed.extend(extra.cloned());
    let d = permission::evaluate(&[&self.defaults, &self.approval, &allowed, &self.mode], key, &target);
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

/// How a tool round leaves the turn
enum Flow {
  Continue,
  /// The user rejected an action or did not approve the plan: the turn ends after the round
  Stop,
  /// A plan was approved: the session is in Agent mode and the turn goes on under it
  Switched,
}

/// Run one round of tool calls
async fn run_tools(
  server: &Arc<Server>,
  session: &Arc<Session>,
  setup: &Setup,
  calls: &[ToolCall],
  ids: &mut HashMap<String, String>,
  corrections: &mut u32,
) -> Flow {
  let (rules, tool_names) = (&setup.rules, setup.tools);
  let cwd = session.cwd.clone();
  let mut prepared = vec![];
  for call in calls {
    let acp = match ids.remove(&call.id) {
      Some(a) => a,
      None => {
        let a = session.next_tool_id();
        server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": a, "title": call_title(&call.name, tool_names), "kind": kind_of(&call.name, tool_names), "status": "pending" }));
        a
      }
    };
    let args: Result<Value, String> = serde_json::from_str::<Value>(&call.arguments)
      .map_err(|e| format!("The arguments are not valid JSON ({e}). Received: {}", crate::budget::cut(&call.arguments, 2000)))
      .and_then(|v| if v.is_object() { Ok(v) } else { Err(format!("The arguments must be a JSON object. Received: {}", crate::budget::cut(&call.arguments, 2000))) });
    let (name, note) = match names::resolve(&call.name, tool_names) {
      Resolved::Exact(n) => (Ok(n), None),
      Resolved::Corrected(n) if *corrections < MAX_CORRECTIONS => {
        *corrections += 1;
        (Ok(n), Some(format!("(\"{}\" was taken as the {n} tool; call tools by their exact names.)", call.name)))
      }
      Resolved::Corrected(n) => (Err(format!("Unknown tool \"{}\" (did you mean \"{n}\"?). Tool names must match exactly: {}.", call.name, tool_names.join(", "))), None),
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
  let mut switched = false;
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
    // The plan approval card is this call's permission
    if let Action::ExitPlan { plan } = &action {
      server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
      let (out, approved) = exit_plan(server, session, p, plan.as_deref()).await;
      finish(server, session, p, out);
      match approved {
        Some(true) => switched = true,
        Some(false) => rejected = true,
        None => {}
      }
      continue;
    }
    match rules.decide(session, &action) {
      Decision::Allow => {}
      Decision::Deny => {
        let (key, target) = action.permission(&cwd);
        finish(server, session, p, Output::error(format!("Not allowed: the permission rules deny {key} on {target}. Do not retry it.")));
        continue;
      }
      Decision::Ask => match ask(server, session, rules, p, &action).await {
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
  if rejected {
    Flow::Stop
  } else if switched {
    Flow::Switched
  } else {
    Flow::Continue
  }
}

/// Plan mode's exit: the plan file goes to the user on the host's plan approval card (`acpira/planApproval`, and the
/// `switch_mode` + `rawInput.plan` shape with `planFilePath`, which `transcript/plans.rs` takes as the whole document).
/// Some(true) when approved, Some(false) when not, None when there was nothing to ask about
async fn exit_plan(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, plan: Option<&str>) -> (Output, Option<bool>) {
  if session.state.lock().mode != modes::PLAN {
    return (Output::error("The session is not in Plan mode, so there is no plan to submit."), None);
  }
  let path = server.plan_file(&session.id);
  if let Some(text) = plan {
    let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, text));
    if let Err(e) = written {
      return (Output::error(format!("Could not write the plan to {}: {e}", path.display())), None);
    }
  }
  let text = std::fs::read_to_string(&path).unwrap_or_default();
  if text.trim().is_empty() {
    return (Output::error(format!("There is no plan yet. Write it to {} (or pass it as \"plan\"), then call exit_plan again.", path.display())), None);
  }
  let raw_input = json!({ "plan": text, "planFilePath": path });
  let meta = json!({ "acpira/planApproval": true });
  let title = EXIT_PLAN_TITLE;
  server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "title": title, "kind": "switch_mode", "rawInput": raw_input, "_meta": meta }));
  let request = json!({
    "sessionId": session.id,
    "toolCall": { "toolCallId": p.acp, "title": title, "kind": "switch_mode", "status": "pending", "rawInput": raw_input, "_meta": meta },
    // The webview localizes these two labels on a plan card
    "options": [
      { "optionId": "approved", "name": "Build", "kind": "allow_once" },
      { "optionId": "rejected", "name": "Revise", "kind": "reject_once" },
    ],
  });
  let answer = server.conn().request("session/request_permission", request).await;
  let approved = answer.is_ok_and(|r| {
    r.pointer("/outcome/outcome").and_then(Value::as_str) == Some("selected") && r.pointer("/outcome/optionId").and_then(Value::as_str) == Some("approved")
  });
  if !approved {
    let out = Output::error("The user did not approve the plan yet. Stop here and wait for their feedback, then revise the plan file and call exit_plan again.");
    return (out, Some(false));
  }
  {
    let mut st = session.state.lock();
    st.mode = modes::AGENT.to_owned();
    st.announced = modes::AGENT.to_owned();
    session.save_state(&st);
  }
  server.update(&session.id, json!({ "sessionUpdate": "current_mode_update", "currentModeId": modes::AGENT }));
  let model = format!("The user approved the plan. {} Carry out the plan now, keeping the to-do list current.", modes::get(modes::AGENT).overlay(&path));
  (Output { model, is_error: false, content: vec![], raw_output: None }, Some(true))
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
async fn ask(server: &Arc<Server>, session: &Arc<Session>, rules: &Rules, p: &Prepared, action: &Action) -> Answer {
  let pres = action.describe(&session.cwd);
  let raw_input = match action {
    Action::Bash { command, .. } => json!({ "command": command }),
    _ => serde_json::from_str(&p.call.arguments).unwrap_or(Value::Null),
  };
  // Offered only when the rule would hold: a mode rule or the guard can still ask after it
  let always = action.always().filter(|(pattern, _)| {
    let rule = Rule::new(action.permission(&session.cwd).0, pattern, Decision::Allow);
    rules.decide_with(session, action, Some(&rule)) == Decision::Allow
  });
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
  let item = Item::ToolResult { call_id: p.call.id.clone(), name: p.call.name.clone(), content, is_error: out.is_error };
  session.store.item(&item);
  session.state.lock().items.push(item);
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
      native: None,
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
