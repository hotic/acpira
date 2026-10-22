//! Bounded readings of one session record for an agent (`read_session` / `search_sessions` in `acpira mcp`): an outline,
//! a page of turns, and a fixed-string search inside the record. Every reading is capped in bytes, so another session's
//! transcript (often hundreds of KB) never lands whole in the caller's context; what does not fit says where to read on.
//!
//! Turns are numbered the way a reader counts them, not by `turns` index: one non-automatic user message plus the agent
//! turns that answer it is one turn, numbered from 1. An automatic user message (compaction, account switch, retry, gate,
//! goal) joins the turn it continues and is marked with its reason; an agent turn with no user message before it gets a
//! number of its own; a message steered into a running turn is listed as a user line of that turn. `session_search` caches
//! the same numbers through `TurnCounter`, so a search hit's turn is the turn `read_session` shows.

use std::collections::BTreeMap;

use acpira_shared::session_links::session_link;
use acpira_shared::subagents::SubagentState;
use acpira_shared::time::iso_of_ms;
use acpira_shared::transcript::{
  AgentBlock, AgentTurn, AutoReason, DiffKind, PlanEntry, PlanStatus, ToolCallBlock, ToolContent, ToolKind, ToolStatus,
  Turn, UserTurn,
};

use crate::store::record::SessionRecord;
use crate::store::session_search::fold;

/// Byte caps of the three readings (a CJK character is three bytes). Claude Code warns about MCP output past ~10k
/// tokens; a full page stays well under that in any script
pub const OUTLINE_BYTES: usize = 12 * 1024;
pub const PAGE_BYTES: usize = 16 * 1024;
pub const SEARCH_BYTES: usize = 4 * 1024;
/// Hits of one in-session search, by default and at most
pub const GREP_HITS: usize = 8;
pub const GREP_HITS_MAX: usize = 20;

/// Numbers turns as they stream past; `session_search` runs the same counter over its lighter parse
#[derive(Default)]
pub struct TurnCounter {
  n: u32,
  after_user: bool,
}

impl TurnCounter {
  /// A user message: a typed one opens the next turn, an automatic one continues the current turn
  pub fn user(&mut self, auto: bool) -> u32 {
    if !auto || self.n == 0 {
      self.n += 1;
    }
    self.after_user = true;
    self.n
  }
  /// An agent turn answers the user message before it; after another agent turn (or first in the record) it opens a turn
  pub fn agent(&mut self) -> u32 {
    if !self.after_user || self.n == 0 {
      self.n += 1;
    }
    self.after_user = false;
    self.n
  }
}

/// One numbered turn: its records in order
struct Round<'a> {
  n: u32,
  items: Vec<&'a Turn>,
}

impl<'a> Round<'a> {
  fn users(&self) -> impl Iterator<Item = &'a UserTurn> + '_ {
    self.items.iter().filter_map(|t| match t {
      Turn::User(u) => Some(u),
      Turn::Agent(_) => None,
    })
  }
  fn agents(&self) -> impl Iterator<Item = &'a AgentTurn> + '_ {
    self.items.iter().filter_map(|t| t.as_agent())
  }
  fn started_at(&self) -> Option<i64> {
    self.agents().find_map(|a| a.started_at)
  }
}

fn rounds(turns: &[Turn]) -> Vec<Round<'_>> {
  let mut counter = TurnCounter::default();
  let mut out: Vec<Round> = vec![];
  for t in turns {
    let n = match t {
      Turn::User(u) => counter.user(u.auto == Some(true)),
      Turn::Agent(_) => counter.agent(),
    };
    match out.last_mut() {
      Some(r) if r.n == n => r.items.push(t),
      _ => out.push(Round { n, items: vec![t] }),
    }
  }
  out
}

/// What the caller knows about the session besides its record
pub struct DigestMeta<'a> {
  pub agent_name: &'a str,
  /// A live session may be mid-turn: its last turn can still change
  pub live: bool,
  /// A fork from another agent: that agent's name, for the turns it copied
  pub fork_agent: Option<&'a str>,
}

/// At most `max` bytes of `s`, cut on a char boundary, with an ellipsis when cut
pub fn cut(s: &str, max: usize) -> String {
  if s.len() <= max {
    return s.to_owned();
  }
  let mut end = max.saturating_sub('…'.len_utf8());
  while end > 0 && !s.is_char_boundary(end) {
    end -= 1;
  }
  format!("{}…", &s[..end])
}

