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

/// Prose retries (pi-acp) mark the live turn as retrying and log the adapter's line; the resume line clears the label.
/// Neither is reply text. pi-acp 0.0.33 sends the resume line even when the last attempt failed, so it leaves the
/// retry row open: the next content settles it as recovered, the end of the turn as failed
pub(super) fn apply_retry_marker(s: &mut NormalizeState, m: &retry_text::Marker, line: &str) {
  use retry_text::Marker;
  let n = |v: u32| v.to_string();
  match *m {
    Marker::Attempt(Some((a, max, wait))) => {
      set_retry(s, TurnRetry {
        attempt: Some(a),
        max: Some(max),
        detail: tp("host.retrying", &[("attempt", &n(a)), ("max", &n(max)), ("seconds", &n(wait))]),
      });
      report_retry(s, RetryReport {
        attempt: Some(a), max: Some(max), text: line.trim().to_owned(), quality: RetryText::Raw, reconnect: false,
        wait_ms: Some(u64::from(wait) * 1000),
      });
    }
    Marker::Attempt(None) => {
      set_retry(s, TurnRetry { attempt: None, max: None, detail: t("host.retryingBare") });
      report_retry(s, RetryReport {
        attempt: None, max: None, text: line.trim().to_owned(), quality: RetryText::Raw, reconnect: false, wait_ms: None,
      });
    }
    Marker::Finished => clear_label(s),
  }
}

/// Retry state belongs to the live turn only; a warning between turns has nothing to label and is dropped
fn set_retry(s: &mut NormalizeState, retry: TurnRetry) {
  if !live_turn(s) {
    return;
  }
  if let Some(Turn::Agent(t)) = s.turns.last_mut() {
    t.retry = Some(retry);
  }
}

/// Whether the transcript ends in a live agent turn, opening one after a user message
fn live_turn(s: &mut NormalizeState) -> bool {
  match s.turns.last() {
    Some(Turn::Agent(t)) if t.stop.is_none() => true,
    Some(Turn::User(_)) => {
      current_agent_turn(s);
      true
    }
    _ => false,
  }
}

fn clear_label(s: &mut NormalizeState) {
  if let Some(Turn::Agent(t)) = s.turns.last_mut() {
    t.retry = None;
  }
}

/// The stream resumed: the retries worked, so the label goes and the open retry row reads as recovered
pub(super) fn clear_retry(s: &mut NormalizeState) {
  clear_label(s);
  settle_retry(s, ToolStatus::Completed);
}

/// Closes the live turn's open retry row, if any
pub(super) fn settle_retry(s: &mut NormalizeState, status: ToolStatus) {
  if let Some(log) = open_retry(&mut s.turns).and_then(|n| n.retry.as_mut()) {
    log.status = status;
    log.next_at = None;
  }
}

/// The open retry row of the last agent turn
fn open_retry(turns: &mut [Turn]) -> Option<&mut NoticeBlock> {
  let Some(Turn::Agent(t)) = turns.last_mut() else { return None };
  t.blocks.iter_mut().rev().find_map(|b| match b {
    AgentBlock::Notice(n) if n.retry.as_ref().is_some_and(|r| r.status == ToolStatus::InProgress) => Some(n),
    _ => None,
  })
}

/// How much a report says about its attempt, so a vaguer report of the same attempt never overwrites a better one.
/// claude-agent-acp's AIR warning names only the attempt, its raw SDK frame adds the status and error kind, and the
/// CLI's session log holds the response body
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RetryText {
  Title,
  Summary,
  Raw,
}

/// One failed attempt as a source reported it
#[derive(Debug, Clone)]
pub struct RetryReport {
  pub attempt: Option<u32>,
  pub max: Option<u32>,
  pub text: String,
  pub quality: RetryText,
  pub reconnect: bool,
  /// How long until the next attempt
  pub wait_ms: Option<u64>,
}

/// Attempts a retry row keeps (the newest), and the text kept per attempt; a gateway's HTML error page can be long
const MAX_ATTEMPTS: usize = 50;
const MAX_TEXT: usize = 4000;

/// Logs a failed attempt on the live turn's retry row, opening one at the current position on the first failure. The
/// same attempt reported twice (an AIR warning and a raw SDK frame) is one entry, keeping the better text
pub fn report_retry(s: &mut NormalizeState, r: RetryReport) -> bool {
  if !live_turn(s) {
    return false;
  }
  let now = now_ms();
  if open_retry(&mut s.turns).is_none() {
    let i = s.turns.len() - 1;
    seal(s, i);
    s.retry_quality.clear();
    push_standalone(s, AgentBlock::Notice(NoticeBlock {
      id: format!("retry:{}", crate::util::random_uuid()),
      revision: Num(1.0),
      category: FailureCategory::Service,
      severity: Severity::Warning,
      info: false,
      // What an engine or app that does not know `retry` shows: a warning with the latest error under it
      title: t("host.retryingBare"),
      details: None,
      actions: vec![],
      retry: Some(RetryLog { status: ToolStatus::InProgress, max: None, next_at: None, reconnect: false, attempts: vec![] }),
    }));
  }
  let NormalizeState { turns, retry_quality, .. } = s;
  let Some(n) = open_retry(turns) else { return false };
  let Some(log) = n.retry.as_mut() else { return false };
  let text = clip(r.text.trim());
  let same = r.attempt.and_then(|a| log.attempts.iter().position(|x| x.attempt == Some(a)));
  match same {
    Some(k) => {
      let a = r.attempt.unwrap_or_default();
      let had = retry_quality.iter().find(|(n, _)| *n == a).map(|(_, q)| *q).unwrap_or(RetryText::Title);
      if r.quality >= had && !text.is_empty() {
        log.attempts[k].text = text;
        retry_quality.retain(|(n, _)| *n != a);
        retry_quality.push((a, r.quality));
      }
      log.reconnect |= r.reconnect;
    }
    None => {
      log.attempts.push(RetryAttempt { at: Num(now as f64), attempt: r.attempt, text });
      if let Some(a) = r.attempt {
        retry_quality.push((a, r.quality));
      }
      if log.attempts.len() > MAX_ATTEMPTS {
        log.attempts.remove(0);
      }
      log.reconnect = r.reconnect;
    }
  }
  if r.max.is_some() {
    log.max = r.max;
  }
  if let Some(w) = r.wait_ms {
    log.next_at = Some(Num(now.saturating_add(i64::try_from(w).unwrap_or(i64::MAX)) as f64));
  }
  n.category = if log.reconnect { FailureCategory::Connection } else { FailureCategory::Service };
  n.details = log.attempts.last().map(|x| x.text.clone()).filter(|d| !d.is_empty());
  true
}

