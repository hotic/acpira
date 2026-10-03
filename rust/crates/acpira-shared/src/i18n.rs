//! Dictionaries and translation (mirror of src/shared/i18n): the JSON is exported from the TS dictionaries, the single source

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// Shipped locales, in the order of `LOCALES` in src/shared/i18n/index.ts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Locale {
  #[serde(rename = "zh-CN")]
  ZhCn,
  #[serde(rename = "zh-TW")]
  ZhTw,
  #[serde(rename = "en")]
  En,
  #[serde(rename = "ja")]
  Ja,
  #[serde(rename = "ko")]
  Ko,
  #[serde(rename = "es")]
  Es,
  #[serde(rename = "de")]
  De,
  #[serde(rename = "fr")]
  Fr,
  #[serde(rename = "ru")]
  Ru,
}

impl Locale {
  pub const ALL: [Locale; 9] =
    [Locale::ZhCn, Locale::ZhTw, Locale::En, Locale::Ja, Locale::Ko, Locale::Es, Locale::De, Locale::Fr, Locale::Ru];

  /// The BCP 47 tag, also the dictionary file name and the wire value
  pub fn tag(self) -> &'static str {
    match self {
      Locale::ZhCn => "zh-CN",
      Locale::ZhTw => "zh-TW",
      Locale::En => "en",
      Locale::Ja => "ja",
      Locale::Ko => "ko",
      Locale::Es => "es",
      Locale::De => "de",
      Locale::Fr => "fr",
      Locale::Ru => "ru",
    }
  }

  pub fn from_tag(tag: &str) -> Option<Locale> {
    Locale::ALL.into_iter().find(|l| l.tag() == tag)
  }
}

/// The acpira.language setting: `auto` follows the host's display language, anything else pins a locale
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
  Auto,
  Pinned(Locale),
}

impl Language {
  pub fn parse(s: &str) -> Option<Language> {
    if s == "auto" { Some(Language::Auto) } else { Locale::from_tag(s).map(Language::Pinned) }
  }

  pub fn tag(self) -> &'static str {
    match self {
      Language::Auto => "auto",
      Language::Pinned(l) => l.tag(),
    }
  }
}

impl Serialize for Language {
  fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(self.tag())
  }
}

impl<'de> Deserialize<'de> for Language {
  fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
    let s = String::deserialize(d)?;
    Language::parse(&s).ok_or_else(|| serde::de::Error::custom(format!("unknown language {s:?}")))
  }
}

/// Every value the acpira.language setting accepts
pub const LANGUAGE_TAGS: [&str; 10] = ["auto", "zh-CN", "zh-TW", "en", "ja", "ko", "es", "de", "fr", "ru"];

type Dict = HashMap<String, String>;

fn parse_dict(locale: Locale, json: &str) -> Dict {
  serde_json::from_str(json).unwrap_or_else(|e| panic!("{}.json: {e}", locale.tag()))
}

static DICTS: LazyLock<[Dict; 9]> = LazyLock::new(|| {
  [
    parse_dict(Locale::ZhCn, include_str!("../i18n/zh-CN.json")),
    parse_dict(Locale::ZhTw, include_str!("../i18n/zh-TW.json")),
    parse_dict(Locale::En, include_str!("../i18n/en.json")),
    parse_dict(Locale::Ja, include_str!("../i18n/ja.json")),
    parse_dict(Locale::Ko, include_str!("../i18n/ko.json")),
    parse_dict(Locale::Es, include_str!("../i18n/es.json")),
    parse_dict(Locale::De, include_str!("../i18n/de.json")),
    parse_dict(Locale::Fr, include_str!("../i18n/fr.json")),
    parse_dict(Locale::Ru, include_str!("../i18n/ru.json")),
  ]
});

fn dict(locale: Locale) -> &'static Dict {
  &DICTS[locale as usize]
}

/// Whether the key exists in the source dictionary
pub fn has_key(key: &str) -> bool {
  dict(Locale::En).contains_key(key)
}

/// CLDR cardinal category for a count, mirroring `pluralCategory` in src/shared/i18n/index.ts;
/// None means the plain key ("other")
pub fn plural_category(locale: Locale, n: u64) -> Option<&'static str> {
  match locale {
    Locale::Ru => {
      let (d, h) = (n % 10, n % 100);
      Some(if d == 1 && h != 11 {
        "one"
      } else if (2..=4).contains(&d) && !(12..=14).contains(&h) {
        "few"
      } else {
        "many"
      })
    }
    Locale::Fr => (n < 2).then_some("one"),
    Locale::En | Locale::Es | Locale::De => (n == 1).then_some("one"),
    _ => None,
  }
}

