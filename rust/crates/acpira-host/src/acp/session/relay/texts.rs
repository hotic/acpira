//! What the child and the caller read of a round: the brief, the note on what the round changed, and the answers
//! an `ask_agent` call returns (dispatched, waiting, a question, done)

use std::path::PathBuf;

use anyhow::anyhow;
use serde_json::Value;

use acpira_shared::subagents::{RelayMode, SubagentDuty, SubagentGroup, SubagentPersona, SubagentState};
use acpira_shared::transcript::{AgentBlock, DiffKind, ToolContent, ToolKind, ToolStatus, Turn};

use super::NOTE_FILES;
use super::personas::{node_persona, who};
use crate::acp::session::hooks::{Reach, edit_paths, row_reach};
use crate::acp::session::{AcpSession, Core};
use crate::relay::wire::HubReply;

pub(super) fn short(id: &str) -> String {
  id.chars().take(8).collect()
}

pub(super) fn clip(s: &str, max: usize) -> String {
  let line = s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
  let mut out: String = line.chars().take(max).collect();
  if line.chars().count() > max {
    out.push('…');
  }
  out
}

/// The task the child receives: the caller's prompt, the persona's standing brief, the read-only rule for a consult, what
/// is handed over of the caller's changes, and how to end: the caller reads only the final reply, so it says what was changed, checked and assumed (a reviewer:
/// its findings by severity and place), which is what the caller weighs before it acts on any of it
pub(super) fn compose_brief(prompt: &str, persona: &SubagentPersona, mode: RelayMode, handover: Option<&str>) -> String {
  let mut parts = vec![prompt.trim().to_owned()];
  if let Some(b) = persona.brief.as_deref().filter(|b| !b.trim().is_empty()) {
    parts.push(b.trim().to_owned());
  }
  if mode == RelayMode::Consult {
    parts.push("You are consulted for an opinion: read whatever you need, but do not modify any file.".into());
  }
  if let Some(h) = handover {
    parts.push(h.to_owned());
  }
  parts.push(format!(
    "(Another agent working in this same repository asked you this. It reads only your final reply, so make that reply \
self-contained. {} When a decision only it can make blocks you, ask it with ask_caller; otherwise decide yourself.)",
    closing(persona.duty, mode)
  ));
  parts.join("\n\n")
}

/// What the final reply ends with, by what the round was for
pub(super) fn closing(duty: SubagentDuty, mode: RelayMode) -> &'static str {
  match (duty, mode) {
    (SubagentDuty::Reviewer, _) => {
      "End with your findings, most severe first, each as: severity (high / medium / low), path:line, what is wrong and a \
concrete way it fails; if you found nothing wrong, say so plainly."
    }
    (_, RelayMode::Consult) => "End with what you found, each point with its path:line, and what you assumed.",
    _ => "End with the files you changed (or that you changed none), what you ran to check the work and its result, and what you assumed.",
  }
}

/// The files a round's edit rows name, with the lines their diffs add and remove (repeated edits of a file add up), and
/// whether it also ran something that can write anywhere (a command, an MCP tool, an edit naming no file): the rule the
/// workspace gate's `row_reach` follows
pub(super) fn round_edits(turns: &[Turn]) -> (Vec<(String, u64, u64)>, bool) {
  let mut files: Vec<(String, u64, u64)> = vec![];
  let mut anywhere = false;
  let mut bump = |path: &str, add: u64, del: u64| match files.iter_mut().find(|f| f.0 == path) {
    Some(f) => {
      f.1 += add;
      f.2 += del;
    }
    None => files.push((path.to_owned(), add, del)),
  };
  for turn in turns.iter().filter_map(Turn::as_agent) {
    for b in &turn.blocks {
      let AgentBlock::ToolCall(tc) = b else { continue };
      if row_reach(tc) == Reach::Any {
        anywhere = true;
      }
      if !matches!(tc.kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move) || tc.status == ToolStatus::Failed {
        continue;
      }
      // `contents` holds every item when there are several, `content` the primary one otherwise
      let items: Vec<&ToolContent> = match &tc.contents {
        Some(list) => list.iter().collect(),
        None => tc.content.iter().collect(),
      };
      let mut counted = false;
      for item in items {
        if let ToolContent::Diff { lines, source: Some(src) } = item {
          let n = |k: DiffKind| lines.iter().filter(|l| l.kind == k).count() as u64;
          bump(&src.path, n(DiffKind::Add), n(DiffKind::Del));
          counted = true;
        }
      }
      if !counted {
        let paths = edit_paths(tc);
        let stat = tc.diff_stat.filter(|_| paths.len() == 1);
        for p in paths {
          bump(&p, stat.map_or(0, |s| s.add), stat.map_or(0, |s| s.del));
        }
      }
    }
  }
  (files, anywhere)
}

