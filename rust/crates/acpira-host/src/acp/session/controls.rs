//! Modes and config options of a session: wire requests,
//! the optimistic pick overlay, effort preservation across model switches, and replaying remembered choices

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::composer_controls::{is_reasoning_control, thought_correction};
use acpira_shared::models::parse_fusion_name;
use acpira_shared::transcript::*;

use crate::acp::agents::model_sources::refine_controls;
use crate::acp::transcript::normalize::{apply_config_options, config_option_set_value, init_controls};
use crate::acp::transport::rpc::BoxFuture;
use crate::acp::session::ultracode::UltraPick;
use crate::acp::session::{AcpSession, Core};
use crate::acp::vendors::claude_ultracode;
use crate::i18n::{t, t_or};

/// pi-acp 0.0.33 advertises its thinking levels twice, as modes and as the thought_level select: such modes select nothing
/// of their own, whichever agent definition launched the adapter
fn modes_mirror_reasoning(controls: &SessionControls) -> bool {
  fn ids(options: &[SessionOption]) -> HashSet<&str> {
    options.iter().map(|o| o.id.as_str()).collect()
  }
  !controls.modes.is_empty() && controls.options.iter().any(|c| is_reasoning_control(c) && ids(&c.options) == ids(&controls.modes))
}

impl AcpSession {
  /// Controls from a start response (session/new / resume / load, an edit's fresh peer). Protocol modes are dropped for an
  /// `ignoreModes` agent and when they only mirror the reasoning select; true when they were
  pub(crate) fn protocol_controls(&self, controls: &mut SessionControls, modes: Option<&Value>, config_options: Option<&Value>) -> bool {
    init_controls(controls, modes, config_options);
    if !self.def().ignore_modes && !modes_mirror_reasoning(controls) {
      return false;
    }
    controls.modes = vec![];
    controls.mode_id = None;
    controls.mode_config_id = None;
    true
  }

