//! The host's own sidekick select, for a session whose CLI offers none: a second model the lead hands work to, in any
//! CLI Acpira knows the models of. Like Claude's Ultra level (`ultracode.rs`) the select is made here, put back on the
//! option list after every agent answer, and a pick on it never reaches the wire. The pick names the CLI and the model
//! (`<agent>/<model id>`), which `sidekick_persona` (`relay/calls.rs`) summons through the relay like any persona; the
//! built-in agent keeps its own select (`acpira-agent/src/acp.rs`)

use acpira_shared::composer_controls::is_reasoning_control;
use acpira_shared::model_shapes::{ModelShapes, is_model_control, shape_for};
use acpira_shared::models::parse_fusion_name;
use acpira_shared::subagents::SIDEKICK_ID;
use acpira_shared::transcript::{ConfigControl, ControlType, OptionGroup, OptionSource, SessionOption, SourceKind};

use crate::acp::agents::registry::SELF_AGENT_ID;
use crate::acp::session::relay::personas::{
  NO_SIDEKICK, SIDEKICK_CATEGORY, SIDEKICK_EFFORT_CATEGORY, SIDEKICK_EFFORT_CONFIG as NATIVE_EFFORT_CONFIG, SIDEKICK_FAST_CATEGORY,
  SIDEKICK_FAST_CONFIG as NATIVE_FAST_CONFIG,
};
use crate::acp::session::{AcpSession, Core};

/// The host's selects: the built-in agent's categories, which every client draws the sidekick from, under ids of their
/// own, so a CLI's own sidekick select is never taken for the host's
pub(crate) const SIDEKICK_CONFIG: &str = "host_sidekick";
pub(crate) const SIDEKICK_EFFORT_CONFIG: &str = "host_sidekick_effort";

/// One CLI a sidekick can run in: its model select as last seen, and the parameters its models came with
#[derive(Debug, Clone, Default)]
pub struct SidekickSource {
  pub agent: String,
  pub name: String,
  pub model: ConfigControl,
  pub shapes: Option<ModelShapes>,
}

#[derive(Default)]
pub(crate) struct HostSidekick {
  /// This session's CLI gets the host's select (`offered`), decided at each connection
  pub offered: bool,
  /// `<agent>/<model id>`; None: no sidekick
  pub pick: Option<String>,
  pub effort: Option<String>,
  /// The CLIs to pick from, read at each connection
  pub sources: Vec<SidekickSource>,
  /// The selects as last built; empty until a connection read the sources
  pub controls: Vec<ConfigControl>,
  /// What each option of the select runs with once picked (`option_shapes`), built with `controls`
  pub shapes: ModelShapes,
  /// The pick the lead was last told of in a note (`sidekick_note`); `told_any`: it was told once
  pub told: Option<String>,
  pub told_any: bool,
}

pub(crate) fn ours(o: &ConfigControl) -> bool {
  o.id == SIDEKICK_CONFIG || o.id == SIDEKICK_EFFORT_CONFIG
}

/// The CLI's own sidekick select (the built-in agent's): the host adds none beside it
fn native(options: &[ConfigControl]) -> bool {
  options.iter().any(|o| o.category.as_deref() == Some(SIDEKICK_CATEGORY) && !ours(o))
}

/// The model in use (`held`: a pick in flight) is a Devin Fusion pair, a lead and a sidekick of the CLI's own: the host
/// offers no second sidekick beside it, and keeps its pick for when the session leaves the pair
fn fusion_pair(options: &[ConfigControl], held: impl Fn(&str) -> Option<String>) -> bool {
  let Some(model) = options.iter().find(|o| is_model_control(o)) else { return false };
  let Some(value) = held(&model.id).or_else(|| model.value.clone()) else { return false };
  model.options.iter().find(|o| o.id == value).is_some_and(|o| parse_fusion_name(&o.name).is_some())
}