/// One line of text, whitespace collapsed, cut to `max` bytes
fn line(s: &str, max: usize) -> String {
  cut(&s.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

fn stamp(ms: i64) -> String {
  iso_stamp(&iso_of_ms(ms))
}

/// `2026-10-12 14:03 UTC` from an ISO timestamp; anything else as it is
fn iso_stamp(iso: &str) -> String {
  match (iso.get(..10), iso.get(11..16)) {
    (Some(d), Some(t)) if iso.ends_with('Z') => format!("{d} {t} UTC"),
    _ => iso.to_owned(),
  }
}

fn auto_label(reason: Option<AutoReason>) -> &'static str {
  match reason {
    None | Some(AutoReason::Compact) => "automatic: compaction",
    Some(AutoReason::AccountSwitch) => "automatic: continued on another account",
    Some(AutoReason::Retry) => "automatic: retry",
    Some(AutoReason::Gate) => "automatic: workspace gate findings",
    Some(AutoReason::Goal) => "automatic: goal control",
    Some(AutoReason::GoalContinue) => "automatic: the goal's next turn",
    Some(AutoReason::Subagents) => "automatic: summoned agents reported back",
  }
}

fn user_label(u: &UserTurn) -> String {
  if u.auto == Some(true) { format!("User ({})", auto_label(u.auto_reason)) } else { "User".into() }
}

fn status_word(s: ToolStatus) -> &'static str {
  match s {
    ToolStatus::Pending => "pending",
    ToolStatus::InProgress => "running",
    ToolStatus::Completed => "done",
    ToolStatus::Failed => "failed",
    ToolStatus::Cancelled => "cancelled",
  }
}

/// A path relative to the session's folder when it lies inside
fn rel<'a>(path: &'a str, cwd: &str) -> &'a str {
  let root = cwd.trim_end_matches(['/', '\\']);
  if root.is_empty() {
    return path;
  }
  path.strip_prefix(root).and_then(|r| r.strip_prefix(['/', '\\'])).filter(|r| !r.is_empty()).unwrap_or(path)
}

fn tool_path(t: &ToolCallBlock) -> Option<&str> {
  t.locations.as_ref().and_then(|l| l.first()).map(|l| l.path.as_str()).or(t.target.as_deref())
}

fn tool_line(t: &ToolCallBlock, cwd: &str) -> String {
  let mut s = format!("- [tool] {}", line(&t.verb, 60));
  if let Some(target) = t.target.as_deref().filter(|x| !x.is_empty()) {
    s.push(' ');
    s.push_str(&line(rel(target, cwd), 160));
  }
  if let Some(d) = &t.diff_stat {
    s.push_str(&format!(" (+{} −{})", d.add, d.del));
  }
  if t.status != ToolStatus::Completed {
    s.push_str(&format!(" [{}]", status_word(t.status)));
  }
  s
}

/// A tool's output for `detail: full`: text and lists as they are, diffs as +/- lines, all within `max` bytes
fn tool_output(t: &ToolCallBlock, max: usize) -> String {
  let mut out = String::new();
  let contents = t.contents.iter().flatten().chain(t.content.iter().filter(|_| t.contents.is_none()));
  for c in contents {
    match c {
      ToolContent::Text { text } => out.push_str(text),
      ToolContent::List { items } => out.push_str(&items.join("\n")),
      ToolContent::Diff { lines, source } => {
        if let Some(src) = source {
          out.push_str(&format!("--- {}\n", src.path));
        }
        for l in lines {
          let mark = match l.kind {
            DiffKind::Add => '+',
            DiffKind::Del => '-',
            _ => ' ',
          };
          out.push(mark);
          out.push_str(&l.text);
          out.push('\n');
        }
      }
      ToolContent::Image(_) => out.push_str("[image]"),
    }
    out.push('\n');
    if out.len() > max {
      break;
    }
  }
  cut(out.trim_end(), max)
}

fn plan_mark(s: PlanStatus) -> &'static str {
  match s {
    PlanStatus::Completed => "[x]",
    PlanStatus::InProgress => "[~]",
    _ => "[ ]",
  }
}

fn reply_texts(a: &AgentTurn) -> impl Iterator<Item = &str> {
  a.blocks.iter().filter_map(|b| match b {
    AgentBlock::Text(t) if !t.markdown.trim().is_empty() => Some(t.markdown.as_str()),
    _ => None,
  })
}

