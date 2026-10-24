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
use acpira_shared::model_catalog::Cost;
use acpira_shared::providers::{Provider, ProviderModel, Sampling, Thinking, model_pick};

use crate::acp::{Server, Session, SessionState};
use crate::compact::{self, Policy, Reason};
use crate::goal;
use crate::llm::{self, Endpoint, Event, Item, LlmError, Part, Request, StopReason, ToolCall, ToolSpec, Usage};
use crate::{modes, prompt};
use crate::permission::{self, Decision, Guard, Rule};
use crate::skills::{self, Skill};
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
  /// The session's Fast switch, where the model offers a fast tier (`llm::fast`)
  fast: bool,
  /// The model's sampling over its prompt variant's defaults
  sampling: Sampling,
  thinking: Thinking,
  system: String,
  /// (variant, version, digest) of the system prompt
  prompt: (String, String, String),
  tools: &'static [&'static str],
  specs: Vec<ToolSpec>,
  rules: Rules,
  /// The host's auto-compaction setting, off the prompt's `_meta`
  policy: Policy,
  /// The skills found at the start of the turn (`skills.rs`); with any, `skill` is among the tools
  skills: Vec<Skill>,
  /// The MCP servers' tools, offered after the built-in ones (`mcp.rs`)
  mcp: Vec<crate::mcp::Tool>,
  /// The model takes images, so images an MCP tool returns go to it
  images: bool,
  /// Raw mode (`modes::RAW`): no tools, no composed prompt, and no compaction but an explicit /compact
  raw: bool,
}

impl Setup {
  /// Every tool name of the turn, built-in first
  fn names(&self) -> Vec<&str> {
    self.tools.iter().copied().chain(self.mcp.iter().map(|t| t.name.as_str())).collect()
  }
}

fn setup(server: &Server, session: &Session, params: &Value, mcp: &[crate::mcp::Tool]) -> Result<Setup, String> {
  let config = server.config.get();
  if let Some(e) = &config.error {
    return Err(e.clone());
  }
  let (pick, effort, fast, mode) = {
    let mut st = session.state.lock();
    // A session opened before any model was configured takes the default now, and keeps it, with that model's effort
    if crate::acp::reconcile(&config, &server.catalog.get(), &mut st) {
      session.save_state(&st);
    }
    (st.model.clone(), st.effort.clone(), st.fast, modes::get(&st.mode))
  };
  let pick = pick.ok_or("No model is configured. Add a model source in Acpira's settings (Agents → Acpira).")?;
  let (provider, model) = config.find(&pick).ok_or_else(|| format!("The model {pick} is no longer configured; pick another one."))?;
  let fast = fast && llm::fast::offered(provider, model);
  let endpoint = Endpoint::of(provider, model, config.api_key(&provider.id))
    .ok_or_else(|| format!("{}: the API format \"{}\" is not supported", provider.display_name(), provider.format))?;
  let raw = mode.id == modes::RAW;
  // The prompt changes only when the model now matches another variant (a view change at a turn boundary). Raw mode
  // sends the settings page's text alone and leaves the session's composed prompt for the turn that switches back
  let places = prompt::Places::new(&session.cwd, user_home().as_deref());
  let composed = if raw {
    prompt::raw(config.raw_system_prompt())
  } else {
    let variant = prompt::variant_name(&places, Some((provider, model)));
    let mut st = session.state.lock();
    if st.prompt.variant != variant || st.prompt.text.is_empty() {
      st.prompt = prompt::compose(&places, Some((provider, model)));
      session.save_prompt(&st.prompt);
    }
    st.prompt.clone()
  };
  let approval = config.approval();
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
    // Full access widens the mode's ask rules (Plan mode's commands then run); what the mode denies does not move
    mode: mode.rules(&server.plan_file(&session.id), &session.cwd, approval),
    guard: Guard::new(server.config.home(), &session.cwd, user_home().as_deref()),
  };
  // Limits the configuration leaves open come from the catalogue, marked estimated (never sent as max_tokens)
  let mut model = model.clone();
  let cost = crate::catalog::complete(&mut model, &server.catalog.get());
  // Raw mode offers nothing: no tools, skills or MCP servers (so no `/skill` expansion either)
  let skills = if raw { vec![] } else { skills::scan(&places) };
  let mcp = if raw { &[] } else { mcp };
  // Chunk compaction brings its history tools; the set is fixed for the turn like the rest
  let tools: &'static [&'static str] = match (raw, config.chunked(), skills.is_empty()) {
    (true, _, _) => &[],
    (false, false, true) => modes::TOOLS,
    (false, false, false) => modes::TOOLS_WITH_SKILL,
    (false, true, true) => modes::TOOLS_CHUNKED,
    (false, true, false) => modes::TOOLS_CHUNKED_WITH_SKILL,
  };
  let mut specs = tools::specs(tools);
  if !skills.is_empty() {
    specs.push(tools::skill::spec(&skills));
  }
  specs.extend(mcp.iter().map(crate::mcp::Tool::spec));
  let images = model.takes_images();
  // Raw mode never compacts on its own: a summary in the history is the agent's text, so only /compact writes one
  let mut policy = Policy::from_prompt(params);
  policy.auto &= !raw;
  Ok(Setup {
    pick,
    provider: provider.clone(),
    model,
    endpoint,
    cost,
    effort,
    fast,
    sampling,
    thinking,
    system: composed.text,
    prompt: (composed.variant, composed.version, composed.digest),
    tools,
    specs,
    rules,
    policy,
    skills,
    mcp: mcp.to_vec(),
    images,
    raw,
  })
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

