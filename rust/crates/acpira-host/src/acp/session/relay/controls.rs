//! The CLI's options as a round sets them: the persona's model and effort, and the mode a consult, a work or a
//! bypass round runs in

use serde_json::{Value, json};

use acpira_shared::composer_controls::is_reasoning_select;
use acpira_shared::model_catalog::parse_level;
use acpira_shared::subagents::{RelayMode, SubagentPersona};

use super::{BYPASS_MODES, MODE_KEYS, MODEL_KEYS, READ_ONLY_MODES};
use crate::acp::session::Core;
use crate::acp::transport::process::AgentProcess;

/// The config option whose category or id is one of `keys`
fn find_option<'a>(options: Option<&'a Value>, keys: &[&str]) -> Option<&'a Value> {
  options?.as_array()?.iter().find(|o| {
    let id = o.get("id").and_then(Value::as_str).unwrap_or("");
    let cat = o.get("category").and_then(Value::as_str).unwrap_or("");
    keys.iter().any(|k| *k == id || *k == cat)
  })
}

/// A select's `(value, name)` pairs; grouped selects nest their values one level down
fn option_values(o: &Value) -> Vec<(String, String)> {
  let mut values = vec![];
  for v in o.get("options").and_then(Value::as_array).into_iter().flatten() {
    let inner = v.get("options").and_then(Value::as_array);
    for x in inner.map(|l| l.iter().collect::<Vec<_>>()).unwrap_or_else(|| vec![v]) {
      if let Some(val) = x.get("value").and_then(Value::as_str) {
        values.push((val.to_owned(), x.get("name").and_then(Value::as_str).unwrap_or(val).to_owned()));
      }
    }
  }
  values
}

/// `(config id, chosen value)` for a select whose category or id is one of `keys`, matching `want` by value or name
pub(super) fn pick_option(options: Option<&Value>, keys: &[&str], want: &str) -> Option<(String, String)> {
  let o = find_option(options, keys)?;
  let id = o.get("id").and_then(Value::as_str).unwrap_or("");
  let want = want.trim().to_lowercase();
  let values = option_values(o);
  let exact = values.iter().find(|(v, n)| v.to_lowercase() == want || n.to_lowercase() == want);
  let loose = || values.iter().find(|(v, n)| v.to_lowercase().contains(&want) || n.to_lowercase().contains(&want));
  exact.or_else(loose).map(|(v, _)| (id.to_owned(), v.clone()))
}

/// The CLI's reasoning-effort select, recognised the way the composer recognises it (`is_reasoning_select`)
fn find_effort(options: Option<&Value>) -> Option<&Value> {
  options?.as_array()?.iter().find(|o| {
    is_reasoning_select(o.get("id").and_then(Value::as_str).unwrap_or(""), o.get("category").and_then(Value::as_str))
  })
}

/// `(config id, chosen value)` for the persona's effort: its value or name as stored, else the option on the same level
/// (`Thinking: high` for `high`). Never one whose name merely contains it: a stored `high` on a model offering low /
/// medium / xhigh is not offered, so the CLI's default applies and the round says so, rather than running at xhigh
pub(super) fn pick_effort(options: Option<&Value>, want: &str) -> Option<(String, String)> {
  let o = find_effort(options)?;
  let id = o.get("id").and_then(Value::as_str).unwrap_or("");
  let values = option_values(o);
  let lower = want.trim().to_lowercase();
  let exact = values.iter().find(|(v, n)| v.to_lowercase() == lower || n.to_lowercase() == lower);
  let same_level = || {
    let level = parse_level(want)?;
    values.iter().find(|(v, n)| parse_level(v).or_else(|| parse_level(n)) == Some(level))
  };
  exact.or_else(same_level).map(|(v, _)| (id.to_owned(), v.clone()))
}

/// The display name of a select's current value
pub(super) fn current_name(options: Option<&Value>, keys: &[&str]) -> Option<String> {
  value_name(find_option(options, keys)?)
}

fn value_name(o: &Value) -> Option<String> {
  let cur = o.get("currentValue").and_then(Value::as_str)?;
  Some(option_values(o).into_iter().find(|(v, _)| v == cur).map(|(_, n)| n).unwrap_or_else(|| cur.to_owned()))
}

/// The session's mode ids: the `modes` list, else the values of a `mode` select
pub(super) fn mode_ids(opened: &Value) -> Vec<String> {
  let listed: Vec<String> = opened
    .pointer("/modes/availableModes")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned))
    .collect();
  if !listed.is_empty() {
    return listed;
  }
  find_option(opened.get("configOptions"), &MODE_KEYS).map(|o| option_values(o).into_iter().map(|(v, _)| v).collect()).unwrap_or_default()
}