fn header(r: &SessionRecord, meta: &DigestMeta, rounds: usize) -> String {
  let mut s = format!("# {}\n", if r.title.trim().is_empty() { "(untitled)" } else { r.title.trim() });
  s.push_str(&format!("Link: {}\n", session_link(&r.id)));
  let model = r.controls.options.iter().find(|c| c.category.as_deref() == Some("model")).and_then(|c| {
    let v = c.value.as_deref()?;
    Some(c.options.iter().find(|o| o.id == v).map(|o| o.name.as_str()).unwrap_or(v))
  });
  s.push_str(&format!("Agent: {}{}\n", meta.agent_name, model.map(|m| format!(" ({m})")).unwrap_or_default()));
  if !r.cwd.is_empty() {
    s.push_str(&format!("Project: {}\n", r.cwd));
  }
  let from = if r.created_at.is_empty() { None } else { Some(iso_stamp(&r.created_at)) };
  s.push_str(&format!("Time: {}{}\n", from.map(|f| format!("{f} → ")).unwrap_or_default(), iso_stamp(&r.updated_at)));
  s.push_str(&format!("Turns: {rounds}\n"));
  if let Some(f) = r.forked_from.as_ref()
    && let Some(n) = usize::try_from(f.turn_index).ok().and_then(|i| round_numbers(&r.turns).get(i).copied())
  {
    let from = meta.fork_agent.map(|a| format!(", run by {a}")).unwrap_or_default();
    s.push_str(&format!("Forked: turns 1-{n} were copied from {}{from}\n", session_link(&f.session_id)));
  }
  if meta.live {
    s.push_str("This session is open right now; its last turn may still be running.\n");
  }
  s
}

/// Lines that hold whatever the session left behind: files, plan, documents, subagents, the last error
fn key_facts(r: &SessionRecord) -> Vec<String> {
  let mut out = turn_facts(&r.turns, &r.cwd);
  let subs: Vec<String> = r
    .subagents
    .iter()
    .flatten()
    .take(10)
    .map(|s| {
      let name = s.core.title.as_deref().or(s.core.task.as_deref()).unwrap_or("subagent");
      let state = match s.core.state {
        SubagentState::Running => "running",
        SubagentState::Completed => "completed",
        SubagentState::Failed => "failed",
        SubagentState::Cancelled => "cancelled",
        SubagentState::Disconnected => "disconnected",
      };
      format!("{} ({state})", line(name, 80))
    })
    .collect();
  if !subs.is_empty() {
    // Ahead of the last error, which stays the closing fact
    let at = out.iter().position(|l| l.starts_with("- Last error")).unwrap_or(out.len());
    out.insert(at, format!("- Subagents: {}", subs.join(", ")));
  }
  out
}

/// The key facts the turns themselves hold: edited files, the last plan, plan documents, the last error
fn turn_facts(turns: &[Turn], cwd: &str) -> Vec<String> {
  let mut edits: BTreeMap<String, (u64, u64, u32)> = BTreeMap::new();
  let mut plan: Option<&[PlanEntry]> = None;
  let mut docs: Vec<String> = vec![];
  let mut error: Option<(u32, String)> = None;
  for round in rounds(turns) {
    for a in round.agents() {
      for b in &a.blocks {
        match b {
          AgentBlock::ToolCall(t) => {
            if let Some(entries) = t.todo_entries.as_deref().filter(|e| !e.is_empty()) {
              plan = Some(entries);
            }
            let edit = matches!(t.kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move) || t.diff_stat.is_some();
            if edit
              && t.status == ToolStatus::Completed
              && let Some(p) = tool_path(t)
            {
              let e = edits.entry(rel(p, cwd).to_owned()).or_default();
              if let Some(d) = &t.diff_stat {
                e.0 += d.add;
                e.1 += d.del;
              }
              e.2 = round.n;
            }
          }
          AgentBlock::Plan(p) if !p.entries.is_empty() => plan = Some(&p.entries),
          AgentBlock::PlanDocument(d) => {
            let status = serde_json::to_value(d.status).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
            let entry = format!("\"{}\" ({status}, turn {})", line(&d.title, 120), round.n);
            docs.retain(|x| !x.starts_with(&format!("\"{}\"", line(&d.title, 120))));
            docs.push(entry);
          }
          _ => {}
        }
      }
      if let Some(e) = &a.error {
        error = Some((round.n, line(&e.message, 300)));
      }
    }
  }
  let mut out = vec![];
  if !edits.is_empty() {
    let n = edits.len();
    let list: Vec<String> = edits.iter().take(20).map(|(p, (add, del, _))| format!("{p} (+{add} −{del})")).collect();
    out.push(format!("- Edited files ({n}): {}{}", list.join(", "), if n > 20 { format!(", +{} more", n - 20) } else { String::new() }));
  }
  if let Some(entries) = plan {
    let list: Vec<String> = entries.iter().take(15).map(|e| format!("{} {}", plan_mark(e.status), line(&e.title, 100))).collect();
    out.push(format!("- Plan / to-dos (last state): {}{}", list.join("; "), if entries.len() > 15 { "; …" } else { "" }));
  }
  if !docs.is_empty() {
    out.push(format!("- Plan documents: {}", docs.join(", ")));
  }
  if let Some((n, e)) = error {
    out.push(format!("- Last error (turn {n}): {e}"));
  }
  out
}