/// A compaction's models: the turn's model sets the tail; the summary is written by the fast model of the settings
/// page when one is set and usable (read at the moment, like the title's), then by the turn's model when that fails
fn summary_call<'a>(server: &Server, s: &'a Setup) -> compact::Call<'a> {
  let mut writers = Vec::with_capacity(2);
  let config = server.config.get();
  if let Some((provider, model)) = config.fast_model()
    && model_pick(&provider.id, &model.id) != s.pick
    && let Some(endpoint) = Endpoint::of(provider, model, config.api_key(&provider.id))
  {
    let mut model = model.clone();
    let catalog = server.catalog.get();
    let cost = crate::catalog::complete(&mut model, &catalog);
    let pick = model_pick(&provider.id, &model.id);
    // The fast model thinks at the level a new session on it would start with
    let effort = crate::acp::default_effort(&config, &catalog, Some(&pick));
    writers.push(compact::Writer { endpoint, thinking: model.thinking, model, pick, cost, effort });
  }
  writers.push(compact::Writer { endpoint: s.endpoint.clone(), model: s.model.clone(), pick: s.pick.clone(), cost: s.cost, thinking: s.thinking, effort: s.effort.clone() });
  let limit = s.policy.threshold(s.model.context.unwrap_or(DEFAULT_CONTEXT), s.model.output);
  compact::Call { http: server.http.clone(), model: &s.model, writers, limit }
}

/// Compact the history the way the settings page picked, read at the moment: chunks first when picked (no model
/// call, `chunk.rs`), the summary when chunking found nothing to fold (except on `/compact`, which then has nothing to
/// do) or could not read the log. Ok(Some(chunked)) when the history changed
async fn compact_history(server: &Server, session: &Session, s: &Setup, reason: Reason, measured: Option<crate::chunk::Measured>) -> Result<Option<bool>, String> {
  // Chunks point the model at history tools Raw mode does not offer, so a /compact there always summarizes
  if server.config.get().chunked() && !s.raw {
    let context = s.model.context.unwrap_or(DEFAULT_CONTEXT);
    let usable = compact::usable(context, s.model.output);
    let limits = crate::chunk::Limits {
      usable,
      goal: match reason {
        Reason::Manual => None,
        Reason::Threshold => Some(s.policy.threshold(context, s.model.output)),
        Reason::Overflow => Some(usable),
      },
      force_fold: reason == Reason::Overflow,
      fixed: approx(&s.system, &[], &s.specs),
      measured,
    };
    match crate::chunk::run(server, session, reason, &limits) {
      Ok(true) => return Ok(Some(true)),
      Ok(false) if reason == Reason::Manual => return Ok(None),
      Ok(false) => {}
      Err(e) => eprintln!("[acpira agent] chunk compaction failed, summarizing instead: {e}"),
    }
  }
  compact::run(server, session, &summary_call(server, s), reason).await.map(|done| done.then_some(false))
}

/// The context after a compaction, as an estimate until the next call reports it
fn report_usage(server: &Server, session: &Session, s: &Setup) {
  let used = approx(&s.system, &session.state.lock().items, &s.specs);
  send_usage(server, session, used, s.model.context.unwrap_or(DEFAULT_CONTEXT));
}

/// A `usage_update`, remembered so a window edited later can be pushed with the same `used` (`Server::push_controls`)
fn send_usage(server: &Server, session: &Session, used: u64, size: u64) {
  session.state.lock().reported = Some((used, size));
  server.update(&session.id, json!({ "sessionUpdate": "usage_update", "used": used, "size": size }));
}

