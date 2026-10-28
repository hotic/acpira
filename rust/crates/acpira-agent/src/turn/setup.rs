//! What a turn runs with: the model, tools and prompt layers set up from the session and the request, the context
//! window's limits and usage reports, compaction, and the sidekick note

use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::model_catalog::Cost;
use acpira_shared::providers::{Provider, ProviderModel, Sampling, Thinking, model_pick};

use super::approval::Rules;
use super::request::approx;
use super::{DEFAULT_CONTEXT, user_home};
use crate::acp::{Server, Session};
use crate::compact::{self, Policy, Reason};
use crate::llm::{self, Endpoint, ToolSpec};
use crate::permission::{self, Guard};
use crate::skills::{self, Skill};
use crate::tools;
use crate::{modes, prompt};

/// What the model calls of a turn use: read from the session at the start, and again after an approved plan switched
/// the mode (the user may have picked the model to build with on the plan card)
pub(super) struct Setup {
  pub(super) pick: String,
  pub(super) provider: Provider,
  pub(super) model: ProviderModel,
  pub(super) endpoint: Endpoint,
  /// List prices from the catalogue, when it knows the model
  pub(super) cost: Option<Cost>,
  pub(super) effort: Option<String>,
  /// The session's Fast switch, where the model offers a fast tier (`llm::fast`)
  pub(super) fast: bool,
  /// The model's sampling over its prompt variant's defaults
  pub(super) sampling: Sampling,
  pub(super) thinking: Thinking,
  pub(super) system: String,
  /// (variant, version, digest) of the system prompt
  pub(super) prompt: (String, String, String),
  tools: &'static [&'static str],
  pub(super) specs: Vec<ToolSpec>,
  pub(super) rules: Rules,
  /// The host's auto-compaction setting, off the prompt's `_meta`
  pub(super) policy: Policy,
  /// The skills found at the start of the turn (`skills.rs`); with any, `skill` is among the tools
  pub(super) skills: Vec<Skill>,
  /// The MCP servers' tools, offered after the built-in ones (`mcp.rs`)
  pub(super) mcp: Vec<crate::mcp::Tool>,
  /// The model takes images, so images an MCP tool returns, and image files `read` opens, go to it
  pub(super) images: bool,
  /// Raw mode (`modes::RAW`): no tools, no composed prompt, and no compaction but an explicit /compact
  pub(super) raw: bool,
  /// Tools a specialised agent's profile or the plugins' `tools` hook left out this turn: not offered, and not run when
  /// the model calls one anyway
  withheld: Vec<String>,
}

/// The plugins' `tools` hook on a fresh setup: the turn's tool list with descriptions changed or tools left out
pub(super) async fn plugin_tools(server: &Arc<Server>, session: &Arc<Session>, s: &mut Setup) {
  if let Some(out) = crate::plugin_hooks::ask(server, session, "tools", json!({ "tools": crate::plugin_hooks::tools_json(&s.specs) })).await {
    let before: Vec<String> = s.specs.iter().map(|t| t.name.clone()).collect();
    crate::plugin_hooks::apply_tools(&mut s.specs, &out);
    // On top of what a specialised agent's profile left out already
    s.withheld.extend(before.into_iter().filter(|n| !s.specs.iter().any(|t| &t.name == n)));
  }
}

impl Setup {
  /// Every tool name of the turn, built-in first
  pub(super) fn names(&self) -> Vec<&str> {
    self.tools.iter().copied().chain(self.mcp.iter().map(|t| t.name.as_str())).filter(|n| !self.withheld.iter().any(|w| w == n)).collect()
  }
}

pub(super) fn setup(server: &Server, session: &Session, params: &Value, mcp: &[crate::mcp::Tool]) -> Result<Setup, String> {
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
  let places = server.places_of(&session.cwd, session.remote.as_deref(), session.layers.clone());
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
  // A specialised agent offers only the tools its profile lists, the same set every turn (`profile.rs`)
  let mut withheld = vec![];
  if let Some(p) = &server.profile {
    withheld = specs.iter().filter(|t| !p.allows(&t.name)).map(|t| t.name.clone()).collect();
    specs.retain(|t| p.allows(&t.name));
  }
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
    withheld,
  })
}

/// A compaction's models: the turn's model sets the tail; the summary is written by the fast model of the settings
/// page when one is set and usable (read at the moment, like the title's), then by the turn's model when that fails
pub(super) fn summary_call<'a>(server: &Server, s: &'a Setup) -> compact::Call<'a> {
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
pub(super) async fn compact_history(server: &Server, session: &Session, s: &Setup, reason: Reason, measured: Option<crate::chunk::Measured>) -> Result<Option<bool>, String> {
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
pub(super) fn report_usage(server: &Server, session: &Session, s: &Setup) {
  let used = approx(&s.system, &session.state.lock().items, &s.specs);
  send_usage(server, session, used, s.model.context.unwrap_or(DEFAULT_CONTEXT));
}

/// A `usage_update`, remembered so a window edited later can be pushed with the same `used` (`Server::push_controls`)
pub(super) fn send_usage(server: &Server, session: &Session, used: u64, size: u64) {
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
pub(super) fn refresh_limits(server: &Server, s: &mut Setup) {
  if let Some(m) = limits(&server.config.get(), &server.catalog.get(), &s.pick) {
    s.model.context = m.context;
    s.model.output = m.output;
    s.model.estimated = m.estimated;
  }
}

/// acpira's ask_agent, which the sidekick is summoned through
pub(super) fn offers_ask(offer: &crate::mcp::Offer) -> bool {
  offer.tools.iter().any(|t| t.server == crate::acp::ACPIRA_SERVER && t.tool == "ask_agent")
}

/// A sidekick the model has not learned of, with no ask_agent to summon it through (Raw mode offers no MCP tools at all)
pub(super) fn sidekick_unlisted(server: &Server, session: &Session, offer: &crate::mcp::Offer) -> bool {
  if offers_ask(offer) {
    return false;
  }
  let config = server.config.get();
  let st = session.state.lock();
  let now = crate::acp::sidekick_of(&config, &st);
  st.mode != modes::RAW && now.is_some() && now != st.sidekick_told
}

/// What the model is told of a sidekick picked (`had`: there was one before), changed or dropped since it last heard
pub(super) fn sidekick_note(config: &crate::config::Config, had: bool, now: Option<&str>) -> String {
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