fn round_line(round: &Round) -> String {
  let mut parts = vec![];
  for u in round.users() {
    let text = if u.text.trim().is_empty() { u.command.clone().unwrap_or_default() } else { u.text.clone() };
    if u.auto == Some(true) {
      parts.push(format!("({})", auto_label(u.auto_reason)));
    } else {
      parts.push(format!("User: {}", line(&text, 200)));
    }
  }
  let mut tools = 0;
  let mut edits = 0;
  let mut last_reply = None;
  for a in round.agents() {
    for b in &a.blocks {
      match b {
        AgentBlock::Steer(s) => parts.push(format!("Steer: {}", line(&s.text, 120))),
        AgentBlock::ToolCall(t) => {
          tools += 1;
          if t.diff_stat.is_some() || matches!(t.kind, ToolKind::Edit | ToolKind::Delete | ToolKind::Move) {
            edits += 1;
          }
        }
        AgentBlock::Text(t) if !t.markdown.trim().is_empty() => last_reply = Some(t.markdown.as_str()),
        _ => {}
      }
    }
    if let Some(e) = &a.error {
      parts.push(format!("Error: {}", line(&e.message, 120)));
    }
  }
  if let Some(r) = last_reply {
    parts.push(format!("Reply: {}", line(r, 240)));
  }
  if tools > 0 {
    parts.push(if edits > 0 { format!("{tools} tools, {edits} edits") } else { format!("{tools} tools") });
  }
  format!("{}. {}", round.n, parts.join(" | "))
}

const OUTLINE_HINT: &str = "Read whole turns with read_session {session, from, to}; find text in this session with search_sessions {session, query}.";

/// The outline: header, key facts, one line per turn (the first and last ones when they do not all fit), the last reply
pub fn outline(r: &SessionRecord, meta: &DigestMeta) -> String {
  let rounds = rounds(&r.turns);
  let mut out = header(r, meta, rounds.len());
  let facts = key_facts(r);
  if !facts.is_empty() {
    out.push_str("\n## Key facts\n");
    out.push_str(&cut(&facts.join("\n"), 3 * 1024));
    out.push('\n');
  }
  let last = rounds.iter().rev().find_map(|round| round.agents().flat_map(reply_texts).last().map(|t| (round.n, t)));
  let last_block = last.map(|(n, t)| format!("\n## Last reply (turn {n})\n{}\n", cut(t.trim(), 3 * 1024))).unwrap_or_default();
  let tail = format!("\n{OUTLINE_HINT}\n");
  out.push_str("\n## Turns\n");
  let lines: Vec<String> = rounds.iter().map(round_line).collect();
  let room = OUTLINE_BYTES.saturating_sub(out.len() + last_block.len() + tail.len() + 80);
  let total: usize = lines.iter().map(|l| l.len() + 1).sum();
  if total <= room {
    for l in &lines {
      out.push_str(l);
      out.push('\n');
    }
  } else {
    // The opening turns set the task, the closing ones hold where it ended; the middle folds into one line
    let head: Vec<&String> = lines.iter().take(3).collect();
    let mut used: usize = head.iter().map(|l| l.len() + 1).sum();
    let mut tail_lines: Vec<&String> = vec![];
    for l in lines.iter().skip(head.len()).rev() {
      if used + l.len() + 1 > room {
        break;
      }
      used += l.len() + 1;
      tail_lines.push(l);
    }
    tail_lines.reverse();
    for l in &head {
      out.push_str(&cut(l, room / 3));
      out.push('\n');
    }
    let first_kept = rounds.len() - tail_lines.len();
    if first_kept > head.len() {
      out.push_str(&format!("(turns {}-{} omitted; read them with read_session from/to)\n", head.len() + 1, first_kept));
    }
    for l in tail_lines {
      out.push_str(l);
      out.push('\n');
    }
  }
  out.push_str(&last_block);
  out.push_str(&tail);
  cut(&out, OUTLINE_BYTES)
}

/// Each turn's number as `read_session` counts it (`TurnCounter`), by index into `turns`
pub fn round_numbers(turns: &[Turn]) -> Vec<u32> {
  let mut counter = TurnCounter::default();
  turns
    .iter()
    .map(|t| match t {
      Turn::User(u) => counter.user(u.auto == Some(true)),
      Turn::Agent(_) => counter.agent(),
    })
    .collect()
}

