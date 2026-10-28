//! The session's controls from a session/new / resume / load answer and its config options, and a pick turned back
//! into a session/set_config_option payload

use serde_json::{Map, Value};

use acpira_shared::transcript::*;

use crate::json::{str_of, text_of};

fn kind_of(meta: Option<&Value>) -> Option<String> {
  meta.and_then(|m| text_of(m, "kind").or_else(|| m.pointer("/jetbrains/air").and_then(|a| text_of(a, "kind")))).map(str::to_owned)
}

/// Build the controls from a session/new / resume / load response
pub fn init_controls(controls: &mut SessionControls, modes: Option<&Value>, config_options: Option<&Value>) {
  let modes = modes.filter(|m| !m.is_null());
  controls.modes = modes
    .and_then(|m| m.get("availableModes"))
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .map(|m| SessionOption {
      id: str_of(m, "id").unwrap_or("").to_owned(),
      name: str_of(m, "name").unwrap_or("").to_owned(),
      description: str_of(m, "description").map(str::to_owned),
      kind: kind_of(m.get("_meta")),
      ..Default::default()
    })
    .collect();
  controls.mode_id = modes.and_then(|m| str_of(m, "currentModeId")).map(str::to_owned);
  if let Some(opts) = config_options.and_then(Value::as_array) {
    apply_config_options(controls, opts);
  }
}

const CATEGORY_ORDER: [&str; 3] = ["model", "thought_level", "model_config"];

fn bool_options() -> Vec<SessionOption> {
  vec![
    SessionOption { id: "false".into(), name: "Off".into(), ..Default::default() },
    SessionOption { id: "true".into(), name: "On".into(), ..Default::default() },
  ]
}

/// A category=mode select is treated purely as modes; the remaining selects / booleans are ordered model →
/// thought_level → model_config → others, keeping the agent's order within each class
pub fn apply_config_options(controls: &mut SessionControls, options: &[Value]) {
  let mode = options.iter().find(|o| str_of(o, "type") == Some("select") && str_of(o, "category") == Some("mode"));
  if let Some(mode) = mode {
    let id = str_of(mode, "id").unwrap_or("");
    if controls.modes.is_empty() || controls.mode_config_id.as_deref() == Some(id) {
      controls.modes = flatten_select(mode.get("options"));
      controls.mode_id = str_of(mode, "currentValue").map(str::to_owned);
      controls.mode_config_id = Some(id.to_owned());
    }
  }
  let rank = |c: &ConfigControl| CATEGORY_ORDER.iter().position(|x| Some(*x) == c.category.as_deref()).unwrap_or(CATEGORY_ORDER.len());
  let mut list: Vec<ConfigControl> = options
    .iter()
    .filter(|o| matches!(str_of(o, "type"), Some("select" | "boolean")) && str_of(o, "category") != Some("mode"))
    .map(|o| {
      let category = str_of(o, "category").map(str::to_owned);
      let (id, name) = (str_of(o, "id").unwrap_or("").to_owned(), str_of(o, "name").unwrap_or("").to_owned());
      if str_of(o, "type") == Some("boolean") {
        let value = match o.get("currentValue") {
          Some(Value::Bool(b)) => b.to_string(),
          Some(Value::String(s)) => s.clone(),
          Some(Value::Null) => "null".into(),
          Some(other) => other.to_string(),
          None => "undefined".into(),
        };
        ConfigControl { id, name, category, kind: Some(ControlType::Boolean), options: bool_options(), value: Some(value) }
      } else {
        ConfigControl {
          id,
          name,
          category,
          kind: None,
          options: flatten_select(o.get("options")),
          value: str_of(o, "currentValue").map(str::to_owned),
        }
      }
    })
    .collect();
  list.sort_by_key(|c| rank(c));
  controls.options = list;
}

/// A user pick → the session/set_config_option payload fields; booleans travel as `type: 'boolean'` + a real boolean
pub fn config_option_set_value(control: Option<&ConfigControl>, value: &str) -> Map<String, Value> {
  let mut m = Map::new();
  if control.is_some_and(|c| c.kind == Some(ControlType::Boolean)) {
    m.insert("type".into(), Value::from("boolean"));
    m.insert("value".into(), Value::Bool(value == "true"));
  } else {
    m.insert("value".into(), Value::from(value));
  }
  m
}

fn flatten_select(opts: Option<&Value>) -> Vec<SessionOption> {
  let mut out = vec![];
  for o in opts.and_then(Value::as_array).into_iter().flatten() {
    if o.get("group").is_some() {
      let gname = str_of(o, "name").unwrap_or("").to_owned();
      let gid = o.get("group").and_then(Value::as_str).unwrap_or("").to_owned();
      for x in o.get("options").and_then(Value::as_array).into_iter().flatten() {
        out.push(SessionOption {
          id: str_of(x, "value").unwrap_or("").to_owned(),
          name: str_of(x, "name").unwrap_or("").to_owned(),
          description: Some(str_of(x, "description").map(str::to_owned).unwrap_or_else(|| gname.clone())),
          group: Some(OptionGroup { id: gid.clone(), name: gname.clone() }),
          source: None,
          kind: kind_of(x.get("_meta")),
        });
      }
    } else {
      out.push(SessionOption {
        id: str_of(o, "value").unwrap_or("").to_owned(),
        name: str_of(o, "name").unwrap_or("").to_owned(),
        description: str_of(o, "description").map(str::to_owned),
        group: None,
        source: None,
        kind: kind_of(o.get("_meta")),
      });
    }
  }
  out
}