/// A configured model with the limits the configuration leaves open filled from the catalogue (marked estimated)
pub fn limits(config: &crate::config::Config, catalog: &acpira_shared::model_catalog::Catalog, pick: &str) -> Option<ProviderModel> {
  let (_, m) = config.find(pick)?;
  let mut model = m.clone();
  crate::catalog::complete(&mut model, catalog);
  Some(model)
}

/// Take the model's window and output limit as configured now. The rest of the setup stays frozen for the turn (the
/// prompt cache keys on it), but a limit edited in the settings mid-turn applies from the next model call, where
/// before it waited for the next turn and a window corrected from a 128k guess still compacted at the old threshold
fn refresh_limits(server: &Server, s: &mut Setup) {
  if let Some(m) = limits(&server.config.get(), &server.catalog.get(), &s.pick) {
    s.model.context = m.context;
    s.model.output = m.output;
    s.model.estimated = m.estimated;
  }
}

/// acpira's ask_agent, which the sidekick is summoned through
fn offers_ask(offer: &crate::mcp::Offer) -> bool {
  offer.tools.iter().any(|t| t.server == crate::acp::ACPIRA_SERVER && t.tool == "ask_agent")
}

/// A sidekick the model has not learned of, with no ask_agent to summon it through (Raw mode offers no MCP tools at all)
fn sidekick_unlisted(server: &Server, session: &Session, offer: &crate::mcp::Offer) -> bool {
  if offers_ask(offer) {
    return false;
  }
  let config = server.config.get();
  let st = session.state.lock();
  let now = crate::acp::sidekick_of(&config, &st);
  st.mode != modes::RAW && now.is_some() && now != st.sidekick_told
}

/// What the model is told of a sidekick picked (`had`: there was one before), changed or dropped since it last heard
fn sidekick_note(config: &crate::config::Config, had: bool, now: Option<&str>) -> String {
  let name = |pick: &str| config.find(pick).map(|(_, m)| m.display_name().to_owned()).unwrap_or_else(|| pick.to_owned());
  match (had, now) {
    (_, None) => "You no longer have a sidekick: agent \"sidekick\" of acpira_ask_agent is gone. Do that work yourself or give it to \
another agent."
      .to_owned(),
    (false, Some(pick)) => format!(
      "You now have a sidekick, {}: summon it with acpira_ask_agent, agent \"sidekick\". It works at its own pace, often \
slower than you: hand it work that runs while you do something else (edits you can describe precisely, scouting that does \
not decide the plan) or a second look at your finished change, not a quick step you would then wait for, and keep the \
decisions yourself; check its diff afterwards, and when it falls short say so in the same thread.",
      name(pick)
    ),
    (true, Some(pick)) => format!("Your sidekick now runs {} (acpira_ask_agent, agent \"sidekick\"); its thread continues.", name(pick)),
  }
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
      let mut rx = llm::stream(server.http.clone(), s.endpoint.clone(), request.clone());
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
      server.update(&session.id, retry_notice(s.provider.display_name(), &e, attempt + 1));
      tokio::time::sleep(retry_delay(&e, attempt)).await;
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
      Flow::Switched => s = setup(server, session, params, &offer.tools)?,
    }
  }
}

