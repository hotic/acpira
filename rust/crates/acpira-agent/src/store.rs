//! A session's history: `<home>/agent/sessions/<id>.jsonl`, append-only, one event per line, beside the session's own
//! folder (`outputs/`, `plan.md`). Nothing is rewritten: what the model is sent is a view projected from the events.
//!
//! Events (each with `t`, milliseconds since the epoch):
//! - `session` — first line: id, cwd, format version
//! - `item` — one conversation entry as the model sees it (the view's source)
//! - `update` — a `session/update` as sent to the client, for `session/load`'s replay; streamed text and terminal
//!   output are merged per block and tool call, so a long session stays a reasonable size
//! - `state` — mode, model and effort at the start and after each change
//! - `prompt` — the system prompt whenever it was (re)composed
//! - `request` — one per model call: model and source, prompt variant and version, policies, usage with cache reads and
//!   writes, duration, stop reason or error
//! - `view` — what changed in the request prefix between two calls (prompt variant, tool set, model), so a cache miss
//!   can be traced afterwards
//! - `title` — the fast model's call naming the session (`title.rs`): model, duration, usage and cost, then `title` or
//!   `error`
//! - `compaction` — a summary compaction (`compact.rs`): reason, model, duration, usage and cost, then `summary`, `request` and
//!   `tail` (how many of the latest items were kept behind it) or `error`; the view from then on is the summary item
//!   and the tail. With `strategy: "chunk"` (`chunk.rs`) it holds the `chunks` added or changed, `tail` and the
//!   `budget`, and the view is the earlier summary (if any), the rendered chunks and the tail
//! - `prune` — the tool results cleared by a prune (`calls`, the tokens freed)
//! - `mark` — the start of a `session/prompt` the client named (`_meta["acpira/turn"]`): the turn id, plus what the model
//!   had been told up to there (`announced`, `mcp_note`, `sidekick_told`)
//! - `goal` — the session's goal after a change (`goal.rs`), `null` once cleared; the last one is the goal. Like the
//!   controls it describes the session, so a rewind keeps it
//!
//! A line cut short by a crash is skipped when the file is read back.
//!
//! The one exception to append-only is a rewind (`Store::rewind`, the client's history edit): the file is cut back to a
//! turn's `mark`, so the log, the view, the replay and the chunks' ordinals all agree again on the shorter conversation

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::llm::{Image, Item, Native, Part, ToolCall};

pub const FORMAT: u32 = 1;

