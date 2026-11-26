//! The steps of an `act` request, as a model writes them: objects (`{"do":"click","on":"b1"}`) or short arrays
//! (`["click","b1"]`).

use serde_json::{Value as Json, json};

pub const MAX_WAIT_STEP: u64 = 5000;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
  Left,
  Right,
}

#[derive(Debug, PartialEq)]
pub enum Place {
  Ref(String),
  /// A point in the window, measured in the last picture of it.
  At(f64, f64),
}

#[derive(Debug, PartialEq)]
pub enum Step {
  Click { place: Place, button: Button, count: u8 },
  Type { on: Option<String>, text: String },
  Set { on: String, value: String },
  Key { keys: String, repeat: u16 },
  Scroll { place: Option<Place>, down: i32, right: i32 },
  Menu { path: String },
  Wait { ms: u64 },
}

/// A step in its short array form (`["click","b1"]`) as the object form.
fn from_array(parts: &[Json]) -> Json {
  let text = |index: usize| parts.get(index).cloned().unwrap_or(Json::Null);
  let verb = parts.first().and_then(Json::as_str).unwrap_or("");
  match verb {
    "type" if parts.len() == 2 => json!({"do": verb, "text": text(1)}),
    "type" => json!({"do": verb, "on": text(1), "text": text(2)}),
    "set" => json!({"do": verb, "on": text(1), "value": text(2)}),
    "key" => json!({"do": verb, "keys": text(1), "repeat": text(2)}),
    "menu" => json!({"do": verb, "path": text(1)}),
    "wait" => json!({"do": verb, "ms": text(1)}),
    "scroll" => json!({"do": verb, "on": text(1), "down": text(2)}),
    _ => json!({"do": verb, "on": text(1)}),
  }
}