  /// A set_config_option answer is the full configOptions set; it is narrowed at once, so the effort restore and
  /// `sync_thought` below only ever pick values the view will offer
  fn adopt_config_response(&self, r: &Value) {
    let mut c = self.core.lock();
    apply_config_options(&mut c.state.controls, r.get("configOptions").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]));
    self.refine_controls(&mut c);
  }

  fn editing_guard(&self) -> Result<()> {
    if self.core.lock().phase.editing { Err(anyhow!(t("history.unavailable"))) } else { Ok(()) }
  }

  pub fn set_mode(self: &Arc<Self>, id: String) -> BoxFuture<Result<()>> {
    let me = self.clone();
    Box::pin(async move {
      me.editing_guard()?;
      let (proc, sid, mode_config, current) = {
        let c = me.core.lock();
        let Some(proc) = c.proc.clone() else { return Ok(()) };
        if c.status != SessionStatus::Ready {
          return Ok(());
        }
        (proc, c.acp_session_id.clone(), c.state.controls.mode_config_id.clone(), c.state.controls.mode_id.clone())
      };
      if let Some(config_id) = mode_config {
        let r = proc.request("session/set_config_option", json!({ "sessionId": sid, "configId": config_id, "value": id })).await?;
        me.adopt_config_response(&r);
        me.sync_thought().await?;
      } else if me.synthetic_modes().is_some() {
        // yolo is host-side auto-approval: the CLI stays in default (pulled back first when coming from plan)
        let wire = if id == "yolo" { (current.as_deref() == Some("plan")).then(|| "default".to_owned()) } else { Some(id.clone()) };
        me.core.lock().perms.auto_approve = id == "yolo";
        if let Some(w) = wire {
          proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": w })).await?;
        }
        let mut c = me.core.lock();
        c.state.controls.mode_id = Some(id.clone());
        if c.perms.auto_approve {
          me.flush_permissions(&mut c);
        }
      } else {
        proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": id })).await?;
        me.core.lock().state.controls.mode_id = Some(id.clone());
      }
      let mut c = me.core.lock();
      me.touch(&mut c);
      Ok(())
    })
  }

  /// Any configOption (model / reasoning level / boolean toggles); the response is the full configOptions set. On
  /// Claude, picking the host's Ultra level or leaving it switches ultracode instead (`session/ultracode.rs`): Ultra
  /// never reaches the wire as an effort value
  pub fn set_config(self: &Arc<Self>, config_id: String, value: String) -> BoxFuture<Result<()>> {
    let me = self.clone();
    Box::pin(async move {
      me.editing_guard()?;
      let pick = {
        let mut c = me.core.lock();
        me.ultracode_pick(&mut c, &config_id, &value)
      };
      match pick {
        UltraPick::Wire => me.set_config_wire(config_id, value).await,
        UltraPick::Done => Ok(()),
        UltraPick::Target(wire) => me.set_config_wire(config_id, wire).await,
        UltraPick::Switch(on, level) => me.set_ultracode(on, level).await,
      }
    })
  }

  /// set_config_option itself, with a value the agent offers. A model switch keeps the effort (Ultra too, as its wire
  /// target) when the new model has it, and leaves Ultra when the new model has no effort to map it to
  pub(crate) fn set_config_wire(self: &Arc<Self>, config_id: String, value: String) -> BoxFuture<Result<()>> {
    let me = self.clone();
    Box::pin(async move {
      me.editing_guard()?;
      let (proc, sid, control, reasoning) = {
        let c = me.core.lock();
        let opts = &c.state.controls.options;
        let Some(control) = opts.iter().find(|o| o.id == config_id).cloned() else { return Ok(()) };
        let Some(proc) = c.proc.clone() else { return Ok(()) };
        if c.status != SessionStatus::Ready {
          return Ok(());
        }
        let model = opts.iter().find(|o| o.id == config_id && o.category.as_deref() == Some("model"));
        let name_of =
          |v: Option<&String>| model.and_then(|m| m.options.iter().find(|o| Some(&o.id) == v)).map(|o| o.name.clone()).unwrap_or_default();
        let before = parse_fusion_name(&name_of(model.and_then(|m| m.value.as_ref())));
        let after = parse_fusion_name(&name_of(Some(&value)));
        // A user's model switch keeps the chosen effort when the new model offers it; while replaying remembered
        // controls only a sidekick-only Fusion change is preserved (adopt sets effort itself right after)
        let sidekick_only = match (&before, &after) {
          (Some(b), Some(a)) => {
            b.lead == a.lead && b.effort == a.effort && b.fast == a.fast && b.long == a.long && b.sidekick != a.sidekick
          }
          _ => false,
        };
        let keep = model.is_some() && (sidekick_only || !c.picks.adopting);
        let reasoning: Vec<(String, Option<String>)> =
          if keep { opts.iter().filter(|o| is_reasoning_control(o)).map(|o| (o.id.clone(), o.value.clone())).collect() } else { vec![] };
        (proc, c.acp_session_id.clone(), control, reasoning)
      };
      let mut params = json!({ "sessionId": sid, "configId": config_id });
      for (k, v) in config_option_set_value(Some(&control), &value) {
        params[k] = v;
      }
      let r = proc.request("session/set_config_option", params).await?;
      me.adopt_config_response(&r);
      for (id, prev) in reasoning {
        let Some(prev) = prev else { continue };
        let restore = {
          let mut c = me.core.lock();
          me.wire_pick(&mut c, &id, &prev).filter(|(wire, current)| {
            current.as_ref() != Some(wire)
              && c.state.controls.options.iter().find(|o| o.id == id).is_some_and(|cur| cur.options.iter().any(|o| &o.id == wire))
          })
        };
        if let Some((wire, _)) = restore {
          me.set_config_wire(id, wire).await?;
        }
      }
      // A model without an effort to map Ultra to: ultracode goes off, the effort falls back like any narrowed-away value
      let leave_ultra = {
        let mut c = me.core.lock();
        control.category.as_deref() == Some("model")
          && c.ultracode.on
          && me.ultracode_available(&mut c)
          && claude_ultracode::effort_index(&c.state.controls.options).is_none()
      };
      if leave_ultra {
        me.set_ultracode(false, None).await?;
      }
      if !me.core.lock().picks.syncing_thought {
        me.sync_thought().await?;
      }
      let polled_model = {
        let c = me.core.lock();
        me.vendor.usage_poll().filter(|_| {
          !c.usage.notifications
            && c.state.controls.options.iter().find(|o| o.id == config_id).and_then(|o| o.category.as_deref()) == Some("model")
        })
      };
      if let Some(poll) = polled_model {
        if poll.per_model() {
          me.core.lock().state.usage = None;
        }
        me.refresh_context_usage().await;
      }
      let mut c = me.core.lock();
      me.touch(&mut c);
      Ok(())
    })
  }

  /// The composer's click path: same validation as set_config, then an optimistic overlay in front of the request
  pub async fn select_config(self: &Arc<Self>, config_id: String, value: String) -> Result<()> {
    self.editing_guard()?;
    let mut refused = None;
    let held = {
      let mut c = self.core.lock();
      let control = c.state.controls.options.iter().find(|o| o.id == config_id).cloned();
      let ok =
        c.proc.is_some() && c.status == SessionStatus::Ready && control.as_ref().is_some_and(|x| x.options.iter().any(|o| o.id == value));
      if !ok {
        // set_config below does nothing for a session that is not ready: the log is all that tells why the chip stayed put
        if c.proc.is_none() || c.status != SessionStatus::Ready {
          refused = Some(format!("setConfig {config_id}={value} ignored: session {:?}, process {}", c.status, c.proc.is_some()));
        }
        None
      } else {
        // A model switch keeps the chosen effort: hold it on screen so the agent's interim reset never flashes
        let mut held = vec![];
        if control.is_some_and(|x| x.category.as_deref() == Some("model")) {
          let reasoning: Vec<(String, String)> = c
            .state
            .controls
            .options
            .iter()
            .filter(|o| is_reasoning_control(o) && o.value.is_some() && !c.picks.values.contains_key(&o.id))
            .map(|o| (o.id.clone(), o.value.clone().unwrap()))
            .collect();
          for (id, v) in reasoning {
            c.picks.seq += 1;
            let token = c.picks.seq;
            c.picks.values.insert(id.clone(), (v, token));
            held.push((id, token));
          }
        }
        Some(held)
      }
    };
    if let Some(line) = refused {
      self.log(&line);
    }
    let Some(held) = held else { return self.set_config(config_id, value).await };
    let me = self.clone();
    let (cid, v) = (config_id.clone(), value.clone());
    let result = self.pick(config_id, value, move || me.set_config(cid, v)).await;
    let mut c = self.core.lock();
    for (id, token) in &held {
      if c.picks.values.get(id).is_some_and(|(_, t)| t == token) {
        c.picks.values.remove(id);
      }
    }
    if !held.is_empty() {
      self.touch(&mut c);
    }
    result
  }

  pub async fn select_mode(self: &Arc<Self>, id: String) -> Result<()> {
    self.editing_guard()?;
    let ok = {
      let c = self.core.lock();
      c.proc.is_some() && c.status == SessionStatus::Ready && c.state.controls.modes.iter().any(|m| m.id == id)
    };
    if !ok {
      return self.set_mode(id).await;
    }
    let me = self.clone();
    let id2 = id.clone();
    self.pick(MODE_PICK.to_owned(), id, move || me.set_mode(id2)).await
  }

  /// Show the pick at once, then serialize the wire requests: a superseded pick never reaches the wire, a failed
  /// request drops the overlay so the view reverts to agent truth
  async fn pick(self: &Arc<Self>, key: String, value: String, run: impl FnOnce() -> BoxFuture<Result<()>>) -> Result<()> {
    let token = {
      let mut c = self.core.lock();
      c.picks.seq += 1;
      let token = c.picks.seq;
      c.picks.values.insert(key.clone(), (value, token));
      self.touch(&mut c);
      token
    };
    let _serial = self.pick_lock.lock().await;
    if self.core.lock().picks.values.get(&key).is_none_or(|(_, t)| *t != token) {
      return Ok(());
    }
    let result = run().await;
    let mut c = self.core.lock();
    if c.picks.values.get(&key).is_some_and(|(_, t)| *t == token) {
      c.picks.values.remove(&key);
      self.touch(&mut c);
    }
    result
  }

  /// Kimi appends the previous thinking value when the new model does not offer it, and the catalogue may narrow the
  /// current one away: push a native value instead
  pub(crate) async fn sync_thought(self: &Arc<Self>) -> Result<()> {
    self.core.lock().picks.syncing_thought = true;
    let ids: Vec<String> = self.core.lock().state.controls.options.iter().map(|o| o.id.clone()).collect();
    let mut result = Ok(());
    for id in ids {
      let next = self.core.lock().state.controls.options.iter().find(|o| o.id == id).and_then(thought_correction);
      if let Some(next) = next
        && let Err(e) = self.set_config_wire(id, next).await
      {
        result = Err(e);
        break;
      }
    }
    self.core.lock().picks.syncing_thought = false;
    result
  }

  /// Keep a new session's remembered choices on screen from the preview until `adopt_controls` has replayed them, and
  /// queue prompts meanwhile: session/new's defaults never flash, and a first prompt never runs under them
  pub fn hold_settings(&self, settings: &TurnSettings) {
    let mut c = self.core.lock();
    let config = settings.config.iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k.clone(), v.clone()));
    let mode = settings.mode_id.clone().filter(|m| !m.is_empty()).map(|m| (MODE_PICK.to_owned(), m));
    let entries: Vec<(String, String)> = config.chain(mode).collect();
    // The first session request already carries a remembered Ultra, so the replay only has to set its wire effort
    self.seed_ultracode(&mut c, Some(settings));
    c.picks.adopt_pending = true;
    for (key, value) in entries {
      c.picks.seq += 1;
      let token = c.picks.seq;
      c.picks.values.insert(key.clone(), (value, token));
      c.picks.holds.push((key, token));
    }
  }

  /// A held key the user has picked over since: the replay leaves it to that pick
  fn superseded(&self, key: &str) -> bool {
    let c = self.core.lock();
    c.picks.holds.iter().any(|(k, t)| k == key && c.picks.values.get(k).is_none_or(|(_, cur)| cur != t))
  }

  /// Drop the holds still in place (agent truth or the user's own picks show from here) and let queued prompts go
  fn release_holds(self: &Arc<Self>) {
    {
      let mut c = self.core.lock();
      if !c.picks.adopt_pending && c.picks.holds.is_empty() {
        return;
      }
      for (key, token) in std::mem::take(&mut c.picks.holds) {
        if c.picks.values.get(&key).is_some_and(|(_, t)| *t == token) {
          c.picks.values.remove(&key);
        }
      }
      c.picks.adopt_pending = false;
      self.touch(&mut c);
    }
    self.flush_queue();
  }

  /// Replay what was chosen last time in this agent, one request per difference in control order; choices the agent
  /// no longer offers are skipped, a refused one is logged and the rest go on. Runs under the pick lock, so a composer
  /// pick made meanwhile lands after the replay and wins
  pub async fn adopt_controls(self: &Arc<Self>, settings: TurnSettings) {
    {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready || c.proc.is_none() {
        drop(c);
        self.release_holds();
        return;
      }
      c.picks.adopting = true;
    }
    let serial = self.pick_lock.lock().await;
    let effort = self.core.lock().state.controls.options.iter().find(|o| acpira_shared::composer_controls::is_reasoning_control(o)).map(|o| o.id.clone());
    if !effort.is_some_and(|id| self.superseded(&id)) {
      self.adopt_ultracode(&settings).await;
    }
    self.replay_settings(&settings, |key| self.superseded(key)).await;
    self.core.lock().picks.adopting = false;
    drop(serial);
    self.release_holds();
  }

  /// Set the given values again, one request per difference in control order (model before effort), then the mode;
  /// values the agent no longer offers are skipped, a refused one is logged and the rest go on. Claude's Ultra goes out
  /// as its wire target and is compared with the agent's own effort. `skip` leaves a key (a config id or `MODE_PICK`)
  /// alone. The caller holds the pick lock
  pub(crate) async fn replay_settings(self: &Arc<Self>, settings: &TurnSettings, skip: impl Fn(&str) -> bool) {
    let adopting = std::mem::replace(&mut self.core.lock().picks.adopting, true);
    let ids: Vec<String> = self.core.lock().state.controls.options.iter().map(|o| o.id.clone()).collect();
    for id in ids {
      let Some(value) = settings.config.get(&id).filter(|v| !v.is_empty()).cloned() else { continue };
      if skip(&id) {
        continue;
      }
      let differs = {
        let mut c = self.core.lock();
        self.wire_pick(&mut c, &id, &value).filter(|(wire, current)| {
          current.as_ref() != Some(wire) && c.state.controls.options.iter().find(|o| o.id == id).is_some_and(|ctl| ctl.options.iter().any(|o| &o.id == wire))
        })
      };
      let Some((value, _)) = differs else { continue };
      if let Err(e) = self.set_config_wire(id.clone(), value.clone()).await {
        self.log(&format!("adopt {id}={value} refused: {e}"));
      }
    }
    self.core.lock().picks.adopting = adopting;
    if let Some(mode) = settings.mode_id.clone().filter(|m| !m.is_empty() && !skip(MODE_PICK)) {
      let differs = {
        let c = self.core.lock();
        c.state.controls.mode_id.as_ref() != Some(&mode) && c.state.controls.modes.iter().any(|m| m.id == mode)
      };
      if differs && let Err(e) = self.set_mode(mode.clone()).await {
        self.log(&format!("adopt mode {mode} refused: {e}"));
      }
    }
  }
}

