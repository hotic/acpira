//! One turn: the user's prompt, then model calls and tool rounds until the model stops. The configuration, tool set,
//! system prompt and model parameters are read once at the start and stay fixed for the turn; a settings change applies
//! from the next one.
//!
//! Cancellation: `session/cancel` fires the turn's signal, the turn answers `cancelled` at once and sends nothing after
//! it. Its future is dropped, which kills a running command's process group; a model stream's reading thread notices
//! the closed channel at its next chunk and drops the connection. The history keeps what finished, plus the cut-off
//! text and a "cancelled" result for every tool call left without one, so the next request is well formed
//!
//! Steering (`steer.rs`): a message the client steers in while the turn runs goes in front of the turn's next model
//! call, and a stop that would end the turn (a reply without tool calls, a rejected card) does not while one waits

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::RpcError;

use crate::acp::{Server, Session, SessionState};
use crate::compact::{self, Policy, Reason};
use crate::goal;
use crate::llm::{self, Event, Item, LlmError, Part, Request, StopReason, ToolCall, Usage};
use crate::modes;
use crate::skills;
use crate::tools;

mod approval;
mod calls;
mod request;
mod riders;
mod setup;

use self::calls::{Flow, call_title, hidden, kind_of, mcp_meta, run_tools};
use self::request::{
  EMPTY_REPLY, MAX_ATTEMPTS, Unfinished, approx, describe, empty_reply, log_request, log_view, max_tokens, request_event, retriable,
  retry_delay, retry_notice, stream_idle,
};
pub use self::setup::limits;
use self::setup::{
  compact_history, offers_ask, plugin_tools, refresh_limits, report_usage, send_usage, setup, sidekick_note, sidekick_unlisted,
  summary_call,
};

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
  /// Tool calls run, the goal's own left out (`goal::end_turn`)
  tools: u64,
  /// The turn ended waiting for the user: an action rejected, the questions closed, the plan not approved
  held: bool,
}

