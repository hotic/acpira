//! A session's history: `<home>/agent/sessions/<id>.jsonl`, append-only, one event per line, beside the session's own
//! folder (`outputs/`, `plan.md`). Nothing is rewritten: what the model is sent is a view projected from the events.
//!
//! Events (each with `t`, milliseconds since the epoch):
//! - `session` — first line: id, cwd, format version
//! - `item` — one conversation entry as the model sees it (the view's source)
//! - `update` — a `session/update` as sent to the client, for `session/load`'s replay; streamed text and terminal
//!   output are merged per block and tool call, so a long session stays a reasonable size
//! - `state` — mode, model, effort and approval after a change
//! - `prompt` — the system prompt whenever it was (re)composed
//! - `request` — one per model call: model and source, prompt variant and version, policies, usage with cache reads and
//!   writes, duration, stop reason or error
//! - `view` — what changed in the request prefix between two calls (prompt variant, tool set, model), so a cache miss
//!   can be traced afterwards
//!
//! A line cut short by a crash is skipped when the file is read back

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::llm::{Item, Native, Part, ToolCall};

pub const FORMAT: u32 = 1;

pub fn now_ms() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct Store {
  path: PathBuf,
  /// Written as the first line when the file is created
  header: Value,
  inner: parking_lot::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
  file: Option<File>,
  /// Written after the header when the file is created
  prelude: Vec<Value>,
  /// The last `update`, held back while the next one may still merge into it
  pending: Option<Value>,
  failed: bool,
}

impl Store {
  pub fn new(path: PathBuf, id: &str, cwd: &Path) -> Store {
    Store { path, header: json!({ "type": "session", "id": id, "cwd": cwd, "format": FORMAT }), inner: Default::default() }
  }

  pub fn path(&self) -> &Path {
    &self.path
  }

  /// Append one event. The file is created at the first event, so a session never prompted leaves nothing behind; a
  /// write error is logged once and the session goes on unrecorded
  pub fn append(&self, event: Value) {
    let mut inner = self.inner.lock();
    if let Some(p) = inner.pending.take() {
      Self::write(&mut inner, &self.path, &self.header, update_event(p));
    }
    Self::write(&mut inner, &self.path, &self.header, event);
  }

  fn write(inner: &mut Inner, path: &Path, header: &Value, mut event: Value) {
    if inner.failed {
      return;
    }
    if inner.file.is_none() {
      let opened = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| OpenOptions::new().create(true).append(true).open(path));
      match opened {
        Ok(mut f) => {
          // An existing file (a resumed session) already has its header
          if f.metadata().map(|m| m.len() == 0).unwrap_or(false) {
            let t = now_ms();
            let mut h = header.clone();
            h["t"] = t.into();
            let _ = writeln!(f, "{h}");
            for mut e in std::mem::take(&mut inner.prelude) {
              e["t"] = t.into();
              let _ = writeln!(f, "{e}");
            }
          }
          inner.file = Some(f);
        }
        Err(e) => {
          eprintln!("[acpira agent] cannot write {}: {e}", path.display());
          inner.failed = true;
          return;
        }
      }
    }
    if event.get("t").is_none() {
      event["t"] = now_ms().into();
    }
    if let Some(f) = inner.file.as_mut()
      && let Err(e) = writeln!(f, "{event}")
    {
      eprintln!("[acpira agent] cannot write {}: {e}", path.display());
      inner.failed = true;
    }
  }

  /// An event that belongs at the start of the file but is not worth creating it for (a new session's prompt)
  pub fn prelude(&self, event: Value) {
    let mut inner = self.inner.lock();
    if inner.file.is_some() {
      drop(inner);
      self.append(event);
    } else {
      inner.prelude.push(event);
    }
  }

  /// Record a `session/update` for the replay, merging it into the one before when it continues it
  pub fn record_update(&self, update: &Value) {
    let mut inner = self.inner.lock();
    let merged = match inner.pending.as_mut() {
      Some(prev) => merge(prev, update),
      None => false,
    };
    if !merged && let Some(prev) = inner.pending.replace(update.clone()) {
      Self::write(&mut inner, &self.path, &self.header, update_event(prev));
    }
  }

  /// Write a held update (end of a turn)
  pub fn flush(&self) {
    let mut inner = self.inner.lock();
    if let Some(p) = inner.pending.take() {
      Self::write(&mut inner, &self.path, &self.header, update_event(p));
    }
  }

  pub fn item(&self, item: &Item) {
    self.append(json!({ "type": "item", "item": item_json(item) }));
  }
}