/// A model a sidekick can run on: not a Devin Fusion pair, which is a lead and a sidekick of its own
fn candidate(o: &SessionOption) -> bool {
  parse_fusion_name(&o.name).is_none()
}

/// The CLI and the model id of a pick
pub(crate) fn split_pick(pick: &str) -> Option<(&str, &str)> {
  pick.split_once('/').filter(|(agent, model)| !agent.is_empty() && !model.is_empty())
}

/// The effort select of a source's model, as its CLI last showed it
fn effort_of<'a>(sources: &'a [SidekickSource], pick: &str) -> Option<&'a ConfigControl> {
  let (agent, model) = split_pick(pick)?;
  let src = sources.iter().find(|s| s.agent == agent)?;
  shape_for(src.shapes.as_ref(), &src.model, model)?.iter().find(|c| is_reasoning_control(c) && !c.options.is_empty())
}

/// The CLI an option of the host's select runs in, as its `group` (the agent id and the CLI's name): clients file the
/// rows by CLI first and by channel (`source`) inside one
fn cli_group(agent: &str, name: &str) -> OptionGroup {
  OptionGroup { id: agent.to_owned(), name: name.to_owned() }
}

/// The channel a CLI's model comes through, as that CLI's own list shows it: its source, else its ACP option group
/// (the host's select takes `group` for the CLI)
fn channel(o: &SessionOption) -> Option<OptionSource> {
  o.source.clone().or_else(|| o.group.as_ref().map(|g| OptionSource { id: g.id.clone(), name: g.name.clone(), kind: SourceKind::Official }))
}

/// The sidekick select over every source's models (the session's own CLI first, each option filed under its CLI and,
/// inside it, its channel), and the effort select of the picked model where its CLI has one. A pick no source lists
/// any more stays offered, so a stale model list never drops it. `description` keeps the CLI's name for clients that
/// file by it alone
pub(crate) fn build(own: &str, sources: &[SidekickSource], pick: Option<&str>, effort: Option<&str>) -> Vec<ConfigControl> {
  let mut options = vec![SessionOption { id: NO_SIDEKICK.into(), name: "None".into(), ..Default::default() }];
  let ordered = sources.iter().filter(|s| s.agent == own).chain(sources.iter().filter(|s| s.agent != own));
  for s in ordered {
    options.extend(s.model.options.iter().filter(|o| candidate(o)).map(|o| SessionOption {
      id: format!("{}/{}", s.agent, o.id),
      name: o.name.clone(),
      description: Some(s.name.clone()),
      group: Some(cli_group(&s.agent, &s.name)),
      source: channel(o),
      ..Default::default()
    }));
  }
  if let Some(p) = pick.filter(|p| !options.iter().any(|o| o.id == *p))
    && let Some((agent, model)) = split_pick(p)
  {
    options.push(SessionOption {
      id: p.to_owned(),
      name: model.to_owned(),
      description: Some(agent.to_owned()),
      group: Some(cli_group(agent, agent)),
      ..Default::default()
    });
  }
  let value = pick.filter(|p| options.iter().any(|o| o.id == *p)).unwrap_or(NO_SIDEKICK);
  let mut out = vec![ConfigControl {
    id: SIDEKICK_CONFIG.into(),
    name: "Sidekick".into(),
    category: Some(SIDEKICK_CATEGORY.into()),
    kind: None,
    options,
    value: Some(value.to_owned()),
  }];
  if let Some(levels) = pick.and_then(|p| effort_of(sources, p)) {
    out.push(effort_select(SIDEKICK_EFFORT_CONFIG, levels, effort));
  }
  out
}

