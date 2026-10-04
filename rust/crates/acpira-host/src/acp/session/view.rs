//! What a session shows and stores: the list entry and state dot, the view every viewer renders, and the record the
//! store writes, both serialized straight from `Core` without cloning the transcript

use std::sync::Arc;

use acpira_shared::protocol::{HostMsg, RawJson};
use acpira_shared::session_patch::{ViewParts, ViewPartsData, encode_turns, raw};
use acpira_shared::subagents::SubagentSummary;
use acpira_shared::model_shapes::ModelShapes;
use acpira_shared::transcript::*;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::acp::session::controls::picked_controls;
use crate::acp::session::queue::queue_snapshot;
use crate::acp::session::{AcpSession, Core};
use crate::store::record::{ForkedFrom, ImportedFrom, RecordSource, SessionRecord};

impl AcpSession {
  /// The list entry of this session
  pub fn summary(&self) -> SessionSummary {
    let c = self.core.lock();
    SessionSummary {
      id: self.id.clone(),
      external: None,
      title: Self::title_of(&c),
      agent: self.agent.clone(),
      account_id: c.account_id.clone(),
      acp_session_id: c.acp_session_id.clone(),
      cwd: self.cwd.clone(),
      updated_at: c.updated_at.clone(),
      pinned: c.pinned,
      state: None,
    }
  }

  /// The session list's state dot: working, waiting on the user, or ended in error
  pub fn list_state(&self) -> Option<SummaryState> {
    let c = self.core.lock();
    if c.phase.running {
      return Some(SummaryState::Working);
    }
    let waiting = visible_turns(&c).any(|t| {
      t.as_agent().is_some_and(|a| {
        a.blocks.iter().any(|b| matches!(b, AgentBlock::Permission(_)) || matches!(b, AgentBlock::Question(q) if q.outcome.is_none()))
      })
    });
    if waiting {
      return Some(SummaryState::Waiting);
    }
    match visible_turns(&c).last() {
      Some(Turn::Agent(a)) if a.stop == Some(TurnStop::Error) => Some(SummaryState::Error),
      _ => None,
    }
  }

  /// The whole view, serialized once for every viewer showing this session
  pub fn view_json(&self) -> (RawJson, bool) {
    let (raw, running, _) = self.view_encoded();
    (raw, running)
  }

  /// The push every viewer of one change shares: the whole view, plus its turn-by-turn encoding that a patching viewer is
  /// sent the difference of (`bridge_core.rs`)
  pub fn view_msg(&self) -> HostMsg {
    let (session, running, parts) = self.view_encoded();
    HostMsg::Session { session, running, parts: Some(parts) }
  }

  fn view_encoded(&self) -> (RawJson, bool, ViewParts) {
    let shapes = self.deps.model_shapes.as_ref().and_then(|f| f(&self.agent));
    let mut c = self.core.lock();
    // Each visible turn is encoded on its own, the last one also block by block when it is an agent turn; the whole view
    // is spliced from the same fragments, so the transcript is still serialized once per change
    let (turns, last) = {
      let core = &mut *c;
      match core.pending_prompt.as_mut() {
        Some(p) => encode_turns(&core.state.turns.iter().collect::<Vec<_>>(), Some(p)),
        None => match core.state.turns.split_last_mut() {
          Some((last, before)) => encode_turns(&before.iter().collect::<Vec<_>>(), Some(last)),
          None => (vec![], None),
        },
      }
    };
    let subagents = if c.tree.is_empty() { None } else { Some(c.tree.summaries()) };
    let picked = (!c.picks.values.is_empty()).then(|| picked_controls(&c));
    let queued = queue_snapshot(&c);
    let head = ViewRef {
      id: &self.id,
      agent: &self.agent,
      account_id: c.account_id.as_deref(),
      title: Self::title_of(&c),
      cwd: &self.cwd,
      status: c.status,
      error: c.error.as_deref(),
      can_take_over: c.status == SessionStatus::Error && c.lock_holder.is_some(),
      auth_methods: c.auth_methods.as_deref(),
      turns: &[],
      running: c.phase.running,
      rev: c.rev,
      controls: picked.as_ref().unwrap_or(&c.state.controls),
      model_shapes: shapes.as_ref(),
      usage: c.state.usage.as_ref(),
      commands: &c.state.commands,
      queued,
      can_steer: self.can_steer_of(&c),
      subagents: subagents.as_deref(),
      created_at: &self.created_at,
      updated_at: &c.updated_at,
    };
    let encoded_head = raw(&head);
    let full = RawJson::new(&ViewRef { turns: &turns, ..head });
    let parts = ViewParts(Arc::new(ViewPartsData { id: self.id.clone(), rev: c.rev, head: encoded_head, turns, last }));
    (full, c.phase.running, parts)
  }

  /// An owned view, for callers that inspect it (tests, plan lookups)
  pub fn view(&self) -> SessionView {
    serde_json::from_str(self.view_json().0.get()).expect("view round trip")
  }

  pub fn to_record(&self) -> SessionRecord {
    serde_json::from_slice(&self.record_json()).expect("record round trip")
  }

