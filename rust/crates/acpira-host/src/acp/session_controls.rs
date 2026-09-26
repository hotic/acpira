//! Modes and config options of a session: wire requests,
//! the optimistic pick overlay, effort preservation across model switches, and replaying remembered choices

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::composer_controls::{is_reasoning_control, thought_correction};
use acpira_shared::models::parse_fusion_name;
use acpira_shared::transcript::*;

use super::normalize::{apply_config_options, config_option_set_value, init_controls};
use super::rpc::BoxFuture;
use super::session::{AcpSession, MODE_PICK};
use crate::i18n::t;

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
        me.core.lock().auto_approve = id == "yolo";
        if let Some(w) = wire {
          proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": w })).await?;
        }
        let mut c = me.core.lock();
        c.state.controls.mode_id = Some(id.clone());
        if c.auto_approve {
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

  /// Any configOption (model / reasoning level / boolean toggles); the response is the full configOptions set
  pub fn set_config(self: &Arc<Self>, config_id: String, value: String) -> BoxFuture<Result<()>> {
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
        let keep = model.is_some() && (sidekick_only || !c.adopting);
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
          let c = me.core.lock();
          c.state
            .controls
            .options
            .iter()
            .find(|o| o.id == id)
            .is_some_and(|cur| cur.value.as_ref() != Some(&prev) && cur.options.iter().any(|o| o.id == prev))
        };
        if restore {
          me.set_config(id, prev).await?;
        }
      }
      if !me.core.lock().syncing_thought {
        me.sync_thought().await?;
      }
      let polled_model = {
        let c = me.core.lock();
        matches!(me.agent.as_str(), "grok" | "pi")
          && !c.usage_notifications
          && c.state.controls.options.iter().find(|o| o.id == config_id).and_then(|o| o.category.as_deref()) == Some("model")
      };
      if polled_model {
        // Grok's snapshot is per model; Pi keeps the same messages and only the window moves
        if me.agent == "grok" {
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
    let held = {
      let mut c = self.core.lock();
      let control = c.state.controls.options.iter().find(|o| o.id == config_id).cloned();
      let ok =
        c.proc.is_some() && c.status == SessionStatus::Ready && control.as_ref().is_some_and(|x| x.options.iter().any(|o| o.id == value));
      if !ok {
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
            .filter(|o| is_reasoning_control(o) && o.value.is_some() && !c.picks.contains_key(&o.id))
            .map(|o| (o.id.clone(), o.value.clone().unwrap()))
            .collect();
          for (id, v) in reasoning {
            c.pick_seq += 1;
            let token = c.pick_seq;
            c.picks.insert(id.clone(), (v, token));
            held.push((id, token));
          }
        }
        Some(held)
      }
    };
    let Some(held) = held else { return self.set_config(config_id, value).await };
    let me = self.clone();
    let (cid, v) = (config_id.clone(), value.clone());
    let result = self.pick(config_id, value, move || me.set_config(cid, v)).await;
    let mut c = self.core.lock();
    for (id, token) in &held {
      if c.picks.get(id).is_some_and(|(_, t)| t == token) {
        c.picks.remove(id);
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
      c.pick_seq += 1;
      let token = c.pick_seq;
      c.picks.insert(key.clone(), (value, token));
      self.touch(&mut c);
      token
    };
    let _serial = self.pick_lock.lock().await;
    if self.core.lock().picks.get(&key).is_none_or(|(_, t)| *t != token) {
      return Ok(());
    }
    let result = run().await;
    let mut c = self.core.lock();
    if c.picks.get(&key).is_some_and(|(_, t)| *t == token) {
      c.picks.remove(&key);
      self.touch(&mut c);
    }
    result
  }

  /// Kimi appends the previous thinking value when the new model does not offer it, and the catalogue may narrow the
  /// current one away: push a native value instead
  pub(crate) async fn sync_thought(self: &Arc<Self>) -> Result<()> {
    self.core.lock().syncing_thought = true;
    let ids: Vec<String> = self.core.lock().state.controls.options.iter().map(|o| o.id.clone()).collect();
    let mut result = Ok(());
    for id in ids {
      let next = self.core.lock().state.controls.options.iter().find(|o| o.id == id).and_then(thought_correction);
      if let Some(next) = next
        && let Err(e) = self.set_config(id, next).await
      {
        result = Err(e);
        break;
      }
    }
    self.core.lock().syncing_thought = false;
    result
  }

  /// Replay what was chosen last time in this agent, one request per difference in control order; choices the agent
  /// no longer offers are skipped, a refused one is logged and the rest go on
  pub async fn adopt_controls(self: &Arc<Self>, settings: TurnSettings) {
    {
      let mut c = self.core.lock();
      if c.status != SessionStatus::Ready || c.proc.is_none() {
        return;
      }
      c.adopting = true;
    }
    let ids: Vec<String> = self.core.lock().state.controls.options.iter().map(|o| o.id.clone()).collect();
    for id in ids {
      let Some(value) = settings.config.get(&id).filter(|v| !v.is_empty()).cloned() else { continue };
      let differs = {
        let c = self.core.lock();
        c.state
          .controls
          .options
          .iter()
          .find(|o| o.id == id)
          .is_some_and(|ctl| ctl.value.as_ref() != Some(&value) && ctl.options.iter().any(|o| o.id == value))
      };
      if !differs {
        continue;
      }
      if let Err(e) = self.set_config(id.clone(), value.clone()).await {
        self.log(&format!("adopt {id}={value} refused: {e}"));
      }
    }
    self.core.lock().adopting = false;
    if let Some(mode) = settings.mode_id.filter(|m| !m.is_empty()) {
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