/// The sidekick's effort select over a model's levels: the level picked while the model offers it, else the one shown
/// last, else the first
fn effort_select(id: &str, levels: &ConfigControl, effort: Option<&str>) -> ConfigControl {
  let offered = |v: &&str| levels.options.iter().any(|o| o.id == *v);
  let value = effort.filter(offered).or(levels.value.as_deref().filter(offered)).unwrap_or(&levels.options[0].id);
  ConfigControl {
    id: id.into(),
    name: "Sidekick effort".into(),
    category: Some(SIDEKICK_EFFORT_CATEGORY.into()),
    kind: None,
    options: levels.options.clone(),
    value: Some(value.to_owned()),
  }
}

/// A model's levels as its CLI last showed them
fn levels_in(shape: &[ConfigControl]) -> Option<&ConfigControl> {
  shape.iter().find(|c| is_reasoning_control(c) && !c.options.is_empty())
}

/// What each option of the host's select runs with once picked, keyed by option id (the view's `sidekickShapes`): the
/// effort select `build` would show for it. A model whose levels no session showed yet is left out
pub(crate) fn option_shapes(sources: &[SidekickSource], effort: Option<&str>) -> ModelShapes {
  let mut out = ModelShapes::new();
  for s in sources {
    for o in s.model.options.iter().filter(|o| candidate(o)) {
      if let Some(levels) = shape_for(s.shapes.as_ref(), &s.model, &o.id).and_then(|shape| levels_in(shape)) {
        out.insert(format!("{}/{}", s.agent, o.id), vec![effort_select(SIDEKICK_EFFORT_CONFIG, levels, effort)]);
      }
    }
  }
  out
}

/// A Fast switch: a boolean, or the Standard / Fast select (`isFastControl` in packages/shared/composerControls.ts)
fn is_fast(c: &ConfigControl) -> bool {
  match c.kind {
    Some(ControlType::Boolean) => c.id.split(['_', '-', ' ']).any(|w| w.eq_ignore_ascii_case("fast")),
    None => c.options.len() == 2 && ["standard", "fast"].iter().all(|id| c.options.iter().any(|o| o.id == *id)),
  }
}

/// The same for the built-in agent's own select, whose options are its own models: the levels and the Fast switch each
/// came with as the lead (`shapes`), the effort picked for the sidekick kept while a model offers it and its Fast while
/// a model has a fast tier (`sidekick_effort_of` / `sidekick_fast_of` in acpira-agent). None without that select
pub(crate) fn native_option_shapes(options: &[ConfigControl], shapes: Option<&ModelShapes>) -> Option<ModelShapes> {
  let select = options.iter().find(|o| o.category.as_deref() == Some(SIDEKICK_CATEGORY) && !ours(o))?;
  let model = options.iter().find(|o| is_model_control(o))?;
  let value = |category: &str| options.iter().find(|o| o.category.as_deref() == Some(category)).and_then(|o| o.value.as_deref());
  let effort = value(SIDEKICK_EFFORT_CATEGORY);
  let fast = value(SIDEKICK_FAST_CATEGORY).is_some_and(|v| v == "true" || v == "fast");
  let mut out = ModelShapes::new();
  for o in select.options.iter().filter(|o| o.id != NO_SIDEKICK) {
    let Some(shape) = shape_for(shapes, model, &o.id) else { continue };
    let mut controls: Vec<ConfigControl> = levels_in(shape).map(|levels| effort_select(NATIVE_EFFORT_CONFIG, levels, effort)).into_iter().collect();
    if let Some(switch) = shape.iter().find(|c| is_fast(c)) {
      let on = match switch.kind {
        Some(ControlType::Boolean) => fast.to_string(),
        None => (if fast { "fast" } else { "standard" }).to_owned(),
      };
      controls.push(ConfigControl {
        id: NATIVE_FAST_CONFIG.into(),
        name: "Sidekick fast".into(),
        category: Some(SIDEKICK_FAST_CATEGORY.into()),
        kind: switch.kind,
        options: switch.options.clone(),
        value: Some(on),
      });
    }
    if !controls.is_empty() {
      out.insert(o.id.clone(), controls);
    }
  }
  Some(out)
}

