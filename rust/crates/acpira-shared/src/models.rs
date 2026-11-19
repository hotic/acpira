//! Model-name parsing (mirror of the host-used part of packages/shared/models.ts): Devin flattens model × effort × Fast × 1M
//! into flat options, so the structure is recovered from the names

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::json;

use crate::transcript::SessionOption;

pub const FUSION: &str = "Fusion";
static FUSION_NAME: LazyLock<Regex> = LazyLock::new(|| crate::re::compile(r"(?i)^Fusion\s*\((.+?)\s+\+\s+(.+)\)$"));

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedModel {
  pub family: String,
  pub effort: String,
  pub fast: bool,
  pub long: bool,
  pub lead: Option<String>,
  pub sidekick: Option<String>,
}

pub fn parse_fusion_name(name: &str) -> Option<ParsedModel> {
  let m = FUSION_NAME.captures(name.trim())?;
  let lead = parse_model_name(&m[1]);
  let sk = parse_model_name(&m[2]);
  let sidekick = [sk.family.as_str(), sk.effort.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
  Some(ParsedModel {
    family: FUSION.into(),
    effort: lead.effort,
    fast: lead.fast || sk.fast,
    long: lead.long,
    lead: Some(lead.family),
    sidekick: Some(sidekick),
  })
}

/// The sidekick half of a Fusion pair's name, as (model family, effort label; empty when the name carries none)
pub fn fusion_sidekick(name: &str) -> Option<(String, String)> {
  let m = FUSION_NAME.captures(name.trim())?;
  let sk = parse_model_name(&m[2]);
  Some((sk.family, sk.effort))
}

fn level(word: &str) -> Option<&'static str> {
  Some(match word {
    "none" => "None",
    "minimal" => "Minimal",
    "low" => "Low",
    "medium" => "Medium",
    "high" => "High",
    "xhigh" | "x-high" => "XHigh",
    "max" => "Max",
    _ => return None,
  })
}

const EFFORT_ORDER: [&str; 9] = ["", "Thinking", "None", "Minimal", "Low", "Medium", "High", "XHigh", "Max"];

fn parse_effort_tokens<'a>(tokens: &[&'a str]) -> (Vec<&'a str>, String) {
  let mut w = tokens.to_vec();
  let mut thinking = false;
  let mut effort = String::new();
  if w.last().is_some_and(|s| s.eq_ignore_ascii_case("thinking")) {
    thinking = true;
    w.pop();
  }
  let last = w.last().map(|s| s.to_lowercase()).unwrap_or_default();
  if last == "high" && w.len() >= 2 && w[w.len() - 2].eq_ignore_ascii_case("extra") {
    effort = "XHigh".into();
    w.pop();
    w.pop();
  } else if let Some(l) = level(&last) {
    effort = l.into();
    w.pop();
  } else if last == "no" && thinking {
    effort = "None".into();
    thinking = false;
    w.pop();
  }
  if effort.is_empty() && thinking {
    effort = "Thinking".into();
  }
  (w, effort)
}

fn strip_trailing_flags(t: &mut Vec<&str>, fast: &mut bool, long: &mut bool) {
  loop {
    let last = t.last().map(|s| s.to_lowercase()).unwrap_or_default();
    if t.len() > 1 && last == "fast" {
      *fast = true;
      t.pop();
    } else if t.len() > 2 && last == "mode" && t[t.len() - 2].eq_ignore_ascii_case("fast") {
      // Droid spells it "Opus 5.5 Fast Mode"
      *fast = true;
      t.truncate(t.len() - 2);
    } else if t.len() > 1 && last == "1m" {
      *long = true;
      t.pop();
    } else {
      break;
    }
  }
}