/// What a handoff puts in place of the turns it had to leave out: the key facts of the whole conversation (`turns`),
/// then one line per omitted turn (numbers 1 to `omitted`), the middle folded when they do not fit. No last reply: the
/// turns kept after it carry the end. `hint` closes it when the receiving agent can read the rest on its own
pub fn handoff_outline(turns: &[Turn], cwd: &str, omitted: u32, hint: Option<&str>) -> String {
  let rounds = rounds(turns);
  let mut out = format!("Turns 1-{omitted} of this conversation were left out to fit the size limit; this outline stands in for them.\n");
  let facts = turn_facts(turns, cwd);
  if !facts.is_empty() {
    out.push_str("\n## Key facts (whole conversation)\n");
    out.push_str(&cut(&facts.join("\n"), 3 * 1024));
    out.push('\n');
  }
  out.push_str("\n## Omitted turns\n");
  let tail = hint.map(|h| format!("\n{h}\n")).unwrap_or_default();
  let lines: Vec<String> = rounds.iter().take_while(|r| r.n <= omitted).map(round_line).collect();
  let room = OUTLINE_BYTES.saturating_sub(out.len() + tail.len() + 80);
  let total: usize = lines.iter().map(|l| l.len() + 1).sum();
  if total <= room {
    for l in &lines {
      out.push_str(l);
      out.push('\n');
    }
  } else {
    // The opening turns set the task, the ones just before the kept history lead into it
    let head: Vec<&String> = lines.iter().take(3).collect();
    let mut used: usize = head.iter().map(|l| cut(l, room / 3).len() + 1).sum();
    let mut closing: Vec<&String> = vec![];
    for l in lines.iter().skip(head.len()).rev() {
      if used + l.len() + 1 > room {
        break;
      }
      used += l.len() + 1;
      closing.push(l);
    }
    closing.reverse();
    for l in &head {
      out.push_str(&cut(l, room / 3));
      out.push('\n');
    }
    let first_kept = lines.len() - closing.len();
    if first_kept > head.len() {
      out.push_str(&format!("(turns {}-{first_kept} folded)\n", head.len() + 1));
    }
    for l in closing {
      out.push_str(l);
      out.push('\n');
    }
  }
  out.push_str(&tail);
  cut(&out, OUTLINE_BYTES)
}

/// One turn in full: user lines, steers, replies, tools as one line each (with their output under `full`); no thoughts
fn round_text(round: &Round, cwd: &str, full: bool) -> String {
  let mut s = format!("## Turn {}{}\n", round.n, round.started_at().map(|ms| format!(" ({})", stamp(ms))).unwrap_or_default());
  for t in &round.items {
    match t {
      Turn::User(u) => {
        s.push_str(&format!("\n**{}:**\n{}\n", user_label(u), if u.text.trim().is_empty() { u.command.as_deref().unwrap_or("") } else { u.text.trim() }));
        let names: Vec<String> = u.attachments.iter().flatten().map(|a| a.label().unwrap_or_else(|| "quote".into())).collect();
        if !names.is_empty() {
          s.push_str(&format!("(attachments: {})\n", names.join(", ")));
        }
      }
      Turn::Agent(a) => {
        let mut tools_open = false;
        for b in &a.blocks {
          let is_tool = matches!(b, AgentBlock::ToolCall(_));
          if tools_open && !is_tool {
            tools_open = false;
          }
          match b {
            AgentBlock::Text(t) if !t.markdown.trim().is_empty() => s.push_str(&format!("\n**Agent:**\n{}\n", t.markdown.trim())),
            AgentBlock::Steer(st) => s.push_str(&format!("\n**User (steered mid-turn):**\n{}\n", st.text.trim())),
            AgentBlock::ToolCall(tc) => {
              if !tools_open {
                s.push('\n');
                tools_open = true;
              }
              s.push_str(&tool_line(tc, cwd));
              s.push('\n');
              if full {
                let body = tool_output(tc, 2 * 1024);
                if !body.is_empty() {
                  s.push_str(&format!("```\n{body}\n```\n"));
                }
              }
            }
            AgentBlock::PlanDocument(d) => {
              s.push_str(&format!("\n**Plan document \"{}\":**\n{}\n", d.title, if full { cut(d.markdown.trim(), 4 * 1024) } else { cut(d.markdown.trim(), 600) }))
            }
            AgentBlock::Plan(p) if !p.entries.is_empty() => {
              let list: Vec<String> = p.entries.iter().map(|e| format!("{} {}", plan_mark(e.status), line(&e.title, 100))).collect();
              s.push_str(&format!("\n(plan: {})\n", list.join("; ")));
            }
            AgentBlock::Question(q) => {
              let texts: Vec<String> = q.questions.iter().map(|x| line(&x.text, 200)).collect();
              s.push_str(&format!("\n(asked the user: {})\n", texts.join(" / ")));
            }
            AgentBlock::Compaction(_) => s.push_str("\n(context compacted)\n"),
            _ => {}
          }
        }
        if let Some(e) = &a.error {
          s.push_str(&format!("\n**Error:** {}\n", line(&e.message, 500)));
        }
      }
    }
  }
  s
}

