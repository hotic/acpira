//! The host's own sidekick select, for a session whose CLI offers none: a second model the lead hands work to, in any
//! CLI Acpira knows the models of. Like Claude's Ultra level (`ultracode.rs`) the select is made here, put back on the
//! option list after every agent answer, and a pick on it never reaches the wire. The pick names the CLI and the model
//! (`<agent>/<model id>`), which `sidekick_persona` (`relay/calls.rs`) summons through the relay like any persona; the
//! built-in agent keeps its own select (`acpira-agent/src/acp.rs`)

use acpira_shared::composer_controls::is_reasoning_control;
use acpira_shared::model_shapes::{ModelShapes, is_model_control, shape_for};
use acpira_shared::subagents::SIDEKICK_ID;
use acpira_shared::transcript::{ConfigControl, SessionOption};

use crate::acp::agents::registry::SELF_AGENT_ID;
use crate::acp::session::relay::personas::{NO_SIDEKICK, SIDEKICK_CATEGORY, SIDEKICK_EFFORT_CATEGORY};
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

/// The sidekick select over every source's models (the session's own CLI first, each option filed under its CLI's
/// name), and the effort select of the picked model where its CLI has one. A pick no source lists any more stays
/// offered, so a stale model list never drops it
pub(crate) fn build(own: &str, sources: &[SidekickSource], pick: Option<&str>, effort: Option<&str>) -> Vec<ConfigControl> {
  let mut options = vec![SessionOption { id: NO_SIDEKICK.into(), name: "None".into(), ..Default::default() }];
  let ordered = sources.iter().filter(|s| s.agent == own).chain(sources.iter().filter(|s| s.agent != own));
  for s in ordered {
    options.extend(s.model.options.iter().map(|o| SessionOption {
      id: format!("{}/{}", s.agent, o.id),
      name: o.name.clone(),
      description: Some(s.name.clone()),
      ..Default::default()
    }));
  }
  if let Some(p) = pick.filter(|p| !options.iter().any(|o| o.id == *p))
    && let Some((agent, model)) = split_pick(p)
  {
    options.push(SessionOption { id: p.to_owned(), name: model.to_owned(), description: Some(agent.to_owned()), ..Default::default() });
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
    let offered = |v: &&str| levels.options.iter().any(|o| o.id == *v);
    let value = effort.filter(offered).or(levels.value.as_deref().filter(offered)).unwrap_or(&levels.options[0].id);
    out.push(ConfigControl {
      id: SIDEKICK_EFFORT_CONFIG.into(),
      name: "Sidekick effort".into(),
      category: Some(SIDEKICK_EFFORT_CATEGORY.into()),
      kind: None,
      options: levels.options.clone(),
      value: Some(value.to_owned()),
    });
  }
  out
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
    self.place_host_sidekick(c);
  }

  /// Put the selects back after the options changed (`refine_controls`): an agent answer replaces the whole list.
  /// Before the first connection built them, the ones a record or an earlier session showed stay as they are
  pub(crate) fn place_host_sidekick(&self, c: &mut Core) {
    let Core { sidekick, state, .. } = c;
    let options = &mut state.controls.options;
    if native(options) {
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
    if !c.sidekick.offered || native(&c.state.controls.options) {
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
    if !c.sidekick.offered || native(&c.state.controls.options) {
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
    assert_eq!(built[0].value.as_deref(), Some("antigravity/flash"));
    assert_eq!(build("claude", &sources, None, None)[0].value.as_deref(), Some("none"));
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
  fn a_record_gives_back_its_pick() {
    let saved = build("claude", &[], Some("codex/luna"), None);
    assert_eq!(restored(&saved).pick.as_deref(), Some("codex/luna"));
    assert!(restored(&build("claude", &[], None, None)).pick.is_none());
    // The built-in agent's select is its own
    let own = ConfigControl { id: "sidekick".into(), category: Some(SIDEKICK_CATEGORY.into()), value: Some("openai/luna".into()), ..Default::default() };
    assert!(restored(&[own]).pick.is_none());
  }
}
