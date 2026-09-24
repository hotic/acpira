//! Per-agent memory of the dependent controls each model came with (mirror of src/shared/modelShapes.ts)

use std::collections::BTreeMap;

use crate::models::group_models;
use crate::transcript::{ConfigControl, SessionControls};

pub type ModelShapes = BTreeMap<String, Vec<ConfigControl>>;

pub fn is_model_control(c: &ConfigControl) -> bool {
  c.category.as_deref() == Some("model") || (c.category.is_none() && c.id == "model")
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
