//! Permission levels split off a mode list that mixes them with working modes (Claude Code's default / acceptEdits /
//! auto / bypassPermissions beside plan, Antigravity's default / accept-edits / auto beside plan). The session shows the
//! built-in agent's shape instead: an `approval` select (category `_permission_mode`) holding the levels, and modes
//! reduced to the resting mode plus the working ones. The CLI's own mode (`AccessState::wire`) is the resting mode's level
//! or the working mode picked; a level the CLI has no mode for (Antigravity's full access) runs on another level's mode
//! with the host approving every request (`perms.auto_approve`)

use std::sync::Arc;

use anyhow::Result;
use serde_json::json;

use acpira_shared::transcript::{ConfigControl, SessionControls, SessionOption};

use crate::acp::agents::registry::AccessLevel;
use crate::acp::session::{AcpSession, Core};
use crate::i18n::t_or;

/// The select's id: the built-in agent's, so a sidekick round copies the level and a pick syncs the sidekick's access
pub const APPROVAL: &str = acpira_agent::acp::APPROVAL;
const CATEGORY: &str = "_permission_mode";
/// The resting mode: the CLI's `default`, standing for whichever level is picked
pub const REST: &str = "default";
/// The levels' ids that grant full access (marked `full_access`, so plugins and the composer treat them as such)
const FULL: &str = "full";

/// The session's level and the mode its CLI is in
#[derive(Debug, Clone, Default)]
pub(crate) struct AccessState {
  /// The picked level's id; None before the CLI first reported a mode
  pub level: Option<String>,
  /// The mode the CLI is in, as it last reported or was sent
  pub wire: Option<String>,
  /// The levels this session offers (those whose mode the CLI listed, plus host ones), with the CLI's name and
  /// description for each
  pub offered: Vec<SessionOption>,
}

fn level<'a>(levels: &'a [AccessLevel], id: &str) -> Option<&'a AccessLevel> {
  levels.iter().find(|l| l.id == id)
}

/// The level a CLI mode stands for, the native one before a host one on the same mode
fn level_of_wire<'a>(levels: &'a [AccessLevel], wire: &str) -> Option<&'a AccessLevel> {
  levels.iter().filter(|l| l.wire == wire).min_by_key(|l| l.host)
}

/// Whether the mode list is the CLI's own (levels not split off yet): it lists a level's mode other than the resting one
fn is_wire_list(levels: &[AccessLevel], modes: &[SessionOption]) -> bool {
  modes.iter().any(|m| m.id != REST && levels.iter().any(|l| l.wire == m.id))
}

/// The CLI reported mode `wire`: a level's mode picks that level (a host level riding on it stays), anything else is a
/// working mode. Returns the mode the session shows
fn adopt_wire(levels: &[AccessLevel], st: &mut AccessState, wire: &str) -> String {
  st.wire = Some(wire.to_owned());
  let Some(l) = level_of_wire(levels, wire) else { return wire.to_owned() };
  let keep = st.level.as_deref().and_then(|id| level(levels, id)).is_some_and(|cur| cur.host && cur.wire == wire);
  if !keep {
    st.level = Some(l.id.clone());
  }
  REST.to_owned()
}

/// Fresh controls from a start / rebuild response: split the CLI's mode list into the levels and the working modes, and
/// read its current mode as a report. Controls already split are left as they are
pub(crate) fn split(levels: &[AccessLevel], controls: &mut SessionControls, st: &mut AccessState) {
  if !is_wire_list(levels, &controls.modes) {
    return;
  }
  st.offered = levels
    .iter()
    .filter_map(|l| {
      let native = controls.modes.iter().find(|m| m.id == l.wire);
      let kind = (l.id == FULL).then(|| "full_access".to_owned());
      if l.host {
        // No mode of the CLI's own: the host's wording, as Grok's synthesized full access
        return native.map(|_| SessionOption { id: l.id.clone(), name: "Full access".into(), description: Some(t_or("mode.grok.yolo")), kind, ..Default::default() });
      }
      native.map(|m| SessionOption { id: l.id.clone(), name: m.name.clone(), description: m.description.clone(), kind, ..Default::default() })
    })
    .collect();
  controls.modes.retain(|m| m.id == REST || !levels.iter().any(|l| l.wire == m.id));
  if let Some(wire) = controls.mode_id.clone() {
    controls.mode_id = Some(adopt_wire(levels, st, &wire));
  }
  place(controls, st);
}

