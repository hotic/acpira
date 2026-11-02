//! Compaction: the completion latch and the automatic /compact policy. Devin and Kimi run /compact in the background and
//! report the result as prose; other peers use the RPC lifetime plus structured compaction_update events

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use anyhow::{Result, anyhow};
use regex::Regex;
use serde_json::Value;
use tokio::sync::oneshot;

use acpira_shared::transcript::{SessionStatus, TurnStop};

use crate::acp::session::{AcpSession, Core};
use crate::acp::transport::rpc::BoxFuture;
use crate::i18n::t;
use crate::json::str_of;
use crate::util::js_num;

static COMPACT: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"^/compact(?:\s|$)"));
static TOKENS_AFTER: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"- Tokens after:\s*([\d,]+)"));
static DEVIN_DONE: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(r"Context compacted|Nothing to compact\.|(?:Force compaction|Compaction) failed:|Compaction cancel(?:ed|led)\.")
});
static KIMI_DONE: LazyLock<Regex> = LazyLock::new(|| {
  acpira_shared::re::compile(r"Compaction completed\.|Compaction cancelled\.|Compaction is blocked by the current turn;|/compact failed:")
});

/// Room kept below the window when the threshold follows the model (the built-in agent's RESERVED_OUTPUT cap)
const MODEL_RESERVE: f64 = 32_000.0;

/// The threshold for "the model's window": the reported context size less a tenth of it, at most `MODEL_RESERVE`;
/// 0 (never) while no size is known. CLIs with their own auto-compaction usually fire first
fn model_threshold(size: f64) -> f64 {
  if size <= 0.0 { 0.0 } else { size - (size / 10.0).min(MODEL_RESERVE) }
}

/// The session's compaction bookkeeping
#[derive(Default)]
pub(crate) struct CompactionState {
  /// Usage when the last compaction ran; auto-compaction waits for another tenth of the threshold beyond it
  pub at: Option<f64>,
  /// The running turn's completion latch
  pub completion: Option<CompactionCompletion>,
  /// The last turn was a live, successfully completed user turn: a late usage report may still trigger auto-compaction
  pub auto_eligible: bool,
}

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

  /// A compaction is under way in this turn: the turn is a /compact, or the agent reported one of its own (a
  /// `compaction_update` mid-turn) that has not ended yet
  pub fn running(&self) -> bool {
    self.agent.is_some() || !self.pending.is_empty()
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

impl AcpSession {
  pub(crate) fn log_usage_threshold(&self, c: &Core, what: &str) {
    let used = c.state.usage.map(|u| js_num(u.used.0)).unwrap_or_else(|| "undefined".into());
    self.log(&format!("usage {used} ≥ threshold, {what}"));
  }

  /// ACP has no dedicated compaction request: send the agent's own /compact
  pub fn compact(self: &Arc<Self>, auto: bool) -> BoxFuture<Result<()>> {
    let me = self.clone();
    Box::pin(async move {
      let can = Self::can_compact_of(&me.core.lock());
      if !can {
        return if auto { Ok(()) } else { Err(anyhow!(t("host.noCompact"))) };
      }
      me.prompt("/compact".into(), vec![], auto, None, None).await;
      Ok(())
    })
  }

  pub(crate) fn should_auto_compact(&self, c: &Core) -> bool {
    let Some(policy) = self.deps.policy.compaction() else { return false };
    let used = c.state.usage.map(|u| u.used.0).unwrap_or(0.0);
    let size = c.state.usage.map(|u| u.size.0).unwrap_or(0.0);
    let at = if policy.at_tokens > 0.0 { policy.at_tokens } else { model_threshold(size) };
    if !policy.auto || used == 0.0 || at <= 0.0 || !Self::can_compact_of(c) || c.status != SessionStatus::Ready || used < at {
      return false;
    }
    c.compaction.at.is_none_or(|last| used >= last + at / 10.0)
  }

  /// Compact before flushing so a queued follow-up is not the request that runs over budget
  pub(crate) fn after_prompt(self: &Arc<Self>, auto: bool, stop: TurnStop) {
    let compact = {
      let c = self.core.lock();
      if c.pending_prompt.is_some() {
        return;
      }
      let yes = !auto && stop == TurnStop::EndTurn && self.should_auto_compact(&c);
      if yes {
        self.log_usage_threshold(&c, "auto /compact");
      }
      yes
    };
    if compact {
      let me = self.clone();
      tokio::spawn(async move {
        if let Err(e) = me.compact(true).await {
          me.log(&format!("auto /compact failed: {e}"));
          me.flush_queue();
        }
      });
      return;
    }
    self.flush_queue();
  }
}

fn tail16(s: &str, n: usize) -> String {
  let total = crate::json::len16(s);
  let skip = total - n;
  let mut units = 0;
  for (i, c) in s.char_indices() {
    if units >= skip {
      return s.get(i..).unwrap_or_default().to_owned();
    }
    units += c.len_utf16();
  }
  String::new()
}