pub fn now_ms() -> u64 {
  SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct Store {
  path: PathBuf,
  /// How many `item` events the file holds: the ordinal the next one gets (chunks address the log by them)
  items: std::sync::atomic::AtomicUsize,
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
    Store { path, items: Default::default(), header: json!({ "type": "session", "id": id, "cwd": cwd, "format": FORMAT }), inner: Default::default() }
  }

  /// How many `item` events have been written
  pub fn items_logged(&self) -> usize {
    self.items.load(std::sync::atomic::Ordering::SeqCst)
  }

  /// The count a reopened session's file already holds (`Loaded::logged`)
  pub fn set_items_logged(&self, n: usize) {
    self.items.store(n, std::sync::atomic::Ordering::SeqCst);
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
          // An existing file (a resumed session) already has its header; the held events still go in, since they
          // are what changed after the resume
          let t = now_ms();
          if f.metadata().map(|m| m.len() == 0).unwrap_or(false) {
            let mut h = header.clone();
            h["t"] = t.into();
            let _ = writeln!(f, "{h}");
          }
          for mut e in std::mem::take(&mut inner.prelude) {
            e["t"] = t.into();
            let _ = writeln!(f, "{e}");
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

  /// An event that belongs at the start of the file but is not worth creating it for (a new session's prompt). Once the
  /// file exists (also one a resumed session has not written to yet) it is appended at once
  pub fn prelude(&self, event: Value) {
    let mut inner = self.inner.lock();
    if inner.file.is_some() || self.path.metadata().is_ok_and(|m| m.len() > 0) {
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
    self.items.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
  }

  /// Cut the file back to just before the latest `mark` of `turn`, returning that mark; None when the file holds no such
  /// mark. Of the events after it, only those describing the session rather than the conversation stay (controls,
  /// system prompt, title, the request and view records), so the controls and the cost records survive the cut. The
  /// file is replaced through a temporary one, so a crash leaves either version whole. `dry` only looks the mark up
  pub fn rewind(&self, turn: &str, dry: bool) -> std::io::Result<Option<Value>> {
    let mut inner = self.inner.lock();
    if !dry && let Some(p) = inner.pending.take() {
      Self::write(&mut inner, &self.path, &self.header, update_event(p));
    }
    let text = match std::fs::read_to_string(&self.path) {
      Ok(t) => t,
      Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
      Err(e) => return Err(e),
    };
    let mut found = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
      if line.starts_with(MARK_LINE)
        && let Ok(ev) = serde_json::from_str::<Value>(line)
        && ev.get("turn").and_then(Value::as_str) == Some(turn)
      {
        found = Some((offset, ev));
      }
      offset += line.len();
    }
    let Some((at, mark)) = found else { return Ok(None) };
    if dry {
      return Ok(Some(mark));
    }
    let mut out = text[..at].to_owned();
    // A last line without its newline was cut short by a crash and is dropped with the rest
    for line in text[at..].split_inclusive('\n').skip(1).filter(|l| l.ends_with('\n')) {
      if !CONVERSATION_LINES.iter().any(|p| line.starts_with(p)) {
        out.push_str(line);
      }
    }
    let tmp = self.path.with_extension("jsonl.tmp");
    std::fs::write(&tmp, out)?;
    std::fs::rename(&tmp, &self.path)?;
    // The open handle still points at the replaced file: the next write opens the new one
    inner.file = None;
    Ok(Some(mark))
  }
}

/// How a `mark` event's line starts (`json!` keeps the key order it was written in)
const MARK_LINE: &str = r#"{"type":"mark""#;

/// The events a rewind drops after the mark: the conversation itself, what the replay shows of it and what reshaped it
const CONVERSATION_LINES: [&str; 5] =
  [r#"{"type":"item""#, r#"{"type":"update""#, r#"{"type":"compaction""#, r#"{"type":"prune""#, MARK_LINE];

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
    Item::ToolResult { call_id, name, content, is_error, images } => {
      let mut v = json!({ "role": "tool", "id": call_id, "name": name, "content": content, "error": is_error });
      // Only when there are any: older logs have no such field, and most results never will
      if !images.is_empty() {
        v["images"] = images.iter().map(|im| json!({ "image": im.data, "mime": im.mime })).collect();
      }
      v
    }
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
    "tool" => Item::ToolResult {
      call_id: s("id"),
      name: s("name"),
      content: s("content"),
      is_error: v.get("error").and_then(Value::as_bool).unwrap_or(false),
      images: v
        .get("images")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| Some(Image { data: i.get("image")?.as_str()?.to_owned(), mime: i.get("mime").and_then(Value::as_str).unwrap_or("image/png").to_owned() }))
        .collect(),
    },
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
  /// The title the fast model gave the session, else the first user message's text
  pub title: Option<String>,
  pub updated_ms: u64,
  /// The first item is a compaction's summary
  pub summarized: bool,
  /// What that summary item carries (`compact::Carry`), off its `compaction` event
  pub carry: crate::compact::Carry,
  /// The chunks shown in the view (`chunk.rs`), and how many items they render to after the summary
  pub chunks: Vec<crate::chunk::Chunk>,
  pub folded: usize,
  /// How many `item` events the file holds
  pub logged: usize,
  /// The latest `goal` event's goal
  pub goal: Option<crate::goal::Goal>,
}

/// Every item the log holds, as first written (a prune does not touch them), and every chunk it recorded, the latest
/// version of each: what the history tools read and chunk compaction folds from
pub fn history(path: &Path) -> std::io::Result<(Vec<Item>, Vec<crate::chunk::Chunk>)> {
  let (mut items, mut chunks) = (vec![], vec![]);
  for line in BufReader::new(File::open(path)?).lines() {
    let Ok(line) = line else { break };
    // Events are written with `type` first (`json!` keeps the key order); the rest is skipped unparsed
    if !line.starts_with(r#"{"type":"item""#) && !line.starts_with(r#"{"type":"compaction""#) {
      continue;
    }
    let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
    match ev.get("type").and_then(Value::as_str) {
      Some("item") => items.extend(ev.get("item").and_then(item_from)),
      Some("compaction") if ev.get("strategy").and_then(Value::as_str) == Some(crate::chunk::STRATEGY) => crate::chunk::merge(&mut chunks, chunk_list(&ev)),
      _ => {}
    }
  }
  Ok((items, chunks))
}

fn chunk_list(ev: &Value) -> Vec<crate::chunk::Chunk> {
  ev.get("chunks").and_then(Value::as_array).into_iter().flatten().filter_map(|c| serde_json::from_value(c.clone()).ok()).collect()
}

/// Read a session file; `only_summary` skips the history for listings
pub fn load(path: &Path, only_summary: bool) -> std::io::Result<Loaded> {
  let file = File::open(path)?;
  let mut out = Loaded { updated_ms: file.metadata()?.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0), ..Default::default() };
  let mut named: Option<String> = None;
  for line in BufReader::new(file).lines() {
    let Ok(line) = line else { break };
    // A listing needs only the head up to the first message, and a title event after it: skip parsing the rest
    if only_summary && out.title.is_some() && !line.contains(TITLE_MARK) {
      continue;
    }
    let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
    match ev.get("type").and_then(Value::as_str) {
      Some("title") => {
        if let Some(t) = ev.get("title").and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
          named = Some(t.to_owned());
        }
      }
      Some("session") => {
        out.id = ev.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        out.cwd = PathBuf::from(ev.get("cwd").and_then(Value::as_str).unwrap_or(""));
      }
      Some("item") => {
        let Some(item) = ev.get("item").and_then(item_from) else { continue };
        if out.title.is_none()
          && !item.is_steer()
          && let Item::User(parts) = &item
        {
          // A message of notes alone (a goal's continuation) names nothing; the next one may
          out.title = parts.iter().find_map(|p| match p {
            Part::Text(t) => Some(title_of(t)).filter(|t| !t.is_empty()),
            _ => None,
          });
        }
        if only_summary {
          continue;
        }
        out.logged += 1;
        out.items.push(item);
      }
      _ if only_summary => {}
      Some("update") => out.updates.extend(ev.get("update").cloned()),
      // Chunks: the earlier summary, the chunks rendered as they were, the items the tail kept
      Some("compaction") if ev.get("strategy").and_then(Value::as_str) == Some(crate::chunk::STRATEGY) => {
        let tail = ev.get("tail").and_then(Value::as_u64).unwrap_or(0) as usize;
        let budget = ev.get("budget").cloned().and_then(|b| serde_json::from_value(b).ok()).unwrap_or_default();
        crate::chunk::merge(&mut out.chunks, chunk_list(&ev));
        let rest = out.items.split_off(out.items.len().saturating_sub(tail));
        let summary = out.summarized.then(|| out.items[0].clone());
        let projection = crate::chunk::render(&out.chunks, budget);
        out.folded = projection.len();
        out.items = summary.into_iter().chain(projection).chain(rest).collect();
      }
      // The view from a compaction on: its summary in front of the items the tail kept; the chunks it took in are no
      // longer shown (the history tools still find them in the log)
      Some("compaction") => {
        let Some(summary) = ev.get("summary").and_then(Value::as_str) else { continue };
        out.chunks.clear();
        out.folded = 0;
        let tail = ev.get("tail").and_then(Value::as_u64).unwrap_or(0) as usize;
        let rest = out.items.split_off(out.items.len().saturating_sub(tail));
        out.carry = crate::compact::Carry::from_event(&ev);
        out.items = vec![crate::compact::summary_item(summary, &out.carry)];
        out.items.extend(rest);
        out.summarized = true;
      }
      Some("prune") => {
        let calls: Vec<String> = ev.get("calls").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
        crate::compact::apply_prune(&mut out.items, &calls);
      }
      Some("state") => {
        for (k, v) in ev.as_object().into_iter().flatten() {
          if k != "type" && k != "t" {
            out.state.insert(k.clone(), v.clone());
          }
        }
      }
      Some("prompt") => out.prompt = Some(ev),
      Some("goal") => out.goal = ev.get("goal").filter(|g| !g.is_null()).and_then(|g| serde_json::from_value(g.clone()).ok()),
      _ => {}
    }
  }
  if named.is_some() {
    out.title = named;
  }
  Ok(out)
}