/// A reopened record's pick and level, from the selects it was saved with
pub(crate) fn restored(options: &[ConfigControl]) -> HostSidekick {
  let value = |id: &str| options.iter().find(|o| o.id == id).and_then(|o| o.value.clone()).filter(|v| !v.is_empty());
  HostSidekick {
    pick: value(SIDEKICK_CONFIG).filter(|v| v != NO_SIDEKICK),
    effort: value(SIDEKICK_EFFORT_CONFIG),
    ..Default::default()
  }
}

/// The host's select is set aside: the CLI has its own, or the model in use is a Fusion pair
fn set_aside(c: &Core) -> bool {
  let options = &c.state.controls.options;
  native(options) || fusion_pair(options, |id| c.picks.values.get(id).map(|(v, _)| v.clone()))
}

impl AcpSession {
  /// Whether this session gets the host's sidekick select: a CLI without one of its own that starts Acpira's MCP
  /// server (the sidekick is summoned through `ask_agent`), in a local folder (a folder on an SSH host may only summon
  /// the built-in agent, `relay/round.rs`)
  fn host_sidekick_fits(&self) -> bool {
    self.agent != SELF_AGENT_ID
      && crate::shared_config::mcp::takes_mcp(&self.agent)
      && self.deps.host_mcp.as_ref().is_some_and(|h| h.entry_for(&self.agent).is_some())
      && self.remote_folder().is_none()
  }

  /// A connection is being made: read the CLIs a sidekick can run in and build the selects over them
  pub(crate) async fn load_host_sidekick(&self) {
    let offered = self.host_sidekick_fits();
    let sources = if offered { self.deps.policy.sidekick_sources().await } else { vec![] };
    let mut c = self.core.lock();
    c.sidekick.offered = offered;
    c.sidekick.sources = sources;
    self.rebuild_host_sidekick(&mut c);
  }

  fn rebuild_host_sidekick(&self, c: &mut Core) {
    let Core { sidekick, state, .. } = c;
    state.controls.options.retain(|o| !ours(o));
    sidekick.controls = if sidekick.offered { build(&self.agent, &sidekick.sources, sidekick.pick.as_deref(), sidekick.effort.as_deref()) } else { vec![] };
    sidekick.shapes = if sidekick.offered { option_shapes(&sidekick.sources, sidekick.effort.as_deref()) } else { ModelShapes::new() };
    self.place_host_sidekick(c);
  }

  /// Put the selects back after the options changed (`refine_controls`): an agent answer replaces the whole list.
  /// Before the first connection built them, the ones a record or an earlier session showed stay as they are
  pub(crate) fn place_host_sidekick(&self, c: &mut Core) {
    let aside = set_aside(c);
    let Core { sidekick, state, .. } = c;
    let options = &mut state.controls.options;
    if aside {
      options.retain(|o| !ours(o));
      return;
    }
    if sidekick.controls.is_empty() || options.iter().any(ours) || !options.iter().any(is_model_control) {
      return;
    }
    options.extend(sidekick.controls.iter().cloned());
  }

  /// A set_config on one of the host's selects: taken here, never sent. False: not one of them
  pub(crate) fn host_sidekick_pick(&self, id: &str, value: &str) -> bool {
    let mut c = self.core.lock();
    if id != SIDEKICK_CONFIG && id != SIDEKICK_EFFORT_CONFIG {
      return false;
    }
    if !c.sidekick.offered || set_aside(&c) {
      return true;
    }
    let known = |agent: &str| self.deps.registry.try_get(agent).is_some();
    match id {
      SIDEKICK_CONFIG if value == NO_SIDEKICK => c.sidekick.pick = None,
      SIDEKICK_CONFIG if split_pick(value).is_some_and(|(agent, _)| known(agent)) => c.sidekick.pick = Some(value.to_owned()),
      SIDEKICK_EFFORT_CONFIG => c.sidekick.effort = Some(value.to_owned()),
      _ => return true,
    }
    self.rebuild_host_sidekick(&mut c);
    self.touch(&mut c);
    true
  }

