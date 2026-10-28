//! Rows a turn gets besides the agent's own blocks: compaction and retry markers read from prose, session failures,
//! goal changes and ACP notices

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

use acpira_shared::num::Num;
use acpira_shared::transcript::*;

use crate::acp::session::failure::SessionFailure;
use crate::acp::transcript::compaction_text;
use crate::acp::transcript::retry_text;
use crate::i18n::{t, tp};
use crate::json::{str_of, text_of};
use crate::util::now_ms;

use super::{NormalizeState, agent, current_agent_turn, seal};

/// Prose compaction markers become compaction blocks. A start opens a row in place; an outcome settles the latest open
/// prose compaction, which may sit in an earlier turn (Devin / Kimi finish /compact in the background), or stands alone.
/// Detail lines are absorbed only right under a compaction row; false leaves the chunk to render as text
pub(super) fn apply_compaction_markers(s: &mut NormalizeState, i: usize, found: &[compaction_text::Marker]) -> bool {
  use compaction_text::Marker;
  let under_row = matches!(agent(s, i).blocks.last(), Some(AgentBlock::Compaction(_)));
  if found.iter().all(|m| *m == Marker::Detail) {
    return under_row;
  }
  seal(s, i);
  for m in found {
    let (status, error) = match m {
      Marker::Detail => continue,
      // A repeated start while this turn's prose compaction is still open is the same compaction
      Marker::Start if agent(s, i).blocks.iter().any(open_text_compaction) => continue,
      Marker::Start => {
        s.image_seq += 1;
        let id = format!("{TEXT_COMPACTION}{}-{}", now_ms(), s.image_seq);
        agent(s, i).blocks.push(AgentBlock::Compaction(CompactionBlock { id, status: CompactionStatus::InProgress, error: None }));
        continue;
      }
      Marker::Done => (CompactionStatus::Completed, None),
      Marker::Cancelled => (CompactionStatus::Cancelled, None),
      Marker::Failed(e) => (CompactionStatus::Failed, Some(e.clone())),
    };
    let open = s.turns.iter_mut().rev().filter_map(Turn::as_agent_mut).find_map(|t| {
      t.blocks.iter_mut().rev().find(|b| open_text_compaction(b)).and_then(|b| match b {
        AgentBlock::Compaction(c) => Some(c),
        _ => None,
      })
    });
    match open {
      Some(c) => {
        c.status = status;
        c.error = error;
      }
      None => {
        s.image_seq += 1;
        let id = format!("{TEXT_COMPACTION}{}-{}", now_ms(), s.image_seq);
        agent(s, i).blocks.push(AgentBlock::Compaction(CompactionBlock { id, status, error }));
      }
    }
  }
  true
}

/// Id prefix of compaction rows synthesized from prose, so an outcome never settles a structured compaction
const TEXT_COMPACTION: &str = "text-compaction-";

fn open_text_compaction(b: &AgentBlock) -> bool {
  matches!(b, AgentBlock::Compaction(c) if c.status == CompactionStatus::InProgress && c.id.starts_with(TEXT_COMPACTION))
}

pub(super) fn find_compaction<'a>(turns: &'a mut [Turn], id: &str) -> Option<&'a mut CompactionBlock> {
  turns.iter_mut().rev().filter_map(Turn::as_agent_mut).find_map(|t| {
    t.blocks.iter_mut().find_map(|b| match b {
      AgentBlock::Compaction(c) if c.id == id => Some(c),
      _ => None,
    })
  })
}

/// Prose retries (pi-acp) mark the live turn as retrying; the adapter's resume line clears it. Neither is reply text
pub(super) fn apply_retry_marker(s: &mut NormalizeState, m: &retry_text::Marker) {
  use retry_text::Marker;
  let n = |v: u32| v.to_string();
  match *m {
    Marker::Attempt(Some((a, max, wait))) => set_retry(s, TurnRetry {
      attempt: Some(a),
      max: Some(max),
      detail: tp("host.retrying", &[("attempt", &n(a)), ("max", &n(max)), ("seconds", &n(wait))]),
    }),
    Marker::Attempt(None) => set_retry(s, TurnRetry { attempt: None, max: None, detail: t("host.retryingBare") }),
    Marker::Finished => clear_retry(s),
  }
}

/// Retry state belongs to the live turn only; a warning between turns has nothing to label and is dropped
fn set_retry(s: &mut NormalizeState, retry: TurnRetry) {
  match s.turns.last() {
    Some(Turn::Agent(t)) if t.stop.is_none() => {}
    Some(Turn::User(_)) => {
      current_agent_turn(s);
    }
    _ => return,
  }
  if let Some(Turn::Agent(t)) = s.turns.last_mut() {
    t.retry = Some(retry);
  }
}

/// The stream resumed, a real error took over or the turn ended: the retry has served its purpose
pub(super) fn clear_retry(s: &mut NormalizeState) {
  if let Some(Turn::Agent(t)) = s.turns.last_mut() {
    t.retry = None;
  }
}

static ATTEMPT_OF: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)\battempt (\d+) of (\d+)\b"));