pub async fn run(server: Arc<Server>, params: Value, cancel: Cancel) -> Result<Value, RpcError> {
  let sid = params.get("sessionId").and_then(Value::as_str).ok_or_else(|| RpcError::new(-32602, "sessionId is required"))?;
  let session = server.session(sid)?;
  let prompt = params.get("prompt").unwrap_or(&Value::Null);
  let text = prompt_text(prompt);
  let compacting = skills::is_compact(&text);
  // A `/goal …` command (`goal.rs`): set, resume and continue run a goal turn, the others are answered at once
  let command = goal::command(&text);
  let instant = matches!(command, Some(goal::Command::Show | goal::Command::Pause | goal::Command::Clear));
  let first = {
    let mut st = session.state.lock();
    if st.turn.is_some() {
      return Err(RpcError::new(-32603, "A turn is already running in this session"));
    }
    st.turn = Some(cancel.clone());
    // Messages may be steered in from now on, except into a turn that calls no model they could reach (a /compact, a
    // goal command answered at once)
    st.steerable = !compacting && !instant;
    goal::start_clock(&mut st);
    // A turn the client named can be rewound to (`_session/rewind`): the mark goes before anything the turn writes, with
    // what the model had been told so far, which the turn may change in its user message, and the goal as it stood
    if let Some(turn) = params.pointer("/_meta/acpira~1turn").and_then(Value::as_str).filter(|t| !t.is_empty()) {
      session.store.append(json!({
        "type": "mark", "turn": turn, "announced": st.announced, "mcp_note": st.mcp_note, "sidekick_told": st.sidekick_told,
        "goal": st.goal,
      }));
    }
    !st.titled && !st.items.iter().any(|i| matches!(i, Item::User(_)))
  };
  let (answer, kind) = match command.clone().map(|c| goal::begin(&server, &session, c)) {
    Some(goal::Begin::Answer(a)) => (Some(a), None),
    Some(goal::Begin::Turn(k)) => (None, Some(k)),
    None => (None, None),
  };
  if answer.is_some() {
    // A continue that found nothing to continue calls no model either
    session.state.lock().steerable = false;
  }
  // The first prompt also names the session, beside the turn and outside its cancel: a goal set by its objective; a
  // /compact and the other goal commands name nothing
  if first && !compacting {
    let source = match (&command, kind) {
      (None, _) => Some(prompt.clone()),
      (Some(goal::Command::Set(objective)), Some(goal::Kind::Set)) => Some(json!([{ "type": "text", "text": objective }])),
      _ => None,
    };
    if let Some(source) = source
      && crate::title::spawn(&server, &session, &source)
    {
      session.state.lock().titled = true;
    }
  }
  let stats = Arc::new(parking_lot::Mutex::new(Stats::default()));
  let result = match &answer {
    Some(reply) => {
      answer_goal_command(&server, &session, prompt, reply, command == Some(goal::Command::Continue));
      Ok(Stop::EndTurn)
    }
    None => tokio::select! {
      r = body(&server, &session, &params, &stats, kind) => r,
      _ = cancel.cancelled() => Ok(Stop::Cancelled),
    },
  };
  // No more steering. A message no call carried (cancelled, failed, stopped at a limit) stays in the history behind
  // what the turn left, so the next turn's model gets what the user sent and saw land in this turn
  let leftover = crate::steer::close(&server, &session);
  {
    let mut st = session.state.lock();
    st.turn = None;
    let before = st.items.len();
    settle(&mut st);
    if let Some((item, prompts)) = leftover {
      crate::steer::record(&session, &prompts);
      st.items.push(item);
    }
    for item in &st.items[before..] {
      session.store.item(item);
    }
  }
  // The goal's side of the end: a turn that ran the model may leave it paused or blocked (`goal::end_turn`), before the
  // response, so the client decides on continuing with the goal's new status
  if answer.is_none() && !compacting {
    let (worked, held) = {
      let s = stats.lock();
      (s.tools, s.held)
    };
    let outcome = match &result {
      Ok(Stop::Cancelled) => goal::Outcome::Cancelled,
      Ok(Stop::Refusal) | Err(_) => goal::Outcome::Failed,
      Ok(_) if held => goal::Outcome::Held,
      Ok(_) => goal::Outcome::Ended,
    };
    goal::end_turn(&server, &session, kind, outcome, worked);
  } else {
    goal::stop_clock(&server, &session);
  }
  // Old tool output is cleared between turns, as OpenCode does, once enough of it has piled up and a compaction is near
  let pick = stats.lock().pick.clone();
  if prune_due(&server, &session, &params, pick.as_deref()) {
    compact::prune(&session);
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

/// A goal command answered at once: the prompt and the answer go into the replay, nothing into the history. A
/// continuation is the client's, not the user's, so the replay leaves its prompt out
fn answer_goal_command(server: &Server, session: &Session, prompt: &Value, reply: &str, continuation: bool) {
  if !continuation {
    for block in prompt.as_array().into_iter().flatten() {
      session.store.record_update(&json!({ "sessionUpdate": "user_message_chunk", "content": block }));
    }
  }
  if !reply.is_empty() {
    server.update(&session.id, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": reply } }));
  }
}

/// The share of the compaction threshold (tenths) the last call must reach before old tool output is pruned
const PRUNE_AT_TENTHS: u64 = 7;

/// Whether to prune after this turn. A prune rewrites the history from the first result it clears, so the provider's
/// prompt cache misses there once (seen 2026-10-13: the call after a prune read only the system prompt and tools from
/// cache); far from the threshold that miss costs more than the smaller requests save before the next compaction.
/// So only with automatic compaction on, the summary strategy picked (chunks fold the finished turns' output
/// themselves) and the last call at `PRUNE_AT_TENTHS` of the threshold or more, where the prune puts a compaction off
fn prune_due(server: &Server, session: &Session, params: &Value, pick: Option<&str>) -> bool {
  let policy = Policy::from_prompt(params);
  let config = server.config.get();
  if !policy.auto || config.chunked() || session.state.lock().mode == modes::RAW {
    return false;
  }
  let Some((used, size)) = session.state.lock().reported else { return false };
  let output = pick.and_then(|p| limits(&config, &server.catalog.get(), p)).and_then(|m| m.output);
  used >= policy.threshold(size, output) * PRUNE_AT_TENTHS / 10
}

/// Close the history after a turn ended any way: cut-off text becomes an assistant message, and every tool call
/// without a result gets one
pub(crate) fn settle(st: &mut SessionState) {
  let text = std::mem::take(&mut st.partial_text);
  let reasoning = std::mem::take(&mut st.partial_reasoning);
  if !text.is_empty() {
    st.items.push(Item::Assistant { text, reasoning, tool_calls: vec![], native: None });
  }
  let Some(last_call) = st.items.iter().rposition(|i| matches!(i, Item::Assistant { tool_calls, .. } if !tool_calls.is_empty())) else { return };
  // Only results after that reply answer it: providers reuse call ids across replies, so an older result with the
  // same id says nothing about the current call
  let answered: std::collections::HashSet<&str> = st.items[last_call + 1..]
    .iter()
    .filter_map(|i| match i {
      Item::ToolResult { call_id, .. } => Some(call_id.as_str()),
      _ => None,
    })
    .collect();
  let Item::Assistant { tool_calls, .. } = &st.items[last_call] else { return };
  let missing: Vec<ToolCall> = tool_calls.iter().filter(|c| !answered.contains(c.id.as_str())).cloned().collect();
  // A change that landed before its command was cut off is reported as made, so the model does not make it again
  let landed = std::mem::take(&mut st.landed);
  for c in missing {
    let item = match landed.iter().find(|(id, _)| *id == c.id) {
      Some((_, text)) => Item::ToolResult { call_id: c.id, name: c.name, content: text.clone(), is_error: false, images: vec![] },
      None => Item::ToolResult { call_id: c.id, name: c.name, content: "Cancelled by the user before it finished.".into(), is_error: true, images: vec![] },
    };
    st.items.push(item);
  }
}

/// The prompt's text blocks, joined (attachments and images left out)
pub(crate) fn prompt_text(prompt: &Value) -> String {
  prompt
    .as_array()
    .into_iter()
    .flatten()
    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
    .filter_map(|b| b.get("text").and_then(Value::as_str))
    .collect::<Vec<_>>()
    .join("\n")
}

async fn body(server: &Arc<Server>, session: &Arc<Session>, params: &Value, stats: &Arc<parking_lot::Mutex<Stats>>, kind: Option<goal::Kind>) -> Result<Stop, String> {
  // The MCP servers still starting are waited for here, so the tool set is complete from the first request on
  let mut offer = session.mcp.offer(&tools::names::ALL).await;
  // A sidekick picked where acpira offered no ask_agent at all (no persona when its tools were listed) has no tool a
  // note could name: acpira lists its tools once more, and the table grows by ask_agent with the sidekick first
  if sidekick_unlisted(server, session, &offer) {
    session.mcp.relist(crate::acp::ACPIRA_SERVER);
    offer = session.mcp.offer(&tools::names::ALL).await;
    let config = server.config.get();
    let mut st = session.state.lock();
    st.sidekick_told = crate::acp::sidekick_of(&config, &st);
  }
  let mut s = setup(server, session, params, &offer.tools)?;
  // Which of the plugins' hooks into this loop are declared, for this turn
  crate::plugin_hooks::refresh(server, session).await;
  plugin_tools(server, session, &mut s).await;
  let prompt = params.get("prompt").unwrap_or(&Value::Null);
  let text = prompt_text(prompt);
  // /compact summarizes the history and ends the turn; nothing of it enters the history
  if skills::is_compact(&text) {
    match compact_history(server, session, &s, Reason::Manual, None).await {
      Ok(Some(_)) => report_usage(server, session, &s),
      Ok(None) => server.update(&session.id, json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Nothing to compact yet." } })),
      Err(_) => {}
    }
    return Ok(Stop::EndTurn);
  }
  // A goal turn's message opens with the goal's note (`goal::turn_parts`); the goal may have been cleared meanwhile
  let goal_now = session.state.lock().goal.clone();
  let (mut parts, words) = match (kind, &goal_now) {
    (None, _) => (prompt_parts(prompt, s.model.takes_images()), text.as_str()),
    (Some(kind), Some(g)) => {
      let note = goal::note(kind, g, &goal::objective_file(server, session));
      let words = if kind == goal::Kind::Set { g.objective.as_str() } else { "" };
      (goal::turn_parts(kind, note, Some(&g.objective), prompt, s.model.takes_images()), words)
    }
    (Some(_), None) => return Ok(Stop::EndTurn),
  };
  // A `/name` anywhere in the user's words puts the skill's instructions behind the message (a goal's objective, not
  // the `/goal` in front of it)
  let expansion = skills::expansion(&skills::mentioned(words, &s.skills));
  if !expansion.is_empty() {
    match parts.last_mut() {
      Some(Part::Text(t)) => t.push_str(&expansion),
      _ => parts.push(Part::Text(expansion.trim_start().to_owned())),
    }
  }
  {
    let mut st = session.state.lock();
    // A mode entered since the model last heard about it is announced in the user message: the system prompt stays fixed.
    // Raw mode is announced to no one, and Agent mode right after it needs no note: its system prompt says it all
    if st.mode != st.announced {
      let overlay = modes::get(&st.mode).overlay(&server.plan_file(&session.id));
      let silent = s.raw || (st.mode == modes::AGENT && st.announced == modes::RAW);
      st.announced = st.mode.clone();
      if !silent {
        match parts.first_mut() {
          Some(Part::Text(t)) => *t = format!("<mode>\n{overlay}\n</mode>\n\n{t}"),
          _ => parts.insert(0, Part::Text(format!("<mode>\n{overlay}\n</mode>"))),
        }
      }
    }
    // The MCP servers' instructions (and the ones that failed) the same way, when they changed since the model heard.
    // Raw mode offers no MCP tools; the note waits for the first turn of a mode that does
    if !s.raw && offer.note != st.mcp_note {
      st.mcp_note = offer.note.clone();
      if let Some(note) = &offer.note {
        match parts.first_mut() {
          Some(Part::Text(t)) => *t = format!("<mcp>\n{note}\n</mcp>\n\n{t}"),
          _ => parts.insert(0, Part::Text(format!("<mcp>\n{note}\n</mcp>"))),
        }
      }
    }
    // A sidekick picked, changed or dropped since the model learned of it is told the same way while ask_agent is
    // offered: the tool table stays as it was listed, so the prompt cache holds
    let sidekick = crate::acp::sidekick_of(&server.config.get(), &st);
    if !s.raw && sidekick != st.sidekick_told && offers_ask(&offer) {
      let note = sidekick_note(&server.config.get(), st.sidekick_told.is_some(), sidekick.as_deref());
      st.sidekick_told = sidekick;
      match parts.first_mut() {
        Some(Part::Text(t)) => *t = format!("<sidekick>\n{note}\n</sidekick>\n\n{t}"),
        _ => parts.insert(0, Part::Text(format!("<sidekick>\n{note}\n</sidekick>"))),
      }
    }
    let item = Item::User(parts);
    session.store.item(&item);
    st.items.push(item);
  }
  // The replay shows the prompt as the user sent it, without the mode note; a continuation is the client's, not the
  // user's, and stays out
  if kind != Some(goal::Kind::Continue) {
    for block in params.get("prompt").and_then(Value::as_array).into_iter().flatten() {
      session.store.record_update(&json!({ "sessionUpdate": "user_message_chunk", "content": block }));
    }
  }
  let mut corrections = 0u32;
  let mut step = 0u32;
  loop {
    if s.model.max_steps.is_some_and(|max| step >= max) {
      return Ok(Stop::MaxTurnRequests);
    }
    step += 1;
    // What the user steered in since the last call goes in front of this one, behind the round's tool results. The
    // first call carries the prompt alone, as Codex samples a turn's own input first
    if step > 1 {
      crate::steer::take(session, s.model.takes_images(), &s.skills);
      // The goal's budget ran out in this turn: the model is told to wrap up (`goal::account`)
      goal::take_note(session);
    }
    {
      let mut st = stats.lock();
      st.pick = Some(s.pick.clone());
      st.prompt = Some(s.prompt.clone());
    }
    refresh_limits(server, &mut s);
    let mut request = Request {
      model: s.model.id.clone(),
      system: s.system.clone(),
      items: session.state.lock().items.clone(),
      tools: s.specs.clone(),
      max_tokens: max_tokens(&s.model),
      output_limit: s.model.output,
      sampling: s.sampling.clone(),
      thinking: s.thinking,
      effort: s.effort.clone(),
      cache_key: Some(session.id.clone()),
      fast: s.fast,
    };
    // The `context` hook: this call's system prompt, messages, tools and options, for this call only; `model.request`
    // adds headers to it
    if let Some(out) = crate::plugin_hooks::ask(server, session, "context", crate::plugin_hooks::request_json(&request)).await {
      crate::plugin_hooks::apply_request(&mut request, &out);
    }
    let mut endpoint = s.endpoint.clone();
    if let Some(out) = crate::plugin_hooks::ask(server, session, "model.request", json!({ "model": request.model, "url": endpoint.url, "headers": {} })).await {
      endpoint.headers.extend(crate::plugin_hooks::headers_of(&out));
    }
    log_view(session, &s, &request);
    let mut input_estimate = approx(&request.system, &request.items, &request.tools);
    // Model tool-call id → ACP tool call id: providers reuse ids across calls, ACP needs them unique per session
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut attempt = 0u32;
    // A request refused for its size is compacted and sent again: once, or twice when chunks went first and the
    // retry still overflowed, which the summary then answers
    let mut overflows = 0u32;
    let mut summarized_overflow = false;
    let (reply, stop, usage, started) = loop {
      attempt += 1;
      let started = std::time::Instant::now();
      let mut unfinished = Unfinished { session, event: Some(request_event(&s)), started };
      let mut rx = llm::stream(server.http.clone(), endpoint.clone(), request.clone());
      let mut outcome = Err(LlmError::Protocol("the stream ended without a result".into()));
      let idle = stream_idle();
      loop {
        let ev = match tokio::time::timeout(idle, rx.recv()).await {
          Ok(Some(ev)) => ev,
          Ok(None) => break,
          // A stream gone silent is dropped (its reader thread stops at the next chunk) and retried like a broken one
          Err(_) => {
            outcome = Err(LlmError::Network(format!("no data from the provider for {} s", idle.as_secs())));
            break;
          }
        };
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
            let names = s.names();
            if hidden(&name, &names) {
              continue;
            }
            let acp = session.next_tool_id();
            let mut call = json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": call_title(&name, &names), "kind": kind_of(&name, &names), "status": "pending" });
            if let Some(meta) = mcp_meta(&name, &s) {
              call["_meta"] = meta;
            }
            server.update(&session.id, call);
            ids.insert(id, acp);
          }
          // Already counted: receiving it restarted the idle timeout
          Ok(Event::Alive) => {}
          Ok(Event::Done { reply, stop, usage }) => {
            outcome = Ok((reply, stop, usage));
            break;
          }
          Err(e) => {
            outcome = Err(e);
            break;
          }
        }
      }
      unfinished.event = None;
      let e = match outcome {
        Ok((reply, stop, usage)) if !empty_reply(&reply, &stop) => break (reply, stop, usage, started),
        // Nothing but reasoning: retried like a broken stream, the attempt logged with what it cost
        Ok((reply, stop, usage)) => {
          let e = LlmError::Protocol(EMPTY_REPLY.into());
          log_request(session, &s, started, attempt, Some(&stop), usage.as_ref(), false, Some(&e.to_string()), Some(&reply));
          e
        }
        Err(e) => {
          log_request(session, &s, started, attempt, None, None, false, Some(&e.to_string()), None);
          e
        }
      };
      // Refused for its size: the provider says so, or refuses a request near the window without saying why
      let near = input_estimate >= s.model.context.unwrap_or(DEFAULT_CONTEXT) * 4 / 5;
      // Raw mode leaves the provider's refusal as the turn's error rather than writing a summary of its own
      let overflow = !s.raw && (compact::is_overflow(&e) || (near && compact::is_refusal(&e))) && !summarized_overflow && overflows < 2;
      if !overflow && (attempt >= MAX_ATTEMPTS || !retriable(&e)) {
        return Err(describe(s.provider.display_name(), &e));
      }
      // The failed attempt's output is dropped: its tool rows are settled and the next attempt streams afresh
      for acp in ids.drain().map(|(_, acp)| acp) {
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "status": "failed" }));
      }
      {
        let mut st = session.state.lock();
        st.partial_text.clear();
        st.partial_reasoning.clear();
      }
      if overflow {
        overflows += 1;
        // Compacted, the same step goes out again with the compacted history; nothing to compact (or a failed
        // summary) leaves the provider's refusal as the turn's error
        let compacted = if overflows == 1 {
          compact_history(server, session, &s, Reason::Overflow, None).await
        } else {
          compact::run(server, session, &summary_call(server, &s), Reason::Overflow).await.map(|done| done.then_some(false))
        };
        match compacted {
          Ok(Some(chunked)) => {
            summarized_overflow = !chunked;
            request.items = session.state.lock().items.clone();
            input_estimate = approx(&request.system, &request.items, &request.tools);
            report_usage(server, session, &s);
            continue;
          }
          _ => return Err(describe(s.provider.display_name(), &e)),
        }
      }
      let wait = retry_delay(&e, attempt);
      server.update(&session.id, retry_notice(s.provider.display_name(), &e, attempt + 1, wait));
      tokio::time::sleep(wait).await;
    };
    // A window edited while the call ran decides this step's compaction already
    refresh_limits(server, &mut s);
    let context = s.model.context.unwrap_or(DEFAULT_CONTEXT);
    // A call always has input, so a reported 0 is a provider gap (a gateway's streamed usage): estimate it instead
    let estimated = usage.is_some_and(|u| u.input == 0);
    let usage = usage.map(|u| if estimated { Usage { input: input_estimate, ..u } } else { u });
    log_request(session, &s, started, attempt, Some(&stop), usage.as_ref(), estimated, None, Some(&reply));
    // The call's tokens toward the goal, and its budget (`goal::account`)
    goal::account(server, session, usage.as_ref());
    if reply.cut.is_some() {
      // The call that was still streaming is lost: its row is settled
      for (_, acp) in ids.extract_if(|id, _| !reply.tool_calls.iter().any(|c| &c.id == id)) {
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "status": "failed" }));
      }
    }
    // The call's own size against the threshold: at or over it the history is compacted once this step's tool
    // results are in, so the next call is the small one (Claude Code and Codex compact mid-turn the same way)
    let mut over = false;
    // The provider's own count of this call, for chunk compaction to scale its estimates by (a stand-in is no count)
    let mut measured = None;
    if let Some(u) = usage {
      let mut st = stats.lock();
      st.usage.input += u.input;
      st.usage.output += u.output;
      st.usage.cache_read += u.cache_read;
      st.usage.cache_write += u.cache_write;
      st.usage.reasoning += u.reasoning;
      st.calls += 1;
      let used = u.input + u.output;
      send_usage(server, session, used, context);
      over = s.policy.auto && used >= s.policy.threshold(context, s.model.output);
      measured = (!estimated).then_some(crate::chunk::Measured { input: u.input, items: request.items.len() });
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
    // `stopped`: the user stopped the round (an action rejected, the questions closed, the plan not approved)
    let (flow, stopped) = match stop {
      _ if !reply.tool_calls.is_empty() && stop != StopReason::MaxTokens => {
        let names = s.names();
        stats.lock().tools += reply.tool_calls.iter().filter(|c| !hidden(&c.name, &names)).count() as u64;
        let flow = run_tools(server, session, &s, &reply.tool_calls, &mut ids, &mut corrections).await;
        let stopped = matches!(flow, Flow::Stop);
        (flow, stopped)
      }
      StopReason::MaxTokens => return Ok(Stop::MaxTokens),
      StopReason::Refusal => return Ok(Stop::Refusal),
      _ => (Flow::Stop, false),
    };
    if over && let Ok(Some(_)) = compact_history(server, session, &s, Reason::Threshold, measured).await {
      report_usage(server, session, &s);
    }
    match flow {
      Flow::Continue => {}
      Flow::Stop if crate::steer::may_end(session) => {
        // A turn the user stopped waits for their direction, and so does its goal (`goal::Outcome::Held`)
        stats.lock().held = stopped;
        return Ok(Stop::EndTurn);
      }
      // A message steered in meanwhile keeps the turn going: the model answers it here (Codex's follow-up rule), also
      // after a rejected card, where the message is the direction the model was told to wait for
      Flow::Stop => {}
      // The approved plan is carried out in this turn, with Agent mode's tools and rules
      Flow::Switched => {
        s = setup(server, session, params, &offer.tools)?;
        plugin_tools(server, session, &mut s).await;
      }
    }
  }
}

