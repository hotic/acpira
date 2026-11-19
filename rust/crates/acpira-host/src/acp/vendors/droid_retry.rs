//! Droid 1.14 omits daemon `llm_retry` events from ACP. Only count a failed managed Gemini attempt when the CLI
//! itself diagnosed thinking without text or tools and logged the matching retryable error in the same span.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::util::{ms_of_iso, now_ms};

const THINKING_ONLY: &str = "WARN: [LLM] Response contained only thinking content with no text or tool calls";
const ERROR: &str = "WARN: [LLM] LLM error";
const WINDOW: u64 = 256 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cut {
  pub attempt: u32,
  pub repeated: bool,
}

/// One outstanding prompt. Never follow the whole machine log from its beginning or carry evidence across a gap.
#[derive(Default)]
pub(crate) struct Cursor {
  offset: u64,
  warning: Option<(String, i64)>,
  previous: Option<(String, u32)>,
}

impl Cursor {
  /// Read complete new lines only. On a busy host skip a backlog rather than interrupt a request that has recovered.
  pub fn poll(&mut self, path: &Path, session: &str, model: &str, since: i64) -> Option<Cut> {
    let meta = std::fs::symlink_metadata(path).ok().filter(|m| m.is_file())?;
    let end = meta.len();
    if end == self.offset { return None; }
    if end < self.offset || end - self.offset > WINDOW {
      *self = Self { offset: end.saturating_sub(WINDOW), ..Default::default() };
    }
    let mut f = std::fs::File::open(path).ok()?;
    f.seek(SeekFrom::Start(self.offset)).ok()?;
    let mut bytes = Vec::new();
    f.take(end - self.offset).read_to_end(&mut bytes).ok()?;
    let last = bytes.iter().rposition(|&b| b == b'\n')?;
    self.offset += last as u64 + 1;
    // The shared log can flush old events in batches. Such a retry may already have recovered on the wire.
    let since = since.max(now_ms() - 10_000);
    let mut cut = None;
    for line in bytes[..last].split(|&b| b == b'\n') {
      if let Ok(line) = std::str::from_utf8(line) {
        self.line(line, session, model, since, &mut cut);
      }
    }
    cut
  }

  fn line(&mut self, line: &str, session: &str, model: &str, since: i64, cut: &mut Option<Cut>) {
    let Some((ts, rest)) = line.strip_prefix('[').and_then(|s| s.split_once("] ")) else { return };
    let Some((msg, json)) = rest.split_once(" | Context: ") else { return };
    if !matches!(msg, THINKING_ONLY | ERROR | "INFO: [LLM] sendMessage" | "INFO: [Agent] Streaming result" | "INFO: [Daemon] Agent run complete") { return; }
    let Some(at) = ms_of_iso(ts).filter(|&at| at >= since) else { return };
    let Ok(d) = serde_json::from_str::<Value>(json) else { return };
    if d.pointer("/tags/sessionId").and_then(Value::as_str) != Some(session) { return; }
    if msg.starts_with("INFO:") {
      // A later call/result in the same batch wins over a stale failure. No cross-call cancellation.
      self.warning = None;
      self.previous = None;
      *cut = None;
      return;
    }
    if d["modelId"].as_str() != Some(model) { return; }
    let Some(span) = d.pointer("/tags/spanId").and_then(Value::as_str).filter(|id| !id.is_empty()) else { return };
    if msg == THINKING_ONLY {
      self.warning = (d["outputTokens"].as_u64() == Some(0) && d["length"].as_u64().is_some_and(|n| n > 0) && d["reason"].is_null())
        .then(|| (span.to_owned(), at));
      return;
    }
    let previous = self.previous.take();
    *cut = None;
    if !self.warning.take().is_some_and(|(id, t)| id == span && (0..=2000).contains(&(at - t)))
      || d["providerType"] != "gemini" || d["apiProvider"] != "google" || d["isRetryable"] != true
      || !matches!(d["errorName"].as_str(), Some("LLMNetworkError" | "LLMHostDurationLimitError")) { return; }
    let Some(id) = d["assistantMessageId"].as_str().filter(|id| !id.is_empty()) else { return };
    let Some(attempt) = d["attempt"].as_u64().and_then(|n| u32::try_from(n).ok()).filter(|&n| n > 0) else { return };
    let repeated = previous.is_some_and(|(old, n)| old == id && attempt.checked_sub(1) == Some(n));
    self.previous = Some((id.to_owned(), attempt));
    *cut = Some(Cut { attempt, repeated });
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn only_two_consecutive_cuts_of_the_same_request_can_trip() {
    let model = "gemini-4-carbon";
    let mut cur = Cursor::default();
    let mut cut = None;
    let mut feed = |msg: &str, id: &str, span: &str, attempt: u32, session: &str| {
      let ctx = json!({"tags":{"sessionId":session,"spanId":span},"modelId":model,"providerType":"gemini","apiProvider":"google",
        "isRetryable":true,"errorName":"LLMNetworkError","attempt":attempt,"assistantMessageId":id,"outputTokens":0,"length":100});
      cur.line(&format!("[2026-01-01T00:00:00.000Z] {msg} | Context: {ctx}"), "root", model, 0, &mut cut);
      cut
    };
    feed(THINKING_ONLY, "a", "s", 1, "root");
    assert_eq!(feed(ERROR, "a", "s", 1, "root"), Some(Cut { attempt: 1, repeated: false }));
    // A child session, a different span, or a different assistant request is not corroboration.
    feed(THINKING_ONLY, "a", "s", 2, "child");
    assert_eq!(feed(ERROR, "a", "s", 2, "root"), None);
    feed(THINKING_ONLY, "a", "s", 2, "root");
    assert_eq!(feed(ERROR, "a", "other", 2, "root"), None);
    for (id, n, repeated) in [("a", 1, false), ("b", 2, false), ("b", 2, false), ("b", 3, true)] {
      feed(THINKING_ONLY, id, "s", n, "root");
      assert_eq!(feed(ERROR, id, "s", n, "root"), Some(Cut { attempt: n, repeated }));
    }
    // A recovery later in the same read wins — a delayed diagnostic cannot cancel the next model call.
    assert_eq!(feed("INFO: [Agent] Streaming result", "b", "s", 3, "root"), None);
    feed(THINKING_ONLY, "b", "s", 4, "root");
    assert_eq!(feed(ERROR, "b", "s", 4, "root"), Some(Cut { attempt: 4, repeated: false }));
  }
}
