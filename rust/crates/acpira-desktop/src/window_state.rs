//! Where the chat windows were and what they showed, so the next launch reopens them (<app data>/windows.json). Windows
//! open at quit are restored as they were; a window closed by hand leaves the list, and the last one closed lends its
//! place to the next new window

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Bounds {
  pub x: f64,
  pub y: f64,
  pub width: f64,
  pub height: f64,
}

/// Saved bounds as they can still show: all four, or only the size for the OS to place
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placement {
  At(Bounds),
  Size { width: f64, height: f64 },
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowEntry {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub bounds: Option<Bounds>,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub maximized: bool,
  /// The session it showed
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowsState {
  pub windows: Vec<WindowEntry>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last: Option<WindowEntry>,
}

fn num(v: Option<&Value>) -> Option<f64> {
  v?.as_f64().filter(|n| n.is_finite())
}

fn entry_of(raw: &Value) -> Option<WindowEntry> {
  let r = raw.as_object()?;
  let bounds = r.get("bounds").and_then(Value::as_object).and_then(|b| {
    let (x, y, width, height) = (num(b.get("x"))?, num(b.get("y"))?, num(b.get("width"))?, num(b.get("height"))?);
    (width > 0.0 && height > 0.0).then_some(Bounds { x, y, width, height })
  });
  Some(WindowEntry {
    bounds,
    maximized: r.get("maximized") == Some(&Value::Bool(true)),
    session: r.get("session").and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned),
  })
}

/// The file's content, shape-checked: anything unreadable is left out rather than failing the launch
pub fn parse_windows_state(raw: Option<&Value>) -> WindowsState {
  let r = raw.and_then(Value::as_object);
  let windows =
    r.and_then(|r| r.get("windows")).and_then(Value::as_array).map(|a| a.iter().filter_map(entry_of).collect()).unwrap_or_default();
  let last = r.and_then(|r| r.get("last")).and_then(entry_of);
  WindowsState { windows, last }
}

/// Saved bounds where they can still show: kept when the middle of the window's top edge lies on a display's work area
/// (a monitor unplugged since leaves it off screen), else only the size, capped to the largest work area
pub fn placeable(b: Bounds, areas: &[Bounds]) -> Placement {
  let mx = b.x + b.width / 2.0;
  if areas.iter().any(|a| mx >= a.x && mx < a.x + a.width && b.y >= a.y && b.y < a.y + a.height) {
    return Placement::At(b);
  }
  let widest = areas.iter().fold(None::<&Bounds>, |m, a| match m {
    Some(m) if m.width * m.height >= a.width * a.height => Some(m),
    _ => Some(a),
  });
  match widest {
    Some(w) => Placement::Size { width: b.width.min(w.width), height: b.height.min(w.height) },
    None => Placement::Size { width: b.width, height: b.height },
  }
}

pub struct WindowStateStore {
  pub file: PathBuf,
}

impl WindowStateStore {
  pub fn new(file: impl Into<PathBuf>) -> WindowStateStore {
    WindowStateStore { file: file.into() }
  }

  pub fn read(&self) -> WindowsState {
    parse_windows_state(crate::json_file::read_json(&self.file).as_ref())
  }

  pub fn write(&self, state: &WindowsState) {
    if let Ok(v) = serde_json::to_value(state) {
      crate::json_file::write_json(&self.file, &v);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  // Ported from the Electron app's windowState.test.ts
  #[test]
  fn keeps_what_parses_and_drops_the_rest() {
    let raw = json!({
      "windows": [
        { "bounds": { "x": 10, "y": 20, "width": 800, "height": 600 }, "maximized": true, "session": "s1" },
        { "bounds": { "x": 0, "y": 0, "width": 0, "height": 600 }, "session": "" },
        "junk",
      ],
      "last": { "bounds": { "x": "a" } },
    });
    let s = parse_windows_state(Some(&raw));
    assert_eq!(
      serde_json::to_value(&s).unwrap(),
      json!({
        "windows": [{ "bounds": { "x": 10.0, "y": 20.0, "width": 800.0, "height": 600.0 }, "maximized": true, "session": "s1" }, {}],
        "last": {},
      })
    );
    assert_eq!(parse_windows_state(None), WindowsState::default());
  }

  const MAIN: Bounds = Bounds { x: 0.0, y: 0.0, width: 1440.0, height: 875.0 };
  const SIDE: Bounds = Bounds { x: 1440.0, y: -200.0, width: 1920.0, height: 1055.0 };

  #[test]
  fn keeps_bounds_whose_title_bar_lies_on_a_display() {
    let b = Bounds { x: 1500.0, y: 0.0, width: 1000.0, height: 700.0 };
    assert_eq!(placeable(b, &[MAIN, SIDE]), Placement::At(b));
  }

  #[test]
  fn keeps_only_the_size_capped_to_the_largest_display_when_the_old_place_is_off_screen() {
    assert_eq!(
      placeable(Bounds { x: 1500.0, y: 0.0, width: 2400.0, height: 1400.0 }, &[MAIN]),
      Placement::Size { width: 1440.0, height: 875.0 }
    );
    assert_eq!(
      placeable(Bounds { x: -3000.0, y: 100.0, width: 900.0, height: 700.0 }, &[MAIN, SIDE]),
      Placement::Size { width: 900.0, height: 700.0 }
    );
    assert_eq!(placeable(Bounds { x: 0.0, y: 0.0, width: 900.0, height: 700.0 }, &[]), Placement::Size { width: 900.0, height: 700.0 });
  }
}