/// Turns `from`..=`to` (the last turn when `to` is absent or past it), one page: whole turns while they fit, the first one
/// cut when it alone does not. `offset` starts the first turn that many bytes into its text, where an earlier page cut it.
/// A page that stops early names where to read next: the next turn, or the same turn further in
pub fn range(r: &SessionRecord, meta: &DigestMeta, from: u32, to: Option<u32>, offset: usize, full: bool) -> String {
  let rounds = rounds(&r.turns);
  let total = rounds.len() as u32;
  let title = if r.title.trim().is_empty() { "(untitled)" } else { r.title.trim() };
  let mut out = format!("# {title}\nLink: {}\nAgent: {} | Turns: {total}\n", session_link(&r.id), meta.agent_name);
  if meta.live {
    out.push_str("This session is open right now; its last turn may still be running.\n");
  }
  let from = from.max(1);
  let to = to.unwrap_or(total).min(total);
  if total == 0 || from > to {
    out.push_str(&format!("\nNo turns in {from}-{to}; this session has {total}.\n"));
    return out;
  }
  let budget = PAGE_BYTES - 200;
  // The next page: its first turn and the offset into it
  let mut next = None;
  for round in rounds.iter().filter(|x| x.n >= from && x.n <= to) {
    let body = round_text(round, &r.cwd, full);
    let start = if round.n == from { forward_boundary(&body, offset) } else { 0 };
    let lead = if start > 0 { format!("\n(turn {} continued from byte {start})\n", round.n) } else { "\n".to_owned() };
    let rest = &body[start..];
    if out.len() + lead.len() + rest.len() > budget {
      if round.n == from {
        // A turn larger than a page: as much as fits, ending on a line when one ends late enough, and where it goes on
        let note = 160;
        let room = budget.saturating_sub(out.len() + lead.len() + note).max(1024).min(rest.len());
        let mut end = room;
        while !rest.is_char_boundary(end) {
          end -= 1;
        }
        if let Some(nl) = rest[..end].rfind('\n').filter(|&nl| nl > end / 2) {
          end = nl + 1;
        }
        out.push_str(&lead);
        out.push_str(&rest[..end]);
        let at = start + end;
        out.push_str(&format!("\n\n(turn {} is cut here, {at} of {} bytes)\n", round.n, body.len()));
        next = Some((round.n, at));
      } else {
        next = Some((round.n, 0));
      }
      break;
    }
    out.push_str(&lead);
    out.push_str(rest);
  }
  match next {
    Some((n, 0)) => out.push_str(&format!("\nMore: read_session {{session, from: {n}, to: {to}}}\n")),
    Some((n, at)) => out.push_str(&format!("\nMore: read_session {{session, from: {n}, offset: {at}, to: {to}}}\n")),
    None if to < total => out.push_str(&format!("\n(turns {}-{total} follow)\n", to + 1)),
    None => {}
  }
  out
}

/// `at` moved forward to a char boundary of `s`, at most its length
fn forward_boundary(s: &str, at: usize) -> usize {
  let mut at = at.min(s.len());
  while !s.is_char_boundary(at) {
    at += 1;
  }
  at
}

/// A fixed string (case-insensitive) in one record: user messages, steers, replies, tool targets and output, plan
/// documents. Each hit gives its turn and a little context; at most `limit` hits within `SEARCH_BYTES`
pub fn grep(r: &SessionRecord, query: &str, limit: usize) -> String {
  let needle = fold(query.trim());
  let title = if r.title.trim().is_empty() { "(untitled)" } else { r.title.trim() };
  let mut out = format!("Search in \"{}\" ({}) for \"{}\":\n", line(title, 120), session_link(&r.id), line(query.trim(), 120));
  if needle.is_empty() {
    out.push_str("(empty query)\n");
    return out;
  }
  let limit = limit.clamp(1, GREP_HITS_MAX);
  let mut hits = 0;
  let mut more = 0;
  // One hit per text block is enough to point at it; reading the turn shows the rest
  let mut push = |out: &mut String, n: u32, kind: &str, text: &str| {
    let folded = fold(text);
    let Some(pos) = folded.find(&needle) else { return };
    let entry = format!("- turn {n} ({kind}): {}\n", context(text, &folded, pos, needle.len()));
    if hits >= limit || out.len() + entry.len() > SEARCH_BYTES - 120 {
      more += 1;
    } else {
      out.push_str(&entry);
      hits += 1;
    }
  };
  for round in rounds(&r.turns) {
    for t in &round.items {
      match t {
        Turn::User(u) => push(&mut out, round.n, if u.auto == Some(true) { "automatic message" } else { "user" }, &u.text),
        Turn::Agent(a) => {
          for b in &a.blocks {
            match b {
              AgentBlock::Text(x) => push(&mut out, round.n, "reply", &x.markdown),
              AgentBlock::Steer(x) => push(&mut out, round.n, "steer", &x.text),
              AgentBlock::PlanDocument(d) => push(&mut out, round.n, "plan document", &d.markdown),
              AgentBlock::ToolCall(tc) => {
                // The label names the tool, so a hit deep in its output still says where it came from
                let target = format!("{} {}", tc.verb, tc.target.as_deref().map(|t| rel(t, &r.cwd)).unwrap_or(""));
                let body = tool_output(tc, 64 * 1024);
                push(&mut out, round.n, &format!("tool: {}", line(&target, 80)), &format!("{target}\n{body}"));
              }
              _ => {}
            }
          }
        }
      }
    }
  }
  if hits == 0 {
    out.push_str("No matches.\n");
  } else if more > 0 {
    out.push_str(&format!("(+{more} more matches; narrow the query or read the turns)\n"));
  }
  out
}

