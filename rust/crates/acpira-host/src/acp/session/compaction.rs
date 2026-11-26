//! Compaction: the completion latch and the automatic /compact policy. Devin and Kimi run /compact in the background and
//! report the result as prose; other peers use the RPC lifetime plus structured compaction_update events

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use anyhow::{Result, anyhow};
use regex::Regex;
use serde_json::Value;
use tokio::sync::oneshot;

use acpira_shared::transcript::{SessionStatus, TurnStop};

use crate::acp::session::late_reply::has_background_commands;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transport::rpc::BoxFuture;
use crate::acp::vendors::Vendor;
use crate::i18n::t;
use crate::json::str_of;
use crate::util::js_num;

static COMPACT: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"^/compact(?:\s|$)"));
static TOKENS_AFTER: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"- Tokens after:\s*([\d,]+)"));

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
      manual: agent.is_some_and(|a| Vendor::of(a).manual_prose_compaction()),
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
      if self.manual && Vendor::of(&agent).compaction_prose_done(&self.text) {
        self.manual = false;
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
    let Some(usage) = c.state.usage else { return false };
    // Stale pre-compaction usage belongs to an already-finished /compact (e.g. agy / Devin): wait for a fresh snapshot
    if usage.compacted_from.is_some() {
      return false;
    }
    let (used, size) = (usage.used.0, usage.size.0);
    let at = if policy.at_tokens > 0.0 { policy.at_tokens } else { model_threshold(size) };
    if !policy.auto || used == 0.0 || at <= 0.0 || !Self::can_compact_of(c) || c.status != SessionStatus::Ready || used < at {
      return false;
    }
    c.compaction.at.is_none_or(|last| used >= last + at / 10.0)
  }

  /// Compact before flushing so a queued follow-up is not the request that runs over budget. While a command the turn
  /// sent to the background still runs, its late reply is still owed (`late_reply.rs`): compacting now would cut the
  /// model off from the history it wakes up to, so the turn boundary waits for the detached turn of that reply, whose
  /// end comes back here. A prompt sent meanwhile still compacts first (`claim`), so a command that never exits cannot
  /// hold compaction off for good
  pub(crate) fn after_prompt(self: &Arc<Self>, auto: bool, stop: TurnStop) {
    let compact = {
      let c = self.core.lock();
      if c.pending_prompt.is_some() {
        return;
      }
      let due = !auto && stop == TurnStop::EndTurn && self.should_auto_compact(&c);
      let deferred = due && has_background_commands(&c);
      if deferred {
        self.log_usage_threshold(&c, "auto compaction waits for background commands");
      } else if due {
        self.log_usage_threshold(&c, "auto /compact");
      }
      due && !deferred
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

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use serde_json::json;

  use acpira_shared::num::Num;
  use acpira_shared::transcript::{AgentBlock, AgentTurn, CompactionBlock, CompactionStatus, SessionStatus, Turn, TurnStop, Usage, UserTurn};

  use crate::acp::agents::registry::AgentRegistry;
  use crate::acp::session::{AcpSession, CompactionPolicy, SessionDeps, SessionPolicy};
  use crate::store::record::SessionRecord;
  use crate::store::transcript_store::TranscriptStore;

  struct FixedPolicy(CompactionPolicy);

  impl SessionPolicy for FixedPolicy {
    fn compaction(&self) -> Option<CompactionPolicy> {
      Some(self.0)
    }
  }

  fn restored_session(turns: Vec<Turn>, usage: Option<Usage>, at_tokens: f64) -> (Arc<AcpSession>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let deps = SessionDeps {
      registry: Arc::new(AgentRegistry::new(&json!({}))),
      log: Arc::new(|_| {}),
      on_change: Arc::new(|_, _| {}),
      blobs: TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_| {}), None),
      notify: None,
      accounts: None,
      policy: Arc::new(FixedPolicy(CompactionPolicy { at_tokens, auto: true })),
      pool: None,
      shared_mcp: None,
      plugin_hooks: None,
      plugin_prompts: None,
      data_home: None,
      plugin_runtime: None,
      host_mcp: None,
      remote: None,
    };
    let mut record: SessionRecord = serde_json::from_value(json!({
      "id": "s1",
      "agent": "antigravity",
      "cwd": dir.path().to_string_lossy(),
      "updatedAt": "2026-11-26T00:00:00Z",
      "turns": [],
      "commands": [{ "name": "compact", "description": "Compact" }]
    }))
    .unwrap();
    record.turns = turns;
    record.usage = usage;
    let s = AcpSession::new(record, deps);
    s.core.lock().status = SessionStatus::Ready;
    (s, dir)
  }

  #[test]
  fn stale_compacted_usage_never_retriggers_auto_compaction() {
    let usage = Usage { used: Num(506_052.0), size: Num(1_000_000.0), cost: None, compacted_from: Some(Num(506_052.0)) };
    let (s, _dir) = restored_session(vec![], Some(usage), 400_000.0);
    let c = s.core.lock();
    assert!(!s.should_auto_compact(&c));
  }

  #[test]
  fn restored_session_ending_on_completed_compaction_preserves_usage_floor() {
    let turns = vec![
      Turn::User(UserTurn { text: "/compact".into(), auto: Some(true), ..Default::default() }),
      Turn::Agent(AgentTurn {
        blocks: vec![AgentBlock::Compaction(CompactionBlock { id: "c1".into(), status: CompactionStatus::Completed, error: None })],
        stop: Some(TurnStop::EndTurn),
        ..Default::default()
      }),
    ];
    // Even if post-compaction usage is non-stale and still above threshold (or re-emitted on resume at the same size),
    // reopening after an idle process release must not immediately run a second /compact
    let usage = Usage { used: Num(420_000.0), size: Num(1_000_000.0), cost: None, compacted_from: None };
    let (s, _dir) = restored_session(turns, Some(usage), 400_000.0);
    {
      let c = s.core.lock();
      assert_eq!(c.compaction.at, Some(420_000.0));
      assert!(!s.should_auto_compact(&c));
    }
    // Once real usage grows by another tenth of the threshold (420k + 40k = 460k), auto-compaction becomes eligible again
    {
      let mut c = s.core.lock();
      c.state.usage = Some(Usage { used: Num(460_000.0), size: Num(1_000_000.0), cost: None, compacted_from: None });
      assert!(s.should_auto_compact(&c));
    }
  }

  #[test]
  fn restored_session_without_trailing_compaction_still_auto_compacts_over_threshold() {
    let turns = vec![
      Turn::User(UserTurn { text: "big".into(), ..Default::default() }),
      Turn::Agent(AgentTurn { stop: Some(TurnStop::EndTurn), ..Default::default() }),
    ];
    let usage = Usage { used: Num(420_000.0), size: Num(1_000_000.0), cost: None, compacted_from: None };
    let (s, _dir) = restored_session(turns, Some(usage), 400_000.0);
    let c = s.core.lock();
    assert_eq!(c.compaction.at, None);
    assert!(s.should_auto_compact(&c));
  }
}
