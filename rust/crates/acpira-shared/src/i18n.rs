//! Dictionaries and translation (mirror of src/shared/i18n): the JSON is exported from the TS dictionaries, the single source

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
  #[serde(rename = "zh-CN")]
  ZhCn,
  #[serde(rename = "en")]
  En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
  #[serde(rename = "auto")]
  Auto,
  #[serde(rename = "zh-CN")]
  ZhCn,
  #[serde(rename = "en")]
  En,
}

impl Language {
  pub fn parse(s: &str) -> Option<Language> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
  }
}

static EN: LazyLock<HashMap<String, String>> = LazyLock::new(|| serde_json::from_str(include_str!("../i18n/en.json")).expect("en.json"));
static ZH: LazyLock<HashMap<String, String>> =
  LazyLock::new(|| serde_json::from_str(include_str!("../i18n/zh-CN.json")).expect("zh-CN.json"));

/// Whether the key exists in the source dictionary
pub fn has_key(key: &str) -> bool {
  EN.contains_key(key)
}

/// The locale's string, falling back to en and then to the key; `{name}` placeholders are filled from params
pub fn translate(locale: Locale, key: &str, params: &[(&str, &str)]) -> String {
  let dict = match locale {
    Locale::ZhCn => &*ZH,
    Locale::En => &*EN,
  };
  let s = dict.get(key).or_else(|| EN.get(key)).map(String::as_str).unwrap_or(key);
  if params.is_empty() {
    return s.to_owned();
  }
  fill(s, params)
}

fn fill(s: &str, params: &[(&str, &str)]) -> String {
  let mut out = String::with_capacity(s.len() + 16);
  let mut rest = s;
  while let Some(open) = rest.find('{') {
    out.push_str(&rest[..open]);
    let tail = &rest[open + 1..];
    let close = tail.find('}');
    let name = close.map(|c| &tail[..c]).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    match (name, close) {
      (Some(n), Some(c)) => {
        match params.iter().find(|(k, _)| *k == n) {
          Some((_, v)) => out.push_str(v),
          None => {
            out.push('{');
            out.push_str(n);
            out.push('}');
          }
        }
        rest = &tail[c + 1..];
      }
      _ => {
        out.push('{');
        rest = tail;
      }
    }
  }
  out.push_str(rest);
  out
}

/// Maps the setting plus the host's display language onto a shipped locale
pub fn resolve_locale(language: Option<Language>, host_language: &str) -> Locale {
  match language {
    Some(Language::ZhCn) => Locale::ZhCn,
    Some(Language::En) => Locale::En,
    _ => {
      if host_language.to_lowercase().starts_with("zh") {
        Locale::ZhCn
      } else {
        Locale::En
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn fills_and_falls_back() {
    assert_eq!(fill("a {x} {y} {", &[("x", "1")]), "a 1 {y} {");
    assert_eq!(translate(Locale::En, "no.such.key", &[]), "no.such.key");
    assert!(has_key("session.untitled"));
  }
}