/// About 80 bytes either side of a match, whitespace collapsed; `pos` and `len` index the folded text, which keeps the
/// original's char positions (session_search::fold)
fn context(text: &str, folded: &str, pos: usize, len: usize) -> String {
  let chars: Vec<char> = text.chars().collect();
  let at = folded[..pos].chars().count();
  let n = folded[pos..pos + len].chars().count();
  let start = at.saturating_sub(40);
  let end = (at + n + 80).min(chars.len());
  let mut s = String::new();
  if start > 0 {
    s.push('…');
  }
  s.extend(&chars[start..end]);
  if end < chars.len() {
    s.push('…');
  }
  line(&s, 400)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn record(turns: serde_json::Value) -> SessionRecord {
    serde_json::from_value(serde_json::json!({
      "id": "s1", "agent": "claude", "cwd": "/repo", "title": "Fix the parser", "createdAt": "2026-10-01T08:00:00.000Z",
      "updatedAt": "2026-10-02T09:30:00.000Z", "turns": turns,
    }))
    .unwrap()
  }

  const META: DigestMeta<'static> = DigestMeta { agent_name: "Claude", live: false, fork_agent: None };

  fn sample() -> SessionRecord {
    record(serde_json::json!([
      { "role": "agent", "blocks": [{ "type": "text", "markdown": "Welcome, ready." }] },
      { "role": "user", "text": "Make the lexer faster" },
      { "role": "agent", "startedAt": 1_760_000_000_000_i64, "blocks": [
        { "type": "thought", "text": "secret thinking" },
        { "type": "tool_call", "id": "t1", "kind": "edit", "verb": "Edit", "target": "/repo/src/lexer.rs", "status": "completed",
          "diffStat": { "add": 12, "del": 3 }, "locations": [{ "path": "/repo/src/lexer.rs" }],
          "content": { "type": "diff", "lines": [{ "kind": "add", "text": "fast_path()" }] } },
        { "type": "steer", "id": "st", "text": "also keep the old API" },
        { "type": "text", "markdown": "Done: the lexer now takes a fast path." }] },
      { "role": "user", "text": "", "auto": true, "autoReason": "compact" },
      { "role": "agent", "blocks": [{ "type": "compaction", "id": "c", "status": "completed" }] },
      { "role": "user", "text": "Now the parser" },
      { "role": "agent", "blocks": [{ "type": "plan", "entries": [{ "title": "rewrite", "status": "completed" }, { "title": "bench", "status": "pending" }] },
        { "type": "text", "markdown": "Parser rewritten." }], "error": { "message": "rate limited" } },
    ]))
  }

  #[test]
  fn numbers_turns_by_user_message() {
    let r = sample();
    let ns: Vec<u32> = rounds(&r.turns).iter().map(|x| x.n).collect();
    // A leading agent turn is 1, the typed prompt and its answer 2, the compaction joins 2, the next prompt 3
    assert_eq!(ns, vec![1, 2, 3]);
    let sizes: Vec<usize> = rounds(&r.turns).iter().map(|x| x.items.len()).collect();
    assert_eq!(sizes, vec![1, 4, 2]);
    // Two agent turns in a row are two turns
    let mut c = TurnCounter::default();
    assert_eq!((c.agent(), c.agent(), c.user(false), c.agent(), c.user(true), c.agent()), (1, 2, 3, 3, 3, 3));
  }

  #[test]
  fn outline_lists_facts_turns_and_the_last_reply() {
    let o = outline(&sample(), &META);
    assert!(o.starts_with("# Fix the parser\nLink: acpira://session/s1\nAgent: Claude\nProject: /repo\n"));
    assert!(o.contains("Turns: 3"));
    assert!(o.contains("- Edited files (1): src/lexer.rs (+12 −3)"));
    assert!(o.contains("- Plan / to-dos (last state): [x] rewrite; [ ] bench"));
    assert!(o.contains("- Last error (turn 3): rate limited"));
    assert!(o.contains("2. User: Make the lexer faster | (automatic: compaction) | Steer: also keep the old API | Reply: Done: the lexer now takes a fast path. | 1 tools, 1 edits"));
    assert!(o.contains("## Last reply (turn 3)\nParser rewritten."));
    assert!(!o.contains("secret thinking"));
  }

  #[test]
  fn long_sessions_fold_the_middle_of_the_outline() {
    let mut turns = vec![];
    for i in 0..400 {
      turns.push(serde_json::json!({ "role": "user", "text": format!("question {i} {}", "x".repeat(150)) }));
      turns.push(serde_json::json!({ "role": "agent", "blocks": [{ "type": "text", "markdown": format!("answer {i} {}", "y".repeat(200)) }] }));
    }
    let o = outline(&record(serde_json::Value::Array(turns)), &META);
    assert!(o.len() <= OUTLINE_BYTES);
    assert!(o.contains("1. User: question 0"));
    assert!(o.contains("400. User: question 399"));
    assert!(o.contains("omitted; read them with read_session"));
  }

  #[test]
  fn range_pages_stop_on_turn_boundaries() {
    let r = sample();
    let p = range(&r, &META, 2, Some(2), 0, false);
    assert!(p.contains("## Turn 2"));
    assert!(p.contains("**User:**\nMake the lexer faster"));
    assert!(p.contains("**User (steered mid-turn):**\nalso keep the old API"));
    assert!(p.contains("- [tool] Edit src/lexer.rs (+12 −3)"));
    assert!(p.contains("**User (automatic: compaction):**"));
    assert!(!p.contains("fast_path()"));
    assert!(!p.contains("secret thinking"));
    assert!(p.contains("(turns 3-3 follow)"));
    assert!(range(&r, &META, 2, Some(2), 0, true).contains("+fast_path()"));

    let mut turns = vec![];
    for i in 0..30 {
      turns.push(serde_json::json!({ "role": "user", "text": format!("q{i}") }));
      turns.push(serde_json::json!({ "role": "agent", "blocks": [{ "type": "text", "markdown": "z".repeat(3000) }] }));
    }
    let big = record(serde_json::Value::Array(turns));
    let page = range(&big, &META, 1, None, 0, false);
    assert!(page.len() <= PAGE_BYTES);
    let next: u32 = page.split("from: ").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
    assert!(next > 1 && next < 30);
    assert!(page.contains(&format!("## Turn {}", next - 1)) && !page.contains(&format!("## Turn {next}\n")));
    // One turn bigger than a page is cut, and the following pages go on inside it until the next turn
    let huge = record(serde_json::json!([{ "role": "user", "text": "a" }, { "role": "agent", "blocks": [{ "type": "text", "markdown": format!("{}THE-END", "w".repeat(40_000)) }] }, { "role": "user", "text": "b" }]));
    let mut offset = 0;
    let mut pages = vec![];
    loop {
      let page = range(&huge, &META, 1, None, offset, false);
      assert!(page.len() <= PAGE_BYTES);
      let more = page.split("More: read_session {session, from: ").nth(1).map(|m| m.to_owned());
      pages.push(page);
      let Some(more) = more else { break };
      if more.starts_with("2,") {
        break;
      }
      let at: usize = more.split("offset: ").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
      assert!(at > offset);
      offset = at;
      assert!(pages.len() < 10);
    }
    assert!(pages.len() >= 3 && pages[0].contains("turn 1 is cut here"));
    assert!(pages[1].contains("(turn 1 continued from byte"));
    let w: usize = pages.iter().map(|p| p.matches('w').count()).sum();
    assert!(w >= 40_000, "every byte of the turn is read once: {w}");
    assert!(pages.last().unwrap().contains("THE-END") && pages.last().unwrap().contains("**User:**\nb"));
    assert!(range(&huge, &META, 9, None, 0, false).contains("No turns in 9-2"));
  }

  #[test]
  fn grep_finds_text_in_tools_and_replies_with_turns() {
    let r = sample();
    let g = grep(&r, "FAST", GREP_HITS);
    assert!(g.contains("- turn 2 (user): Make the lexer faster"));
    assert!(g.contains("- turn 2 (tool: Edit src/lexer.rs): Edit src/lexer.rs +fast_path()"));
    assert!(g.contains("- turn 2 (reply): Done: the lexer now takes a fast path."));
    assert!(grep(&r, "secret", GREP_HITS).contains("No matches."));
    let mut turns = vec![];
    for _ in 0..50 {
      turns.push(serde_json::json!({ "role": "user", "text": format!("needle {}", "n".repeat(300)) }));
    }
    let g = grep(&record(serde_json::Value::Array(turns)), "needle", GREP_HITS);
    assert!(g.len() <= SEARCH_BYTES);
    assert_eq!(g.matches("- turn ").count(), GREP_HITS);
    assert!(g.contains("(+42 more matches"));
  }

  #[test]
  fn cut_respects_char_boundaries() {
    assert_eq!(cut("会话会话", 7), "会…");
    assert_eq!(cut("abc", 3), "abc");
  }
}
