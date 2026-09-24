//! Model-name parsing (mirror of the host-used part of src/shared/models.ts): Devin flattens model × effort × Fast × 1M
//! into flat options, so the structure is recovered from the names

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::json;

use crate::transcript::SessionOption;

pub const FUSION: &str = "Fusion";
static FUSION_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^Fusion\s*\((.+?)\s+\+\s+(.+)\)$").unwrap());

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

pub fn parse_model_name(name: &str) -> ParsedModel {
  if let Some(f) = parse_fusion_name(name) {
    return f;
  }
  let mut t: Vec<&str> = name.split_whitespace().collect();
  if t.is_empty() {
    t.push("");
  }
  let (mut fast, mut long, mut thinking, mut effort) = (false, false, false, String::new());
  loop {
    let last = t.last().map(|s| s.to_lowercase()).unwrap_or_default();
    if t.len() > 1 && last == "fast" {
      fast = true;
      t.pop();
    } else if t.len() > 1 && last == "1m" {
      long = true;
      t.pop();
    } else {
      break;
    }
  }
  if t.len() > 1 && t.last().unwrap().eq_ignore_ascii_case("thinking") {
    thinking = true;
    t.pop();
  }
  let last = t.last().unwrap().to_lowercase();
  if let (true, Some(l)) = (t.len() > 1, level(&last)) {
    effort = l.into();
    t.pop();
  } else if t.len() > 1 && last == "no" && thinking {
    effort = "None".into();
    thinking = false;
    t.pop();
  }
  if effort.is_empty() && thinking {
    effort = "Thinking".into();
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
    let f = map.entry(key.clone()).or_insert_with(|| {
      order.push(key.clone());
      ModelFamily { key: key.clone(), name: p.family.clone(), variants: vec![] }
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
    .map(|k| {
      let mut f = map.remove(&k).unwrap();
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
      f
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_devin_names() {
    let p = parse_model_name("Claude Opus 4.6 High Thinking Fast 1M");
    assert_eq!((p.family.as_str(), p.effort.as_str(), p.fast, p.long), ("Claude Opus 4.6", "High", true, true));
    assert_eq!(parse_model_name("Claude Opus 4.6 Thinking").effort, "Thinking");
    assert_eq!(parse_model_name("GPT No Thinking").effort, "None");
    let f = parse_fusion_name("Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)").unwrap();
    assert_eq!(f.lead.as_deref(), Some("GPT-6 Astra"));
    assert_eq!(f.sidekick.as_deref(), Some("GPT-5.6 Luna High"));
    assert!(f.fast);
  }
}
