//! Turn settings capture and replay (mirror of src/shared/turnSettings.ts)

use crate::model_shapes::{ModelShapes, is_model_control, shape_for};
use crate::transcript::{SessionControls, TurnSettings};

/// Persist wire ids rather than display names, including model effort variants
pub fn capture_turn_settings(controls: &SessionControls) -> TurnSettings {
  TurnSettings {
    mode_id: controls.mode_id.clone(),
    config: controls.options.iter().filter_map(|c| c.value.clone().map(|v| (c.id.clone(), v))).collect(),
  }
}

fn reshape(fallback: SessionControls, settings: &TurnSettings, live: &SessionControls, shapes: Option<&ModelShapes>) -> SessionControls {
  let Some(model) = live.options.iter().find(|c| is_model_control(c)) else { return fallback };
  let Some(value) = settings.config.get(&model.id) else { return fallback };
  if !model.options.iter().any(|o| &o.id == value) {
    return fallback;
  }
  let shape =
    shapes.and_then(|s| s.get(value)).or_else(|| if Some(value) == model.value.as_ref() { None } else { shape_for(shapes, model, value) });
  if let Some(shape) = shape {
    let mut options: Vec<_> = live.options.iter().filter(|c| is_model_control(c)).cloned().collect();
    options.extend(shape.iter().cloned());
    return controls_for_turn(&SessionControls { options, ..live.clone() }, Some(settings));
  }
  if Some(value) == model.value.as_ref() { controls_for_turn(live, Some(settings)) } else { fallback }
}

/// The editor's controls when it opens on a historical turn
pub fn open_turn_controls(live: &SessionControls, settings: Option<&TurnSettings>, shapes: Option<&ModelShapes>) -> SessionControls {
  let fallback = controls_for_turn(live, settings);
  match settings {
    Some(s) => reshape(fallback, s, live, shapes),
    None => fallback,
  }
}

/// Historical selections may disappear after a CLI update: unavailable values keep the current choice
pub fn controls_for_turn(controls: &SessionControls, settings: Option<&TurnSettings>) -> SessionControls {
  let mode_id = match settings.and_then(|s| s.mode_id.as_ref()) {
    Some(m) if controls.modes.iter().any(|x| &x.id == m) => Some(m.clone()),
    _ => controls.mode_id.clone(),
  };
  let options = controls
    .options
    .iter()
    .map(|c| {
      let mut c = c.clone();
      if let Some(v) = settings.and_then(|s| s.config.get(&c.id))
        && c.options.iter().any(|o| &o.id == v)
      {
        c.value = Some(v.clone());
      }
      c
    })
    .collect();
  SessionControls { mode_id, options, ..controls.clone() }
}
