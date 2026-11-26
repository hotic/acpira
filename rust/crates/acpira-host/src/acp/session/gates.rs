//! Permission and question cards of a session. A request lands a card in the owning transcript (root or a child's), the answer travels back through
//! a oneshot; the agent withdrawing its request (the connection's cancel signal) closes the card

use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};
use tokio::sync::oneshot;

use acpira_shared::transcript::*;

use crate::acp::session::updates::current_turn_index;
use crate::acp::session::{AcpSession, Core};
use crate::acp::transcript::normalize::{NormalizeState, activity_of};
use crate::acp::transcript::subagent_tree::RouteCtx;
use crate::acp::vendors::{Vendor, antigravity};

mod permissions;
mod questions;

use self::permissions::remove_perm_blocks_except;

pub(crate) struct PendingPermission {
  pub tx: oneshot::Sender<Value>,
  pub block_id: String,
  pub options: Vec<Value>,
  pub plan_id: Option<String>,
  pub node_id: Option<String>,
  /// A plugin's `permission` hook asked for a card: no later switch to yolo answers it
  pub insist: bool,
}

pub(crate) enum QuestionReply {
  /// `images`: the request takes images with the answers (`_meta["acpira/answerImages"]`, the built-in agent's)
  Form { schema: Value, images: bool },
  Grok,
  /// A `session/request_permission` asked as a question (`Vendor::question_permission`): the answer is the picked option
  Permission,
  /// A summoned child's `ask_caller` (`relay.rs`): the answers as given, or skipped
  Caller,
}

pub(crate) struct PendingQuestion {
  pub tx: oneshot::Sender<Value>,
  pub block_id: String,
  pub questions: Vec<Question>,
  pub node_id: Option<String>,
  pub reply: QuestionReply,
}

/// Open permission cards
#[derive(Default)]
pub(crate) struct PermissionGate {
  pub pending: Vec<PendingPermission>,
  pub seq: u64,
  /// Bumped when the turn or process a request arrived in is gone: a request still being prepared under an older epoch
  /// is answered cancelled instead of opening a card
  pub epoch: u64,
  /// yolo mode: requests are answered with their best allow option instead of a card, unless that choice is ambiguous
  /// (`auto_allow`)
  pub auto_approve: bool,
}

/// Open question cards (elicitation forms, Grok interviews)
#[derive(Default)]
pub(crate) struct QuestionGate {
  pub pending: Vec<PendingQuestion>,
  /// Summoned children's cards only their caller is asked about: the user is not, until that caller cannot answer
  /// (`relay_escalate`)
  pub callers: HashSet<String>,
  pub seq: u64,
  pub raw: crate::acp::transcript::questions::RawMemory,
}

/// Which transcript a request belongs to
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) enum Target {
  Root,
  Node(String),
}

impl Target {
  fn node_id(&self) -> Option<String> {
    match self {
      Target::Root => None,
      Target::Node(id) => Some(id.clone()),
    }
  }
}

fn cancelled_permission() -> Value {
  json!({ "outcome": { "outcome": "cancelled" } })
}

impl AcpSession {
  /// A request addresses a session id: root → root state, a live child peer id → that node's transcript
  pub(crate) fn target_for(c: &mut Core, session_id: Option<&str>) -> Option<Target> {
    match session_id {
      None => Some(Target::Root),
      Some(s) if c.acp_session_id.is_none() || c.acp_session_id.as_deref() == Some(s) => Some(Target::Root),
      Some(s) => c.tree.state_for_peer(s).map(|(_, id)| Target::Node(id)),
    }
  }

  /// Like `target_for`, additionally routing Antigravity child requests whose `toolCallId` carries `<child_uuid>:` on
  /// the root session into that child's node
  pub(crate) fn target_for_call(&self, c: &mut Core, vendor: Vendor, session_id: Option<&str>, tool_call_id: Option<&str>) -> Option<Target> {
    let target = Self::target_for(c, session_id)?;
    if target == Target::Root
      && vendor == Vendor::Antigravity
      && let Some(tid) = tool_call_id
      && let Some((child_cid, _)) = antigravity::split_child_tool_call_id(tid, c.acp_session_id.as_deref())
    {
      let turn_index = current_turn_index(c);
      let node_id = {
        let Core { tree, state, .. } = &mut *c;
        tree.ensure_antigravity_child(child_cid, &mut RouteCtx { turn_index, root_turns: &mut state.turns })
      }?;
      self.schedule_antigravity_subagents(c, true);
      return Some(Target::Node(node_id));
    }
    // A call of an orchestrator's executor asks inside its round (`orchestrator.rs`)
    if target == Target::Root
      && vendor == Vendor::Antigravity
      && let Some(node_id) = tool_call_id.and_then(|tid| c.tree.orchestrator_owner(tid))
    {
      return Some(Target::Node(node_id));
    }
    Some(target)
  }