pub(crate) const MODE_PICK: &str = "\0mode";

/// The optimistic overlay over the agent's controls: picks in flight, remembered choices held on screen while they are
/// replayed, and the flags of the replays themselves
#[derive(Default)]
pub(crate) struct ControlPicks {
  /// key (a config id or `MODE_PICK`) → (value, token); a request that finds its token replaced was superseded
  pub values: HashMap<String, (String, u64)>,
  pub seq: u64,
  /// The pick overlay entries (key, token) that hold remembered choices on screen while they are replayed
  pub holds: Vec<(String, u64)>,
  /// `adopt_controls` is replaying remembered config values: a model switch does not carry the old effort over
  pub adopting: bool,
  /// A new session's remembered choices are on screen and still to be replayed: prompts queue until `adopt_controls` ends
  pub adopt_pending: bool,
  /// `sync_thought` is correcting the effort: the set_config calls it makes do not start another correction
  pub syncing_thought: bool,
}

impl AcpSession {
  /// Model sources and catalogue-narrowed efforts over whatever the agent last sent, plus Claude's Ultra level
  pub(crate) fn refine_controls(&self, c: &mut Core) {
    refine_controls(&self.agent, &mut c.state.controls.options, &c.model_facts);
    self.place_ultracode(c);
  }

  pub(crate) fn synthetic_modes(&self) -> Option<Vec<SessionOption>> {
    self.def().modes.map(|modes| {
      modes
        .into_iter()
        .map(|mut m| {
          m.description = m.description.map(|d| t_or(&d));
          m
        })
        .collect()
    })
  }

