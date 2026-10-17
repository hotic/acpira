//! What agents read of other Acpira sessions (`search_sessions` / `read_session` in `acpira mcp`, through the relay hub).
//! `SessionManager` implements the narrow `SessionLibrary`: it supplies the summaries, the record snapshots and the
//! conversation hits of `SessionSearch`; the choices on top (which sessions a caller sees, how a reference resolves, how a
//! list reads) are the pure functions here. Every answer is Markdown text within `session_digest`'s byte caps, and every
//! session in it is written as `[title](acpira://session/<id>)`, so a model quoting it hands the user a clickable link.
//!
//! The caller is whoever holds the MCP entry's grant: a session's own agent (`summoned: false`) never gets its own
//! session back (it has that conversation in context already), a summoned child may read the session that summoned it

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;

use acpira_shared::session_links::{session_link, session_ref_id};
use acpira_shared::transcript::{AgentInfo, SessionSummary};

use crate::relay::wire::{SessionReadArgs, SessionSearchArgs};
use crate::store::session_digest::{SEARCH_BYTES, cut};
use crate::store::session_search::{fold, terms_of};

/// Sessions a list answer shows, by default and at most
pub const LIST_DEFAULT: usize = 8;
pub const LIST_MAX: usize = 20;

/// The answer to a root agent asking for its own session
pub const SELF_READ: &str = "That is your own current session: its conversation is already in your context, so there is nothing to read. To search older sessions, call search_sessions without `session`.";

/// Who is asking, from the hub's grant
#[derive(Debug, Clone)]
pub struct LibraryCaller {
  pub session_id: String,
  pub cwd: String,
  /// A summoned child's CLI (the grant names a relay thread): the session is its parent, not its own conversation
  pub summoned: bool,
}

impl LibraryCaller {
  /// The caller's own conversation: a root agent's session (a summoned child's grant names its parent instead)
  pub fn is_self_session(&self, id: &str) -> bool {
    !self.summoned && self.session_id == id
  }
}

/// `Ok` is the tool's text, `Err` a message the model should read
pub type LibraryFuture<'a> = Pin<Box<dyn Future<Output = Result<String, String>> + Send + 'a>>;

pub trait SessionLibrary: Send + Sync {
  fn search_sessions<'a>(&'a self, caller: &'a LibraryCaller, args: SessionSearchArgs) -> LibraryFuture<'a>;
  fn read_session<'a>(&'a self, caller: &'a LibraryCaller, args: SessionReadArgs) -> LibraryFuture<'a>;
}

/// The session a reference names: a link or id the list knows, else the agent's own session id of one. An id-shaped
/// reference the list does not know is still returned, so the record load can say what is wrong with it
pub fn resolve_ref(input: &str, sessions: &[SessionSummary]) -> Option<String> {
  let input = input.trim();
  let parsed = session_ref_id(input);
  if let Some(id) = &parsed
    && sessions.iter().any(|s| &s.id == id)
  {
    return parsed;
  }
  sessions.iter().find(|s| s.acp_session_id.as_deref() == Some(input)).map(|s| s.id.clone()).or(parsed)
}

fn same_dir(a: &str, b: &str) -> bool {
  let trim = |s: &str| s.trim_end_matches(['/', '\\']).to_owned();
  let (a, b) = (trim(a), trim(b));
  if cfg!(windows) { a.eq_ignore_ascii_case(&b) } else { a == b }
}

/// The agent ids an `agent` filter means: an id or display name, case-insensitive; an unknown one is taken as an id
pub fn agent_ids(filter: &str, agents: &[AgentInfo]) -> HashSet<String> {
  let f = filter.trim().to_lowercase();
  let hit: HashSet<String> = agents.iter().filter(|a| a.id.to_lowercase() == f || a.name.to_lowercase() == f).map(|a| a.id.clone()).collect();
  if hit.is_empty() { HashSet::from([f]) } else { hit }
}

/// The sessions a list may show this caller, newest first: its project unless `all`, the agent filter, never itself
pub fn candidates<'a>(sessions: &'a [SessionSummary], caller: &LibraryCaller, all: bool, agents: Option<&HashSet<String>>) -> Vec<&'a SessionSummary> {
  let mut out: Vec<&SessionSummary> = sessions
    .iter()
    .filter(|s| !caller.is_self_session(&s.id))
    .filter(|s| all || same_dir(&s.cwd, &caller.cwd))
    .filter(|s| agents.is_none_or(|ids| ids.contains(&s.agent)))
    .collect();
  out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
  out
}

fn title_of(s: &SessionSummary) -> String {
  let t = s.title.split_whitespace().collect::<Vec<_>>().join(" ");
  // Brackets would end the Markdown link text early
  let t = t.replace(['[', ']'], "");
  if t.is_empty() { "(untitled)".into() } else { cut(&t, 120) }
}

fn when(iso: &str) -> String {
  match (iso.get(..10), iso.get(11..16)) {
    (Some(d), Some(t)) => format!("{d} {t} UTC"),
    _ => iso.to_owned(),
  }
}

/// One listed session: the link, then agent, time and (outside the caller's project) the folder; a hit's turn and snippet
pub fn session_entry(s: &SessionSummary, hit: Option<&(u32, String)>, caller_cwd: &str, names: &HashMap<String, String>) -> String {
  let agent = names.get(&s.agent).cloned().unwrap_or_else(|| s.agent.clone());
  let mut line = format!("- [{}]({}) | {agent} | {}", title_of(s), session_link(&s.id), when(&s.updated_at));
  if !same_dir(&s.cwd, caller_cwd) && !s.cwd.is_empty() {
    line.push_str(&format!(" | {}", s.cwd));
  }
  match s.state {
    Some(acpira_shared::transcript::SummaryState::Working) => line.push_str(" | running now"),
    Some(acpira_shared::transcript::SummaryState::Background) => line.push_str(" | turn ended, background task still running"),
    _ => {}
  }
  if let Some((turn, snippet)) = hit {
    line.push_str(&format!("\n  turn {turn}: {}", cut(snippet, 240)));
  }
  line
}

