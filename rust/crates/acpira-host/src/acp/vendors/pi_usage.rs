//! Pi context occupancy. pi-acp 0.0.33 sends no `usage_update` and keeps pi's `get_session_stats` to its `/session`
//! command, so the snapshot is read from pi's own session file: `<ts>_<id>.jsonl`, append-only, one entry per line,
//! a tree by `parentId` whose last line is the current leaf. The count follows pi 0.86.0's footer
//! (`AgentSession.getContextUsage` / `estimateContextTokens`): the last valid assistant usage on the branch plus a
//! chars/4 estimate of every message after it. Right after a compaction pi reports the tokens as unknown until the next
//! reply; here the compaction summary and the kept messages are estimated instead, so the meter drops at once.
//! The window comes from `models.json` (custom models and `modelOverrides`) or `models-store.json` (pi's built-in
//! catalogue); a model found in neither leaves the usage unknown

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use acpira_shared::num::Num;
use acpira_shared::transcript::Usage;
use serde_json::Value;

use crate::store::data_dir::home_dir;

pub const AGENT_DIR_ENV: &str = "PI_CODING_AGENT_DIR";
/// pi's `ESTIMATED_IMAGE_CHARS`
const IMAGE_CHARS: usize = 4800;
/// pi's default `contextWindow` for a custom model that does not set one
const DEFAULT_WINDOW: f64 = 128_000.0;

/// Identity of the session file a snapshot was read from: unchanged size and mtime skip the re-read
#[derive(Debug, Clone, PartialEq)]
pub struct Stamp {
  pub path: PathBuf,
  len: u64,
  modified: Option<SystemTime>,
}

#[derive(Debug, PartialEq)]
pub enum Snapshot {
  /// No session file yet: pi writes it with the first reply
  Missing,
  Unchanged,
  Fresh(Stamp, Option<Usage>),
}

/// `PI_CODING_AGENT_DIR` (with `~` expanded) or `~/.pi/agent`, like pi's `getAgentDir`
pub fn agent_dir() -> PathBuf {
  agent_dir_of(std::env::var(AGENT_DIR_ENV).ok())
}

/// `agent_dir` for a given `PI_CODING_AGENT_DIR` value (an agent entry's own env)
pub fn agent_dir_of(value: Option<String>) -> PathBuf {
  match value.filter(|s| !s.trim().is_empty()) {
    Some(d) if d == "~" => home_dir(),
    Some(d) => d.strip_prefix("~/").map(|rest| home_dir().join(rest)).unwrap_or_else(|| PathBuf::from(d)),
    None => home_dir().join(".pi").join("agent"),
  }
}

/// Read the current snapshot of one pi session; blocking file IO, run it off the async threads
pub fn read(cwd: &str, session_id: &str, prev: Option<&Stamp>) -> Snapshot {
  let agent_dir = agent_dir();
  let path = match prev.filter(|s| s.path.is_file()) {
    Some(s) => s.path.clone(),
    None => match session_file(&home_dir(), &agent_dir, cwd, session_id) {
      Some(p) => p,
      None => return Snapshot::Missing,
    },
  };
  let Ok(meta) = std::fs::metadata(&path) else { return Snapshot::Missing };
  let stamp = Stamp { path, len: meta.len(), modified: meta.modified().ok() };
  if prev == Some(&stamp) {
    return Snapshot::Unchanged;
  }
  let Ok(text) = std::fs::read_to_string(&stamp.path) else { return Snapshot::Missing };
  let usage = context_usage(&text, |provider, model| session_window(&agent_dir, provider, model));
  Snapshot::Fresh(stamp, usage)
}

/// The window of the session's model, or of pi's default model when that one is gone from the registry (pi falls
/// back to `settings.json`'s `defaultProvider` / `defaultModel` when it reopens such a session)
pub fn session_window(agent_dir: &Path, provider: &str, model: &str) -> Option<f64> {
  context_window(agent_dir, provider, model).or_else(|| {
    let settings = read_json(&agent_dir.join("settings.json"))?;
    context_window(agent_dir, str_of(&settings, "defaultProvider")?, str_of(&settings, "defaultModel")?)
  })
}