/// A better text for an attempt already logged (the CLI's session log, read after the frame), on the live turn's latest
/// retry row whether or not it has settled meanwhile. Never opens a row
pub fn refine_retry(s: &mut NormalizeState, attempt: u32, text: &str, quality: RetryText) -> bool {
  let NormalizeState { turns, retry_quality, .. } = s;
  let Some(Turn::Agent(t)) = turns.last_mut() else { return false };
  let Some(log) = t.blocks.iter_mut().rev().find_map(|b| match b {
    AgentBlock::Notice(NoticeBlock { retry: Some(r), .. }) => Some(r),
    _ => None,
  }) else {
    return false;
  };
  let Some(entry) = log.attempts.iter_mut().find(|x| x.attempt == Some(attempt)) else { return false };
  let had = retry_quality.iter().find(|(n, _)| *n == attempt).map(|(_, q)| *q).unwrap_or(RetryText::Title);
  let text = clip(text.trim());
  if quality < had || text.is_empty() || entry.text == text {
    return false;
  }
  entry.text = text;
  retry_quality.retain(|(n, _)| *n != attempt);
  retry_quality.push((attempt, quality));
  let last = log.attempts.last().map(|x| x.text.clone());
  if let Some(AgentBlock::Notice(n)) = t.blocks.iter_mut().rev().find(|b| matches!(b, AgentBlock::Notice(NoticeBlock { retry: Some(_), .. }))) {
    n.details = last;
  }
  true
}

/// At most MAX_TEXT bytes, cut on a char boundary
fn clip(text: &str) -> String {
  if text.len() <= MAX_TEXT {
    return text.to_owned();
  }
  let mut end = MAX_TEXT;
  while !text.is_char_boundary(end) {
    end -= 1;
  }
  format!("{}…", text.get(..end).unwrap_or_default())
}

static ATTEMPT_OF: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"(?i)\battempt (\d+) of (\d+)\b"));

/// claude-agent-acp 0.81.0 words its api_retry title "Reconnecting to Claude, attempt N of M." / "Retrying Claude, …";
/// the built-in agent "Retrying <provider>, attempt N of M" with the error in `details`
fn retry_of_failure(f: &SessionFailure) -> (TurnRetry, RetryReport) {
  let c = ATTEMPT_OF.captures(&f.title);
  let num = |i: usize| c.as_ref().and_then(|c| c.get(i)).and_then(|m| m.as_str().parse().ok());
  let detail = match &f.details {
    Some(d) => format!("{}\n{d}", f.title),
    None => f.title.clone(),
  };
  let report = RetryReport {
    attempt: num(1),
    max: num(2),
    text: f.details.clone().unwrap_or_else(|| f.title.clone()),
    quality: if f.details.is_some() { RetryText::Raw } else { RetryText::Title },
    reconnect: f.category == FailureCategory::Connection,
    wait_ms: f.retry_after_ms,
  };
  (TurnRetry { attempt: num(1), max: num(2), detail }, report)
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

/// One row per failure id in the whole transcript; a higher revision rewrites it in place. Retry warnings get no row
/// of their own (claude-agent-acp 0.81.0 even mints a new session-scoped id per attempt): they label the live turn and
/// log the attempt on the turn's retry row, which an error closes as failed
pub fn apply_session_failure(s: &mut NormalizeState, f: &SessionFailure) -> bool {
  if is_retry_warning(f) {
    let (label, report) = retry_of_failure(f);
    set_retry(s, label);
    report_retry(s, report);
    return true;
  }
  if f.severity == Severity::Error {
    clear_label(s);
    settle_retry(s, ToolStatus::Failed);
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
    retry: None,
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
  // Retry rows are host-made and absent from an agent's replay, so they never shift the count
  let n = s.turns.iter().filter_map(Turn::as_agent).flat_map(|t| &t.blocks).filter(|b| matches!(b, AgentBlock::Notice(n) if n.retry.is_none())).count();
  let notice = AgentBlock::Notice(NoticeBlock {
    id: format!("notice:{}", n + 1),
    revision: Num(1.0),
    category: FailureCategory::Unknown,
    severity,
    info,
    title: title.to_owned(),
    details: text_of(u, "description").map(str::trim).filter(|d| !d.is_empty()).map(str::to_owned),
    actions: vec![],
    retry: None,
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