const READ_HINT: &str = "Read one with read_session {session: \"<link>\"}: the outline first, then only the turns you need (from / to).";

/// The list answer: a heading, the entries that fit in `SEARCH_BYTES`, and how to read on
pub fn render_list(heading: &str, entries: &[String], total: usize, empty: &str) -> String {
  let mut out = format!("{heading}\n");
  if entries.is_empty() {
    out.push_str(empty);
    out.push('\n');
    return out;
  }
  let mut shown = 0;
  for e in entries {
    if out.len() + e.len() + READ_HINT.len() + 80 > SEARCH_BYTES {
      break;
    }
    out.push_str(e);
    out.push('\n');
    shown += 1;
  }
  if total > shown {
    out.push_str(&format!("(showing {shown} of {total}; narrow the query or raise limit)\n"));
  }
  out.push('\n');
  out.push_str(READ_HINT);
  out.push('\n');
  out
}

/// Sessions matching every term in the title or the conversation (`body`: id → turn and snippet), newest first
pub fn matches<'a>(cands: &[&'a SessionSummary], query: &str, body: &HashMap<String, (u32, String)>) -> Vec<&'a SessionSummary> {
  let terms = terms_of(query);
  cands.iter().copied().filter(|s| body.contains_key(&s.id) || (!terms.is_empty() && terms.iter().all(|t| fold(&s.title).contains(t.as_str())))).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  fn s(id: &str, title: &str, agent: &str, cwd: &str, at: &str) -> SessionSummary {
    serde_json::from_value(serde_json::json!({ "id": id, "title": title, "agent": agent, "cwd": cwd, "updatedAt": at })).unwrap()
  }

  fn list() -> Vec<SessionSummary> {
    let mut native = s("n1", "Native one", "codex", "/repo", "2026-10-03T00:00:00.000Z");
    native.acp_session_id = Some("019a-codex-rollout".into());
    vec![
      s("me", "This chat", "claude", "/repo", "2026-10-05T00:00:00.000Z"),
      s("a1", "Fix [lexer] speed", "claude", "/repo/", "2026-10-04T00:00:00.000Z"),
      native,
      s("x1", "Other project", "claude", "/elsewhere", "2026-10-06T00:00:00.000Z"),
    ]
  }

  const ROOT: fn() -> LibraryCaller = || LibraryCaller { session_id: "me".into(), cwd: "/repo".into(), summoned: false };

  #[test]
  fn candidates_keep_the_project_drop_the_caller_and_filter_agents() {
    let l = list();
    let ids = |v: Vec<&SessionSummary>| v.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(candidates(&l, &ROOT(), false, None)), vec!["a1", "n1"]);
    assert_eq!(ids(candidates(&l, &ROOT(), true, None)), vec!["x1", "a1", "n1"]);
    let agents: Vec<AgentInfo> = serde_json::from_value(serde_json::json!([{ "id": "codex", "name": "Codex" }])).unwrap();
    assert_eq!(ids(candidates(&l, &ROOT(), true, Some(&agent_ids("CODEX", &agents)))), vec!["n1"]);
    // A summoned child reads its parent: the session is not hidden from it
    let child = LibraryCaller { summoned: true, ..ROOT() };
    assert_eq!(ids(candidates(&l, &child, false, None)), vec!["me", "a1", "n1"]);
  }

  #[test]
  fn references_resolve_by_link_id_and_native_id() {
    let l = list();
    assert_eq!(resolve_ref("vscode://hotic.acpira/session/a1", &l).as_deref(), Some("a1"));
    assert_eq!(resolve_ref(" a1 ", &l).as_deref(), Some("a1"));
    assert_eq!(resolve_ref("019a-codex-rollout", &l).as_deref(), Some("n1"));
    assert_eq!(resolve_ref("unknown-id", &l).as_deref(), Some("unknown-id"));
    assert_eq!(resolve_ref("two words", &l), None);
  }

  #[test]
  fn title_matches_join_conversation_hits_and_entries_link_back() {
    let l = list();
    let c = candidates(&l, &ROOT(), true, None);
    let body = HashMap::from([("n1".to_string(), (3, "…the rollout file".to_string()))]);
    let m = matches(&c, "LEXER", &body);
    assert_eq!(m.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["a1", "n1"]);
    let names = HashMap::from([("claude".to_string(), "Claude".to_string())]);
    assert_eq!(session_entry(m[0], None, "/repo", &names), "- [Fix lexer speed](acpira://session/a1) | Claude | 2026-10-04 00:00 UTC");
    assert_eq!(session_entry(&l[3], body.get("n1"), "/repo", &names), "- [Other project](acpira://session/x1) | Claude | 2026-10-06 00:00 UTC | /elsewhere\n  turn 3: …the rollout file");
  }

  #[test]
  fn lists_stay_within_the_search_cap() {
    let entries: Vec<String> = (0..200).map(|i| format!("- entry {i} {}", "x".repeat(100))).collect();
    let out = render_list("Sessions:", &entries, 200, "none");
    assert!(out.len() <= SEARCH_BYTES);
    assert!(out.contains("(showing") && out.contains("read_session"));
    assert!(render_list("Sessions:", &[], 0, "No sessions match.").contains("No sessions match."));
  }
}