pub fn parse_model_name(name: &str) -> ParsedModel {
  if let Some(f) = parse_fusion_name(name) {
    return f;
  }
  let mut t: Vec<&str> = name.split_whitespace().collect();
  if t.is_empty() {
    t.push("");
  }
  let (mut fast, mut long, mut effort) = (false, false, String::new());
  strip_trailing_flags(&mut t, &mut fast, &mut long);
  if t.len() > 1 && t.last().is_some_and(|s| s.ends_with(')')) {
    let mut open = None;
    for i in (1..t.len()).rev() {
      if t[i].starts_with('(') {
        open = Some(i);
        break;
      }
    }
    if let Some(idx) = open {
      let mut group: Vec<&str> = t[idx..].to_vec();
      if let Some(first) = group.first_mut() {
        *first = first.strip_prefix('(').unwrap_or(first);
      }
      if let Some(last) = group.last_mut() {
        *last = last.strip_suffix(')').unwrap_or(last);
      }
      let inner: Vec<&str> = group.into_iter().filter(|s| !s.is_empty()).collect();
      if !inner.is_empty() && inner.iter().all(|w| !w.contains('(') && !w.contains(')')) {
        let (rest, parsed_effort) = parse_effort_tokens(&inner);
        if !parsed_effort.is_empty() && rest.is_empty() {
          effort = parsed_effort;
          t.truncate(idx);
          strip_trailing_flags(&mut t, &mut fast, &mut long);
        }
      }
    }
  }
  if effort.is_empty() && t.len() > 1 {
    let (rest, parsed_effort) = parse_effort_tokens(&t[1..]);
    if !parsed_effort.is_empty() {
      effort = parsed_effort;
      t.truncate(1);
      t.extend(rest);
      strip_trailing_flags(&mut t, &mut fast, &mut long);
    }
  }
  ParsedModel { family: t.join(" "), effort, fast, long, lead: None, sidekick: None }
}

