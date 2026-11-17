//! A session's controls: modes, the model / effort / Fast / approval / sidekick config options, and pushing them when settings change

use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_rpc::rpc::RpcError;
use acpira_shared::model_catalog::Catalog;

use super::{Server, Session, SessionState, str_param};
use crate::config::Config;
use crate::llm::Item;
use crate::modes;
use crate::permission::Approval;

impl Server {
  /// The model a new session starts on: the profile's when it is configured, else the settings page's default
  pub(super) fn default_pick(&self, config: &Config) -> Option<String> {
    self.profile.as_ref().and_then(|p| p.model.clone()).filter(|m| config.find(m).is_some()).or_else(|| config.default_pick())
  }

  /// A mode this agent's sessions may be in (a specialised agent offers fewer)
  pub(super) fn offers_mode(&self, mode: &str) -> bool {
    modes::find(mode).is_some() && self.profile.as_ref().is_none_or(|p| p.offers_mode(mode))
  }

  /// The mode picker: Raw mode only while the settings page offers it, Read-only only to a summoned session; either
  /// also while the session is in it
  fn mode_options(&self, config: &Config, st: &SessionState) -> Vec<Value> {
    let listed = |id: &str| match id {
      modes::RAW => config.raw_mode() || st.mode == modes::RAW,
      modes::READ_ONLY => st.summoned || st.mode == modes::READ_ONLY,
      _ => true,
    };
    let shown = |id: &str| self.offers_mode(id) && listed(id);
    modes::MODES.iter().filter(|m| shown(m.id)).map(|m| json!({ "id": m.id, "name": m.name, "description": m.description })).collect()
  }

  pub(super) fn modes_of(&self, config: &Config, st: &mut SessionState) -> Value {
    st.sent_modes = self.mode_options(config, st);
    json!({ "currentModeId": st.mode, "availableModes": st.sent_modes })
  }

  pub(super) fn set_mode(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let mode = str_param(params, "modeId")?;
    if !self.offers_mode(mode) {
      return Err(RpcError::new(-32602, format!("Unknown mode: {mode}")));
    }
    let mut st = session.state.lock();
    // Raw mode sends no tools, and Anthropic refuses a request whose history holds tool calls without them (other
    // formats vary), so a session that has used tools stays out of it; Raw → Agent / Plan is always fine
    if mode == modes::RAW && st.mode != modes::RAW && st.items.iter().any(uses_tools) {
      return Err(RpcError::new(-32602, "Raw mode needs a session without tool calls; start a new session for it"));
    }
    st.mode = mode.to_owned();
    session.save_state(&st);
    session.permissions_changed.notify_waiters();
    Ok(json!({}))
  }