/// pi-acp's own session map first (`~/.pi/pi-acp/session-map.json`, also holds sessions moved by pi's `sessionDir`),
/// then pi's default directory for the cwd
pub fn session_file(home: &Path, agent_dir: &Path, cwd: &str, session_id: &str) -> Option<PathBuf> {
  let mapped = read_json(&home.join(".pi").join("pi-acp").join("session-map.json"))
    .and_then(|m| m.get("sessions")?.get(session_id)?.get("sessionFile")?.as_str().map(PathBuf::from))
    .filter(|p| p.is_file());
  if mapped.is_some() {
    return mapped;
  }
  let safe = cwd.trim_start_matches(['/', '\\']).replace(['/', '\\', ':'], "-");
  let suffix = format!("_{session_id}.jsonl");
  std::fs::read_dir(agent_dir.join("sessions").join(format!("--{safe}--")))
    .ok()?
    .filter_map(|e| e.ok())
    .find(|e| e.file_name().to_str().is_some_and(|n| n.ends_with(&suffix)))
    .map(|e| e.path())
}

/// Context window of `provider/model`: `modelOverrides`, then the custom model (128k when unset), then the catalogue
pub fn context_window(agent_dir: &Path, provider: &str, model: &str) -> Option<f64> {
  let window = |m: &Value| m.get("contextWindow").and_then(Value::as_f64).filter(|w| *w > 0.0);
  let find = |models: Option<&Value>| models?.as_array()?.iter().find(|m| m.get("id").and_then(Value::as_str) == Some(model)).cloned();
  if let Some(p) = read_json(&agent_dir.join("models.json")).and_then(|v| v.get("providers")?.get(provider).cloned()) {
    if let Some(w) = p.get("modelOverrides").and_then(|o| o.get(model)).and_then(window) {
      return Some(w);
    }
    if let Some(m) = find(p.get("models")) {
      return Some(window(&m).unwrap_or(DEFAULT_WINDOW));
    }
  }
  let store = read_json(&agent_dir.join("models-store.json"))?;
  find(store.get(provider)?.get("models")).as_ref().and_then(window)
}

fn read_json(path: &Path) -> Option<Value> {
  serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

enum Walk {
  /// Looking for the last valid assistant usage
  Usage,
  /// Past a compaction with no reply after it: estimating back to its first kept entry
  Kept(Option<String>),
  Done,
}

/// The snapshot of one session file's text; `window` resolves `(provider, model id)` to the context size
pub fn context_usage(jsonl: &str, window: impl Fn(&str, &str) -> Option<f64>) -> Option<Usage> {
  let mut want: Option<String> = None;
  let mut started = false;
  let mut walk = Walk::Usage;
  let mut estimate = 0.0;
  let mut used: Option<f64> = None;
  let mut model: Option<(String, String)> = None;
  let mut cost = 0.0;
  for line in jsonl.lines().rev() {
    let Ok(e) = serde_json::from_str::<Value>(line.trim()) else { continue };
    cost += entry_cost(&e);
    let id = e.get("id").and_then(Value::as_str);
    if str_of(&e, "type") == Some("session") || id.is_none() {
      continue;
    }
    if !started {
      started = true;
      want = id.map(str::to_owned);
    }
    // Entries off the current branch (an abandoned fork) are not in the context
    if id != want.as_deref() {
      continue;
    }
    want = str_of(&e, "parentId").map(str::to_owned);
    if model.is_none() {
      model = model_of(&e);
    }
    match &walk {
      Walk::Usage => {
        if let Some(t) = assistant_tokens(&e) {
          used = Some(t + estimate);
          walk = Walk::Done;
        } else if str_of(&e, "type") == Some("compaction") {
          estimate += entry_tokens(&e);
          walk = Walk::Kept(str_of(&e, "firstKeptEntryId").map(str::to_owned));
        } else {
          estimate += entry_tokens(&e);
        }
      }
      Walk::Kept(first) => {
        estimate += entry_tokens(&e);
        if first.as_deref() == id {
          used = Some(estimate);
          walk = Walk::Done;
        }
      }
      Walk::Done => {}
    }
    if matches!(walk, Walk::Done) && model.is_some() && want.is_none() {
      break;
    }
  }
  // A branch that ran out before a reply (or before the kept entry) is all estimate, as in pi
  let used = used.or((!matches!(walk, Walk::Done) && estimate > 0.0).then_some(estimate))?;
  let (provider, id) = model?;
  let size = window(&provider, &id)?;
  Some(Usage { used: Num(used), size: Num(size), cost: (cost > 0.0).then_some(Num(cost)) })
}

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
  v.get(key).and_then(Value::as_str)
}

/// The model an entry says is current: a `model_change`, or the model that wrote an assistant message
fn model_of(e: &Value) -> Option<(String, String)> {
  let (provider, id) = match str_of(e, "type")? {
    "model_change" => (str_of(e, "provider")?, str_of(e, "modelId")?),
    "message" => {
      let m = e.get("message")?;
      if str_of(m, "role")? != "assistant" {
        return None;
      }
      (str_of(m, "provider")?, str_of(m, "model")?)
    }
    _ => return None,
  };
  Some((provider.to_owned(), id.to_owned()))
}

