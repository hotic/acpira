//! Claude's host-made Ultra effort level (`vendors/claude_ultracode`): its state, its place on the effort select, and
//! the same-process rebuild switching ultracode needs. The adapter has no config option for ultracode, so picking Ultra
//! (or leaving it for another level) never goes out as session/set_config_option: the next session/resume (or load) on
//! the live process carries the new `_meta.claudeCode.options.settings`, which changes the adapter's session fingerprint
//! and makes it recreate the query with the same transcript; the rebuild then sets the wire effort (`xhigh` for Ultra,
//! the picked level otherwise). A pick made while a turn (or background work) runs is applied right before the next
//! prompt goes out

use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

use acpira_shared::composer_controls::is_reasoning_control;
use acpira_shared::transcript::*;
use acpira_shared::turn_settings::capture_turn_settings;

use crate::acp::session::controls::MODE_PICK;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::has_live_async_task;
use crate::acp::transport::process::AgentProcess;
use crate::acp::vendors::claude_ultracode as uc;

#[derive(Default)]
pub(crate) struct UltracodeState {
  /// Ultra is the chosen effort: what the select shows and the next session request carries (when the agent can take it)
  pub on: bool,
  /// What the last session/new, resume or load request carried
  pub sent: bool,
  /// A rebuild owns the wire: prompts queue and nothing else reopens the session until it ends
  pub rebuilding: bool,
  /// Dynamic workflows are on for this agent and cwd; read once per connection (the settings files can change)
  pub workflows: Option<bool>,
  /// The agent's own value of the effort select, which shows as Ultra while `on`
  pub wire: Option<String>,
  /// The level picked to leave Ultra, shown and set by the rebuild that turns ultracode off
  pub level: Option<String>,
}

/// What an effort pick on a Claude session means for ultracode (`ultracode_pick`)
pub(crate) enum UltraPick {
  /// Not the Ultra select, or a level while ultracode is and stays off: the normal set_config_option
  Wire,
  /// Nothing to send (Ultra while it cannot be offered, a level the coming rebuild takes along)
  Done,
  /// Ultra while ultracode is on: only the wire effort may still have to move to Ultra's target
  Target(String),
  /// Ultracode on, or off with this level: a rebuild
  Switch(bool, Option<String>),
}

impl AcpSession {
  /// Claude with dynamic workflows on: Ultra can do something
  pub(crate) fn ultracode_available(&self, c: &mut Core) -> bool {
    if !self.vendor.ultracode() {
      return false;
    }
    *c.ultracode.workflows.get_or_insert_with(|| uc::workflows_enabled_for(self.def().env.as_ref(), &self.cwd))
  }

  /// The value a session request carries now
  fn ultracode_wanted(&self, c: &mut Core) -> bool {
    c.ultracode.on && self.ultracode_available(c)
  }

  /// Put Ultra back on the effort select after the options changed (every publish runs this through
  /// `refine_controls`): an agent answer or update replaces the whole option list
  pub(crate) fn place_ultracode(&self, c: &mut Core) {
    if !self.vendor.ultracode() {
      return;
    }
    let available = self.ultracode_available(c);
    let Core { ultracode, state, .. } = c;
    uc::place(&mut state.controls.options, available, ultracode.on, ultracode.level.as_deref(), &mut ultracode.wire);
  }

  /// Remembered or per-turn settings for a session request still to be made (new session, edit): an Ultra effort turns
  /// ultracode on, any other level turns it off
  pub(crate) fn seed_ultracode(&self, c: &mut Core, settings: Option<&TurnSettings>) {
    if !self.vendor.ultracode() {
      return;
    }
    if let Some(on) = settings.and_then(|s| uc::requested(&s.config, &c.state.controls.options)) {
      c.ultracode.on = on;
      c.ultracode.level = None;
    }
  }

  /// The Ultra select's id while Ultra can be offered
  fn ultra_effort_id(&self, c: &mut Core) -> Option<String> {
    if !self.vendor.ultracode() || !self.ultracode_available(c) {
      return None;
    }
    uc::effort_index(&c.state.controls.options).map(|i| c.state.controls.options[i].id.clone())
  }

  /// The value to send for `value` on control `id` and the agent's own current value, for replays that compare before
  /// they send: on the Ultra select, Ultra becomes its wire target and the noted agent value is what it is compared
  /// with. None when Ultra has nothing to stand for
  pub(crate) fn wire_pick(&self, c: &mut Core, id: &str, value: &str) -> Option<(String, Option<String>)> {
    self.refine_controls(c);
    let control = c.state.controls.options.iter().find(|o| o.id == id);
    let current = control.and_then(|o| o.value.clone());
    let ours = self.vendor.ultracode() && control.is_some_and(|o| o.options.iter().any(|x| x.id == uc::LEVEL_ID));
    if !ours {
      // Claude has no real `ultra` effort: a remembered one the select no longer offers is dropped, never sent
      let stray = self.vendor.ultracode() && value == uc::LEVEL_ID;
      return (!stray).then(|| (value.to_owned(), current));
    }
    let wire = uc::wire_value(&c.state.controls.options, id, value)?;
    Some((wire, c.ultracode.wire.clone()))
  }