  pub(super) fn set_config_option(&self, params: &Value) -> Result<Value, RpcError> {
    let session = self.session(str_param(params, "sessionId")?)?;
    let config_id = str_param(params, "configId")?;
    let raw = params.get("value").unwrap_or(&Value::Null);
    let value = raw.as_str().unwrap_or_default();
    if config_id != FAST && config_id != SIDEKICK_FAST && !raw.is_string() {
      return Err(RpcError::new(-32602, "value must be a string"));
    }
    let config = session.config();
    let catalog = self.catalog.get();
    let mut st = session.state.lock();
    // The effort check below must see the model the picker shows
    reconcile(&config, &catalog, &mut st);
    match config_id {
      "model" => {
        if config.find(value).is_none() {
          return Err(RpcError::new(-32602, format!("Unknown model: {value}")));
        }
        st.model = Some(value.to_owned());
        // An effort the new model does not offer falls back to its default
        let efforts = efforts_of(&config, &catalog, st.model.as_deref());
        if !st.effort.as_ref().is_some_and(|e| efforts.contains(e)) {
          st.effort = default_effort(&config, &catalog, st.model.as_deref());
        }
      }
      "effort" => {
        if !efforts_of(&config, &catalog, st.model.as_deref()).iter().any(|e| e == value) {
          return Err(RpcError::new(-32602, format!("Unknown effort: {value}")));
        }
        st.effort = Some(value.to_owned());
      }
      APPROVAL => {
        st.approval = Approval::parse(value).ok_or_else(|| RpcError::new(-32602, format!("Unknown approval level: {value}")))?;
        session.permissions_changed.notify_waiters();
      }
      FAST => {
        if !st.model.as_deref().and_then(|p| config.find(p)).is_some_and(|(p, m)| crate::llm::fast::offered(p, m)) {
          return Err(RpcError::new(-32602, "Fast is not offered for this model"));
        }
        // A real boolean from a client that takes boolean options, else the select's value
        st.fast = match (raw.as_bool(), value) {
          (Some(on), _) => on,
          (None, "true" | FAST) => true,
          (None, "false" | STANDARD) => false,
          _ => return Err(RpcError::new(-32602, format!("Unknown fast value: {raw}"))),
        };
      }
      SIDEKICK => {
        let next = match value {
          NO_SIDEKICK => None,
          pick if config.find(pick).is_some() => Some(pick.to_owned()),
          _ => return Err(RpcError::new(-32602, format!("Unknown model: {value}"))),
        };
        st.sidekick = next;
        // A level the new sidekick does not offer falls back to its default
        let efforts = efforts_of(&config, &catalog, st.sidekick.as_deref());
        if !st.sidekick_effort.as_ref().is_some_and(|e| efforts.contains(e)) {
          st.sidekick_effort = None;
        }
        // Before the first message nothing went out with the tool table yet: acpira lists its tools again, so the first
        // request names the sidekick in ask_agent. Later the next message carries a note instead (`turn.rs`)
        if st.items.is_empty() {
          st.sidekick_told = sidekick_of(&config, &st);
          session.mcp.relist(ACPIRA_SERVER);
        }
      }
      SIDEKICK_EFFORT => {
        // The sidekick runs on another model, so its levels are that model's; the lead is never told of them
        let pick = sidekick_of(&config, &st);
        if !efforts_of(&config, &catalog, pick.as_deref()).iter().any(|e| e == value) {
          return Err(RpcError::new(-32602, format!("Unknown sidekick effort: {value}")));
        }
        st.sidekick_effort = Some(value.to_owned());
      }
      SIDEKICK_FAST => {
        if sidekick_fast_of(&config, &st).is_none() {
          return Err(RpcError::new(-32602, "Fast is not offered for the sidekick's model"));
        }
        st.sidekick_fast = match (raw.as_bool(), value) {
          (Some(on), _) => on,
          (None, "true" | FAST) => true,
          (None, "false" | STANDARD) => false,
          _ => return Err(RpcError::new(-32602, format!("Unknown sidekick fast value: {raw}"))),
        };
      }
      other => return Err(RpcError::new(-32602, format!("Unknown config option: {other}"))),
    }
    session.save_state(&st);
    Ok(json!({ "configOptions": sent_options(&config, &catalog, &mut st, self.offers()) }))
  }

  /// Push changed modes and config options to open sessions: settings edits update the composer's own pickers,
  /// not just the settings page's probe
  pub fn push_controls(&self) {
    let sessions: Vec<Arc<Session>> = self.sessions.lock().values().cloned().collect();
    for s in sessions {
      self.push_controls_of(&s);
    }
  }