/// The locale's string (its plural variant first when the params carry a count `n` / `count`), falling back to en
/// and then to the key; `{name}` placeholders are filled from params
pub fn translate(locale: Locale, key: &str, params: &[(&str, &str)]) -> String {
  let d = dict(locale);
  let count = params.iter().find(|(k, _)| *k == "n").or_else(|| params.iter().find(|(k, _)| *k == "count"));
  let variant = count
    .and_then(|(_, v)| v.parse::<u64>().ok())
    .and_then(|n| plural_category(locale, n))
    .and_then(|c| d.get(&format!("{key}#{c}")));
  let s = variant.or_else(|| d.get(key)).or_else(|| dict(Locale::En).get(key)).map(String::as_str).unwrap_or(key);
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

/// Maps the setting plus the host's display language (e.g. "zh-cn", "zh-Hant-TW", "ja-JP") onto a shipped locale:
/// Chinese splits by script / region, others by language, unknown ones fall back to en
pub fn resolve_locale(language: Option<Language>, host_language: &str) -> Locale {
  if let Some(Language::Pinned(l)) = language {
    return l;
  }
  let tag = host_language.to_lowercase().replace('_', "-");
  let mut parts = tag.split('-');
  let lang = parts.next().unwrap_or("");
  if lang == "zh" {
    return if parts.any(|p| matches!(p, "hant" | "tw" | "hk" | "mo")) { Locale::ZhTw } else { Locale::ZhCn };
  }
  Locale::ALL.into_iter().find(|l| l.tag() == lang).unwrap_or(Locale::En)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn fills_and_falls_back() {
    assert_eq!(fill("a {x} {y} {", &[("x", "1")]), "a 1 {y} {");
    assert_eq!(translate(Locale::En, "no.such.key", &[]), "no.such.key");
    assert_eq!(translate(Locale::Ja, "settings.tab.mcp", &[]), "MCP");
    assert!(has_key("session.untitled"));
  }

  #[test]
  fn every_dictionary_loads() {
    for l in Locale::ALL {
      assert!(!dict(l).is_empty(), "{}", l.tag());
    }
  }

  #[test]
  fn picks_plural_variants() {
    assert_eq!(translate(Locale::Ru, "host.images", &[("n", "1")]), "1 изображение");
    assert_eq!(translate(Locale::Ru, "host.images", &[("n", "3")]), "3 изображения");
    assert_eq!(translate(Locale::Ru, "host.images", &[("n", "5")]), "5 изображений");
    assert_eq!(translate(Locale::Es, "host.images", &[("n", "1")]), "1 imagen");
    assert_eq!(translate(Locale::En, "host.images", &[("n", "1")]), "1 images");
  }

  #[test]
  fn plural_categories_match_the_ts_table() {
    let ru: Vec<_> = [0, 1, 2, 4, 5, 11, 12, 14, 21, 22, 25, 101, 111, 112].map(|n| plural_category(Locale::Ru, n)).into();
    let want = ["many", "one", "few", "few", "many", "many", "many", "many", "one", "few", "many", "one", "many", "many"];
    assert_eq!(ru, want.map(Some).to_vec());
    assert_eq!([0, 1, 2].map(|n| plural_category(Locale::Fr, n)), [Some("one"), Some("one"), None]);
    assert_eq!([0, 1, 2].map(|n| plural_category(Locale::De, n)), [None, Some("one"), None]);
    assert_eq!(plural_category(Locale::Ja, 1), None);
  }

  #[test]
  fn resolves_host_languages() {
    let auto = |h: &str| resolve_locale(Some(Language::Auto), h);
    assert_eq!(auto("zh-cn"), Locale::ZhCn);
    assert_eq!(auto("zh-Hant-TW"), Locale::ZhTw);
    assert_eq!(auto("zh-HK"), Locale::ZhTw);
    assert_eq!(auto("ja-JP"), Locale::Ja);
    assert_eq!(auto("fr_CA"), Locale::Fr);
    assert_eq!(auto("pt-br"), Locale::En);
    assert_eq!(resolve_locale(Language::parse("ru"), "zh-cn"), Locale::Ru);
    assert_eq!(resolve_locale(None, ""), Locale::En);
  }

  #[test]
  fn language_round_trips_on_the_wire() {
    for tag in LANGUAGE_TAGS {
      let lang = Language::parse(tag).unwrap();
      assert_eq!(serde_json::to_value(lang).unwrap(), serde_json::Value::from(tag));
      assert_eq!(serde_json::from_value::<Language>(serde_json::Value::from(tag)).unwrap(), lang);
    }
    assert_eq!(Language::parse("pt"), None);
  }
}