/// The approval select at the head of the options, holding the picked level (the first offered before any report)
pub(crate) fn place(controls: &mut SessionControls, st: &AccessState) {
  controls.options.retain(|o| !(o.id == APPROVAL && o.category.as_deref() == Some(CATEGORY)));
  if st.offered.is_empty() {
    return;
  }
  let value = st.level.clone().filter(|v| st.offered.iter().any(|o| &o.id == v)).or_else(|| st.offered.first().map(|o| o.id.clone()));
  controls.options.insert(
    0,
    ConfigControl { id: APPROVAL.into(), name: "Approval".into(), category: Some(CATEGORY.into()), kind: None, options: st.offered.clone(), value },
  );
}

/// The CLI mode for showing mode `shown` at the picked level, and whether the host approves every request there
pub(crate) fn wire_for(levels: &[AccessLevel], st: &AccessState, shown: &str) -> (String, bool) {
  if shown != REST {
    return (shown.to_owned(), false);
  }
  match st.level.as_deref().and_then(|id| level(levels, id)) {
    Some(l) => (l.wire.clone(), l.host),
    None => (REST.to_owned(), false),
  }
}

/// A mode id a client sent: the resting or a working mode as is; a level's own mode (a pick remembered before the split,
/// Claude's `bypassPermissions`) picks that level and shows the resting mode
pub(crate) fn shown_mode(levels: &[AccessLevel], st: &mut AccessState, id: &str) -> String {
  if id == REST {
    return REST.to_owned();
  }
  match level_of_wire(levels, id) {
    Some(l) => {
      st.level = Some(l.id.clone());
      REST.to_owned()
    }
    None => id.to_owned(),
  }
}

/// Whether `id` is a level's own mode other than the resting one (a pick remembered before the split)
pub(crate) fn is_level_mode(levels: &[AccessLevel], id: &str) -> bool {
  id != REST && level_of_wire(levels, id).is_some()
}

/// Whether `id` is one of the levels on offer
pub(crate) fn offers(st: &AccessState, id: &str) -> bool {
  st.offered.iter().any(|o| o.id == id)
}

impl AcpSession {
  /// This agent's levels, when its modes mix them with working modes
  pub(crate) fn access_levels(&self) -> Option<Vec<AccessLevel>> {
    self.def().access
  }

  /// Split freshly reported controls (`apply_controls`, an edit's fresh peer)
  pub(crate) fn split_access(&self, controls: &mut SessionControls, st: &mut AccessState) {
    if let Some(levels) = self.access_levels() {
      split(&levels, controls, st);
    }
  }

  /// A `current_mode_update` from the CLI: a level's mode becomes the level, the select follows
  pub(crate) fn access_mode_update(&self, c: &mut Core) {
    let Some(levels) = self.access_levels() else { return };
    if c.access.offered.is_empty() {
      return;
    }
    let Some(wire) = c.state.controls.mode_id.clone() else { return };
    let shown = adopt_wire(&levels, &mut c.access, &wire);
    c.state.controls.mode_id = Some(shown);
    c.perms.auto_approve = self.access_host_full(c);
  }

  /// Keep the select in the options (a config answer or update replaces them)
  pub(crate) fn place_access(&self, c: &mut Core) {
    if !c.access.offered.is_empty() {
      let Core { access, state, .. } = c;
      place(&mut state.controls, access);
    }
  }

  /// Whether the host approves every request: a host level picked and the resting mode shown
  pub(crate) fn access_host_full(&self, c: &Core) -> bool {
    let Some(levels) = self.access_levels() else { return false };
    let shown = c.state.controls.mode_id.as_deref().unwrap_or(REST);
    !c.access.offered.is_empty() && wire_for(&levels, &c.access, shown).1
  }

  /// Whether the CLI runs in one of its own full-access modes (Claude's bypassPermissions) under the picked level
  pub(crate) fn access_native_full(c: &Core) -> bool {
    c.access.wire.as_deref().is_some_and(|w| crate::acp::session::relay::BYPASS_MODES.contains(&w))
  }

  /// Show mode `shown` at the picked level: the CLI is switched when its mode has to change. The caller holds no lock
  pub(crate) async fn apply_access(self: &Arc<Self>, levels: &[AccessLevel], shown: String) -> Result<()> {
    let (proc, sid, wire, host, current) = {
      let c = self.core.lock();
      let Some(proc) = c.proc.clone() else { return Ok(()) };
      let (wire, host) = wire_for(levels, &c.access, &shown);
      (proc, c.acp_session_id.clone(), wire, host, c.access.wire.clone())
    };
    if current.as_deref() != Some(wire.as_str()) {
      proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": wire })).await?;
    }
    let mut c = self.core.lock();
    c.access.wire = Some(wire);
    c.state.controls.mode_id = Some(shown);
    c.perms.auto_approve = host;
    self.place_access(&mut c);
    Ok(())
  }