  /// `push_controls` for one session, against the settings it reads
  pub(super) fn push_controls_of(&self, s: &Arc<Session>) {
    let config = s.config();
    let catalog = self.catalog.get();
    let (options, modes, usage) = {
      let mut st = s.state.lock();
      if reconcile(&config, &catalog, &mut st) {
        s.save_state(&st);
      }
      let options = config_options(&config, &catalog, &st, self.offers());
      let options = if st.sent_options.as_ref().is_none_or(|sent| *sent == options) {
        None
      } else {
        st.sent_options = Some(options.clone());
        Some(options)
      };
      let modes = self.mode_options(&config, &st);
      let modes = if st.sent_modes == modes {
        None
      } else {
        st.sent_modes = modes.clone();
        Some((st.mode.clone(), modes))
      };
      // The window of the session's model as configured now (edited in the settings, or another model picked)
      let window = st.model.as_deref().and_then(|p| crate::turn::limits(&config, &catalog, p)).map(|m| m.context.unwrap_or(crate::turn::DEFAULT_CONTEXT));
      let usage = match (st.reported, window) {
        (Some((used, size)), Some(w)) if w != size => {
          st.reported = Some((used, w));
          Some((used, w))
        }
        _ => None,
      };
      (options, modes, usage)
    };
    if let Some(options) = options {
      self.conn().notify("session/update", json!({ "sessionId": s.id, "update": { "sessionUpdate": "config_option_update", "configOptions": options } }));
    }
    if let Some((mode, modes)) = modes {
      // ACP has no mode-list update; the standard current-mode notification carries our additive extension
      self.conn().notify("session/update", json!({ "sessionId": s.id, "update": {
        "sessionUpdate": "current_mode_update", "currentModeId": mode, "_meta": { "acpira/availableModes": modes }
      } }));
    }
    if let Some((used, size)) = usage {
      // Recorded like the turn's own, so a replay ends on the current window
      self.update(&s.id, json!({ "sessionUpdate": "usage_update", "used": used, "size": size }));
    }
  }

  /// `push_controls` once a second while the connection lives. Nothing on the wire announces a changed
  /// `providers.json`, and the check is two `stat` calls while nothing changed (`ConfigCache`, `CatalogCache`)
  pub async fn watch_controls(self: Arc<Self>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
      tick.tick().await;
      self.push_controls();
    }
  }

  /// What the client takes beside plain selects, for `config_options`
  pub(super) fn offers(&self) -> Offers {
    Offers { booleans: self.booleans.load(Ordering::Relaxed), sidekick: self.sidekicks.load(Ordering::Relaxed) }
  }
}

/// The levels a configured model offers (`ProviderModel::offered_efforts`, the rule the settings page shows too); none
/// for a pick that is not configured
fn efforts_of(config: &Config, catalog: &Catalog, pick: Option<&str>) -> Vec<String> {
  pick.and_then(|p| config.find(p)).map(|(_, m)| m.offered_efforts(catalog)).unwrap_or_default()
}

/// The model family's preferred level when offered, else the last (strongest) level
pub(crate) fn default_effort(config: &Config, catalog: &Catalog, pick: Option<&str>) -> Option<String> {
  let efforts = efforts_of(config, catalog, pick);
  let preferred = pick.and_then(|p| config.find(p)).map(|(p, m)| crate::llm::family::resolve(p, m).effort);
  efforts.iter().find(|e| Some(e.as_str()) == preferred).or(efforts.last()).cloned()
}

/// Bring the session's model and effort in line with what `config_options` shows: a session opened before any model
/// existed, or holding one no longer usable, takes the default, and an effort the current model does not offer (or none, where it now offers levels)
/// becomes the model's default. Without this the picker showed a fallback the session did not hold, so setting the
/// shown effort was refused and a turn sent no effort until the model was switched once. True when something changed
pub(crate) fn reconcile(config: &Config, catalog: &Catalog, st: &mut SessionState) -> bool {
  let before = (st.model.clone(), st.effort.clone());
  // A model no longer usable (removed, or switched off in settings) gives way to the default, the one the picker shows
  // for it (`config_options`). A broken file, or no usable model at all, keeps the pick for when the file comes back
  if config.error.is_none()
    && !st.model.as_deref().is_some_and(|p| config.find(p).is_some())
    && let Some(pick) = config.default_pick()
  {
    st.model = Some(pick);
  }
  if st.model.as_deref().is_some_and(|p| config.find(p).is_some()) {
    let efforts = efforts_of(config, catalog, st.model.as_deref());
    if !st.effort.as_ref().is_some_and(|e| efforts.contains(e)) {
      st.effort = default_effort(config, catalog, st.model.as_deref());
    }
  }
  (st.model.clone(), st.effort.clone()) != before
}