  pub fn subagent_transcript(&self, id: &str) -> Option<(RawJson, i64, bool)> {
    let c = self.core.lock();
    let (turns, rev, running) = c.tree.transcript(id)?;
    Some((RawJson::new(&turns), rev, running))
  }

  pub fn plan_document(&self, plan_id: &str) -> Option<PlanDocumentBlock> {
    let c = self.core.lock();
    visible_turns(&c).filter_map(Turn::as_agent).flat_map(|t| t.blocks.iter()).find_map(|b| match b {
      AgentBlock::PlanDocument(p) if p.id == plan_id => Some(p.clone()),
      _ => None,
    })
  }
}

pub(crate) fn visible_turns(c: &Core) -> impl DoubleEndedIterator<Item = &Turn> + '_ {
  c.state.turns.iter().chain(c.pending_prompt.iter())
}

/// The visible turns: the transcript plus a message accepted below a pre-send compaction
#[derive(Clone, Copy)]
pub(crate) struct TurnsRef<'a> {
  pub turns: &'a [Turn],
  pub pending: Option<&'a Turn>,
}

impl Serialize for TurnsRef<'_> {
  fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeSeq;
    let mut seq = s.serialize_seq(Some(self.turns.len() + usize::from(self.pending.is_some())))?;
    for t in self.turns {
      seq.serialize_element(t)?;
    }
    if let Some(p) = self.pending {
      seq.serialize_element(p)?;
    }
    seq.end()
  }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ViewRef<'a> {
  id: &'a str,
  agent: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  account_id: Option<&'a str>,
  title: String,
  cwd: &'a str,
  status: SessionStatus,
  #[serde(skip_serializing_if = "Option::is_none")]
  error: Option<&'a str>,
  #[serde(skip_serializing_if = "std::ops::Not::not")]
  can_take_over: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  auth_methods: Option<&'a [AuthMethodInfo]>,
  turns: &'a [Box<RawValue>],
  running: bool,
  rev: i64,
  controls: &'a SessionControls,
  #[serde(skip_serializing_if = "Option::is_none")]
  model_shapes: Option<&'a ModelShapes>,
  #[serde(skip_serializing_if = "Option::is_none")]
  usage: Option<&'a Usage>,
  commands: &'a [SlashCommand],
  #[serde(skip_serializing_if = "Option::is_none")]
  queued: Option<Vec<QueuedPrompt>>,
  #[serde(skip_serializing_if = "std::ops::Not::not")]
  can_steer: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  subagents: Option<&'a [SubagentSummary]>,
  created_at: &'a str,
  updated_at: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecordRef<'a> {
  id: &'a str,
  agent: &'a str,
  #[serde(skip_serializing_if = "Option::is_none")]
  account_id: Option<&'a str>,
  #[serde(skip_serializing_if = "Option::is_none")]
  acp_session_id: Option<&'a str>,
  cwd: &'a str,
  title: String,
  created_at: &'a str,
  updated_at: &'a str,
  turns: TurnsRef<'a>,
  controls: &'a SessionControls,
  #[serde(skip_serializing_if = "Option::is_none")]
  usage: Option<&'a Usage>,
  commands: &'a [SlashCommand],
  #[serde(skip_serializing_if = "Option::is_none")]
  pinned: Option<bool>,
  #[serde(skip_serializing_if = "std::ops::Not::not")]
  history_pending: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  forked_from: Option<&'a ForkedFrom>,
  #[serde(skip_serializing_if = "std::ops::Not::not")]
  import_pending: bool,
  #[serde(skip_serializing_if = "Option::is_none")]
  imported_from: Option<&'a ImportedFrom>,
  #[serde(skip_serializing_if = "Option::is_none")]
  subagents: Option<Vec<crate::acp::transcript::subagent_tree::RecordRef<'a>>>,
}

impl RecordSource for AcpSession {
  fn record_id(&self) -> String {
    self.id.clone()
  }

  fn record_json(&self) -> Vec<u8> {
    let c = self.core.lock();
    let r = RecordRef {
      id: &self.id,
      agent: &self.agent,
      account_id: c.account_id.as_deref(),
      acp_session_id: c.acp_session_id.as_deref(),
      cwd: &self.cwd,
      title: Self::title_of(&c),
      created_at: &self.created_at,
      updated_at: &c.updated_at,
      turns: TurnsRef { turns: &c.state.turns, pending: c.pending_prompt.as_ref() },
      controls: &c.state.controls,
      usage: c.state.usage.as_ref(),
      commands: &c.state.commands,
      pinned: c.pinned,
      history_pending: c.lineage.history_pending,
      forked_from: c.lineage.forked_from.as_ref(),
      import_pending: c.lineage.import_pending,
      imported_from: c.lineage.imported_from.as_ref(),
      subagents: (!c.tree.is_empty()).then(|| c.tree.record_refs()),
    };
    serde_json::to_vec(&r).unwrap_or_default()
  }

  fn record(&self) -> SessionRecord {
    self.to_record()
  }
}