/// A path as the caller reads it: relative to the session's folder when inside it
fn shown_path(cwd: &str, path: &str) -> String {
  let p = std::path::Path::new(path);
  let rel = p.strip_prefix(cwd).ok().map(|r| r.to_path_buf()).or_else(|| {
    let top = std::fs::canonicalize(cwd).ok()?;
    let canon = std::fs::canonicalize(p).ok().or_else(|| Some(std::fs::canonicalize(p.parent()?).ok()?.join(p.file_name()?)))?;
    canon.strip_prefix(&top).ok().map(|r| r.to_path_buf())
  });
  match rel {
    Some(r) if !r.as_os_str().is_empty() => r.to_string_lossy().replace('\\', "/"),
    _ => path.to_owned(),
  }
}

/// The round's change note the caller reads after the reply: what its edit rows changed, and, when it also ran something
/// that can write anywhere, the other files whose state changed meanwhile (`changed`: None outside a git work tree,
/// where nothing can be seen)
pub(super) fn change_note(cwd: &str, edits: &[(String, u64, u64)], anywhere: bool, changed: Option<&[PathBuf]>) -> String {
  let listed: Vec<(String, u64, u64)> = {
    let mut out: Vec<(String, u64, u64)> = vec![];
    for (p, a, d) in edits {
      let shown = shown_path(cwd, p);
      match out.iter_mut().find(|f| f.0 == shown) {
        Some(f) => {
          f.1 += a;
          f.2 += d;
        }
        None => out.push((shown, *a, *d)),
      }
    }
    out
  };
  let extra: Vec<String> = match (anywhere, changed) {
    (true, Some(paths)) => {
      paths.iter().map(|p| shown_path(cwd, &p.to_string_lossy())).filter(|p| !listed.iter().any(|f| &f.0 == p)).collect()
    }
    _ => vec![],
  };
  let names = |list: Vec<String>| {
    let more = list.len().saturating_sub(NOTE_FILES);
    let mut out = list.into_iter().take(NOTE_FILES).collect::<Vec<_>>().join(", ");
    if more > 0 {
      out.push_str(&format!(" and {more} more"));
    }
    out
  };
  let edited = names(listed.iter().map(|(p, a, d)| if a + d > 0 { format!("{p} (+{a} -{d})") } else { p.clone() }).collect());
  match (listed.is_empty(), extra.is_empty()) {
    (true, true) if anywhere && changed.is_none() => {
      "it reported no file edits, and outside a git work tree Acpira cannot see what its commands changed".into()
    }
    (true, true) => "it changed no files".into(),
    (false, true) => format!("files it edited: {edited}"),
    (true, false) => format!("it edited no file itself, but these changed while its commands ran: {}", names(extra)),
    (false, false) => format!("files it edited: {edited}; these also changed while its commands ran: {}", names(extra)),
  }
}