/// `config_options`, remembered as what the client has
pub(super) fn sent_options(config: &Config, catalog: &Catalog, st: &mut SessionState, offers: Offers) -> Vec<Value> {
  let options = config_options(config, catalog, st, offers);
  st.sent_options = Some(options.clone());
  options
}

/// The approval select: its id and its category (custom, ACP leaves names starting with `_` free). The composer folds it
/// into the mode chip's menu, so modes and the level read as one control
pub const APPROVAL: &str = "approval";

const APPROVAL_CATEGORY: &str = "_permission_mode";

/// The Fast switch's id, and its values as a select for a client without boolean options (the shape Devin uses)
const FAST: &str = "fast";

const STANDARD: &str = "standard";

/// The sidekick select: its id, the value for none, and its category (custom, so a client that does not know it shows it
/// as a plain select rather than as a model or a model parameter)
pub const SIDEKICK: &str = "sidekick";

pub const NO_SIDEKICK: &str = "none";

const SIDEKICK_CATEGORY: &str = "_sidekick";

/// The sidekick's effort select, offered while the sidekick's model has levels. Its own custom category, so a client
/// does not take it for the lead's effort
pub const SIDEKICK_EFFORT: &str = "sidekick_effort";

const SIDEKICK_EFFORT_CATEGORY: &str = "_sidekick_effort";

/// The sidekick's Fast switch, offered while the sidekick's model has a fast tier. Its own custom category, so a client
/// does not take it for the lead's Fast
pub const SIDEKICK_FAST: &str = "sidekick_fast";

const SIDEKICK_FAST_CATEGORY: &str = "_sidekick_fast";

/// The engine's own MCP server (`acpira mcp`), whose ask_agent lists the sidekick
pub const ACPIRA_SERVER: &str = "acpira";

/// What the client takes beside plain selects
#[derive(Debug, Clone, Copy, Default)]
pub struct Offers {
  /// Boolean config options (`clientCapabilities.session.configOptions.boolean`): Fast goes out as a switch
  pub booleans: bool,
  /// A sidekick (`clientCapabilities._meta["acpira/sidekick"]`): the `sidekick` select is offered
  pub sidekick: bool,
}

/// The session's sidekick while its model is configured: what the select shows and the model is told of
pub fn sidekick_of(config: &Config, st: &SessionState) -> Option<String> {
  st.sidekick.clone().filter(|p| config.find(p).is_some())
}

/// The sidekick's effort as the select shows it: the level picked while its model offers it, else the model's default.
/// None while there is no sidekick or its model has no levels
fn sidekick_effort_of(config: &Config, catalog: &Catalog, st: &SessionState) -> Option<String> {
  let pick = sidekick_of(config, st)?;
  let efforts = efforts_of(config, catalog, Some(&pick));
  st.sidekick_effort.clone().filter(|e| efforts.contains(e)).or_else(|| default_effort(config, catalog, Some(&pick)))
}

/// The sidekick's Fast as the switch shows it. None while there is no sidekick or its model has no fast tier
fn sidekick_fast_of(config: &Config, st: &SessionState) -> Option<bool> {
  let pick = sidekick_of(config, st)?;
  config.find(&pick).is_some_and(|(p, m)| crate::llm::fast::offered(p, m)).then_some(st.sidekick_fast)
}

/// A Fast switch: a boolean, or a Standard / Fast select for a client that does not take booleans
fn fast_option(id: &str, name: &str, category: &str, on: bool, booleans: bool) -> Value {
  let description = "The provider's faster tier, billed at a higher rate";
  if booleans {
    json!({ "id": id, "name": name, "description": description, "category": category, "type": "boolean", "currentValue": on })
  } else {
    json!({
      "id": id, "name": name, "description": description, "category": category, "type": "select",
      "currentValue": if on { FAST } else { STANDARD },
      "options": [{ "value": STANDARD, "name": "Standard" }, { "value": FAST, "name": "Fast" }],
    })
  }
}