/// A whole number that may arrive as a number or as its text.
pub fn integer(value: &Json) -> Option<i64> {
  value.as_i64().or_else(|| value.as_f64().map(|number| number as i64)).or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn parse_place(step: &Json) -> Result<Option<Place>, String> {
  if let Some(reference) = step["on"].as_str().filter(|reference| !reference.is_empty()) {
    return Ok(Some(Place::Ref(reference.to_string())));
  }
  match step["at"].as_array().map(Vec::as_slice) {
    Some([x, y]) => match (x.as_f64(), y.as_f64()) {
      (Some(x), Some(y)) => Ok(Some(Place::At(x, y))),
      _ => Err("at takes two numbers: [x, y]".into()),
    },
    Some(_) => Err("at takes two numbers: [x, y]".into()),
    None => Ok(None),
  }
}

fn parse_step(step: &Json) -> Result<Step, String> {
  let converted;
  let step = match step.as_array() {
    Some(parts) => {
      converted = from_array(parts);
      &converted
    }
    None => step,
  };
  let verb = step["do"].as_str().or(step["action"].as_str()).unwrap_or("");
  let text = |key: &str| step[key].as_str().map(str::to_string);
  match verb {
    "click" | "double_click" | "right_click" => {
      let place = parse_place(step)?.ok_or_else(|| format!("{verb} needs `on` (a ref) or `at` ([x, y] in the last picture)"))?;
      let button = if verb == "right_click" || step["button"].as_str() == Some("right") { Button::Right } else { Button::Left };
      let count = if verb == "double_click" { 2 } else { integer(&step["count"]).unwrap_or(1).clamp(1, 3) as u8 };
      Ok(Step::Click { place, button, count })
    }
    "type" => Ok(Step::Type { on: text("on").filter(|on| !on.is_empty()), text: text("text").ok_or("type needs `text`")? }),
    "set" => Ok(Step::Set { on: text("on").ok_or("set needs `on` (a ref)")?, value: text("value").or(text("text")).ok_or("set needs `value`")? }),
    "key" => Ok(Step::Key { keys: text("keys").or(text("key")).ok_or("key needs `keys`, such as \"enter\" or a shortcut")?, repeat: integer(&step["repeat"]).unwrap_or(1).clamp(1, 100) as u16 }),
    "scroll" => {
      let amount = |forward: &str, back: &str| (integer(&step[forward]).unwrap_or(0) - integer(&step[back]).unwrap_or(0)).clamp(-200, 200) as i32;
      let (down, right) = (amount("down", "up"), amount("right", "left"));
      if down == 0 && right == 0 {
        return Err("scroll needs `down`, `up`, `left` or `right` (lines)".into());
      }
      Ok(Step::Scroll { place: parse_place(step)?, down, right })
    }
    "menu" => Ok(Step::Menu { path: text("path").ok_or("menu needs `path`, such as \"File > Save\"")? }),
    "wait" => Ok(Step::Wait { ms: integer(&step["ms"]).unwrap_or(500).clamp(0, MAX_WAIT_STEP as i64) as u64 }),
    "" => Err("a step needs `do`: click, double_click, right_click, type, set, key, scroll, menu or wait".into()),
    other => Err(format!("unknown step \"{other}\": use click, double_click, right_click, type, set, key, scroll, menu or wait")),
  }
}

pub fn parse_steps(steps: &Json) -> Result<Vec<Step>, String> {
  let steps = steps.as_array().ok_or("steps is required: a list of steps")?;
  if steps.is_empty() {
    return Err("steps is empty".into());
  }
  steps.iter().enumerate().map(|(index, step)| parse_step(step).map_err(|error| format!("step {}: {error}", index + 1))).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn steps_parse_from_objects() {
    assert_eq!(parse_step(&json!({"do": "click", "on": "b1"})), Ok(Step::Click { place: Place::Ref("b1".into()), button: Button::Left, count: 1 }));
    assert_eq!(parse_step(&json!({"do": "double_click", "at": [10, 20.5]})), Ok(Step::Click { place: Place::At(10.0, 20.5), button: Button::Left, count: 2 }));
    assert_eq!(parse_step(&json!({"do": "click", "on": "e2", "button": "right"})), Ok(Step::Click { place: Place::Ref("e2".into()), button: Button::Right, count: 1 }));
    assert_eq!(parse_step(&json!({"do": "type", "text": "hi"})), Ok(Step::Type { on: None, text: "hi".into() }));
    assert_eq!(parse_step(&json!({"do": "set", "on": "t1", "value": ""})), Ok(Step::Set { on: "t1".into(), value: String::new() }));
    assert_eq!(parse_step(&json!({"do": "key", "keys": "cmd+s"})), Ok(Step::Key { keys: "cmd+s".into(), repeat: 1 }));
    assert_eq!(parse_step(&json!({"do": "scroll", "on": "e9", "up": 3})), Ok(Step::Scroll { place: Some(Place::Ref("e9".into())), down: -3, right: 0 }));
    assert_eq!(parse_step(&json!({"do": "menu", "path": "File > Save"})), Ok(Step::Menu { path: "File > Save".into() }));
    assert_eq!(parse_step(&json!({"do": "wait", "ms": 999_999})), Ok(Step::Wait { ms: MAX_WAIT_STEP }));
  }

  #[test]
  fn steps_parse_from_arrays() {
    assert_eq!(parse_step(&json!(["click", "b1"])), Ok(Step::Click { place: Place::Ref("b1".into()), button: Button::Left, count: 1 }));
    assert_eq!(parse_step(&json!(["type", "t1", "hello"])), Ok(Step::Type { on: Some("t1".into()), text: "hello".into() }));
    assert_eq!(parse_step(&json!(["type", "hello"])), Ok(Step::Type { on: None, text: "hello".into() }));
    assert_eq!(parse_step(&json!(["key", "enter", "3"])), Ok(Step::Key { keys: "enter".into(), repeat: 3 }));
    assert_eq!(parse_step(&json!(["scroll", "e1", "5"])), Ok(Step::Scroll { place: Some(Place::Ref("e1".into())), down: 5, right: 0 }));
  }

  #[test]
  fn bad_steps_say_what_is_missing() {
    assert!(parse_step(&json!({"do": "click"})).is_err_and(|error| error.contains("`on`")));
    assert!(parse_step(&json!({"do": "click", "at": [1]})).is_err_and(|error| error.contains("two numbers")));
    assert!(parse_step(&json!({"do": "scroll", "on": "e1"})).is_err_and(|error| error.contains("down")));
    assert!(parse_step(&json!({"do": "drag"})).is_err_and(|error| error.contains("unknown step")));
    assert!(parse_steps(&json!([])).is_err());
    assert!(parse_steps(&json!([{"do": "click", "on": "b1"}, {"do": "type"}])).is_err_and(|error| error.starts_with("step 2:")));
  }
}