  /// What a set_config of `value` on control `id` does on a Claude session
  pub(crate) fn ultracode_pick(&self, c: &mut Core, id: &str, value: &str) -> UltraPick {
    if !self.vendor.ultracode() {
      return UltraPick::Wire;
    }
    let ultra = value == uc::LEVEL_ID;
    if self.ultra_effort_id(c).as_deref() != Some(id) {
      // An Ultra that cannot be offered never reaches the wire
      let effort = c.state.controls.options.iter().any(|o| o.id == id && is_reasoning_control(o));
      return if ultra && effort { UltraPick::Done } else { UltraPick::Wire };
    }
    match (ultra, c.ultracode.on) {
      (true, true) => {
        let target = uc::wire_value(&c.state.controls.options, id, value);
        match target.filter(|t| c.ultracode.wire.as_ref() != Some(t)) {
          Some(t) => UltraPick::Target(t),
          None => UltraPick::Done,
        }
      }
      (true, false) => UltraPick::Switch(true, None),
      (false, true) => UltraPick::Switch(false, Some(value.to_owned())),
      // Ultracode is already off but its rebuild has not run: the rebuild sets this level instead
      (false, false) if c.ultracode.level.is_some() => {
        c.ultracode.level = Some(value.to_owned());
        self.touch(c);
        UltraPick::Done
      }
      (false, false) => UltraPick::Wire,
    }
  }

  /// Ask for ultracode in a session/new, resume or load request when Ultra is the effort, and note what went out
  pub(crate) fn with_ultracode(&self, req: Value) -> Value {
    if !self.vendor.ultracode() {
      return req;
    }
    let on = {
      let mut c = self.core.lock();
      let on = self.ultracode_wanted(&mut c);
      c.ultracode.sent = on;
      on
    };
    if !on {
      return req;
    }
    let env = self.def().env;
    let model_config = env.as_ref().and_then(|e| e.get(uc::MODEL_CONFIG_ENV).cloned()).or_else(|| std::env::var(uc::MODEL_CONFIG_ENV).ok());
    uc::with_ultracode(req, model_config.as_deref())
  }

  /// The native session was built with another ultracode than the effort shows, and could be rebuilt now
  fn ultracode_due(&self, c: &mut Core) -> bool {
    self.vendor.ultracode()
      && c.status == SessionStatus::Ready
      && !c.ultracode.rebuilding
      && c.proc.is_some()
      && c.acp_session_id.is_some()
      && self.ultracode_wanted(c) != c.ultracode.sent
  }

  /// A rebuild may start now. claude-agent-acp answers it with `teardownSession` (cancel the turns, close the query
  /// stream, abort), which kills whatever still runs in the old query: a workflow's background task, subagents, summoned
  /// rounds, and the permission / question cards they wait on. So nothing of that may be running, and unless the caller
  /// has claimed the next prompt and rebuilds ahead of it (`own_turn`), nothing may be on the wire either: no turn,
  /// edit, compaction, plan build, account switch or detached peer turn
  fn rebuild_allowed(c: &Core, own_turn: bool) -> bool {
    let wire_idle = own_turn || (!Self::busy(c) && c.pending_prompt.is_none() && !c.building_plan && c.compaction.completion.is_none());
    wire_idle
      && !c.peer.detached
      && !c.tree.any_running()
      && !has_live_async_task(&c.state)
      && c.relays.rounds.is_empty()
      && c.perms.pending.is_empty()
      && c.questions.pending.is_empty()
  }

  /// Reserve a due rebuild when `rebuild_allowed`; the caller then runs `run_ultracode_rebuilds` under the pick lock
  pub(crate) fn reserve_ultracode_rebuild(&self, c: &mut Core, own_turn: bool) -> bool {
    if !self.ultracode_due(c) {
      return false;
    }
    let go = Self::rebuild_allowed(c, own_turn);
    if go {
      c.ultracode.rebuilding = true;
    } else if own_turn {
      self.log("ultracode: background work still runs, this prompt goes out without the change; it applies before a later one");
    }
    go
  }

