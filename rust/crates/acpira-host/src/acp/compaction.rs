//! Compaction completion latch (mirror of src/host/acp/compaction.ts). Devin and Kimi run /compact in the background and
//! report the result as prose; other peers use the RPC lifetime plus structured compaction_update events

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;
use tokio::sync::oneshot;

use crate::json::str_of;

static COMPACT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^/compact(?:\s|$)").unwrap());
static TOKENS_AFTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"- Tokens after:\s*([\d,]+)").unwrap());
static DEVIN_DONE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"Context compacted|Nothing to compact\.|(?:Force compaction|Compaction) failed:|Compaction cancel(?:ed|led)\.").unwrap()
});
static KIMI_DONE: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"Compaction completed\.|Compaction cancelled\.|Compaction is blocked by the current turn;|/compact failed:").unwrap()
});

pub fn is_compact_command(text: &str) -> bool {
  COMPACT.is_match(text.trim())
}

pub struct CompactionCompletion {
  agent: Option<String>,
  pending: HashSet<String>,
  manual: bool,
  structured: bool,
  text: String,
  release: Option<oneshot::Sender<()>>,
  pub tokens_after: Option<f64>,
}

impl CompactionCompletion {
  pub fn new(agent: Option<&str>) -> Self {
    CompactionCompletion {
      manual: matches!(agent, Some("devin" | "kimi")),
      agent: agent.map(str::to_owned),
      pending: HashSet::new(),
      structured: false,
      text: String::new(),
      release: None,
      tokens_after: None,
    }
  }

  pub fn update(&mut self, u: &Value) {
    let kind = str_of(u, "sessionUpdate");
    if kind == Some("compaction_update") {
      self.structured = true;
      self.manual = false;
      let id = str_of(u, "compactionId").unwrap_or("").to_owned();
      if matches!(str_of(u, "status"), Some("completed" | "failed" | "cancelled")) {
        self.pending.remove(&id);
      } else {
        self.pending.insert(id);
      }
    } else if let Some(agent) = self.agent.clone()
      && !self.structured
      && kind == Some("agent_message_chunk")
      && u.get("content").and_then(|c| str_of(c, "type")) == Some("text")
    {
      let chunk = u["content"].get("text").and_then(Value::as_str).unwrap_or("");
      let joined = format!("{}{chunk}", self.text);
      // Keep the last 4096 UTF-16 units
      let len = crate::json::len16(&joined);
      self.text = if len > 4096 { tail16(&joined, 4096) } else { joined };
      if let Some(n) = TOKENS_AFTER.captures(&self.text).and_then(|c| c[1].replace(',', "").parse::<f64>().ok()) {
        self.tokens_after = Some(n);
      }
      if self.manual {
        let terminal = if agent == "devin" { &*DEVIN_DONE } else { &*KIMI_DONE };
        if terminal.is_match(&self.text) {
          self.manual = false;
        }
      }
    }
    if !self.manual
      && self.pending.is_empty()
      && let Some(r) = self.release.take()
    {
      let _ = r.send(());
    }
  }

  /// None when nothing is pending; otherwise a receiver that fires on completion or close
  pub fn wait(&mut self) -> Option<oneshot::Receiver<()>> {
    if !self.manual && self.pending.is_empty() {
      return None;
    }
    let (tx, rx) = oneshot::channel();
    self.release = Some(tx);
    Some(rx)
  }

  /// RPC failure, process exit, or session disposal ends the local wait as well
  pub fn close(&mut self) {
    self.manual = false;
    self.pending.clear();
    if let Some(r) = self.release.take() {
      let _ = r.send(());
    }
  }
}

fn tail16(s: &str, n: usize) -> String {
  let total = crate::json::len16(s);
  let skip = total - n;
  let mut units = 0;
  for (i, c) in s.char_indices() {
    if units >= skip {
      return s[i..].to_owned();
    }
    units += c.len_utf16();
  }
  String::new()
}