pub(super) fn norm(s: &str) -> String {
  s.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn busy(persona: &SubagentPersona, thread: &str) -> anyhow::Error {
  anyhow!("{} is still working on the previous round of thread {thread}. Call ask_agent with this thread and an empty prompt to wait for it.", who(persona))
}

/// A finished round as the caller reads it, the same live and later: the reply (and the round's notes: its setup and
/// what it changed), or why there is none (with the same notes: a stopped round may have changed files already)
pub(super) fn round_result(c: &Core, node: &str) -> std::result::Result<String, String> {
  let Some((state, reply, result)) = c.tree.relay_outcome(node) else { return Err("is gone.".into()) };
  let notes = c.relays.notes.get(node).filter(|n| !n.is_empty()).map(|n| format!("\n\n(Acpira: {}.)", n.join("; "))).unwrap_or_default();
  match state {
    SubagentState::Completed if notes.is_empty() => Ok(reply),
    SubagentState::Completed => Ok(format!("{}{notes}", reply.trim())),
    SubagentState::Cancelled if reply.trim().is_empty() => Err(format!("was cancelled.{notes}")),
    SubagentState::Cancelled => Err(format!("was cancelled. Its last words:\n\n{}{notes}", reply.trim())),
    SubagentState::Failed | SubagentState::Disconnected => {
      Err(format!("failed: {}{notes}", result.as_deref().map(str::trim).filter(|r| !r.is_empty()).unwrap_or("no reason given")))
    }
    SubagentState::Running => Err("is still working.".into()),
  }
}

pub(super) fn final_reply(end: std::result::Result<String, String>, thread: &str, persona: &SubagentPersona) -> HubReply {
  match end {
    Ok(reply) => HubReply::Done(done_text(&reply, thread, persona)),
    Err(e) => HubReply::Error(format!("{} {e}", who(persona))),
  }
}

/// What a group call that sent its task off answers: every member's thread, and that the replies come together
pub(super) fn group_dispatched_text(group: &SubagentGroup, started: &[(String, SubagentPersona)]) -> String {
  let members: Vec<String> = started.iter().map(|(th, p)| format!("{} (thread: {th})", who(p))).collect();
  let whom = if started.len() == 1 { "the one member".to_owned() } else { format!("the {} members", started.len()) };
  format!(
    "Sent to {whom} of {}: {}. Each works on its own, without seeing the others. Carry on with other work that does not \
depend on them, or end your turn if there is none: Acpira sends you their replies together in one message when all are \
done. Do not poll, and do not do this task yourself meanwhile.",
    group.name,
    members.join(", ")
  )
}

/// What a call that sent a task off answers: the thread, and that the reply comes by itself
pub(super) fn dispatched_text(thread: &str, persona: &SubagentPersona) -> String {
  format!(
    "Sent to {} (thread: {thread}). Carry on with other work that does not depend on it, or end your turn if there is none: \
Acpira sends you its reply in a message when it is done. Do not poll, and do not do this task yourself meanwhile.",
    who(persona)
  )
}

/// The automatic message delivering finished rounds (each one as a call would have read it, under its persona and
/// thread) and the questions of rounds still running
pub(super) fn wake_text(c: &Core, nodes: &[String], asking: &[String]) -> String {
  let mut parts = vec![];
  if !nodes.is_empty() {
    parts.push(if nodes.len() == 1 {
      "(Acpira) A summoned agent you sent off has finished. Its reply:".to_owned()
    } else {
      format!("(Acpira) The {} summoned agents you sent off have finished. Their replies:", nodes.len())
    });
  }
  for node in asking {
    let Some((h, name)) = c.tree.relay_harness(node) else { continue };
    let Some((_, questions)) = AcpSession::relay_question(c, node) else { continue };
    let persona = node_persona(&h, name.as_deref());
    parts.push(format!("## {} asks\n\n{}", who(&persona), question_text(&questions, &h.thread, &persona)));
  }
  for node in nodes {
    let Some((h, name)) = c.tree.relay_harness(node) else { continue };
    let persona = node_persona(&h, name.as_deref());
    let text = match final_reply(round_result(c, node), &h.thread, &persona) {
      HubReply::Done(t) | HubReply::Error(t) => t,
      _ => continue,
    };
    parts.push(format!("## {}\n\n{text}", who(&persona)));
  }
  parts.join("\n\n")
}

/// A child's open question as its caller reads it: every question with its options, and how to answer
pub(super) fn question_text(questions: &[acpira_shared::transcript::Question], thread: &str, persona: &SubagentPersona) -> String {
  let mut out = vec![format!("{} is waiting for an answer before it goes on (thread: {thread}):", who(persona))];
  for (i, q) in questions.iter().enumerate() {
    let title = q.title.as_deref().filter(|t| !t.trim().is_empty() && t.trim() != q.text.trim()).map(|t| format!("{}: ", t.trim())).unwrap_or_default();
    let many = if q.kind == acpira_shared::transcript::QuestionKind::Multiple { " (one or more)" } else { "" };
    out.push(format!("{}. {title}{}{many}", i + 1, q.text.trim()));
    for o in &q.options {
      out.push(match o.description.as_deref().filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("   - {}: {}", o.label, d.trim()),
        None => format!("   - {}", o.label),
      });
    }
    if !q.options.is_empty() && q.other != Some(false) {
      out.push("   - (or an answer of your own)".into());
    }
  }
  let how = if questions.len() == 1 {
    "pass the answer as `prompt`, or as `answers` with one entry"
  } else {
    "pass `answers`, one entry per question in this order"
  };
  out.push(format!(
    "Answer with ask_agent, thread {thread:?}: {how} (an option's label, or your own words). The user sees the same question and may answer first."
  ));
  out.join("\n")
}

