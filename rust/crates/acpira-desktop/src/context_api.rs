//! The payload a page in the browser panel hands `window.acpira.context.set` / `add` (docs/context-api.md), checked here:
//! anything a page sends is untrusted input bound for a prompt. Mirror of packages/engine-client/contextApi.ts, whose
//! messages the chat page already understands (`contextSelection`, `addContext`)

use std::collections::{BTreeMap, BTreeSet};

use acpira_shared::attachments::MAX_TEXT_BYTES;
use serde::Serialize;
use serde_json::{Value, json};

pub const CONTEXT_API_VERSION: i64 = 1;
pub const MAX_CONTEXT_ITEMS: usize = 20;
const MAX_LABEL: usize = 120;
const MAX_NAME: usize = 60;
const MAX_URI: usize = 2048;
const MAX_COMMENT: usize = 4000;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextSource {
  pub id: String,
  pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ContextItem {
  pub label: String,
  pub text: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub uri: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContextPayload {
  pub source: ContextSource,
  /// Which instance of the source pushed it: only the same owner can clear a live slot
  pub owner: Option<String>,
  pub items: Vec<ContextItem>,
}

/// The first problem with a payload, as the page's promise rejects with it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPayloadError(pub String);

impl std::fmt::Display for ContextPayloadError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.0)
  }
}

fn fail<T>(why: impl std::fmt::Display) -> Result<T, ContextPayloadError> {
  Err(ContextPayloadError(format!("acpira.context: {why}")))
}

fn source_id_ok(s: &str) -> bool {
  let mut c = s.chars();
  s.chars().count() <= 64
    && matches!(c.next(), Some(x) if x.is_ascii_alphanumeric())
    && c.all(|x| x.is_ascii_alphanumeric() || "_.-".contains(x))
}

/// Labels and names show on one line: control characters and runs of whitespace collapse to one space
fn one_line(s: &str, max: usize) -> String {
  let mut out = String::new();
  let mut gap = false;
  for c in s.chars() {
    if c.is_whitespace() || c.is_control() {
      gap = true;
      continue;
    }
    if gap && !out.is_empty() {
      out.push(' ');
    }
    gap = false;
    out.push(c);
  }
  out.chars().take(max).collect()
}

/// Checks and normalizes a payload; the error names the first problem. `items` may be empty (a clear)
pub fn parse_context_payload(raw: &Value) -> Result<ContextPayload, ContextPayloadError> {
  let Some(p) = raw.as_object() else { return fail("the payload must be an object") };
  if let Some(v) = p.get("version").filter(|v| !v.is_null())
    && v.as_i64() != Some(CONTEXT_API_VERSION)
  {
    return fail(format!("unsupported version {v}, expected {CONTEXT_API_VERSION}"));
  }
  let src = p.get("source").and_then(Value::as_object);
  let Some(id) = src.and_then(|s| s.get("id")).and_then(Value::as_str).filter(|id| source_id_ok(id)) else {
    return fail("source.id must be 1-64 letters, digits, \".\", \"_\" or \"-\"");
  };
  let name = src.and_then(|s| s.get("name")).and_then(Value::as_str).map(|n| one_line(n, MAX_NAME)).unwrap_or_default();
  if name.is_empty() {
    return fail("source.name must be a non-empty string");
  }
  let owner = match p.get("owner") {
    None | Some(Value::Null) => None,
    Some(Value::String(o)) if o.chars().count() <= MAX_URI => Some(o.clone()),
    Some(_) => return fail("owner must be a string"),
  };
  let list = match p.get("items") {
    None | Some(Value::Null) => &vec![],
    Some(Value::Array(a)) => a,
    Some(_) => return fail("items must be an array"),
  };
  if list.len() > MAX_CONTEXT_ITEMS {
    return fail(format!("at most {MAX_CONTEXT_ITEMS} items"));
  }
  let mut bytes = 0;
  let mut items = Vec::with_capacity(list.len());
  for (i, it) in list.iter().enumerate() {
    let Some(o) = it.as_object() else { return fail(format!("items[{i}] must be an object")) };
    let Some(text) = o.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) else {
      return fail(format!("items[{i}].text must be a non-empty string"));
    };
    let label = o.get("label").and_then(Value::as_str).map(|l| one_line(l, MAX_LABEL)).unwrap_or_default();
    if label.is_empty() {
      return fail(format!("items[{i}].label must be a non-empty string"));
    }
    let uri = match o.get("uri") {
      None | Some(Value::Null) => None,
      Some(Value::String(u)) if u.chars().count() <= MAX_URI => Some(u.clone()).filter(|u| !u.is_empty()),
      Some(_) => return fail(format!("items[{i}].uri must be a string")),
    };
    let comment = match o.get("comment") {
      None | Some(Value::Null) => None,
      Some(Value::String(c)) if c.chars().count() <= MAX_COMMENT => Some(c.trim().to_owned()).filter(|c| !c.is_empty()),
      Some(_) => return fail(format!("items[{i}].comment must be a string under {MAX_COMMENT} characters")),
    };
    bytes += text.len();
    if bytes > MAX_TEXT_BYTES {
      return fail(format!("item texts must stay under {} KB in total", MAX_TEXT_BYTES >> 10));
    }
    items.push(ContextItem { label, text: text.to_owned(), uri, comment });
  }
  Ok(ContextPayload { source: ContextSource { id: id.to_owned(), name }, owner, items })
}

/// A page in the browser panel has its owner filed under the panel: a page clears only what a page in the same panel
/// set, never another tool's slot, and the panel can drop them all
pub fn page_owner(panel_id: &str, owner: Option<&str>) -> String {
  format!("browser:{panel_id}:{}", owner.unwrap_or(""))
}