  /// Every session/new / resume / load response: synthetic modes fill in, a resumed session keeps its persisted mode
  pub(crate) fn apply_controls(&self, c: &mut Core, modes: Option<&Value>, config_options: Option<&Value>) {
    let wanted = c.state.controls.mode_id.clone();
    if self.protocol_controls(&mut c.state.controls, modes, config_options) {
      return;
    }
    let Some(syn) = self.synthetic_modes() else { return };
    if !c.state.controls.modes.is_empty() {
      return;
    }
    c.state.controls.mode_id = Some(match wanted {
      Some(w) if syn.iter().any(|m| m.id == w) => w,
      _ => "default".into(),
    });
    c.state.controls.modes = syn;
    c.perms.auto_approve = c.state.controls.mode_id.as_deref() == Some("yolo");
  }

  /// Paint last-known chips before session/new returns so the composer isn't empty during start: the remembered values
  /// over the options and modes an earlier session of this agent showed
  pub fn preview_controls(&self, known: &SessionControls, settings: Option<&TurnSettings>) {
    let mut c = self.core.lock();
    self.seed_ultracode(&mut c, settings);
    let remembered_mode = settings.and_then(|s| s.mode_id.clone());
    if let Some(syn) = self.synthetic_modes().filter(|s| !s.is_empty()) {
      c.state.controls.mode_id = Some(match remembered_mode {
        Some(m) if syn.iter().any(|x| x.id == m) => m,
        _ => syn[0].id.clone(),
      });
      c.state.controls.modes = syn;
      c.perms.auto_approve = c.state.controls.mode_id.as_deref() == Some("yolo");
    } else if let Some(m) = remembered_mode.filter(|m| known.modes.iter().any(|x| &x.id == m)) {
      // Protocol modes only arrive with session/new; without a remembered one the chip stays empty rather than guess
      c.state.controls.modes = known.modes.clone();
      c.state.controls.mode_id = Some(m);
      c.state.controls.mode_config_id = known.mode_config_id.clone();
    }
    if known.options.is_empty() {
      return;
    }
    let mut next = known.options.clone();
    for ctl in &mut next {
      if let Some(v) = settings.and_then(|s| s.config.get(&ctl.id))
        && ctl.options.iter().any(|o| &o.id == v)
      {
        ctl.value = Some(v.clone());
      }
    }
    c.state.controls.options = next;
  }
}

/// Controls with in-flight picks overlaid; a pick the controls do not offer (a held remembered value the agent dropped)
/// leaves agent truth showing
pub(crate) fn picked_controls(c: &Core) -> SessionControls {
  let mut out = c.state.controls.clone();
  if let Some((v, _)) = c.picks.values.get(MODE_PICK)
    && out.modes.iter().any(|m| &m.id == v)
  {
    out.mode_id = Some(v.clone());
  }
  for o in &mut out.options {
    if let Some((v, _)) = c.picks.values.get(&o.id)
      && o.options.iter().any(|x| &x.id == v)
    {
      o.value = Some(v.clone());
    }
  }
  out
}