/// A caller's answers as the card's would be: by question order, an option's label or id (any case) picks it, anything
/// else is the question's own text; one question may take the prompt instead
pub(super) fn caller_answers(
  questions: &[acpira_shared::transcript::Question],
  given: Option<&[String]>,
  prompt: &str,
) -> std::result::Result<acpira_shared::transcript::QuestionAnswers, String> {
  use acpira_shared::transcript::QuestionKind;
  let fallback = [prompt.to_owned()];
  let given = match given {
    Some(g) if !g.is_empty() => g,
    _ if questions.len() == 1 && !prompt.trim().is_empty() => &fallback[..],
    _ => return Err(format!("asks {} questions: pass `answers`, one entry per question.", questions.len())),
  };
  let mut out = acpira_shared::transcript::QuestionAnswers::new();
  for (q, a) in questions.iter().zip(given) {
    let a = a.trim();
    if a.is_empty() {
      continue;
    }
    let pick = |x: &str| q.options.iter().find(|o| o.label.trim().eq_ignore_ascii_case(x.trim()) || o.id.eq_ignore_ascii_case(x.trim())).map(|o| o.id.clone());
    let closed = !q.options.is_empty() && q.other == Some(false);
    let one = |x: &str| -> std::result::Result<String, String> {
      match pick(x) {
        Some(id) => Ok(id),
        None if closed => Err(format!(
          "only takes one of its options for {:?}: {}.",
          q.text.trim(),
          q.options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>().join(", ")
        )),
        None => Ok(x.trim().to_owned()),
      }
    };
    let value = if q.kind == QuestionKind::Multiple && pick(a).is_none() {
      let items = a.split([',', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(one).collect::<std::result::Result<Vec<_>, _>>()?;
      Value::from(items)
    } else if q.kind == QuestionKind::Multiple {
      Value::from(vec![one(a)?])
    } else {
      Value::from(one(a)?)
    };
    out.insert(q.id.clone(), value);
  }
  Ok(out)
}

/// An answer as `ask_caller` reports it: the text, or the picks joined
pub(super) fn answer_text(a: &Value) -> String {
  match a {
    Value::String(s) => s.trim().to_owned(),
    Value::Array(l) => l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
    other => other.to_string(),
  }
}

fn done_text(reply: &str, thread: &str, persona: &SubagentPersona) -> String {
  let body = if reply.trim().is_empty() { "(no reply text)" } else { reply.trim() };
  format!("{body}\n\n---\nthread: {thread} — {}. Pass this thread to ask_agent to follow up in the same conversation.", who(persona))
}
