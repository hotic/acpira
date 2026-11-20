//! Per-agent memory of the dependent controls each model came with (mirror of packages/shared/modelShapes.ts)

use std::collections::BTreeMap;

use crate::composer_controls::{is_model_config_control, is_reasoning_control};
use crate::models::group_models;
use crate::transcript::{ConfigControl, SessionControls, SessionModel};

pub type ModelShapes = BTreeMap<String, Vec<ConfigControl>>;

/// What the user last ran each of an agent's models with (mirror of `ModelMemory` in packages/shared/modelShapes.ts):
/// `recent` holds model option ids, newest first; `picks` the values of the model's own parameters per model option id
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelMemory {
  #[serde(default)]
  pub recent: Vec<String>,
  #[serde(default)]
  pub picks: BTreeMap<String, BTreeMap<String, String>>,
}

/// How many models a memory keeps; a model that falls off the end takes its picks along
const MEMORY_CAP: usize = 64;

impl ModelMemory {
  /// The session's model becomes the newest, with its parameters as they stand now. False: no model control, or nothing
  /// changed
  pub fn note(&mut self, controls: &SessionControls) -> bool {
    let Some(model) = controls.options.iter().find(|c| is_model_control(c)).and_then(|c| c.value.clone()) else { return false };
    let picks = model_params(controls);
    if self.recent.first() == Some(&model) && self.picks.get(&model) == Some(&picks) {
      return false;
    }
    self.recent.retain(|m| m != &model);
    self.recent.insert(0, model.clone());
    self.picks.insert(model, picks);
    self.trim();
    true
  }

  /// Takes in what `older` knows and this does not: its models go behind this one's, and a model both hold keeps the
  /// picks made here
  pub fn absorb(&mut self, older: &ModelMemory) {
    for model in &older.recent {
      if !self.recent.contains(model) {
        self.recent.push(model.clone());
        if let Some(picks) = older.picks.get(model) {
          self.picks.insert(model.clone(), picks.clone());
        }
      }
    }
    self.trim();
  }

  fn trim(&mut self) {
    self.recent.truncate(MEMORY_CAP);
    self.picks.retain(|model, _| self.recent.contains(model));
  }
}

/// The values of the controls that belong to the model: its reasoning level and its `model_config` / `speed` parameters. The
/// approval level, the sidekick selects and modes are the session's, never a model's
pub fn model_params(controls: &SessionControls) -> BTreeMap<String, String> {
  controls
    .options
    .iter()
    .filter(|c| is_reasoning_control(c) || is_model_config_control(c))
    .filter_map(|c| Some((c.id.clone(), c.value.clone()?)))
    .collect()
}

pub fn is_model_control(c: &ConfigControl) -> bool {
  c.category.as_deref() == Some("model") || (c.category.is_none() && c.id == "model")
}

// A model id without its trailing `[…]` hint (`fable[1m]` → `fable`)
fn bare(id: &str) -> &str {
  id.strip_suffix(']').and_then(|s| s.rsplit_once('[')).map(|(b, _)| b).filter(|b| !b.is_empty()).unwrap_or(id)
}

/// The offered option a model id picked on another native session stands for on this one. An agent can spell one model
/// differently per session (Claude lists Fable 5.2 as `fable` on a resumed session and `fable[1m]` on a fresh one), so
/// an id the select lacks falls back to the only option that differs by its `[…]` hint; none when that is ambiguous
pub fn offered_model<'a>(model: &'a ConfigControl, value: &str) -> Option<&'a str> {
  if let Some(o) = model.options.iter().find(|o| o.id == value) {
    return Some(&o.id);
  }
  let mut same = model.options.iter().filter(|o| bare(&o.id) == bare(value));
  let first = same.next()?;
  same.next().is_none().then_some(first.id.as_str())
}

/// The current model for the session list: its value and its option's name (the value itself when no option matches)
pub fn session_model(controls: &SessionControls) -> Option<SessionModel> {
  let model = controls.options.iter().find(|c| is_model_control(c))?;
  let value = model.value.as_deref()?;
  let name = model.options.iter().find(|o| o.id == value).map_or(value, |o| o.name.as_str());
  Some(SessionModel { id: value.to_owned(), name: name.to_owned() })
}

// Structure only: a value change is not a new shape
fn structure(controls: &[ConfigControl]) -> Vec<(&str, Option<&str>, bool, Vec<&str>)> {
  controls
    .iter()
    .map(|c| (c.id.as_str(), c.category.as_deref(), c.kind.is_some(), c.options.iter().map(|o| o.id.as_str()).collect()))
    .collect()
}

/// The map with the current model's shape recorded, or None when there is no model or its shape is already known
pub fn learn_shape(shapes: Option<&ModelShapes>, controls: &SessionControls) -> Option<ModelShapes> {
  let model = controls.options.iter().find(|c| is_model_control(c))?;
  let value = model.value.as_ref()?;
  let shape: Vec<ConfigControl> = controls.options.iter().filter(|c| !is_model_control(c)).cloned().collect();
  if let Some(known) = shapes.and_then(|s| s.get(value))
    && structure(known) == structure(&shape)
  {
    return None;
  }
  let mut next = shapes.cloned().unwrap_or_default();
  next.insert(value.clone(), shape);
  Some(next)
}

/// A model's remembered shape; an unseen id borrows one from its family
pub fn shape_for<'a>(shapes: Option<&'a ModelShapes>, model: &ConfigControl, value: &str) -> Option<&'a Vec<ConfigControl>> {
  let shapes = shapes?;
  if let Some(s) = shapes.get(value) {
    return Some(s);
  }
  let families = group_models(&model.options);
  let family = families.iter().find(|f| f.variants.iter().any(|v| v.id == value))?;
  let sibling = family.variants.iter().find(|v| shapes.contains_key(&v.id))?;
  shapes.get(&sibling.id)
}