/// Record what the request's prefix is built from, and a `view` event when that changed since the last request: a
/// changed prompt, tool set or model is where a provider's prompt cache stops matching, and so is a switch between the
/// standard and the fast tier (Anthropic documents that a speed change invalidates the cache). The tool set counts as
/// changed when a definition does, not only a name: an MCP server's relisting after `tools/list_changed` (acpira's
/// `ask_agent` when the roster is edited) keeps the names and rewrites a description and an enum
fn log_view(session: &Session, s: &Setup, request: &Request) {
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
fn request_event(s: &Setup) -> Value {
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

/// A stand-in for a prompt count the provider reported as 0, and the context right after a compaction: about four
/// bytes a token over the text sent, a thousand per image. Rough, and only ever used until a provider reports
fn approx(system: &str, items: &[Item], tools: &[ToolSpec]) -> u64 {
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
fn stream_idle() -> std::time::Duration {
  static IDLE: std::sync::OnceLock<std::time::Duration> = std::sync::OnceLock::new();
  *IDLE.get_or_init(|| {
    let secs = std::env::var("ACPIRA_AGENT_STREAM_IDLE_SECS").ok().and_then(|v| v.parse().ok()).filter(|s| *s > 0).unwrap_or(300);
    std::time::Duration::from_secs(secs)
  })
}

/// Attempts per model call: the first and up to three retries
const MAX_ATTEMPTS: u32 = 4;

/// Why an empty reply is retried, also the turn's error once the attempts run out
const EMPTY_REPLY: &str = "the model ended its reply without any text or tool call";

/// A reply that ends the turn with nothing to show: no text, no tool call, at most reasoning. A gateway that converts
/// Gemini's `functionCall` buffers the whole call and, past its own timeout, sends only the reasoning and then a normal
/// `response.completed` (`gemini-4-argon` on the Responses gateway, 2026-10-22, after 130 s and 156 s while writing a
/// long file); Gemini CLI retries such a stream too (`InvalidStreamError`). An output limit or a refusal is its own
/// stop and is left alone
fn empty_reply(reply: &llm::Reply, stop: &StopReason) -> bool {
  !matches!(stop, StopReason::MaxTokens | StopReason::Refusal) && reply.cut.is_none() && reply.tool_calls.is_empty() && reply.text.trim().is_empty()
}

/// Worth another attempt: the connection, the stream or the service failed, not the request itself. A gateway ending a
/// 200 stream with an `error` event ("response stream interrupted", seen on 2026-10-11 on a Claude route, 1 call in 40
/// or so) is the common case; a rejected key, a bad request or a context overflow would only fail again
fn retriable(e: &LlmError) -> bool {
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
fn retry_delay(e: &LlmError, attempt: u32) -> std::time::Duration {
  match e {
    LlmError::Http { retry_after: Some(d), .. } => (*d).min(std::time::Duration::from_secs(60)),
    _ => std::time::Duration::from_secs(1 << (attempt - 1).min(5)),
  }
}

/// A retry in progress, in the shape the host already labels for other agents (an AIR `sessionFailure` warning
/// titled "attempt N of M", see `retry_of_failure` on the host): the working label shows it until the stream resumes
fn retry_notice(source: &str, e: &LlmError, next: u32) -> Value {
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
#[allow(clippy::too_many_arguments)]
fn log_request(
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

/// A call of a goal tool, which shows no row: the goal's own rows stand for it (`goal.rs`)
fn hidden(name: &str, tools: &[&str]) -> bool {
  goal::is_tool(shown_name(name, tools))
}

/// The tool a model-given name stands for, for display before the call is prepared
fn shown_name<'a>(name: &'a str, tools: &[&'a str]) -> &'a str {
  match names::resolve(name, tools) {
    Resolved::Exact(n) | Resolved::Corrected(n) => n,
    Resolved::Unknown(_) => name,
  }
}

/// The engine's `acpira/mcp` note for a call of an MCP tool (by the name the model wrote, corrected like any other)
fn mcp_meta(name: &str, s: &Setup) -> Option<Value> {
  let names = s.names();
  let resolved = shown_name(name, &names);
  s.mcp.iter().find(|t| t.name == resolved).map(crate::mcp::Tool::meta)
}

/// The title a tool call starts with: the tool's name, except Plan mode's exit, which shows as the plan step the
/// webview already knows (`Exit plan mode`, localized there) from the first update on
fn call_title<'a>(name: &'a str, tools: &[&'a str]) -> &'a str {
  match shown_name(name, tools) {
    tools::EXIT_PLAN => EXIT_PLAN_TITLE,
    // Read as an ask row before the arguments are known
    tools::QUESTION => "Ask questions",
    n => n,
  }
}

const EXIT_PLAN_TITLE: &str = "Exit plan mode";

/// ACP ToolKind for a tool name, before its arguments are known
fn kind_of(name: &str, tools: &[&str]) -> &'static str {
  match shown_name(name, tools) {
    tools::READ | tools::LIST | tools::SKILL => "read",
    tools::WRITE | tools::EDIT => "edit",
    tools::BASH | tools::JOB => "execute",
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
    let mut allowed = session.state.lock().allowed.clone();
    allowed.extend(extra.cloned());
    // The strictest decision over every target the call touches
    let d = action
      .permissions(&session.cwd)
      .iter()
      .map(|(key, target)| permission::evaluate(&[&self.defaults, &self.approval, &allowed, &self.mode], key, target))
      .max_by_key(|d| match d {
        Decision::Allow => 0,
        Decision::Ask => 1,
        Decision::Deny => 2,
      })
      .unwrap_or(Decision::Ask);
    if d == Decision::Allow && action.written().iter().any(|p| self.guard.protects(p)) {
      return Decision::Ask;
    }
    d
  }

  /// The (permission, target) a call the rules deny is denied on: the first of its targets that is
  fn denied(&self, session: &Session, action: &Action) -> (&'static str, String) {
    let allowed = session.state.lock().allowed.clone();
    action
      .permissions(&session.cwd)
      .into_iter()
      .find(|(key, target)| permission::evaluate(&[&self.defaults, &self.approval, &allowed, &self.mode], key, target) == Decision::Deny)
      .unwrap_or_else(|| action.permission(&session.cwd))
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

/// A tool call after parsing and preparation
struct Prepared {
  call: ToolCall,
  acp: String,
  action: Result<Action, String>,
  /// Put before the result: the misnamed call was taken as another tool
  note: Option<String>,
  /// The `acpira/mcp` note of an MCP call, on its permission card too
  meta: Option<Value>,
  riders: Riders,
  /// The note the call's result starts with when it carried the to-do list (`todos`), set while preparing
  todo: Option<String>,
}

/// What a call of a built-in tool carries beside its own arguments, so a model that makes one call per reply needs no
/// request of its own for it (`tools::todo::rider`, `tools::edit::then_run`). Each rider shows as a row of its own,
/// under the rules its own tool would be; the model gets one result for the call
#[derive(Default)]
struct Riders {
  /// `todos`: the to-do list, set before the call runs
  todos: Option<Result<Vec<tools::todo::Todo>, String>>,
  /// `then_run` of an edit or a write: the command to run once the change is made, or why it will not run
  then_run: Option<Result<Action, String>>,
}

impl Riders {
  /// The riders of a call of this built-in tool (an MCP tool's arguments are its server's; the todo tool's list is its
  /// own argument)
  fn of(name: &str, args: &Value, cwd: &std::path::Path) -> Riders {
    let todos = if name == tools::TODO { None } else { tools::todo::rider(args) };
    let then_run = tools::edit::then_run(args).map(|command| match name {
      tools::EDIT | tools::WRITE => tools::bash::prepare(&json!({ "command": command }), cwd),
      _ => Err(format!("then_run is taken only by edit and write, so `{command}` was not run.")),
    });
    Riders { todos, then_run }
  }

  /// The note on a then_run that cannot run whatever the call does
  fn refused(&self) -> Option<String> {
    match &self.then_run {
      Some(Err(e)) => Some(e.clone()),
      _ => None,
    }
  }

  /// The note on a then_run left out, and why: the change was never made
  fn skipped(&self, why: &str) -> Option<String> {
    match &self.then_run {
      Some(Ok(Action::Bash { command, .. })) => Some(format!("then_run `{command}` was not run: {why}.")),
      _ => self.refused(),
    }
  }
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
  let (rules, tool_names) = (&setup.rules, setup.names());
  let tool_names = &tool_names[..];
  let cwd = session.cwd.clone();
  let mut prepared = vec![];
  for (k, call) in calls.iter().enumerate() {
    let meta = mcp_meta(&call.name, setup);
    let announce = || {
      let a = session.next_tool_id();
      let mut first = json!({ "sessionUpdate": "tool_call", "toolCallId": a, "title": call_title(&call.name, tool_names), "kind": kind_of(&call.name, tool_names), "status": "pending" });
      if let Some(m) = &meta {
        first["_meta"] = m.clone();
      }
      server.update(&session.id, first);
      a
    };
    let streamed = ids.remove(&call.id);
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
    // A goal tool's call has no row (`hidden`), unless the stream opened one for a name it took for another tool
    let rowless = streamed.is_none() && matches!(&name, Ok(n) if goal::is_tool(n));
    let riders = match (name.as_ref().ok().copied(), &args) {
      (Some(n), Ok(a)) if !rowless && !setup.mcp.iter().any(|t| t.name == n) => Riders::of(n, a, &cwd),
      _ => Riders::default(),
    };
    // The list a call carries is set now, on a row above the call's own, so the transcript reads in the order things
    // happen. A row the stream opened for the reply's last call becomes the list's row and the call gets a new one
    // below it; any other streamed row stays the call's (a new row would land below the calls after it), so the list
    // follows it there
    let (acp, todo) = match streamed {
      _ if rowless => (String::new(), None),
      Some(row) if k + 1 == calls.len() && todo_row_allowed(session, rules, &riders) => {
        let todo = ride_todo(server, session, rules, &riders, Some(row));
        (announce(), todo)
      }
      Some(row) => (row, ride_todo(server, session, rules, &riders, None)),
      None => {
        let todo = ride_todo(server, session, rules, &riders, None);
        (announce(), todo)
      }
    };
    // A bad argument goes back with what was received, so the model can see its own mistake
    let action = name.and_then(|n| {
      let a = args.clone()?;
      // An MCP tool's arguments are the server's to check against its schema
      if let Some(t) = setup.mcp.iter().find(|t| t.name == n) {
        return Ok(tools::mcp::prepare(t, &a, setup.images));
      }
      tools::prepare(n, &a, &cwd).map_err(|e| format!("{e}. Received arguments: {}", crate::budget::cut(&a.to_string(), 2000)))
    });
    let raw_input = args.unwrap_or_else(|_| Value::String(call.arguments.clone()));
    match &action {
      _ if rowless => {}
      Ok(a) => {
        let p = a.describe(&cwd);
        let mut update = json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "title": p.title, "kind": p.kind, "rawInput": raw_input,
          "locations": p.locations.iter().map(tools::Location::json).collect::<Vec<_>>(), "content": p.content });
        if let Some(m) = &meta {
          update["_meta"] = m.clone();
        }
        server.update(&session.id, update);
      }
      Err(_) => {
        server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "rawInput": raw_input }));
      }
    }
    prepared.push(Prepared { call: call.clone(), acp, action, note, meta, riders, todo });
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
        // An MCP call waits on its server; the rest read files or the network on a blocking thread
        handles.push(match action {
          mcp @ Action::Mcp { .. } => tokio::spawn(mcp.run(ctx)),
          other => tokio::task::spawn_blocking(move || other.run_sync(ctx)),
        });
      }
      for (p, h) in prepared[start..i].iter().zip(handles) {
        let out = h.await.unwrap_or_else(|e| Output::error(format!("The tool crashed: {e}")));
        finish(server, session, p, out, p.todo.clone(), p.riders.refused());
      }
      continue;
    }
    let p = &prepared[i];
    i += 1;
    if rejected {
      finish(server, session, p, Output::error("Skipped: the user rejected an earlier action in this round."), p.todo.clone(), None);
      continue;
    }
    // The list was set while preparing: it moves on whatever comes of the call itself
    let todo = p.todo.clone();
    let action = match &p.action {
      Ok(a) => a.clone(),
      Err(e) => {
        finish(server, session, p, Output::error(e.clone()), todo, p.riders.skipped("the change failed"));
        continue;
      }
    };
    // A goal tool works on the session itself: no permission to ask, and the result is the model's alone
    if let Action::Goal(call) = &action {
      finish(server, session, p, goal::run_tool(server, session, call), todo, None);
      continue;
    }
    // The plan approval card is this call's permission
    if let Action::ExitPlan { plan } = &action {
      server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
      let (out, approved) = exit_plan(server, session, p, plan.as_deref()).await;
      finish(server, session, p, out, todo, p.riders.refused());
      match approved {
        Some(true) => switched = true,
        Some(false) => rejected = true,
        None => {}
      }
      continue;
    }
    // The question card needs no permission; closing it ends the turn
    if let Action::Question { questions } = &action {
      server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
      let (out, dismissed) = question(server, session, p, questions).await;
      finish(server, session, p, out, todo, p.riders.refused());
      rejected |= dismissed;
      continue;
    }
    match rules.decide(session, &action) {
      Decision::Allow => {}
      Decision::Deny => {
        let (key, target) = rules.denied(session, &action);
        finish(server, session, p, Output::error(format!("Not allowed: the permission rules deny {key} on {target}. Do not retry it.")), todo, p.riders.skipped("the change was not allowed"));
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
          finish(server, session, p, Output::error("The user rejected this action. Do not retry it; wait for the user's direction."), todo, None);
          continue;
        }
      },
    }
    server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": p.acp, "status": "in_progress" }));
    // A write of what the file already holds is no change to check
    let unchanged = matches!(&action, Action::Write { files } if files.iter().all(tools::edit::Change::unchanged));
    let out = action.run(ctx_for(server, session, &p.acp)).await;
    let command = match &p.riders.then_run {
      Some(Ok(command)) => command,
      _ => {
        finish(server, session, p, out, todo, p.riders.refused());
        continue;
      }
    };
    if out.is_error || unchanged {
      let why = if out.is_error { "the change failed" } else { "the file already held this content" };
      finish(server, session, p, out, todo, p.riders.skipped(why));
      continue;
    }
    let Action::Bash { command: line, .. } = command else { unreachable!("then_run prepares a bash action") };
    // The change is made: its row completes now, the command gets a row of its own, and the call's one result waits
    // for both (a turn cancelled meanwhile still tells the model the change was made)
    let images = images_of(p, &out);
    let (model, is_error) = report(server, session, &p.acp, out);
    let cut = format!("then_run `{line}` was cancelled by the user before it finished.");
    session.state.lock().landed.push((p.call.id.clone(), joined(todo.as_deref(), &model, Some(&cut))));
    let (section, stop) = ride_command(server, session, rules, command).await;
    session.state.lock().landed.retain(|(id, _)| *id != p.call.id);
    rejected |= stop;
    record(session, p, joined(todo.as_deref(), &model, Some(&section)), is_error, images);
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
    return (Output::error(format!("There is no plan yet. Write it to {}, then call exit_plan again.", path.display())), None);
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
  let model = format!("The user approved the plan. {} Carry out the plan now, item by item in the to-do list's order: mark an item in_progress when you start it and \
completed once it is checked. If the plan turns out wrong, correct the plan file before going on.", modes::get(modes::AGENT).overlay(&path));
  (Output { model, is_error: false, content: vec![], raw_output: None }, Some(true))
}

/// The `question` tool: an `elicitation/create` form on the host's question card. The bool is true when the user
/// closed the card (or the client refused the form), which ends the turn like a rejected action
async fn question(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, questions: &[tools::question::Question]) -> (Output, bool) {
  if !server.forms() {
    return (Output::error("This client cannot show questions. Ask in plain text instead and end your turn."), false);
  }
  let request = tools::question::form(&session.id, &p.acp, questions);
  let out = match server.conn().request("elicitation/create", request).await {
    Ok(r) => tools::question::reply(questions, &r),
    Err(e) => return (Output::error(format!("The questions could not be shown ({}). Ask in plain text instead and end your turn.", e.message)), false),
  };
  match out {
    // A skip is an answer too, so the row does not read as failed
    tools::question::Reply::Answered(text) => (answered(text), false),
    tools::question::Reply::Skipped => (answered("The user skipped the questions without answering. Go on with your own judgement and say what you assumed.".into()), false),
    tools::question::Reply::Dismissed => (Output::error("The user closed the questions. Stop here and wait for their direction."), true),
  }
}

/// The answers are the model's copy only: the question card below the row already shows them
fn answered(text: String) -> Output {
  Output { content: vec![], model: text, is_error: false, raw_output: None }
}

fn ctx_for(server: &Arc<Server>, session: &Arc<Session>, acp: &str) -> Ctx {
  let (srv, sid, id) = (server.clone(), session.id.clone(), acp.to_owned());
  Ctx {
    cwd: session.cwd.clone(),
    outputs: server.session_dir(&session.id).join("outputs"),
    call_id: acp.to_owned(),
    jobs: session.jobs.clone(),
    search: server.config.get().web_search(),
    mcp: session.mcp.clone(),
    history: server.session_file(&session.id),
    progress: Arc::new(move |mut u: Value| {
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
  let mut request = json!({
    "sessionId": session.id,
    "toolCall": {
      "toolCallId": p.acp, "title": pres.title, "kind": pres.kind, "status": "pending", "rawInput": raw_input,
      "locations": pres.locations.iter().map(tools::Location::json).collect::<Vec<_>>(), "content": pres.content,
    },
    "options": options,
  });
  if let Some(m) = &p.meta {
    request["toolCall"]["_meta"] = m.clone();
  }
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

/// Whether a call's to-do list gets a row: it parses and the rules allow setting it
fn todo_row_allowed(session: &Arc<Session>, rules: &Rules, riders: &Riders) -> bool {
  match &riders.todos {
    Some(Ok(todos)) => rules.decide(session, &Action::Todo { todos: todos.clone() }) == Decision::Allow,
    _ => false,
  }
}

/// The to-do list a call carries (`todos`), set before the call runs: a `todo` row of its own, the way the todo tool's
/// call shows, so the host's to-do bar takes it unchanged; on `row`, a row the stream opened, when given. The note in
/// front of the call's result
fn ride_todo(server: &Arc<Server>, session: &Arc<Session>, rules: &Rules, riders: &Riders, row: Option<String>) -> Option<String> {
  let todos = match riders.todos.as_ref()? {
    Ok(todos) => todos.clone(),
    Err(e) => return Some(format!("The to-do list was not updated: {e}.")),
  };
  if !todo_row_allowed(session, rules, riders) {
    return Some("The to-do list was not updated: the permission rules do not allow it.".into());
  }
  let raw = tools::todo::raw(&todos);
  let acp = match row {
    // The streamed row was the call's: every field it showed is set again
    Some(row) => {
      server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": row, "title": tools::TODO, "kind": "other", "status": "pending", "rawInput": raw }));
      row
    }
    None => {
      let acp = session.next_tool_id();
      server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": tools::TODO, "kind": "other", "status": "pending", "rawInput": raw }));
      acp
    }
  };
  report(server, session, &acp, tools::todo::run(&todos));
  Some(tools::todo::note(&todos))
}

/// The command an edit or a write carries (`then_run`), once the change is made: on a command row of its own, under
/// the rules a `bash` call would be, with its own card when they ask. The section for the call's result, and whether
/// the user rejected the command, which ends the turn after the round as a rejected call does
async fn ride_command(server: &Arc<Server>, session: &Arc<Session>, rules: &Rules, action: &Action) -> (String, bool) {
  let Action::Bash { command, .. } = action else { unreachable!("then_run prepares a bash action") };
  let acp = session.next_tool_id();
  let raw_input = json!({ "command": command });
  server.update(&session.id, json!({ "sessionUpdate": "tool_call", "toolCallId": acp, "title": command, "kind": "execute", "status": "pending", "rawInput": raw_input }));
  let header = format!("$ {command}");
  match rules.decide(session, action) {
    Decision::Allow => {}
    Decision::Deny => {
      let (key, target) = rules.denied(session, action);
      report(server, session, &acp, Output::error(format!("Not allowed: the permission rules deny {key} on {target}.")));
      return (format!("{header}\nNot run: the permission rules deny {key} on {target}. Do not retry it."), false);
    }
    Decision::Ask => {
      let call = ToolCall { id: acp.clone(), name: tools::BASH.into(), arguments: raw_input.to_string() };
      let p = Prepared { call, acp: acp.clone(), action: Ok(action.clone()), note: None, meta: None, riders: Riders::default(), todo: None };
      match ask(server, session, rules, &p, action).await {
        Answer::Once => {}
        Answer::Always(pattern) => session.state.lock().allowed.push(Rule::new(permission::BASH, &pattern, Decision::Allow)),
        Answer::Reject => {
          report(server, session, &acp, Output::error("The user rejected this action."));
          return (format!("{header}\nNot run: the user rejected it. Do not retry it; wait for the user's direction."), true);
        }
      }
    }
  }
  server.update(&session.id, json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "status": "in_progress" }));
  let out = action.clone().run(ctx_for(server, session, &acp)).await;
  let section = format!("{header}\n{}", out.model);
  report(server, session, &acp, out);
  (section, false)
}

/// A call's result with the riders' notes: the to-do note in front (a command's exit code stays its last line, where
/// the chunk index reads it), the command after the change behind it
fn joined(front: Option<&str>, model: &str, back: Option<&str>) -> String {
  let mut text = String::new();
  if let Some(f) = front {
    text.push_str(f);
    text.push_str("\n\n");
  }
  text.push_str(model);
  if let Some(b) = back {
    text.push_str("\n\n");
    text.push_str(b);
  }
  text
}

/// An MCP result's images go to the model when it takes them; the card shows them either way
fn images_of(p: &Prepared, out: &Output) -> Vec<crate::llm::Image> {
  match &p.action {
    Ok(Action::Mcp { images: true, .. }) => tools::mcp::model_images(&out.content),
    _ => vec![],
  }
}

/// Report a finished call on its row and add its result, with the riders' notes, to the history
fn finish(server: &Arc<Server>, session: &Arc<Session>, p: &Prepared, out: Output, front: Option<String>, back: Option<String>) {
  let images = images_of(p, &out);
  // A call without a row (a goal tool's) only goes into the history
  let (model, is_error) = if p.acp.is_empty() { (out.model, out.is_error) } else { report(server, session, &p.acp, out) };
  record(session, p, joined(front.as_deref(), &model, back.as_deref()), is_error, images);
}

/// The model's result of a call, into the history
fn record(session: &Arc<Session>, p: &Prepared, model: String, is_error: bool, images: Vec<crate::llm::Image>) {
  let content = match &p.note {
    Some(note) => format!("{note}\n{model}"),
    None => model,
  };
  let item = Item::ToolResult { call_id: p.call.id.clone(), name: p.call.name.clone(), content, is_error, images };
  session.store.item(&item);
  session.state.lock().items.push(item);
}

/// Report a finished call on its row (`acp`); the model's copy and whether it failed come back
fn report(server: &Arc<Server>, session: &Arc<Session>, acp: &str, mut out: Output) -> (String, bool) {
  for c in &mut out.content {
    let emptied = c.get_mut("_meta").and_then(Value::as_object_mut).is_some_and(|m| {
      m.remove("acpira/notForModel");
      m.is_empty()
    });
    if emptied && let Some(o) = c.as_object_mut() {
      o.remove("_meta");
    }
  }
  let mut update = json!({ "sessionUpdate": "tool_call_update", "toolCallId": acp, "status": if out.is_error { "failed" } else { "completed" } });
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
  (out.model, out.is_error)
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