fn update_event(update: Value) -> Value {
  json!({ "type": "update", "update": update })
}

/// Fold `next` into `prev` when it only continues it: more text of the same chunk kind, or more of the same tool call.
/// Tool call updates replace fields, except terminal output, which accumulates
fn merge(prev: &mut Value, next: &Value) -> bool {
  let kind = |v: &Value| v.get("sessionUpdate").and_then(Value::as_str).map(str::to_owned);
  let (Some(a), Some(b)) = (kind(prev), kind(next)) else { return false };
  match (a.as_str(), b.as_str()) {
    // A prompt's blocks stay apart: the replay shows them as sent
    ("agent_message_chunk" | "agent_thought_chunk", _) if a == b => {
      let (Some(p), Some(n)) = (prev.pointer("/content/text").and_then(Value::as_str), next.pointer("/content/text").and_then(Value::as_str)) else {
        return false;
      };
      let joined = format!("{p}{n}");
      prev["content"]["text"] = Value::String(joined);
      true
    }
    ("tool_call" | "tool_call_update", "tool_call_update") if prev.get("toolCallId") == next.get("toolCallId") => {
      let delta = next.pointer("/_meta/terminal_output_delta/data").and_then(Value::as_str);
      for (k, v) in next.as_object().into_iter().flatten() {
        if k == "sessionUpdate" || k == "_meta" {
          continue;
        }
        prev[k] = v.clone();
      }
      if let Some(d) = delta {
        let before = prev.pointer("/_meta/terminal_output_delta/data").and_then(Value::as_str).unwrap_or("").to_owned();
        prev["_meta"]["terminal_output_delta"] = json!({ "data": before + d });
      }
      for (k, v) in next.get("_meta").and_then(Value::as_object).into_iter().flatten() {
        if k != "terminal_output_delta" {
          prev["_meta"][k] = v.clone();
        }
      }
      true
    }
    _ => false,
  }
}

pub fn item_json(item: &Item) -> Value {
  match item {
    Item::User(parts) => json!({
      "role": "user",
      "parts": parts.iter().map(|p| match p {
        Part::Text(t) => json!({ "text": t }),
        Part::Image { mime, data } => json!({ "image": data, "mime": mime }),
      }).collect::<Vec<_>>(),
    }),
    Item::Assistant { text, reasoning, tool_calls, native } => {
      let mut v = json!({
        "role": "assistant", "text": text, "reasoning": reasoning,
        "calls": tool_calls.iter().map(|c| json!({ "id": c.id, "name": c.name, "arguments": c.arguments })).collect::<Vec<_>>(),
      });
      if let Some(n) = native {
        v["native"] = json!({ "origin": n.origin, "blocks": n.blocks });
      }
      v
    }
    Item::ToolResult { call_id, name, content, is_error } => json!({ "role": "tool", "id": call_id, "name": name, "content": content, "error": is_error }),
  }
}

pub fn item_from(v: &Value) -> Option<Item> {
  let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
  Some(match v.get("role")?.as_str()? {
    "user" => Item::User(
      v.get("parts")?
        .as_array()?
        .iter()
        .filter_map(|p| match (p.get("text").and_then(Value::as_str), p.get("image").and_then(Value::as_str)) {
          (Some(t), _) => Some(Part::Text(t.to_owned())),
          (_, Some(data)) => Some(Part::Image { mime: p.get("mime").and_then(Value::as_str).unwrap_or("image/png").to_owned(), data: data.to_owned() }),
          _ => None,
        })
        .collect(),
    ),
    "assistant" => Item::Assistant {
      text: s("text"),
      reasoning: s("reasoning"),
      tool_calls: v
        .get("calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
          let f = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
          ToolCall { id: f("id"), name: f("name"), arguments: f("arguments") }
        })
        .collect(),
      native: v.get("native").and_then(|n| {
        Some(Native { origin: n.get("origin")?.as_str()?.to_owned(), blocks: n.get("blocks")?.as_array()?.clone() })
      }),
    },
    "tool" => Item::ToolResult { call_id: s("id"), name: s("name"), content: s("content"), is_error: v.get("error").and_then(Value::as_bool).unwrap_or(false) },
    _ => return None,
  })
}