/// The page message that adds picked items to the chat (`addContext`)
pub fn add_context_msg(p: &ContextPayload) -> Value {
  json!({ "type": "addContext", "source": p.source, "items": p.items })
}

fn selection(source: &ContextSource, items: Option<&[ContextItem]>) -> Value {
  match items {
    Some(items) => json!({ "type": "contextSelection", "source": source, "items": items }),
    None => json!({ "type": "contextSelection", "source": source }),
  }
}

struct Slot {
  source: ContextSource,
  owner: Option<String>,
  items: Vec<ContextItem>,
}

/// The live slots, one per source id. Any push with items takes the slot over (the page just clicked into wins); an
/// empty push clears it only when it comes from the owner that filled it
#[derive(Default)]
pub struct ContextSlots {
  slots: BTreeMap<String, Slot>,
}

impl ContextSlots {
  /// The page message for this push, or None when it changes nothing (a stale clear)
  pub fn set(&mut self, p: ContextPayload) -> Option<Value> {
    if p.items.is_empty() {
      let cur = self.slots.get(&p.source.id)?;
      if cur.owner != p.owner {
        return None;
      }
      self.slots.remove(&p.source.id);
      return Some(selection(&p.source, None));
    }
    let msg = selection(&p.source, Some(&p.items));
    self.slots.insert(p.source.id.clone(), Slot { source: p.source, owner: p.owner, items: p.items });
    Some(msg)
  }

  /// Clears every slot owned by one of these
  pub fn release(&mut self, owners: &BTreeSet<String>) -> Vec<Value> {
    let gone: Vec<String> =
      self.slots.iter().filter(|(_, s)| s.owner.as_ref().is_some_and(|o| owners.contains(o))).map(|(id, _)| id.clone()).collect();
    gone.into_iter().filter_map(|id| self.slots.remove(&id)).map(|s| selection(&s.source, None)).collect()
  }

  /// Clears every slot a page in this browser panel filled (page_owner): the page it showed is gone
  pub fn release_page(&mut self, panel_id: &str) -> Vec<Value> {
    let prefix = page_owner(panel_id, None);
    let owners: BTreeSet<String> = self.owners().into_iter().filter(|o| o.starts_with(&prefix)).collect();
    self.release(&owners)
  }

  pub fn owners(&self) -> BTreeSet<String> {
    self.slots.values().filter_map(|s| s.owner.clone()).collect()
  }

  /// What a page that (re)initializes needs to show the current slots
  pub fn replay(&self) -> Vec<Value> {
    self.slots.values().map(|s| selection(&s.source, Some(&s.items))).collect()
  }

  /// The messages that take every slot off the pages (sharing turned off); the slots stay for when it comes back
  pub fn hidden(&self) -> Vec<Value> {
    self.slots.values().map(|s| selection(&s.source, None)).collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn payload(owner: Option<&str>, items: Value) -> Value {
    json!({ "source": { "id": "canvas", "name": "Canvas" }, "owner": owner, "items": items })
  }

  #[test]
  fn normalizes_a_good_payload_and_names_the_first_problem() {
    let p = parse_context_payload(&json!({ "version": 1, "source": { "id": "my.tool", "name": " My\n\tTool " },
      "items": [{ "label": "a\u{0007}b   c", "text": "x", "uri": "", "comment": "  note " }] }))
    .unwrap();
    assert_eq!(p.source, ContextSource { id: "my.tool".into(), name: "My Tool".into() });
    assert_eq!(p.items, [ContextItem { label: "a b c".into(), text: "x".into(), uri: None, comment: Some("note".into()) }]);
    let err = |v: Value| parse_context_payload(&v).unwrap_err().0;
    assert_eq!(err(json!(1)), "acpira.context: the payload must be an object");
    assert_eq!(err(json!({ "version": 2, "source": { "id": "a", "name": "A" } })), "acpira.context: unsupported version 2, expected 1");
    assert!(err(json!({ "source": { "id": "-a", "name": "A" } })).contains("source.id"));
    assert!(err(json!({ "source": { "id": "a", "name": "  " } })).contains("source.name"));
    assert!(err(json!({ "source": { "id": "a", "name": "A" }, "items": [{ "label": "l", "text": " " }] })).contains("items[0].text"));
    let many: Vec<Value> = (0..21).map(|_| json!({ "label": "l", "text": "t" })).collect();
    assert!(err(json!({ "source": { "id": "a", "name": "A" }, "items": many })).contains("at most 20 items"));
    let big = "x".repeat(MAX_TEXT_BYTES / 2 + 1);
    assert!(
      err(json!({ "source": { "id": "a", "name": "A" }, "items": [{ "label": "l", "text": big }, { "label": "m", "text": big }] }))
        .contains("KB in total")
    );
  }

  #[test]
  fn a_slot_clears_only_for_its_owner_and_a_panel_drops_its_pages_slots() {
    let mut s = ContextSlots::default();
    let a = page_owner("P1", Some("doc"));
    let set = s.set(parse_context_payload(&payload(Some(&a), json!([{ "label": "l", "text": "t" }]))).unwrap()).unwrap();
    assert_eq!(set["items"][0]["text"], "t");
    // Another owner's clear is stale
    assert_eq!(s.set(parse_context_payload(&payload(Some("other"), json!([]))).unwrap()), None);
    assert_eq!(s.replay().len(), 1);
    assert_eq!(s.release_page("P2"), Vec::<Value>::new());
    let gone = s.release_page("P1");
    assert_eq!(gone, [json!({ "type": "contextSelection", "source": { "id": "canvas", "name": "Canvas" } })]);
    assert!(s.replay().is_empty());
  }
}
