//! Reasoning-control helpers the host needs (mirror of the host-used part of src/shared/composerControls.ts)

use std::sync::LazyLock;

use regex::Regex;

use crate::transcript::{ConfigControl, SessionOption};

static REASONING_IDS: LazyLock<Regex> =
  LazyLock::new(|| Regex::new(r"^(reasoning_effort|thought_level|thinking|thinking_level)$").unwrap());
static STRIP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:reasoning|thinking|effort|level)").unwrap());

pub(crate) fn compact(value: &str) -> String {
  STRIP
    .replace_all(&value.to_lowercase(), "")
    .chars()
    .filter(|c| !matches!(c, ' ' | '\t' | '\n' | '\r' | '_' | '-') && !c.is_whitespace())
    .collect()
}

pub fn is_reasoning_control(control: &ConfigControl) -> bool {
  control.category.as_deref() == Some("thought_level") || (control.category.is_none() && REASONING_IDS.is_match(&control.id))
}

#[derive(PartialEq)]
enum Side {
  On,
  Off,
}

fn toggle_side(option: &SessionOption) -> Option<Side> {
  let id = compact(&option.id);
  let key = if id.is_empty() { compact(&option.name) } else { id };
  match key.as_str() {
    "on" => Some(Side::On),
    "off" => Some(Side::Off),
    _ => None,
  }
}

fn native_thought_options(options: &[SessionOption]) -> Vec<&SessionOption> {
  let all: Vec<&SessionOption> = options.iter().collect();
  let efforts: Vec<&SessionOption> = options.iter().filter(|o| toggle_side(o).is_none()).collect();
  let ons: Vec<&SessionOption> = options.iter().filter(|o| toggle_side(o) == Some(Side::On)).collect();
  if options.iter().any(|o| toggle_side(o) == Some(Side::Off)) {
    return all;
  }
  if !ons.is_empty() && efforts.len() > 1 {
    return efforts;
  }
  if !ons.is_empty() && efforts.len() == 1 {
    return ons;
  }
  all
}

fn thought_value(native: &[&SessionOption]) -> Option<String> {
  let efforts: Vec<&&SessionOption> = native.iter().filter(|o| toggle_side(o).is_none()).collect();
  efforts
    .iter()
    .find(|o| o.id == "high")
    .or(efforts.first())
    .map(|o| o.id.clone())
    .or_else(|| native.iter().find(|o| toggle_side(o) == Some(Side::On)).map(|o| o.id.clone()))
    .or_else(|| native.first().map(|o| o.id.clone()))
}

/// After a model switch: a wire value the new model actually offers
pub fn thought_correction(control: &ConfigControl) -> Option<String> {
  if !is_reasoning_control(control) {
    return None;
  }
  let native = native_thought_options(&control.options);
  let value = control.value.as_ref()?;
  if native.iter().any(|o| &o.id == value) {
    return None;
  }
  thought_value(&native)
}