/// A session read back from its file
#[derive(Debug, Default)]
pub struct Loaded {
  pub id: String,
  pub cwd: PathBuf,
  pub items: Vec<Item>,
  /// The latest `state` event's fields
  pub state: Map<String, Value>,
  /// The latest `prompt` event
  pub prompt: Option<Value>,
  pub updates: Vec<Value>,
  /// The first user message's text
  pub title: Option<String>,
  pub updated_ms: u64,
}

/// Read a session file; `only_summary` skips the history for listings
pub fn load(path: &Path, only_summary: bool) -> std::io::Result<Loaded> {
  let file = File::open(path)?;
  let mut out = Loaded { updated_ms: file.metadata()?.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0), ..Default::default() };
  for line in BufReader::new(file).lines() {
    let Ok(line) = line else { break };
    let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
    match ev.get("type").and_then(Value::as_str) {
      Some("session") => {
        out.id = ev.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        out.cwd = PathBuf::from(ev.get("cwd").and_then(Value::as_str).unwrap_or(""));
      }
      Some("item") => {
        let Some(item) = ev.get("item").and_then(item_from) else { continue };
        if out.title.is_none()
          && let Item::User(parts) = &item
        {
          out.title = parts.iter().find_map(|p| match p {
            Part::Text(t) if !t.trim().is_empty() => Some(title_of(t)),
            _ => None,
          });
        }
        if only_summary {
          if out.title.is_some() {
            break;
          }
          continue;
        }
        out.items.push(item);
      }
      _ if only_summary => {}
      Some("update") => out.updates.extend(ev.get("update").cloned()),
      Some("state") => {
        for (k, v) in ev.as_object().into_iter().flatten() {
          if k != "type" && k != "t" {
            out.state.insert(k.clone(), v.clone());
          }
        }
      }
      Some("prompt") => out.prompt = Some(ev),
      _ => {}
    }
  }
  Ok(out)
}

/// A list title: the first line of the first message, without a mode note the agent put in front
fn title_of(text: &str) -> String {
  let text = match text.strip_prefix("<mode>") {
    Some(rest) => rest.split_once("</mode>").map(|(_, t)| t).unwrap_or(rest),
    None => text,
  };
  let line = text.trim().lines().next().unwrap_or("").trim();
  let mut out: String = line.chars().take(80).collect();
  if line.chars().count() > 80 {
    out.push('…');
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn events_round_trip_and_updates_merge() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let store = Store::new(path.clone(), "s", Path::new("/w"));
    assert!(!path.exists(), "nothing is written before the first event");
    let items = vec![
      Item::User(vec![Part::Text("<mode>\nPlan mode is on.\n</mode>\n\nFix the build\nplease".into()), Part::Image { mime: "image/png".into(), data: "AA".into() }]),
      Item::Assistant {
        text: "ok".into(),
        reasoning: "r".into(),
        tool_calls: vec![ToolCall { id: "c1".into(), name: "bash".into(), arguments: "{}".into() }],
        native: Some(Native { origin: "o".into(), blocks: vec![json!({ "type": "text", "text": "ok" })] }),
      },
      Item::ToolResult { call_id: "c1".into(), name: "bash".into(), content: "out".into(), is_error: true },
    ];
    for i in &items {
      store.item(i);
    }
    for u in [
      json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Hel" } }),
      json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "lo" } }),
      json!({ "sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "bash", "status": "pending" }),
      json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "_meta": { "terminal_output_delta": { "data": "a" } } }),
      json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "_meta": { "terminal_output_delta": { "data": "b" } } }),
      json!({ "sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed", "_meta": { "terminal_exit": { "exit_code": 1 } } }),
      json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "Done" } }),
    ] {
      store.record_update(&u);
    }
    store.append(json!({ "type": "state", "mode": "plan", "model": "p/m" }));
    store.flush();
    // A torn last line is skipped
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"{\"type\":\"item\",\"it").unwrap();
    let back = load(&path, false).unwrap();
    assert_eq!((back.id.as_str(), back.cwd.as_path()), ("s", Path::new("/w")));
    assert_eq!(back.items, items);
    assert_eq!(back.title.as_deref(), Some("Fix the build"));
    assert_eq!(back.state["mode"], "plan");
    assert_eq!(back.updates.len(), 3);
    assert_eq!(back.updates[0]["content"]["text"], "Hello");
    assert_eq!(back.updates[1]["status"], "completed");
    assert_eq!(back.updates[1]["_meta"], json!({ "terminal_output_delta": { "data": "ab" }, "terminal_exit": { "exit_code": 1 } }));
    let summary = load(&path, true).unwrap();
    assert!(summary.items.is_empty() && summary.title.is_some());
  }
}