/// claude-agent-acp 0.81.0 words its api_retry title "Reconnecting to Claude, attempt N of M." / "Retrying Claude, …"
fn retry_of_failure(f: &SessionFailure) -> TurnRetry {
  let c = ATTEMPT_OF.captures(&f.title);
  let num = |i: usize| c.as_ref().and_then(|c| c.get(i)).and_then(|m| m.as_str().parse().ok());
  let detail = match &f.details {
    Some(d) => format!("{}\n{d}", f.title),
    None => f.title.clone(),
  };
  TurnRetry { attempt: num(1), max: num(2), detail }
}

/// An AIR warning that only reports a retry in progress (claude-agent-acp `api_retry`): no remedy to offer and a
/// transient cause. Advisories (category `unknown`) and anything with actions stay transcript rows
fn is_retry_warning(f: &SessionFailure) -> bool {
  f.severity == Severity::Warning
    && f.actions.is_empty()
    && matches!(f.category, FailureCategory::Connection | FailureCategory::Service | FailureCategory::Limit)
}

fn find_notice<'a>(turns: &'a mut [Turn], id: &str) -> Option<&'a mut NoticeBlock> {
  turns.iter_mut().rev().filter_map(Turn::as_agent_mut).find_map(|t| {
    t.blocks.iter_mut().find_map(|b| match b {
      AgentBlock::Notice(n) if n.id == id => Some(n),
      _ => None,
    })
  })
}

/// One row per failure id in the whole transcript; a higher revision rewrites it in place. Retry warnings are no row
/// at all: they only label the live turn (claude-agent-acp 0.81.0 even mints a new session-scoped id per attempt),
/// and an error clears that label
pub fn apply_session_failure(s: &mut NormalizeState, f: &SessionFailure) -> bool {
  if is_retry_warning(f) {
    set_retry(s, retry_of_failure(f));
    return true;
  }
  if f.severity == Severity::Error {
    clear_retry(s);
  }
  if let Some(n) = find_notice(&mut s.turns, &f.id) {
    if n.revision.0 >= f.revision {
      return false;
    }
    n.revision = Num(f.revision);
    n.category = f.category;
    n.severity = f.severity;
    n.title = f.title.clone();
    n.details = f.details.clone();
    n.actions = f.actions.clone();
    return true;
  }
  let notice = AgentBlock::Notice(NoticeBlock {
    id: f.id.clone(),
    revision: Num(f.revision),
    category: f.category,
    severity: f.severity,
    info: false,
    title: f.title.clone(),
    details: f.details.clone().filter(|d| !d.is_empty()),
    actions: f.actions.clone(),
  });
  push_standalone(s, notice);
  true
}

/// A goal snapshot replaces the session's goal; a status or objective change also leaves a row on the last agent turn
/// (the live one, or the last settled one for a control sent while idle). `mark` is off while a history replays
pub fn apply_goal(s: &mut NormalizeState, next: Option<SessionGoal>, mark: bool) -> bool {
  if s.goal == next {
    return false;
  }
  let block = if mark { crate::acp::vendors::goal::event_of(s.goal.as_ref(), next.as_ref()) } else { None };
  s.goal = next;
  if let Some(b) = block {
    if matches!(s.turns.last(), Some(Turn::Agent(_))) {
      let i = s.turns.len() - 1;
      seal(s, i);
    }
    push_standalone(s, AgentBlock::Goal(b));
  }
  true
}

/// An ACP `notice` (RFD session-notices; claude-agent-acp 0.93.0, codex-acp 2.2.2): fire-and-forget information with
/// no id and no remedy, so it becomes a notice row of its own on the last agent turn, live or settled. It never counts as
/// conversation content: a stopped background task's acknowledgement after `end_turn` must not reopen the turn
pub(super) fn apply_notice(s: &mut NormalizeState, u: &Value) -> bool {
  let Some(title) = text_of(u, "title").map(str::trim).filter(|t| !t.is_empty()) else { return false };
  // An unknown severity string reads as information, the quietest level
  let (severity, info) = match str_of(u, "severity") {
    Some("error") => (Severity::Error, false),
    Some("warning") => (Severity::Warning, false),
    _ => (Severity::Warning, true),
  };
  // Positional ids stay stable across a reload of the same transcript; AIR rows carry their own ids
  let n = s.turns.iter().filter_map(Turn::as_agent).flat_map(|t| &t.blocks).filter(|b| matches!(b, AgentBlock::Notice(_))).count();
  let notice = AgentBlock::Notice(NoticeBlock {
    id: format!("notice:{}", n + 1),
    revision: Num(1.0),
    category: FailureCategory::Unknown,
    severity,
    info,
    title: title.to_owned(),
    details: text_of(u, "description").map(str::trim).filter(|d| !d.is_empty()).map(str::to_owned),
    actions: vec![],
  });
  // Text streamed before the notice stays above it; what follows starts a new block
  if matches!(s.turns.last(), Some(Turn::Agent(_))) {
    let i = s.turns.len() - 1;
    seal(s, i);
  }
  push_standalone(s, notice);
  true
}

// A block that lands outside a live turn: on the last agent turn, or a new settled one
pub(super) fn push_standalone(s: &mut NormalizeState, block: AgentBlock) {
  if let Some(Turn::Agent(last)) = s.turns.last_mut() {
    last.blocks.push(block);
  } else {
    s.turns.push(Turn::Agent(AgentTurn { blocks: vec![block], stop: Some(TurnStop::EndTurn), ..Default::default() }));
  }
}