/// pi's `calculateContextTokens` of a usable assistant reply (not aborted / errored, not all zero)
fn assistant_tokens(e: &Value) -> Option<f64> {
  let m = e.get("message").filter(|_| str_of(e, "type") == Some("message"))?;
  if str_of(m, "role") != Some("assistant") || matches!(str_of(m, "stopReason"), Some("aborted" | "error")) {
    return None;
  }
  let u = m.get("usage")?;
  let n = |k: &str| u.get(k).and_then(Value::as_f64).unwrap_or(0.0);
  let total = n("totalTokens");
  let tokens = if total > 0.0 { total } else { n("input") + n("output") + n("cacheRead") + n("cacheWrite") };
  (tokens > 0.0).then_some(tokens)
}

/// The spend `get_session_stats` sums: messages, usage entries, compaction and branch summaries
fn entry_cost(e: &Value) -> f64 {
  let usage = match str_of(e, "type") {
    Some("message") => e.get("message").and_then(|m| m.get("usage")),
    Some("usage" | "compaction" | "branch_summary") => e.get("usage"),
    _ => None,
  };
  usage.and_then(|u| u.get("cost")).and_then(|c| c.get("total")).and_then(Value::as_f64).unwrap_or(0.0)
}

/// pi's `estimateTokens` (chars / 4, rounded up per message) of the context message an entry becomes
fn entry_tokens(e: &Value) -> f64 {
  let chars = match str_of(e, "type") {
    Some("message") => e.get("message").map_or(0, message_chars),
    Some("custom_message") => e.get("content").map_or(0, content_chars),
    Some("compaction" | "branch_summary") => str_of(e, "summary").map_or(0, js_len),
    _ => 0,
  };
  chars.div_ceil(4) as f64
}

fn message_chars(m: &Value) -> usize {
  let content = m.get("content");
  match str_of(m, "role") {
    Some("user" | "toolResult" | "custom") => content.map_or(0, content_chars),
    Some("assistant") => content.and_then(Value::as_array).map_or(0, |blocks| {
      blocks
        .iter()
        .map(|b| match str_of(b, "type") {
          Some("text") => str_of(b, "text").map_or(0, js_len),
          Some("thinking") => str_of(b, "thinking").map_or(0, js_len),
          Some("toolCall") => str_of(b, "name").map_or(0, js_len) + b.get("arguments").map_or(4, |a| js_len(&a.to_string())),
          _ => 0,
        })
        .sum()
    }),
    Some("bashExecution") => str_of(m, "command").map_or(0, js_len) + str_of(m, "output").map_or(0, js_len),
    Some("branchSummary" | "compactionSummary") => str_of(m, "summary").map_or(0, js_len),
    _ => 0,
  }
}

fn content_chars(content: &Value) -> usize {
  match content {
    Value::String(s) => js_len(s),
    Value::Array(blocks) => blocks
      .iter()
      .map(|b| match str_of(b, "type") {
        Some("text") => str_of(b, "text").map_or(0, js_len),
        Some("image") => IMAGE_CHARS,
        _ => 0,
      })
      .sum(),
    _ => 0,
  }
}