  pub(crate) fn target_state<'a>(c: &'a mut Core, t: &Target) -> Option<&'a mut NormalizeState> {
    match t {
      Target::Root => Some(&mut c.state),
      Target::Node(id) => c.tree.state_of(id),
    }
  }

  fn bump_target(c: &mut Core, t: &Target) {
    if let Target::Node(id) = t {
      c.tree.bump(id);
    }
  }

  /// Nodes that went terminal close their pending cards as cancelled (RFD)
  pub(crate) fn drain_terminal(&self, c: &mut Core) {
    let ids = std::mem::take(&mut c.tree.terminal_events);
    for id in ids {
      self.cancel_permissions_for(c, &id);
      self.cancel_questions_for(c, &id);
    }
  }

  /// A root turn settled: the cards of the root and of its native children go. A summoned child's outlive it (its round
  /// may run on after the call returned); they close with its node (`drain_terminal`, `relay_cancel`)
  pub(crate) fn cancel_turn_cards(&self, c: &mut Core) {
    let mut i = 0;
    while i < c.perms.pending.len() {
      if c.perms.pending[i].node_id.as_deref().is_some_and(|id| c.tree.relay_thread_of(id).is_some()) {
        i += 1;
        continue;
      }
      let p = c.perms.pending.remove(i);
      let _ = p.tx.send(cancelled_permission());
    }
    let mut i = 0;
    while i < c.questions.pending.len() {
      if c.questions.pending[i].node_id.as_deref().is_some_and(|id| c.tree.relay_thread_of(id).is_some()) {
        i += 1;
        continue;
      }
      let p = c.questions.pending.remove(i);
      settle_question(c, &p, QuestionOutcome::Cancelled, None, None);
      let reply = cancel_reply(&p);
      let _ = p.tx.send(reply);
    }
    // Every card block without a waiting request goes, summoned children's still waiting stay
    let keep: Vec<String> = c.perms.pending.iter().map(|p| p.block_id.clone()).collect();
    remove_perm_blocks_except(c, &keep);
  }
}

/// The `elicitation/create` `_meta` key of a form that takes images with its answers, and of the answer that carries them
const ANSWER_IMAGES: &str = "acpira/answerImages";

/// Images one answer may carry: a request holds every answer's pictures at once
const MAX_ANSWER_IMAGES: usize = 4;

fn cancel_reply(p: &PendingQuestion) -> Value {
  match p.reply {
    QuestionReply::Grok => json!({ "outcome": "skip_interview" }),
    QuestionReply::Form { .. } => json!({ "action": "cancel" }),
    QuestionReply::Permission => cancelled_permission(),
    QuestionReply::Caller => json!({ "cancelled": true }),
  }
}

/// A card's outcome recorded on its block, in the transcript of the node that owns it
fn settle_question(
  c: &mut Core,
  p: &PendingQuestion,
  outcome: QuestionOutcome,
  answers: Option<&QuestionAnswers>,
  images: Option<BTreeMap<String, Vec<ImageRef>>>,
) {
  c.questions.callers.remove(&p.block_id);
  let target = match &p.node_id {
    None => Target::Root,
    Some(id) => Target::Node(id.clone()),
  };
  let Some(state) = AcpSession::target_state(c, &target) else { return };
  for ti in (0..state.turns.len()).rev() {
    let found = match &mut state.turns[ti] {
      Turn::Agent(turn) => turn.blocks.iter_mut().find_map(|b| match b {
        AgentBlock::Question(q) if q.id == p.block_id => Some(q),
        _ => None,
      }),
      Turn::User(_) => None,
    };
    let Some(q) = found else { continue };
    q.outcome = Some(outcome);
    if let Some(a) = answers.filter(|a| !a.is_empty()) {
      q.answers = Some(a.clone());
    }
    if images.is_some() {
      q.answer_images = images;
    }
    let activity = activity_of(&state.turns);
    if let Some(Turn::Agent(turn)) = state.turns.get_mut(ti) {
      turn.activity = activity;
    }
    AcpSession::bump_target_pub(c, &target);
    return;
  }
}

impl AcpSession {
  pub(crate) fn bump_target_pub(c: &mut Core, t: &Target) {
    Self::bump_target(c, t);
  }
}

fn state_of_block(c: &mut Core, block_id: &str) -> Option<Target> {
  let has = |st: &NormalizeState| {
    st.turns.iter().filter_map(Turn::as_agent).any(|t| t.blocks.iter().any(|b| matches!(b, AgentBlock::Permission(p) if p.id == block_id)))
  };
  if has(&c.state) {
    return Some(Target::Root);
  }
  c.tree.states_mut().find(|(_, st)| has(st)).map(|(id, _)| Target::Node(id.to_owned()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use super::permissions::{default_to_no, single_allow_once};

  #[test]
  fn plan_auto_approval_takes_only_a_lone_allow_once() {
    let opts = |kinds: &[(&str, &str)]| kinds.iter().map(|(id, k)| json!({ "optionId": id, "kind": k })).collect::<Vec<_>>();
    // Claude's Bash ask: Yes / Yes, don't ask again / No → Yes, never the lasting rule
    assert_eq!(single_allow_once(&opts(&[("yes", "allow_once"), ("always", "allow_always"), ("no", "reject_once")])).as_deref(), Some("yes"));
    // Answers rather than a yes, or no plain yes at all, stay with a person
    assert_eq!(single_allow_once(&opts(&[("staging", "allow_once"), ("prod", "allow_once"), ("no", "reject_once")])), None);
    assert_eq!(single_allow_once(&opts(&[("always", "allow_always"), ("no", "reject_once")])), None);
  }

  #[test]
  fn default_to_no_reads_the_adapter_permission_meta() {
    assert_eq!(default_to_no(&json!({ "_meta": { "permission": { "version": 1, "defaultToNo": true } } })), Some(true));
    assert_eq!(default_to_no(&json!({ "_meta": { "permission": { "defaultToNo": false } } })), None);
    assert_eq!(default_to_no(&json!({})), None);
  }
}