/// The full configOptions set: the model select (every usable model, labelled with its source), the effort select when
/// the current model offers levels, the Fast switch when it has a fast tier (a boolean, or a Standard / Fast select
/// for a client that does not take booleans; the composer shows either as its Fast toggle), and for a client that runs
/// one the sidekick select (none, or any usable model) and, while the sidekick's model has levels, its effort select and, while it has a fast tier, its Fast switch;
/// last the approval level, with `full` marked `full_access`
pub fn config_options(config: &Config, catalog: &Catalog, st: &SessionState, offers: Offers) -> Vec<Value> {
  let mut out = vec![];
  let current = st.model.as_deref().filter(|p| config.find(p).is_some()).map(str::to_owned).or_else(|| config.default_pick());
  if let Some(current) = current {
    let options: Vec<Value> = config
      .providers
      .usable()
      .map(|(p, m)| json!({ "value": acpira_shared::providers::model_pick(&p.id, &m.id), "name": m.display_name(), "description": p.display_name() }))
      .collect();
    let sidekicks = offers.sidekick.then(|| {
      let mut list = vec![json!({ "value": NO_SIDEKICK, "name": "None" })];
      list.extend(options.iter().cloned());
      list
    });
    out.push(json!({ "id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": current, "options": options }));
    let efforts = efforts_of(config, catalog, Some(&current));
    if !efforts.is_empty() {
      let value = st.effort.clone().filter(|e| efforts.contains(e)).or_else(|| default_effort(config, catalog, Some(&current))).unwrap_or_default();
      out.push(json!({
        "id": "effort", "name": "Effort", "category": "thought_level", "type": "select", "currentValue": value,
        "options": efforts.iter().map(|e| json!({ "value": e, "name": title_case(e) })).collect::<Vec<_>>(),
      }));
    }
    if config.find(&current).is_some_and(|(p, m)| crate::llm::fast::offered(p, m)) {
      out.push(fast_option(FAST, "Fast", "model_config", st.fast, offers.booleans));
    }
    if let Some(list) = sidekicks {
      let value = sidekick_of(config, st).unwrap_or_else(|| NO_SIDEKICK.to_owned());
      out.push(json!({
        "id": SIDEKICK, "name": "Sidekick", "category": SIDEKICK_CATEGORY, "type": "select", "currentValue": value, "options": list,
        "description": "A second model this agent can hand well-specified work to, in a conversation of its own",
      }));
      if let Some(value) = sidekick_effort_of(config, catalog, st) {
        let efforts = efforts_of(config, catalog, sidekick_of(config, st).as_deref());
        out.push(json!({
          "id": SIDEKICK_EFFORT, "name": "Sidekick effort", "category": SIDEKICK_EFFORT_CATEGORY, "type": "select", "currentValue": value,
          "options": efforts.iter().map(|e| json!({ "value": e, "name": title_case(e) })).collect::<Vec<_>>(),
        }));
      }
      if let Some(on) = sidekick_fast_of(config, st) {
        out.push(fast_option(SIDEKICK_FAST, "Sidekick fast", SIDEKICK_FAST_CATEGORY, on, offers.booleans));
      }
    }
  }
  out.push(json!({
    "id": APPROVAL, "name": "Approval", "category": APPROVAL_CATEGORY, "type": "select", "currentValue": st.approval.id(),
    "options": Approval::ALL.iter().map(|a| {
      let mut o = json!({ "value": a.id(), "name": a.name() });
      if *a == Approval::Full {
        o["_meta"] = json!({ "kind": "full_access" });
      }
      o
    }).collect::<Vec<_>>(),
  }));
  out
}

/// A history item a request without tools cannot carry: a reply with tool calls, or a tool's result
fn uses_tools(item: &Item) -> bool {
  match item {
    Item::Assistant { tool_calls, .. } => !tool_calls.is_empty(),
    Item::ToolResult { .. } => true,
    Item::User(_) => false,
  }
}

fn title_case(s: &str) -> String {
  let mut c = s.chars();
  c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}