  /// Switch ultracode from an effort pick (`UltraPick::Switch`): Ultra (`on`) or `level` shows at once, the rebuild runs
  /// now or right before the next prompt goes out (`claim` in prompt.rs) and sets the wire effort. The caller holds the
  /// pick lock: `select_config` reaches here through `pick`, the remembered replay through `adopt_controls`, a model
  /// switch that leaves no effort select through set_config. A failed rebuild puts the effort back where the native
  /// session is
  pub(crate) async fn set_ultracode(self: &Arc<Self>, on: bool, level: Option<String>) -> Result<()> {
    let start = {
      let mut c = self.core.lock();
      // Ultra is only picked from a select that offers it; leaving it is always allowed
      let offered = self.ultra_effort_id(&mut c).is_some();
      if c.status != SessionStatus::Ready || c.proc.is_none() || (on && !offered) || (c.ultracode.on == on && level.is_none()) {
        return Ok(());
      }
      c.ultracode.on = on;
      c.ultracode.level = if on { None } else { level };
      self.touch(&mut c);
      let start = self.reserve_ultracode_rebuild(&mut c, false);
      if !start && self.ultracode_due(&mut c) {
        self.log(&format!("ultracode {on}: applied before the next prompt"));
      }
      start
    };
    if !start {
      return Ok(());
    }
    let result = self.run_ultracode_rebuilds(false).await;
    self.flush_queue();
    result
  }

  /// Remembered settings replayed on a running session (a fork, a reopen) that ask for another ultracode than the one
  /// the session was started with; a new session's own settings already rode its first request (`seed_ultracode`). The
  /// caller holds the pick lock
  pub(crate) async fn adopt_ultracode(self: &Arc<Self>, settings: &TurnSettings) {
    let want = {
      let mut c = self.core.lock();
      let Some(id) = self.ultra_effort_id(&mut c) else { return };
      let want = uc::requested(&settings.config, &c.state.controls.options);
      match want.filter(|w| *w != c.ultracode.on) {
        Some(w) => (w, settings.config.get(&id).cloned().filter(|_| !w)),
        None => return,
      }
    };
    if let Err(e) = self.set_ultracode(want.0, want.1).await {
      self.log(&format!("adopt ultracode {}: {e}", want.0));
    }
  }

  /// The rebuild a claimed prompt reserved in `claim`, run before the prompt is staged and sent. false = the session did
  /// not come back Ready and the prompt must not go out
  pub(crate) async fn rebuild_before_prompt(self: &Arc<Self>) -> bool {
    {
      let _serial = self.pick_lock.lock().await;
      // The error is logged where it happened; the prompt goes on under whatever the native session now has
      let _ = self.run_ultracode_rebuilds(true).await;
    }
    self.status() == SessionStatus::Ready
  }

  /// One reserved rebuild, then again as long as a pick made meanwhile left the shown value apart from the sent one
  async fn run_ultracode_rebuilds(self: &Arc<Self>, own_turn: bool) -> Result<()> {
    loop {
      self.rebuild_ultracode().await?;
      let again = {
        let mut c = self.core.lock();
        self.reserve_ultracode_rebuild(&mut c, own_turn)
      };
      if !again {
        return Ok(());
      }
    }
  }