/// JavaScript `String.length`: UTF-16 code units
fn js_len(s: &str) -> usize {
  s.encode_utf16().count()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn file(entries: &[Value]) -> String {
    let mut lines = vec![json!({ "type": "session", "version": 3, "id": "s1", "cwd": "/w" }).to_string()];
    lines.extend(entries.iter().map(Value::to_string));
    lines.join("\n") + "\n"
  }

  fn msg(id: &str, parent: Option<&str>, message: Value) -> Value {
    json!({ "type": "message", "id": id, "parentId": parent, "message": message })
  }

  fn reply(input: f64, total: f64, cost: f64) -> Value {
    json!({
      "role": "assistant", "provider": "asgard", "model": "k3", "stopReason": "stop",
      "content": [{ "type": "text", "text": "ok" }],
      "usage": { "input": input, "output": 10, "cacheRead": 0, "cacheWrite": 0, "totalTokens": total, "cost": { "total": cost } }
    })
  }

  fn window(_: &str, model: &str) -> Option<f64> {
    (model == "k3").then_some(256_000.0)
  }

  fn used(text: &str) -> Option<(f64, f64, Option<f64>)> {
    context_usage(text, window).map(|u| (u.used.0, u.size.0, u.cost.map(|c| c.0)))
  }

  #[test]
  fn last_reply_plus_trailing_estimate() {
    let text = file(&[
      json!({ "type": "model_change", "id": "m", "parentId": null, "provider": "asgard", "modelId": "k3" }),
      msg("u1", Some("m"), json!({ "role": "user", "content": "hi" })),
      msg("a1", Some("u1"), reply(100.0, 110.0, 0.5)),
      msg("u2", Some("a1"), json!({ "role": "user", "content": [{ "type": "text", "text": "12345678" }] })),
    ]);
    assert_eq!(used(&text), Some((112.0, 256_000.0, Some(0.5))));
  }

  #[test]
  fn skips_aborted_replies_and_other_branches() {
    let mut aborted = reply(9_000.0, 9_000.0, 0.0);
    aborted["stopReason"] = json!("aborted");
    let text = file(&[
      msg("a1", None, reply(100.0, 0.0, 0.0)),
      msg("x", Some("a1"), reply(50_000.0, 50_000.0, 0.25)),
      msg("a2", Some("a1"), aborted),
    ]);
    // input + output of a1 once totalTokens is 0, plus the aborted reply's text; the abandoned fork "x" still counts
    // toward spend
    assert_eq!(used(&text), Some((111.0, 256_000.0, Some(0.25))));
  }

  #[test]
  fn compaction_without_a_reply_is_estimated() {
    let text = file(&[
      msg("u1", None, json!({ "role": "user", "content": "a".repeat(4000) })),
      msg("a1", Some("u1"), reply(180_000.0, 180_000.0, 0.0)),
      msg("u2", Some("a1"), json!({ "role": "user", "content": "b".repeat(40) })),
      msg("a2", Some("u2"), reply(190_000.0, 190_000.0, 0.0)),
      json!({ "type": "compaction", "id": "c", "parentId": "a2", "summary": "s".repeat(400), "firstKeptEntryId": "u2", "tokensBefore": 190_000 }),
    ]);
    // summary 100 + a2 text 1 + u2 10
    assert_eq!(used(&text), Some((111.0, 256_000.0, None)));
    let after = format!("{text}{}\n", msg("a3", Some("c"), reply(2_000.0, 2_000.0, 0.0)));
    assert_eq!(used(&after), Some((2_000.0, 256_000.0, None)));
  }

  #[test]
  fn unknown_window_leaves_usage_unknown() {
    let mut other = reply(100.0, 100.0, 0.0);
    other["model"] = json!("mystery");
    assert_eq!(used(&file(&[msg("a1", None, other)])), None);
    assert_eq!(used(&file(&[])), None);
  }

  #[test]
  fn windows_follow_overrides_custom_models_and_the_catalogue() {
    let dir = tempfile::tempdir().unwrap();
    let models = json!({ "providers": {
      "asgard": { "models": [{ "id": "k3", "contextWindow": 256_000 }, { "id": "bare" }] },
      "anthropic": { "modelOverrides": { "opus": { "contextWindow": 1_000_000 } } }
    }});
    let store =
      json!({ "anthropic": { "models": [{ "id": "opus", "contextWindow": 200_000 }, { "id": "haiku", "contextWindow": 200_000 }] } });
    std::fs::write(dir.path().join("models.json"), models.to_string()).unwrap();
    std::fs::write(dir.path().join("models-store.json"), store.to_string()).unwrap();
    let w = |p, m| context_window(dir.path(), p, m);
    assert_eq!(w("asgard", "k3"), Some(256_000.0));
    assert_eq!(w("asgard", "bare"), Some(DEFAULT_WINDOW));
    assert_eq!(w("anthropic", "opus"), Some(1_000_000.0));
    assert_eq!(w("anthropic", "haiku"), Some(200_000.0));
    assert_eq!(w("openai", "gpt"), None);
  }

  #[test]
  fn session_file_prefers_the_pi_acp_map() {
    let home = tempfile::tempdir().unwrap();
    let agent = home.path().join("agent");
    let dir = agent.join("sessions").join("--Volumes-P-x--");
    std::fs::create_dir_all(&dir).unwrap();
    let native = dir.join("2026-09-25T00-00-00-000Z_s1.jsonl");
    std::fs::write(&native, "").unwrap();
    assert_eq!(session_file(home.path(), &agent, "/Volumes/P/x", "s1"), Some(native.clone()));
    assert_eq!(session_file(home.path(), &agent, "/Volumes/P/x", "s2"), None);
    let moved = home.path().join("elsewhere.jsonl");
    std::fs::write(&moved, "").unwrap();
    let map = home.path().join(".pi").join("pi-acp");
    std::fs::create_dir_all(&map).unwrap();
    std::fs::write(map.join("session-map.json"), json!({ "version": 1, "sessions": { "s1": { "sessionFile": moved } } }).to_string())
      .unwrap();
    assert_eq!(session_file(home.path(), &agent, "/Volumes/P/x", "s1"), Some(moved));
  }
}