pub fn user_home() -> Option<std::path::PathBuf> {
  std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(std::path::PathBuf::from)
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
  let mut merged = merge_text(parts);
  if merged.is_empty() {
    merged.push(Part::Text(String::new()));
  }
  merged
}

/// Adjacent text parts joined into one, as a message reads
pub(crate) fn merge_text(parts: Vec<Part>) -> Vec<Part> {
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
  merged
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
    st.items.push(Item::ToolResult { call_id: "a".into(), name: "read".into(), content: "ok".into(), is_error: false, images: vec![] });
    settle(&mut st);
    assert!(matches!(&st.items[3], Item::ToolResult { call_id, is_error: true, .. } if call_id == "b"));
    st.partial_text = "half an ans".into();
    settle(&mut st);
    assert!(matches!(st.items.last(), Some(Item::Assistant { text, .. }) if text == "half an ans"));
  }

  #[test]
  fn settle_ignores_an_earlier_result_with_a_reused_call_id() {
    let mut st = SessionState::default();
    let call = |name: &str| Item::Assistant {
      text: String::new(),
      reasoning: String::new(),
      tool_calls: vec![ToolCall { id: "c1".into(), name: name.into(), arguments: "{}".into() }],
      native: None,
    };
    st.items.push(call("read"));
    st.items.push(Item::ToolResult { call_id: "c1".into(), name: "read".into(), content: "ok".into(), is_error: false, images: vec![] });
    st.items.push(call("bash"));
    settle(&mut st);
    assert_eq!(st.items.len(), 4);
    assert!(matches!(&st.items[3], Item::ToolResult { call_id, name, is_error: true, .. } if call_id == "c1" && name == "bash"));
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