  /// Rebuild the live native session with the current ultracode: session/resume (or load) on the same process, then the
  /// model, effort (Ultra as its wire target), Fast and mode the session showed are set again, held on screen meanwhile. Needs `rebuilding`
  /// reserved by the caller; releases it
  pub(crate) async fn rebuild_ultracode(self: &Arc<Self>) -> Result<()> {
    let prep = {
      let mut c = self.core.lock();
      let target = c.proc.clone().filter(|p| p.alive()).zip(c.acp_session_id.clone());
      target.and_then(|(proc, sid)| {
        let caps = proc.caps();
        let method = if crate::json::truthy(caps.get("sessionCapabilities").and_then(|s| s.get("resume"))) {
          "session/resume"
        } else if crate::json::truthy(caps.get("loadSession")) {
          "session/load"
        } else {
          return None;
        };
        // The effort shows Ultra or the level picked to leave it: the replay below turns that into the wire value
        let settings = capture_turn_settings(&c.state.controls);
        let previous_effort = uc::effort_index(&c.state.controls.options)
          .map(|i| c.state.controls.options[i].id.clone())
          .zip(c.ultracode.wire.clone());
        // Hold what the session shows: the recreated query may report the adapter's defaults until they are set again
        let mut held = vec![];
        let shown: Vec<(String, String)> = c
          .state
          .controls
          .options
          .iter()
          .filter_map(|o| o.value.clone().map(|v| (o.id.clone(), v)))
          .chain(c.state.controls.mode_id.clone().map(|m| (MODE_PICK.to_owned(), m)))
          .collect();
        for (key, value) in shown {
          if c.picks.values.contains_key(&key) {
            continue;
          }
          c.picks.seq += 1;
          let token = c.picks.seq;
          c.picks.values.insert(key.clone(), (value, token));
          held.push((key, token));
        }
        if method == "session/load" {
          c.replaying = !c.state.turns.is_empty();
        }
        Some((proc, sid, method, settings, previous_effort, held, c.ultracode.sent))
      })
    };
    let Some((proc, sid, method, mut settings, previous_effort, held, previous)) = prep else {
      let mut c = self.core.lock();
      c.ultracode.rebuilding = false;
      c.ultracode.level = None;
      let sent = c.ultracode.sent;
      let changed = c.ultracode.on != sent;
      c.ultracode.on = sent;
      self.touch(&mut c);
      return if changed { Err(anyhow!("ultracode needs session/resume or session/load on a live process")) } else { Ok(()) };
    };
    let req = self.session_request(&proc, Some(&sid)).await;
    let on = self.core.lock().ultracode.sent;
    self.log(&format!("ultracode {on}: rebuilding the session through {method}"));
    let result = match proc.request_ordered(method, req).await {
      Ok((r, handoff)) => {
        {
          let mut c = self.core.lock();
          c.replaying = false;
          self.adopt_rebuilt_controls(&mut c, &r);
        }
        drop(handoff);
        self.log(&format!("{method} ok (ultracode {on})"));
        Ok(())
      }
      Err(e) => {
        let e = anyhow::Error::new(e);
        {
          let mut c = self.core.lock();
          c.replaying = false;
          c.ultracode.sent = previous;
          // A pick made while this one was on the wire stays; only the value this rebuild tried goes back, and the
          // effort with it to what the native session had
          if c.ultracode.on == on {
            c.ultracode.on = previous;
            c.ultracode.level = None;
            if let Some((id, wire)) = previous_effort.clone() {
              settings.config.insert(id, wire);
            }
          }
        }
        self.log(&format!("ultracode {on}: {method} failed, effort reverted: {}", crate::acp::session::errors::error_text(&e)));
        self.restore_after_failed_rebuild(&proc, &sid, method).await;
        Err(e)
      }
    };
    // The overlay holds the values; the agent gets them back in control order (model before effort). A restored session
    // may have come back with defaults too
    if self.status() == SessionStatus::Ready && proc.alive() {
      self.replay_settings(&settings, |_| false).await;
    }
    let mut c = self.core.lock();
    for (key, token) in held {
      if c.picks.values.get(&key).is_some_and(|(_, t)| *t == token) {
        c.picks.values.remove(&key);
      }
    }
    c.ultracode.rebuilding = false;
    // The level picked to leave Ultra has been set (or refused) by the replay: agent truth shows from here
    if c.ultracode.on == c.ultracode.sent {
      c.ultracode.level = None;
    }
    self.touch(&mut c);
    result
  }

  /// claude-agent-acp tears the old query down before it builds the new one (`getOrCreateSession`), so a rebuild that
  /// failed may have left the adapter without the native session. The same request with the old settings brings it back
  /// (or, when the old query still stands, matches its fingerprint and changes nothing); when that fails too the session
  /// turns Error and Retry reopens it on a fresh process
  async fn restore_after_failed_rebuild(self: &Arc<Self>, proc: &Arc<AgentProcess>, sid: &str, method: &str) {
    // A dead process is the exit handler's to report
    if !proc.alive() {
      return;
    }
    {
      let mut c = self.core.lock();
      if method == "session/load" {
        c.replaying = !c.state.turns.is_empty();
      }
    }
    let req = self.session_request(proc, Some(sid)).await;
    let restored = proc.request_ordered(method, req).await;
    let mut c = self.core.lock();
    c.replaying = false;
    match restored {
      Ok((r, handoff)) => {
        self.adopt_rebuilt_controls(&mut c, &r);
        drop(c);
        drop(handoff);
        self.log(&format!("{method} ok: native session restored with ultracode {}", self.core.lock().ultracode.sent));
      }
      Err(e) => {
        let e = anyhow::Error::new(e);
        self.log(&format!("{method} with the previous settings failed too: {}", crate::acp::session::errors::error_text(&e)));
        self.fail(&mut c, &e);
      }
    }
  }

  /// A rebuild's response: what it carries replaces agent truth, what it leaves out (some peers answer a resume with
  /// modes only) stays as it was
  fn adopt_rebuilt_controls(&self, c: &mut Core, r: &Value) {
    let before = c.state.controls.clone();
    let has_modes = r.get("modes").is_some_and(|m| !m.is_null());
    self.apply_controls(c, r.get("modes"), r.get("configOptions"));
    if !has_modes {
      c.state.controls.modes = before.modes;
      c.state.controls.mode_id = before.mode_id;
      c.state.controls.mode_config_id = before.mode_config_id;
    }
    self.refine_controls(c);
  }
}
