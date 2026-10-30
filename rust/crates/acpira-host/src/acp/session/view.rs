//! What a session shows and stores: the list entry and state dot, the view every viewer renders, and the record the
//! store writes, both serialized straight from `Core` without cloning the transcript

use std::sync::Arc;

use acpira_shared::protocol::{HostMsg, RawJson};
use acpira_shared::session_patch::{ViewParts, ViewPartsData, encode_turns, raw};
use acpira_shared::subagents::SubagentSummary;
use acpira_shared::model_shapes::{ModelShapes, session_model};
use acpira_shared::transcript::*;
use serde::Serialize;
use serde_json::value::RawValue;

use crate::acp::session::controls::picked_controls;
use crate::acp::session::queue::queue_snapshot;
use crate::acp::transcript::normalize::has_live_async_task;
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
      created_at: Some(self.created_at.clone()).filter(|c| !c.is_empty()),
      pinned: c.pinned,
      archived: c.archived,
      category: c.category.clone(),
      state: None,
      unread: None,
      question: None,
      completed_at: completed_at(visible_turns(&c)),
      model: session_model(&c.state.controls),
    }
  }

  /// The session list's state dot: waiting on the user, working, ended in error, or ended with background work still
  /// running. A permission card or question keeps its turn running, so waiting is checked first: otherwise a turn blocked
  /// on the user shows the same spinner as one at work
  pub fn list_state(&self) -> Option<SummaryState> {
    self.list_marks().0
  }

  /// `SessionSummary::completed_at` now, before the record carrying it is written
  pub fn completed_at(&self) -> Option<i64> {
    completed_at(visible_turns(&self.core.lock()))
  }

  /// `list_state`, and beside `Waiting` whether only questions wait (`SessionSummary::question`)
  pub fn list_marks(&self) -> (Option<SummaryState>, bool) {
    let c = self.core.lock();
    // A child's card (a summoned one may ask long after the root turn ended) waits on the user just the same
    let blocks = || visible_turns(&c).filter_map(Turn::as_agent).flat_map(|a| a.blocks.iter());
    let permission = blocks().any(|b| matches!(b, AgentBlock::Permission(_))) || c.perms.pending.iter().any(|p| p.node_id.is_some());
    let question = blocks().any(|b| matches!(b, AgentBlock::Question(q) if q.outcome.is_none())) || c.questions.pending.iter().any(|q| q.node_id.is_some());
    if permission || question {
      return (Some(SummaryState::Waiting), !permission);
    }
    if c.phase.running {
      return (Some(SummaryState::Working), false);
    }
    let state = match visible_turns(&c).last() {
      Some(Turn::Agent(a)) if a.stop == Some(TurnStop::Error) => Some(SummaryState::Error),
      _ if has_live_async_task(&c.state) || Self::relay_background(&c) => Some(SummaryState::Background),
      _ => None,
    };
    (state, false)
  }

  /// What a notification about this session says below its title (`SessionManager::watch_attention`): the open card's
  /// request while one waits, else the last turn's error, else its reply's opening. Plain text; empty when there is none
  pub fn attention_detail(&self, waiting: bool) -> String {
    let c = self.core.lock();
    let mut agent_turns = visible_turns(&c).rev().filter_map(Turn::as_agent);
    if waiting {
      let card = agent_turns.flat_map(|a| a.blocks.iter().rev()).find_map(|b| match b {
        AgentBlock::Permission(p) => Some(match p.command.as_deref().filter(|cmd| !p.title.contains(cmd)) {
          Some(cmd) => format!("{}\n{cmd}", p.title),
          None => p.title.clone(),
        }),
        AgentBlock::Question(q) if q.outcome.is_none() => q.questions.first().map(|x| x.text.clone()).or_else(|| q.message.clone()),
        _ => None,
      });
      return card.map(|t| plain_opening(&t)).unwrap_or_default();
    }
    let Some(last) = agent_turns.next() else { return String::new() };
    if last.stop == Some(TurnStop::Error)
      && let Some(e) = &last.error
    {
      return plain_opening(&e.message);
    }
    last.blocks.iter().rev().find_map(|b| match b {
      AgentBlock::Text(t) if !t.markdown.trim().is_empty() => Some(plain_opening(&t.markdown)),
      _ => None,
    }).unwrap_or_default()
  }

  /// A background task the agent reported (AIR `asyncTasks`: Claude's background Bash, Monitor, workflows) still runs and
  /// is observed, or a summoned child the root turn left running. A task whose process went away is marked unknown on
  /// disconnect, so this cannot outlive the agent
  pub fn background_running(&self) -> bool {
    let c = self.core.lock();
    has_live_async_task(&c.state) || Self::relay_background(&c)
  }

  /// Still inside a turn: running, a prompt waiting on its pre-send compaction, or the workspace gate deciding about a follow-up
  pub fn turn_busy(&self) -> bool {
    let c = self.core.lock();
    c.phase.running || c.pending_prompt.is_some() || c.hooks.gating
  }

  /// How the last visible agent turn stopped; None when the transcript ends on a user turn or the turn has no stop yet
  pub fn last_stop(&self) -> Option<TurnStop> {
    let c = self.core.lock();
    match visible_turns(&c).last() {
      Some(Turn::Agent(a)) => a.stop,
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
    if let Some((rev, status, running, ref cached_shapes, ref full, ref parts)) = c.cached_view
      && rev == c.rev
      && status == c.status
      && running == c.phase.running
      && *cached_shapes == shapes
    {
      return (full.clone(), running, parts.clone());
    }
    // Each visible turn is encoded on its own, the last one also block by block when it is an agent turn; the whole view
    // is spliced from the same fragments, so the transcript is still serialized once per change. Diff sources go without
    // their file texts (`encode_for_view`): the page asks for them when it needs them
    let (turns, last) = encode_for_view(|| {
      let core = &mut *c;
      match core.pending_prompt.as_mut() {
        Some(p) => encode_turns(&core.state.turns.iter().collect::<Vec<_>>(), Some(p)),
        None => match core.state.turns.split_last_mut() {
          Some((last, before)) => encode_turns(&before.iter().collect::<Vec<_>>(), Some(last)),
          None => (vec![], None),
        },
      }
    });
    let subagents = if c.tree.is_empty() { None } else { Some(c.tree.summaries()) };
    let picked = (!c.picks.values.is_empty()).then(|| picked_controls(&c));
    let queued = queue_snapshot(&c);
    // The plugins' slash commands join the agent's own in the menu
    let merged = self.plugin_commands(&c.state.commands);
    let head = ViewRef {
      id: &self.id,
      agent: &self.agent,
      account_id: c.account_id.as_deref(),
      title: Self::title_of(&c),
      cwd: &self.cwd,
      status: c.status,
      error: c.error.as_deref(),
      can_take_over: (c.status == SessionStatus::Error && c.lock_holder.is_some()) || (c.status == SessionStatus::Readonly && c.elsewhere),
      auth_methods: c.auth_methods.as_deref(),
      turns: &[],
      running: c.phase.running,
      rev: c.rev,
      controls: picked.as_ref().unwrap_or(&c.state.controls),
      model_shapes: shapes.as_ref(),
      usage: c.state.usage.as_ref(),
      commands: merged.as_deref().unwrap_or(&c.state.commands),
      queued,
      can_steer: self.can_steer_of(&c),
      goal: c.state.goal.as_ref(),
      goal_actions: Self::goal_actions_of(&c),
      subagents: subagents.as_deref(),
      forked_from: c.lineage.forked_from.as_ref().filter(|f| f.turn_index >= 0).map(|f| ForkOrigin {
        session_id: f.session_id.clone(),
        turn_index: f.turn_index,
        agent: f.agent.clone(),
      }),
      created_at: &self.created_at,
      updated_at: &c.updated_at,
    };
    let encoded_head = raw(&head);
    let full = RawJson::new(&ViewRef { turns: &turns, ..head });
    let parts = ViewParts(Arc::new(ViewPartsData { id: self.id.clone(), epoch: self.epoch, rev: c.rev, head: encoded_head, turns, last }));
    let running = c.phase.running;
    c.cached_view = Some((c.rev, c.status, running, shapes, full.clone(), parts.clone()));
    (full, running, parts)
  }

  /// An owned view, for callers that inspect it (tests, plan lookups)
  #[expect(clippy::expect_used, reason = "reads back this build's own serialization of the same type; its numbers all came from finite JSON numbers")]
  pub fn view(&self) -> SessionView {
    serde_json::from_str(self.view_json().0.get()).expect("view round trip")
  }

  #[expect(clippy::expect_used, reason = "reads back this build's own serialization of the same type; its numbers all came from finite JSON numbers")]
  pub fn to_record(&self) -> SessionRecord {
    serde_json::from_slice(&self.record_json()).expect("record round trip")
  }

  pub fn subagent_transcript(&self, id: &str) -> Option<(RawJson, i64, bool)> {
    let c = self.core.lock();
    let (turns, rev, running) = c.tree.transcript(id)?;
    Some((encode_for_view(|| RawJson::new(&turns)), rev, running))
  }

  /// The whole files behind the `nth` diff of tool call `tool_call_id` (counted in `contents`, or `content` when that is
  /// the only item, as the page lists them), the texts a page view leaves out. The newest matching call wins. Only the
  /// transcript the diff was shown in is searched: the session's own turns, or subagent `subagent`'s when given, since a
  /// child's tool call ids may repeat the parent's (or another child's)
  pub fn diff_source(&self, tool_call_id: &str, nth: usize, subagent: Option<&str>) -> Option<DiffSource> {
    let c = self.core.lock();
    let find = |turns: &mut dyn DoubleEndedIterator<Item = &Turn>| {
      turns.rev().filter_map(Turn::as_agent).flat_map(|t| t.blocks.iter().rev()).find_map(|b| match b {
        AgentBlock::ToolCall(tool) if tool.id == tool_call_id => {
          let items = tool.contents.as_deref().unwrap_or(std::slice::from_ref(tool.content.as_ref()?));
          items
            .iter()
            .filter_map(|item| match item {
              ToolContent::Diff { source, .. } => Some(source),
              _ => None,
            })
            .nth(nth)?
            .clone()
        }
        _ => None,
      })
    };
    match subagent {
      Some(id) => find(&mut c.tree.transcript(id)?.0.iter()),
      None => find(&mut visible_turns(&c)),
    }
  }

  /// Every file the session's own edits touched, in the order it first edited them, with the text it held before that
  /// first edit (None when the record no longer keeps it). Subagents' edits are not counted: they live in their own
  /// transcripts
  pub fn edit_bases(&self) -> Vec<(String, Option<String>)> {
    let c = self.core.lock();
    let mut out: Vec<(String, Option<String>)> = vec![];
    for tool in visible_turns(&c).filter_map(Turn::as_agent).flat_map(|t| t.blocks.iter()).filter_map(|b| match b {
      AgentBlock::ToolCall(tool) => Some(tool),
      _ => None,
    }) {
      let items = match (tool.contents.as_deref(), tool.content.as_ref()) {
        (Some(all), _) => all,
        (None, Some(one)) => std::slice::from_ref(one),
        (None, None) => continue,
      };
      for item in items {
        let ToolContent::Diff { source: Some(source), .. } = item else { continue };
        if source.path.is_empty() || out.iter().any(|(p, _)| *p == source.path) {
          continue;
        }
        out.push((source.path.clone(), (!source.omitted).then(|| source.old_text.clone())));
      }
    }
    out
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
  goal: Option<&'a SessionGoal>,
  #[serde(skip_serializing_if = "Option::is_none")]
  goal_actions: Option<Vec<GoalAction>>,
  #[serde(skip_serializing_if = "Option::is_none")]
  subagents: Option<&'a [SubagentSummary]>,
  #[serde(skip_serializing_if = "Option::is_none")]
  forked_from: Option<ForkOrigin>,
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
  #[serde(skip_serializing_if = "Option::is_none")]
  archived: Option<bool>,
  #[serde(skip_serializing_if = "Option::is_none")]
  category: Option<&'a str>,
  #[serde(skip_serializing_if = "std::ops::Not::not")]
  renamed: bool,
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
  #[serde(skip_serializing_if = "Option::is_none")]
  goal: Option<&'a SessionGoal>,
}

impl RecordSource for AcpSession {
  fn record_id(&self) -> String {
    self.id.clone()
  }

  fn record_cwd(&self) -> String {
    self.cwd.clone()
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
      archived: c.archived,
      category: c.category.as_deref(),
      renamed: c.lineage.renamed,
      history_pending: c.lineage.history_pending,
      forked_from: c.lineage.forked_from.as_ref(),
      import_pending: c.lineage.import_pending,
      imported_from: c.lineage.imported_from.as_ref(),
      subagents: (!c.tree.is_empty()).then(|| c.tree.record_refs()),
      goal: c.state.goal.as_ref(),
    };
    serde_json::to_vec(&r).unwrap_or_default()
  }

  fn record(&self) -> SessionRecord {
    self.to_record()
  }
}