  /// The host's sidekick as picked now, picks in flight included: its CLI, its model id there and its level
  pub(crate) fn host_sidekick(&self) -> Option<(String, String, Option<String>)> {
    let c = self.core.lock();
    if !c.sidekick.offered || set_aside(&c) {
      return None;
    }
    let held = |id: &str| c.picks.values.get(id).map(|(v, _)| v.clone());
    let pick = held(SIDEKICK_CONFIG).or_else(|| c.sidekick.pick.clone()).filter(|p| p != NO_SIDEKICK)?;
    let (agent, model) = split_pick(&pick)?;
    // A level picked for another model is not this one's
    let effort = effort_of(&c.sidekick.sources, &pick).and_then(|levels| {
      let offered = |v: &String| levels.options.iter().any(|o| &o.id == v);
      held(SIDEKICK_EFFORT_CONFIG).filter(offered).or_else(|| c.sidekick.effort.clone().filter(offered))
    });
    Some((agent.to_owned(), model.to_owned(), effort))
  }

  /// What the lead is told of the host's sidekick ahead of a user's prompt, when the pick changed since it last heard:
  /// a CLI learns of it from `ask_agent`'s listing only when it lists its tools again, which most do not mid-session
  /// (`docs/acp-agents-compat.md`). The texts are the built-in agent's (`sidekick/added`, `/changed`, `/removed`)
  pub(crate) fn host_sidekick_note(&self) -> Option<String> {
    let now = self.host_sidekick().map(|(agent, model, _)| format!("{agent}/{model}"));
    let (had, name) = {
      let mut c = self.core.lock();
      if !c.sidekick.offered || (c.sidekick.told == now && (c.sidekick.told_any || now.is_none())) {
        return None;
      }
      let had = c.sidekick.told.is_some();
      c.sidekick.told = now.clone();
      c.sidekick.told_any = true;
      let name = now.as_ref().map(|p| {
        let option = c.sidekick.controls.first().and_then(|ctl| ctl.options.iter().find(|o| &o.id == p));
        match option {
          Some(o) => format!("{} in {}", o.name, o.description.as_deref().unwrap_or_default()),
          None => p.clone(),
        }
      });
      (had, name)
    };
    let prompts = self.prompts();
    let base = acpira_agent::prompt::BASE;
    let (agent, tool) = (("agent", SIDEKICK_ID), ("tool", "ask_agent"));
    let text = match (had, name) {
      (_, None) => prompts.render("sidekick/removed", base, &[agent, tool]),
      (false, Some(model)) => prompts.render("sidekick/added", base, &[("model", &model), agent, tool, ("lead", &prompts.text("sidekick/lead", base))]),
      (true, Some(model)) => prompts.render("sidekick/changed", base, &[("model", &model), agent, tool]),
    };
    Some(format!("<sidekick>\n{text}\n</sidekick>"))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn option(id: &str, name: &str) -> SessionOption {
    SessionOption { id: id.into(), name: name.into(), ..Default::default() }
  }

  fn source(agent: &str, name: &str, models: &[(&str, &str)], levels: Option<(&str, &[&str])>) -> SidekickSource {
    let model = ConfigControl {
      id: "model".into(),
      name: "Model".into(),
      category: Some("model".into()),
      kind: None,
      options: models.iter().map(|(id, name)| option(id, name)).collect(),
      value: models.first().map(|(id, _)| (*id).to_owned()),
    };
    let shapes = levels.map(|(of, ids)| {
      let effort = ConfigControl {
        id: "effort".into(),
        name: "Effort".into(),
        category: Some("thought_level".into()),
        kind: None,
        options: ids.iter().map(|id| option(id, id)).collect(),
        value: ids.last().map(|id| (*id).to_owned()),
      };
      ModelShapes::from([(of.to_owned(), vec![effort])])
    });
    SidekickSource { agent: agent.into(), name: name.into(), model, shapes }
  }

  #[test]
  fn select_files_each_model_under_its_cli_with_the_own_cli_first() {
    let sources = [source("antigravity", "Antigravity", &[("flash", "Gemini Flash")], None), source("claude", "Claude Code", &[("haiku", "Haiku")], None)];
    let built = build("claude", &sources, Some("antigravity/flash"), None);
    assert_eq!(built.len(), 1, "Antigravity has no level select");
    let ids: Vec<&str> = built[0].options.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["none", "claude/haiku", "antigravity/flash"]);
    assert_eq!(built[0].options[2].description.as_deref(), Some("Antigravity"));
    assert_eq!(built[0].options[2].group, Some(cli_group("antigravity", "Antigravity")));
    assert_eq!(built[0].value.as_deref(), Some("antigravity/flash"));
    assert_eq!(build("claude", &sources, None, None)[0].value.as_deref(), Some("none"));
  }