#[derive(Debug, Clone)]
pub struct ModelVariant {
  pub id: String,
  pub name: String,
  pub effort: String,
  pub fast: bool,
  pub long: bool,
  pub lead: Option<String>,
  pub sidekick: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelFamily {
  pub key: String,
  pub name: String,
  pub source: Option<String>,
  pub variants: Vec<ModelVariant>,
}

fn rank(e: &str) -> usize {
  EFFORT_ORDER.iter().position(|x| *x == e).unwrap_or(EFFORT_ORDER.len())
}

/// Families keyed exactly like the TS groupModels (the key is what hiddenOptions stores); only the fields the host reads
pub fn group_models(options: &[SessionOption]) -> Vec<ModelFamily> {
  let ns = |o: &SessionOption| o.source.as_ref().map(|s| s.id.clone()).or_else(|| o.group.as_ref().map(|g| g.id.clone()));
  let mut tuples = HashSet::new();
  let mut ambiguous = HashSet::new();
  for o in options {
    let p = parse_model_name(&o.name);
    let base = json!([ns(o), p.family]).to_string();
    let tuple = json!([base, p.effort, p.fast, p.long, p.lead, p.sidekick]).to_string();
    if !tuples.insert(tuple) {
      ambiguous.insert(base);
    }
  }
  let mut order: Vec<String> = Vec::new();
  let mut map: HashMap<String, ModelFamily> = HashMap::new();
  let mut fusion: HashMap<String, (Vec<String>, Vec<String>)> = HashMap::new();
  for o in options {
    let p = parse_model_name(&o.name);
    let namespace = ns(o);
    let separate = ambiguous.contains(&json!([namespace, p.family]).to_string());
    let key = if separate {
      json!([namespace, p.family, o.id]).to_string()
    } else if let Some(n) = &namespace {
      json!([n, p.family]).to_string()
    } else {
      p.family.clone()
    };
    let source = o
      .source
      .as_ref()
      .map(|s| s.name.clone())
      .or_else(|| o.group.as_ref().map(|g| g.name.clone()))
      .or_else(|| separate.then(|| o.id.clone()));
    let f = map.entry(key.clone()).or_insert_with(|| {
      order.push(key.clone());
      ModelFamily { key: key.clone(), name: p.family.clone(), source, variants: vec![] }
    });
    if let Some(lead) = &p.lead {
      let fu = fusion.entry(key.clone()).or_default();
      if !fu.0.contains(lead) {
        fu.0.push(lead.clone());
      }
      if let Some(sk) = &p.sidekick
        && !fu.1.contains(sk)
      {
        fu.1.push(sk.clone());
      }
    }
    f.variants.push(ModelVariant {
      id: o.id.clone(),
      name: o.name.clone(),
      effort: p.effort,
      fast: p.fast,
      long: p.long,
      lead: p.lead.clone(),
      sidekick: p.lead.as_ref().and(p.sidekick),
    });
  }
  order
    .into_iter()
    .filter_map(|k| {
      let mut f = map.remove(&k)?;
      let fu = fusion.get(&k);
      let idx = |list: Option<&Vec<String>>, v: &Option<String>| -> i64 {
        match list {
          Some(l) => l.iter().position(|x| Some(x) == v.as_ref()).map(|i| i as i64).unwrap_or(-1),
          None => 0,
        }
      };
      f.variants.sort_by(|a, b| {
        idx(fu.map(|x| &x.0), &a.lead)
          .cmp(&idx(fu.map(|x| &x.0), &b.lead))
          .then(rank(&a.effort).cmp(&rank(&b.effort)))
          .then(idx(fu.map(|x| &x.1), &a.sidekick).cmp(&idx(fu.map(|x| &x.1), &b.sidekick)))
          .then(a.fast.cmp(&b.fast))
          .then(a.long.cmp(&b.long))
      });
      Some(f)
    })
    .collect()
}

/// Preferences stored before the family had a source: its bare name, or the `Provider/Name` Pi and OpenCode used to show
/// (mirror of `packages/shared/models.ts` `nameLegacyKeys`).
fn name_legacy_keys(f: &ModelFamily, name: &str) -> Vec<String> {
  let mut keys = match &f.source {
    Some(src) => vec![name.to_owned(), format!("{src}/{name}")],
    None => vec![name.to_owned()],
  };
  if f.key.starts_with('[')
    && let Ok(serde_json::Value::Array(parts)) = serde_json::from_str::<serde_json::Value>(&f.key)
    && parts.len() == 2
    && let Some(ns) = parts[0].as_str()
  {
    keys.push(json!([ns, name]).to_string());
  }
  keys
}

/// Whether `f` is hidden by `hidden` (mirror of `packages/shared/models.ts` `familyHidden`).
#[must_use]
pub fn family_hidden(f: &ModelFamily, hidden: &[String]) -> bool {
  let has = |k: &str| hidden.iter().any(|h| h == k);
  has(&f.key)
    || name_legacy_keys(f, &f.name).iter().any(|k| has(k))
    || (f.variants.len() > 1 && f.variants.iter().all(|v| name_legacy_keys(f, &v.name).iter().any(|k| has(k))))
}

/// Drop the options whose family is hidden; the current value always stays reachable, and a list that would hide
/// everything shows everything (mirror of `packages/shared/models.ts` `visibleOptions`).
#[must_use]
pub fn visible_options(options: &[SessionOption], hidden: Option<&[String]>, current: Option<&str>) -> Vec<SessionOption> {
  let Some(hidden) = hidden.filter(|h| !h.is_empty()) else {
    return options.to_vec();
  };
  let hidden_ids: HashSet<String> = group_models(options)
    .into_iter()
    .filter(|f| family_hidden(f, hidden))
    .flat_map(|f| f.variants.into_iter().map(|v| v.id))
    .collect();
  let kept: Vec<SessionOption> = options.iter().filter(|o| current == Some(o.id.as_str()) || !hidden_ids.contains(&o.id)).cloned().collect();
  if kept.is_empty() { options.to_vec() } else { kept }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::transcript::{OptionSource, SourceKind};

  fn opt(id: &str, name: &str) -> SessionOption {
    SessionOption { id: id.into(), name: name.into(), ..Default::default() }
  }

  #[test]
  fn visible_options_hides_families_keeps_current_and_never_empties() {
    let options = vec![
      opt("opus-5-5", "Opus 5.5"),
      opt("opus-5-5-fast", "Opus 5.5 Fast Mode"),
      opt("opus-5", "Opus 5"),
      opt("opus-5-fast", "Opus 5 Fast Mode"),
    ];
    let hidden = vec!["Opus 5".to_owned()];
    let shown: Vec<String> = visible_options(&options, Some(&hidden), None).into_iter().map(|o| o.id).collect();
    assert_eq!(shown, ["opus-5-5", "opus-5-5-fast"]);
    // Current stays reachable even when its family is hidden
    let with_current: Vec<String> = visible_options(&options, Some(&hidden), Some("opus-5-fast")).into_iter().map(|o| o.id).collect();
    assert_eq!(with_current, ["opus-5-5", "opus-5-5-fast", "opus-5-fast"]);
    // Hiding every family falls back to showing all
    let all_hidden = vec!["Opus 5.5".to_owned(), "Opus 5".to_owned()];
    assert_eq!(visible_options(&options, Some(&all_hidden), None).len(), options.len());
  }

  #[test]
  fn visible_options_matches_source_qualified_and_legacy_keys() {
    let mut a = opt("or/opus", "Opus 5.5");
    a.source = Some(OptionSource { id: "openrouter".into(), name: "OpenRouter".into(), kind: SourceKind::Custom });
    let mut b = opt("ds/opus", "Opus 5.5");
    b.source = Some(OptionSource { id: "deepseek".into(), name: "DeepSeek".into(), kind: SourceKind::Custom });
    let options = vec![a, b];
    // Source-qualified key hides only that source
    let hidden = vec![r#"["openrouter","Opus 5.5"]"#.to_owned()];
    let shown: Vec<String> = visible_options(&options, Some(&hidden), None).into_iter().map(|o| o.id).collect();
    assert_eq!(shown, ["ds/opus"]);
    // Legacy `Source/Name` key matches too
    let legacy = vec!["DeepSeek/Opus 5.5".to_owned()];
    let shown: Vec<String> = visible_options(&options, Some(&legacy), None).into_iter().map(|o| o.id).collect();
    assert_eq!(shown, ["or/opus"]);
  }

  #[test]
  fn parses_devin_and_parenthesized_names() {
    let p = parse_model_name("Claude Opus 4.6 High Thinking Fast 1M");
    assert_eq!((p.family.as_str(), p.effort.as_str(), p.fast, p.long), ("Claude Opus 4.6", "High", true, true));
    assert_eq!(parse_model_name("Claude Opus 4.6 Thinking").effort, "Thinking");
    assert_eq!(parse_model_name("GPT No Thinking").effort, "None");
    assert_eq!(parse_model_name("Gemini 4 Argon (Medium)"), ParsedModel { family: "Gemini 4 Argon".into(), effort: "Medium".into(), ..Default::default() });
    assert_eq!(parse_model_name("Gemini 3.8 Flash (High)"), ParsedModel { family: "Gemini 3.8 Flash".into(), effort: "High".into(), ..Default::default() });
    assert_eq!(parse_model_name("Gemini 3.1 Pro (Low thinking)"), ParsedModel { family: "Gemini 3.1 Pro".into(), effort: "Low".into(), ..Default::default() });
    assert_eq!(parse_model_name("[OAuth] GPT-5.1-Codex-Max (Extra High)"), ParsedModel { family: "[OAuth] GPT-5.1-Codex-Max".into(), effort: "XHigh".into(), ..Default::default() });
    assert_eq!(parse_model_name("Claude Haiku 4.5 (latest)"), ParsedModel { family: "Claude Haiku 4.5 (latest)".into(), ..Default::default() });
    let f = parse_fusion_name("Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)").unwrap();
    assert_eq!(f.lead.as_deref(), Some("GPT-6 Astra"));
    assert_eq!(f.sidekick.as_deref(), Some("GPT-5.6 Luna High"));
    assert!(f.fast);
    assert_eq!(fusion_sidekick("Fusion (Claude Opus 5.5 Medium + SWE-2 Medium)"), Some(("SWE-2".into(), "Medium".into())));
    assert_eq!(fusion_sidekick("Fusion (Claude Opus 5.5 + SWE-2)"), Some(("SWE-2".into(), String::new())));
    assert_eq!(fusion_sidekick("SWE-2 High"), None);
  }
}