pub(super) fn current_mode(opened: &Value) -> Option<String> {
  opened
    .pointer("/modes/currentModeId")
    .and_then(Value::as_str)
    .map(str::to_owned)
    .or_else(|| find_option(opened.get("configOptions"), &MODE_KEYS).and_then(|o| o.get("currentValue")).and_then(Value::as_str).map(str::to_owned))
}

fn is_read_only(mode: &str) -> bool {
  READ_ONLY_MODES.contains(&mode)
}

/// The session's full-access mode ids: a known id, or one whose `_meta.kind` is `full_access` (in the `modes` list or
/// among the values of a `mode` select)
/// Whether the CLI is still in a full-access mode although the round asked for less
pub(super) fn stuck_in_bypass(opened: &Value, mode: RelayMode) -> bool {
  mode != RelayMode::Bypass && current_mode(opened).is_some_and(|c| bypass_modes(opened).contains(&c))
}

pub(super) fn bypass_modes(opened: &Value) -> Vec<String> {
  let full = |m: &Value| {
    m.pointer("/_meta/kind").or_else(|| m.pointer("/_meta/jetbrains/air/kind")).and_then(Value::as_str) == Some("full_access")
  };
  let mut marked: Vec<String> = vec![];
  for m in opened.pointer("/modes/availableModes").and_then(Value::as_array).into_iter().flatten() {
    if full(m)
      && let Some(id) = m.get("id").and_then(Value::as_str)
    {
      marked.push(id.to_owned());
    }
  }
  if let Some(o) = find_option(opened.get("configOptions"), &MODE_KEYS) {
    for v in o.get("options").and_then(Value::as_array).into_iter().flatten() {
      for x in v.get("options").and_then(Value::as_array).map(|l| l.iter().collect::<Vec<_>>()).unwrap_or_else(|| vec![v]) {
        if full(x)
          && let Some(val) = x.get("value").and_then(Value::as_str)
        {
          marked.push(val.to_owned());
        }
      }
    }
  }
  mode_ids(opened).into_iter().filter(|m| BYPASS_MODES.contains(&m.as_str()) || marked.contains(m)).collect()
}

/// One `session/set_config_option`; the answer's option list (model-dependent effort values) replaces the known one
async fn set_config(proc: &AgentProcess, sid: &str, opened: &mut Value, id: &str, value: &str) -> std::result::Result<(), String> {
  let r = proc
    .request("session/set_config_option", json!({ "sessionId": sid, "configId": id, "value": value }))
    .await
    .map_err(|e| e.to_string())?;
  if !opened.is_object() {
    *opened = json!({});
  }
  match r.get("configOptions") {
    Some(list) if list.is_array() => opened["configOptions"] = list.clone(),
    _ => {
      if let Some(o) = opened.get_mut("configOptions").and_then(Value::as_array_mut).and_then(|l| l.iter_mut().find(|o| o.get("id").and_then(Value::as_str) == Some(id))) {
        o["currentValue"] = Value::from(value);
      }
    }
  }
  Ok(())
}

/// The mode a round runs in. A persona with a fixed access (consult, work, bypass) runs in it whatever the call asks:
/// the setting is the user's, and a caller asking for a consult must not bring approval cards back to a bypass persona.
/// Only an inherit persona leaves it to the caller: the call's mode overrides it either way (a caller in plan mode may
/// hand out work, one in work mode may ask for bypass), and without one it takes the caller's own access this round
pub(super) fn round_mode(persona: RelayMode, asked: Option<&str>, caller: impl FnOnce() -> RelayMode) -> RelayMode {
  if persona != RelayMode::Inherit {
    return persona;
  }
  match asked {
    Some("consult") => RelayMode::Consult,
    Some("work") => RelayMode::Work,
    Some("bypass") => RelayMode::Bypass,
    _ => caller(),
  }
}

/// The caller's current access, for an inherit persona: a summoned caller's own round mode; the root's mode by id (a
/// read-only one, a full-access one by id or `_meta.kind`, yolo included) — anything else, or no mode at all, is work
pub(super) fn caller_access(c: &Core, caller: Option<&str>) -> RelayMode {
  if let Some(th) = caller {
    return c.tree.relay_latest(th).map(|(_, h, ..)| h.mode).filter(|m| *m != RelayMode::Inherit).unwrap_or(RelayMode::Consult);
  }
  let controls = &c.state.controls;
  if c.perms.auto_approve || crate::acp::session::controls::in_full_access(controls) {
    RelayMode::Bypass
  } else if controls.mode_id.as_deref().is_some_and(is_read_only) {
    RelayMode::Consult
  } else {
    RelayMode::Work
  }
}