  #[test]
  fn a_model_keeps_the_channel_its_cli_lists_it_under() {
    let mut acpira = source("acpira", "Acpira", &[("openrouter/opus", "Opus 5.5"), ("deepseek/flash", "DeepSeek V4.1 Flash")], None);
    acpira.model.options[0].source = Some(OptionSource { id: "openrouter".into(), name: "OpenRouter".into(), kind: SourceKind::Custom });
    // An ACP option group stands in for a source the CLI does not mark
    acpira.model.options[1].group = Some(OptionGroup { id: "deepseek".into(), name: "DeepSeek".into() });
    let built = build("claude", &[acpira, source("claude", "Claude Code", &[("opus", "Opus 5.5")], None)], None, None);
    let opus = |id: &str| built[0].options.iter().find(|o| o.id == id).unwrap().clone();
    assert_eq!(opus("acpira/openrouter/opus").source.map(|s| s.name).as_deref(), Some("OpenRouter"));
    assert_eq!(opus("acpira/openrouter/opus").group, Some(cli_group("acpira", "Acpira")));
    assert_eq!(opus("acpira/deepseek/flash").source.map(|s| s.name).as_deref(), Some("DeepSeek"));
    assert_eq!(opus("claude/opus").source, None);
    assert_eq!(opus("claude/opus").group, Some(cli_group("claude", "Claude Code")));
  }

  #[test]
  fn fusion_pairs_are_no_sidekick() {
    let sources = [source("devin", "Devin", &[("swe-2-high", "SWE-2 High"), ("fusion-x", "Fusion (Claude Fable 5.2 Medium + SWE-2 Medium)")], None)];
    let ids: Vec<String> = build("claude", &sources, None, None)[0].options.iter().map(|o| o.id.clone()).collect();
    assert_eq!(ids, ["none", "devin/swe-2-high"]);
  }

  #[test]
  fn level_select_follows_the_picked_model() {
    let sources = [source("codex", "Codex", &[("luna", "Luna"), ("mini", "Mini")], Some(("luna", &["low", "high"])))];
    let built = build("claude", &sources, Some("codex/luna"), Some("low"));
    assert_eq!(built[1].category.as_deref(), Some(SIDEKICK_EFFORT_CATEGORY));
    assert_eq!(built[1].value.as_deref(), Some("low"));
    // A level the model lacks falls back to the one its CLI showed
    assert_eq!(build("claude", &sources, Some("codex/luna"), Some("max"))[1].value.as_deref(), Some("high"));
  }

  #[test]
  fn a_pick_no_source_lists_stays_offered() {
    let built = build("claude", &[], Some("codex/gone"), None);
    assert_eq!(built[0].value.as_deref(), Some("codex/gone"));
    assert!(built[0].options.iter().any(|o| o.id == "codex/gone"));
  }