/// How a `title` event's line starts (`json!` keeps the key order it was written in)
const TITLE_MARK: &str = r#"{"type":"title""#;

/// A list title: the first line of the first message, without the notes the agent put in front (`chunk::strip_notes`)
fn title_of(text: &str) -> String {
  let line = crate::chunk::strip_notes(text).trim().lines().next().unwrap_or("").trim();
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
      Item::ToolResult { call_id: "c1".into(), name: "bash".into(), content: "out".into(), is_error: true, images: vec![] },
      // An MCP screenshot comes back with its image
      Item::ToolResult { call_id: "c2".into(), name: "bench_shot".into(), content: "shot".into(), is_error: false, images: vec![Image { mime: "image/png".into(), data: "BB".into() }] },
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

  #[test]
  fn a_compaction_and_a_prune_project_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let store = Store::new(path.clone(), "s", Path::new("/w"));
    let call = |id: &str| Item::Assistant { text: String::new(), reasoning: String::new(), tool_calls: vec![ToolCall { id: id.into(), name: "read".into(), arguments: "{}".into() }], native: None };
    let result = |id: &str| Item::ToolResult { call_id: id.into(), name: "read".into(), content: format!("out {id}"), is_error: false, images: vec![] };
    for i in [Item::User(vec![Part::Text("one".into())]), call("c1"), result("c1"), Item::User(vec![Part::Text("two".into())]), call("c2"), result("c2")] {
      store.item(&i);
    }
    store.append(json!({ "type": "prune", "calls": ["c1"], "tokens": 2 }));
    store.append(json!({ "type": "compaction", "reason": "manual", "summary": "## Objective\n- x", "tail": 3 }));
    store.item(&Item::User(vec![Part::Text("three".into())]));
    store.flush();
    let back = load(&path, false).unwrap();
    assert!(back.summarized);
    assert_eq!(back.items.len(), 5);
    assert!(matches!(&back.items[0], Item::User(p) if matches!(&p[0], Part::Text(t) if t.starts_with("<conversation-summary>\n## Objective\n- x\n</conversation-summary>"))));
    assert_eq!(back.items[1], Item::User(vec![Part::Text("two".into())]));
    assert_eq!(back.items[4], Item::User(vec![Part::Text("three".into())]));
    // The prune cleared the first result before the compaction dropped it; a later one would show in the tail
    assert_eq!(back.title.as_deref(), Some("one"));
  }

  #[test]
  fn the_last_goal_event_is_the_goal_and_a_rewind_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.jsonl");
    let store = Store::new(path.clone(), "s", Path::new("/w"));
    // A goal turn's message: the title is the objective, behind the notes stacked in front of it
    store.append(json!({ "type": "mark", "turn": "t1" }));
    store.item(&Item::User(vec![Part::Text("<mcp>\nM\n</mcp>\n\n<goal>\nN\n</goal>\n\nPort the parser\nfully".into())]));
    let mut goal = crate::goal::Goal::new("Port the parser", Some(9000));
    store.append(json!({ "type": "goal", "goal": goal }));
    store.append(json!({ "type": "mark", "turn": "t2" }));
    store.item(&Item::User(vec![Part::Text("<goal>\ncontinue\n</goal>".into())]));
    goal.tokens_used = 1500;
    goal.status = crate::goal::Status::Paused;
    store.append(json!({ "type": "goal", "goal": goal }));
    let back = load(&path, false).unwrap();
    assert_eq!(back.title.as_deref(), Some("Port the parser"));
    assert_eq!(back.goal.as_ref(), Some(&goal));
    // Like the controls, the goal is the session's, not the conversation's
    store.rewind("t2", false).unwrap().unwrap();
    let back = load(&path, false).unwrap();
    assert_eq!((back.items.len(), back.goal.as_ref()), (1, Some(&goal)));
    store.append(json!({ "type": "goal", "goal": null }));
    assert_eq!(load(&path, false).unwrap().goal, None);
    // A first message of notes alone names nothing; the next one does
    let other = dir.path().join("o.jsonl");
    let store = Store::new(other.clone(), "o", Path::new("/w"));
    store.item(&Item::User(vec![Part::Text("<goal>\nN\n</goal>".into())]));
    store.item(&Item::User(vec![Part::Text("<steer>\nS\n</steer>\n\nsteered".into())]));
    store.item(&Item::User(vec![Part::Text("<sidekick>\nK\n</sidekick>\n\nthen this".into())]));
    assert_eq!(load(&other, true).unwrap().title.as_deref(), Some("then this"));
  }
}