/// The model and reasoning effort the session runs, by their option names
pub(super) fn running_controls(opened: &Value) -> (Option<String>, Option<String>) {
  (current_name(opened.get("configOptions"), &MODEL_KEYS), find_effort(opened.get("configOptions")).and_then(value_name))
}

/// The persona's model, then its effort, picked from the options the model switch answered (effort values depend on
/// the model); what the CLI does not offer or refuses becomes a note
pub(super) async fn relay_controls(proc: &AgentProcess, sid: &str, opened: &mut Value, persona: &SubagentPersona, notes: &mut Vec<String>) {
  if let Some(m) = &persona.model {
    match pick_option(opened.get("configOptions"), &MODEL_KEYS, m) {
      Some((id, value)) => {
        if let Err(e) = set_config(proc, sid, opened, &id, &value).await {
          notes.push(format!("model {m:?} was refused ({e}); the CLI's default model runs"));
        }
      }
      None => notes.push(format!("{} offers no model matching {m:?}; its default model runs", persona.agent)),
    }
  }
  if let Some(e) = &persona.effort {
    match pick_effort(opened.get("configOptions"), e) {
      Some((id, value)) => {
        if let Err(err) = set_config(proc, sid, opened, &id, &value).await {
          notes.push(format!("reasoning effort {e:?} was refused ({err})"));
        }
      }
      None => notes.push(format!("{} offers no reasoning effort {e:?} for this model; its default applies", persona.agent)),
    }
  }
}

/// The round's mode: a consult switches to the CLI's read-only mode where it has one; a bypass round to its full-access
/// mode where it has one (without one the host still answers its permission requests); a work round in a read-only or
/// full-access mode switches to the one the session opened in (or the first ordinary one), as does a consult in a
/// full-access mode on a CLI without a read-only one. Answers false when the CLI refused the switch, so the next round
/// tries again instead of taking the mode as set
pub(super) async fn relay_mode(proc: &AgentProcess, sid: &str, opened: &mut Value, start: Option<&str>, mode: RelayMode, agent: &str, notes: &mut Vec<String>) -> bool {
  let ids = mode_ids(opened);
  let bypass = bypass_modes(opened);
  let current = current_mode(opened);
  let ordinary = |m: &str| !is_read_only(m) && !bypass.iter().any(|b| b == m);
  // The mode to leave a read-only or full-access one for
  let back = || start.filter(|s| ordinary(s)).map(str::to_owned).or_else(|| ids.iter().find(|m| ordinary(m)).cloned());
  let in_bypass = current.as_deref().is_some_and(|c| bypass.iter().any(|b| b == c));
  let want = match mode {
    RelayMode::Consult => match READ_ONLY_MODES.iter().find(|r| ids.iter().any(|m| m == *r)) {
      Some(m) => (*m).to_owned(),
      None => {
        notes.push(format!("{agent} has no read-only mode; it follows its own permission settings, so nothing stops a write"));
        match back().filter(|_| in_bypass) {
          Some(m) => m,
          None => return true,
        }
      }
    },
    RelayMode::Bypass => match bypass.first() {
      Some(m) if !in_bypass => m.clone(),
      _ => return true,
    },
    // Resolved before a round starts (`round_mode`); taken as work should one ever get here
    RelayMode::Work | RelayMode::Inherit => match current.as_deref() {
      Some(c) if !ordinary(c) => match back() {
        Some(m) => m,
        None => return true,
      },
      _ => return true,
    },
  };
  if current.as_deref() == Some(want.as_str()) {
    return true;
  }
  let listed = opened.pointer("/modes/availableModes").and_then(Value::as_array).is_some_and(|l| l.iter().any(|m| m.get("id").and_then(Value::as_str) == Some(want.as_str())));
  let r = if listed {
    match proc.request("session/set_mode", json!({ "sessionId": sid, "modeId": want })).await {
      Ok(_) => {
        if let Some(m) = opened.get_mut("modes").filter(|m| m.is_object()) {
          m["currentModeId"] = Value::from(want.as_str());
        }
        Ok(())
      }
      Err(e) => Err(e.to_string()),
    }
  } else {
    let id = find_option(opened.get("configOptions"), &MODE_KEYS).and_then(|o| o.get("id")).and_then(Value::as_str).map(str::to_owned);
    match id {
      Some(id) => set_config(proc, sid, opened, &id, &want).await,
      None => return true,
    }
  };
  match r {
    Ok(()) => true,
    Err(e) => {
      notes.push(format!("switching {agent} to its {want} mode was refused ({e})"));
      false
    }
  }
}