  #[test]
  fn every_option_gets_the_effort_it_would_run_at() {
    let sources = [
      source("codex", "Codex", &[("luna", "Luna"), ("mini", "Mini")], Some(("luna", &["low", "high"]))),
      source("claude", "Claude Code", &[("opus", "Opus")], Some(("opus", &["low", "medium", "high"]))),
    ];
    let shapes = option_shapes(&sources, Some("medium"));
    // The picked level where the model has it, else the one its CLI showed; no known levels, no entry
    assert_eq!(shapes["claude/opus"][0].value.as_deref(), Some("medium"));
    assert_eq!(shapes["codex/luna"][0].value.as_deref(), Some("high"));
    assert_eq!(shapes["codex/luna"][0].id, SIDEKICK_EFFORT_CONFIG);
    assert!(!shapes.contains_key("codex/mini"));
  }

  #[test]
  fn the_built_in_select_reads_its_models_lead_shapes() {
    let model = ConfigControl { id: "model".into(), category: Some("model".into()), options: vec![option("a/x", "X"), option("a/y", "Y")], ..Default::default() };
    let select = ConfigControl {
      id: "sidekick".into(),
      category: Some(SIDEKICK_CATEGORY.into()),
      options: vec![option(NO_SIDEKICK, "None"), option("a/x", "X"), option("a/y", "Y")],
      value: Some(NO_SIDEKICK.into()),
      ..Default::default()
    };
    let levels = ConfigControl { id: "thought_level".into(), category: Some("thought_level".into()), options: vec![option("low", "Low"), option("high", "High")], value: Some("high".into()), ..Default::default() };
    let fast = ConfigControl { id: "fast".into(), category: Some("model_config".into()), kind: Some(ControlType::Boolean), options: vec![option("false", "Off"), option("true", "On")], value: Some("true".into()), ..Default::default() };
    let shapes = ModelShapes::from([("a/x".to_owned(), vec![levels, fast])]);
    let out = native_option_shapes(&[model, select], Some(&shapes)).unwrap();
    let x = &out["a/x"];
    assert_eq!((x[0].id.as_str(), x[0].value.as_deref()), (NATIVE_EFFORT_CONFIG, Some("high")));
    // Fast starts off for a sidekick: the lead's own switch is not the sidekick's
    assert_eq!((x[1].id.as_str(), x[1].value.as_deref()), (NATIVE_FAST_CONFIG, Some("false")));
    assert!(!out.contains_key("a/y"));
  }

  #[test]
  fn a_fusion_pair_sets_the_select_aside() {
    let model = ConfigControl {
      id: "model".into(),
      category: Some("model".into()),
      options: vec![option("swe-2-high", "SWE-2 High"), option("fusion-claude-fable-5-2-medium-sidekick-swe-2-medium", "Fusion (Claude Fable 5.2 Medium + SWE-2 Medium)")],
      value: Some("swe-2-high".into()),
      ..Default::default()
    };
    let none = |_: &str| None;
    assert!(!fusion_pair(std::slice::from_ref(&model), none));
    assert!(fusion_pair(&[ConfigControl { value: Some("fusion-claude-fable-5-2-medium-sidekick-swe-2-medium".into()), ..model.clone() }], none));
    // A switch in flight counts before the agent answers it
    assert!(fusion_pair(&[model], |_| Some("fusion-claude-fable-5-2-medium-sidekick-swe-2-medium".into())));
  }

  #[test]
  fn a_record_gives_back_its_pick() {
    let saved = build("claude", &[], Some("codex/luna"), None);
    assert_eq!(restored(&saved).pick.as_deref(), Some("codex/luna"));
    assert!(restored(&build("claude", &[], None, None)).pick.is_none());
    // The built-in agent's select is its own
    let own = ConfigControl { id: "sidekick".into(), category: Some(SIDEKICK_CATEGORY.into()), value: Some("openai/luna".into()), ..Default::default() };
    assert!(restored(&[own]).pick.is_none());
  }
}
