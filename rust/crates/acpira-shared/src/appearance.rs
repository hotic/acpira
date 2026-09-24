//! Appearance axes (mirror of src/shared/appearance.ts): one-to-one with acpira.appearance.* settings

use serde_json::{Map, Value};

/// (axis key, allowed values in option order); the order is the digit order of the combo code
pub const AXES: [(&str, &[&str]); 15] = [
  ("density", &["cozy", "compact", "airy"]),
  ("radius", &["12", "8", "16"]),
  ("surface", &["hairline", "tonal", "stroke"]),
  ("font", &["system", "inter", "geist"]),
  ("userMessage", &["bubble", "block", "plain"]),
  ("toolLine", &["text", "icon", "rich"]),
  ("thought", &["text", "shimmer", "orb"]),
  ("sessions", &["dropdown", "drawer"]),
  ("composer", &["island", "flush"]),
  ("beam", &["line", "none", "pulse", "full"]),
  ("beamColor", &["mono", "ocean", "colorful"]),
  ("send", &["accent", "icon", "metal"]),
  ("accent", &["brand", "agent", "vscode"]),
  ("motion", &["on", "none"]),
  ("fold", &["codex", "cursor"]),
];

/// Baseline combo code 201110200302100 decoded
const BASE: &str = "201110200302100";

pub type Appearance = Map<String, Value>;

pub fn base_appearance() -> Appearance {
  AXES.iter().zip(BASE.bytes()).map(|((k, opts), d)| ((*k).to_owned(), Value::from(opts[(d - b'0') as usize]))).collect()
}

pub fn is_axis(key: &str) -> bool {
  AXES.iter().any(|(k, _)| *k == key)
}

/// Builds an Appearance from setting values; invalid values fall back to the baseline
pub fn appearance_from_settings(get: impl Fn(&str) -> Option<Value>) -> Appearance {
  let mut out = base_appearance();
  for (k, opts) in AXES {
    if let Some(Value::String(v)) = get(k)
      && opts.contains(&v.as_str())
    {
      out.insert(k.to_owned(), Value::String(v));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  #[test]
  fn baseline() {
    let a = super::base_appearance();
    assert_eq!(a["density"], "airy");
    assert_eq!(a["thought"], "orb");
    assert_eq!(a["beam"], "full");
    assert_eq!(a["send"], "metal");
    assert_eq!(a["accent"], "agent");
    assert_eq!(a["fold"], "codex");
  }
}