/// The opening of a Markdown text as plain lines for a notification: code blocks, markers and emphasis dropped, at most
/// a few lines and a few hundred characters (the gateway clips again)
pub(crate) fn plain_opening(md: &str) -> String {
  const LINES: usize = 4;
  const CHARS: usize = 400;
  let mut out: Vec<String> = Vec::new();
  let mut fenced = false;
  for line in md.lines() {
    let t = line.trim();
    if t.starts_with("```") || t.starts_with("~~~") {
      fenced = !fenced;
      continue;
    }
    if fenced || t.is_empty() || t.chars().all(|c| matches!(c, '-' | '*' | '_' | '=' | '|' | ':' | ' ')) {
      continue;
    }
    let t = t.trim_start_matches(['#', '>']).trim_start();
    let t = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")).unwrap_or(t);
    let t: String = t.replace("**", "").replace("__", "").replace('`', "");
    if !t.trim().is_empty() {
      out.push(t.trim().to_owned());
    }
    if out.len() >= LINES {
      break;
    }
  }
  let text = out.join("\n");
  if text.chars().count() <= CHARS { text } else { text.chars().take(CHARS - 1).chain(['…']).collect() }
}

#[cfg(test)]
mod attention_tests {
  use super::plain_opening;

  #[test]
  fn a_reply_opens_as_plain_lines() {
    let md = "## Done\n\nFixed **the build** in `main.rs`.\n\n```rust\nfn main() {}\n```\n- tests pass\n---\n> note";
    assert_eq!(plain_opening(md), "Done\nFixed the build in main.rs.\ntests pass\nnote");
    assert_eq!(plain_opening(&"x".repeat(900)).chars().count(), 400);
    assert_eq!(plain_opening("```\nonly code\n```"), "");
  }
}