  /// A set_config on the approval select of a split agent: taken here, the CLI only hears of it as a mode. False: not
  /// that select
  pub(crate) async fn access_pick(self: &Arc<Self>, id: &str, value: &str) -> Result<bool> {
    if id != APPROVAL {
      return Ok(false);
    }
    let Some(levels) = self.access_levels() else { return Ok(false) };
    let shown = {
      let mut c = self.core.lock();
      if !offers(&c.access, value) {
        return Ok(true);
      }
      c.access.level = Some(value.to_owned());
      c.state.controls.mode_id.clone().unwrap_or_else(|| REST.to_owned())
    };
    // In a working mode (plan) the level waits for the resting mode; the select shows it at once
    self.apply_access(&levels, shown).await?;
    Ok(true)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn claude() -> Vec<AccessLevel> {
    crate::acp::agents::registry::builtin_agents().into_iter().find(|d| d.id == "claude").and_then(|d| d.access).unwrap_or_default()
  }

  fn agy() -> Vec<AccessLevel> {
    crate::acp::agents::registry::antigravity().access.unwrap_or_default()
  }

  fn modes(ids: &[&str], current: &str) -> SessionControls {
    SessionControls {
      modes: ids.iter().map(|id| SessionOption { id: (*id).into(), name: (*id).into(), ..Default::default() }).collect(),
      mode_id: Some(current.into()),
      ..Default::default()
    }
  }

  fn approval(c: &SessionControls) -> (Vec<String>, Option<String>) {
    let o = c.options.iter().find(|o| o.id == APPROVAL).unwrap();
    (o.options.iter().map(|x| x.id.clone()).collect(), o.value.clone())
  }

  #[test]
  fn claude_levels_split_off_and_bypass_reads_as_full() {
    let mut c = modes(&["default", "acceptEdits", "plan", "auto", "bypassPermissions"], "bypassPermissions");
    let mut st = AccessState::default();
    split(&claude(), &mut c, &mut st);
    assert_eq!(c.modes.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["default", "plan"]);
    assert_eq!(c.mode_id.as_deref(), Some(REST));
    assert_eq!(approval(&c), (vec!["ask".into(), "auto-edit".into(), "auto".into(), "full".into()], Some("full".into())));
    // Split controls stay as they are
    let before = c.clone();
    split(&claude(), &mut c, &mut st);
    assert_eq!(c, before);
  }

  #[test]
  fn a_working_mode_keeps_the_level_and_the_rest_goes_back_to_it() {
    let levels = claude();
    let mut st = AccessState { level: Some("auto-edit".into()), ..Default::default() };
    assert_eq!(adopt_wire(&levels, &mut st, "plan"), "plan");
    assert_eq!(st.level.as_deref(), Some("auto-edit"));
    assert_eq!(wire_for(&levels, &st, REST), ("acceptEdits".into(), false));
    assert_eq!(wire_for(&levels, &st, "plan"), ("plan".into(), false));
    // Leaving plan through the CLI's own card lands on the level it picked
    assert_eq!(adopt_wire(&levels, &mut st, "default"), REST);
    assert_eq!(st.level.as_deref(), Some("ask"));
  }

  #[test]
  fn antigravity_full_access_rides_on_accept_edits_with_host_approval() {
    let levels = agy();
    let mut c = modes(&["default", "accept-edits", "auto", "plan"], "default");
    let mut st = AccessState::default();
    split(&levels, &mut c, &mut st);
    assert_eq!(approval(&c), (vec!["ask".into(), "auto-edit".into(), "auto".into(), "full".into()], Some("ask".into())));
    st.level = Some("full".into());
    assert_eq!(wire_for(&levels, &st, REST), ("accept-edits".into(), true));
    // The CLI echoing `accept-edits` does not take full access away
    adopt_wire(&levels, &mut st, "accept-edits");
    assert_eq!(st.level.as_deref(), Some("full"));
    st.level = Some("auto".into());
    adopt_wire(&levels, &mut st, "accept-edits");
    assert_eq!(st.level.as_deref(), Some("auto-edit"));
  }

  #[test]
  fn a_remembered_level_mode_picks_the_level() {
    let levels = claude();
    let mut st = AccessState::default();
    assert_eq!(shown_mode(&levels, &mut st, "bypassPermissions"), REST);
    assert_eq!(st.level.as_deref(), Some("full"));
    assert_eq!(shown_mode(&levels, &mut st, "plan"), "plan");
    assert_eq!(shown_mode(&levels, &mut st, REST), REST);
    assert_eq!(st.level.as_deref(), Some("full"));
  }

  #[test]
  fn a_cli_without_a_level_mode_does_not_offer_it() {
    // An older Claude without `auto`
    let mut c = modes(&["default", "acceptEdits", "plan", "bypassPermissions"], "default");
    let mut st = AccessState::default();
    split(&claude(), &mut c, &mut st);
    assert_eq!(approval(&c).0, ["ask", "auto-edit", "full"]);
  }
}
