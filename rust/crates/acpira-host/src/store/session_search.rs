//! Full-text search over the saved session records, for the history list's search box. Only the conversation is
//! indexed: user prompts, steered messages and the agent's reply text; tool output, thoughts, plans and attachments stay
//! out, so a hit reads like a line of the chat. Titles are matched by the webview itself and only join here so a
//! multi-word query may split its terms between the title and the conversation.
//!
//! The extracted text is cached per record file and re-read only when the file's modification time or size changes,
//! so a query typed one key at a time parses each record once. The records directory is shared by every window, which
//! is why the cache revalidates against the disk on every search instead of listening to this host's own writes.
//!
//! Each cached message also carries its turn number (`session_digest::TurnCounter`), which agents' `search_sessions`
//! answers with (`tool_hits`); the history list ignores it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::Deserialize;

use acpira_shared::protocol::SessionHit;

use crate::store::session_digest::TurnCounter;
use crate::store::transcript_store::is_session_id;

/// Context kept before the first match, in display columns (a CJK character counts two), and the whole snippet's length
/// in characters. The history row shows about forty columns and truncates the rest, so the lead stays short enough for
/// the match itself to land on screen
const SNIPPET_BEFORE: usize = 12;
const SNIPPET_LEN: usize = 160;
/// Replies stay bounded however broad the query; the most recently written records win
pub const HITS_MAX: usize = 200;
/// Longer queries are cut here: nothing useful is typed past it and the terms are matched against every record
pub const QUERY_MAX: usize = 200;

/// A record reduced to what search reads. Unknown turn roles and block types deserialize as `Other`, so a record from a
/// newer build still contributes its known parts
#[derive(Deserialize)]
struct LiteRecord {
  #[serde(default)]
  title: String,
  #[serde(default)]
  turns: Vec<LiteTurn>,
}

#[derive(Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
enum LiteTurn {
  User {
    #[serde(default)]
    text: String,
    #[serde(default)]
    auto: Option<bool>,
  },
  Agent {
    #[serde(default)]
    blocks: Vec<LiteBlock>,
  },
  #[serde(other)]
  Other,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum LiteBlock {
  Text {
    #[serde(default)]
    markdown: String,
  },
  Steer {
    #[serde(default)]
    text: String,
  },
  #[serde(other)]
  Other,
}

/// What a record file looked like when it was read: a changed stamp means read it again
type Stamp = (Option<SystemTime>, u64);

struct Entry {
  stamp: Stamp,
  /// Folded title, for terms that only the title carries
  title: String,
  /// Each message with its whitespace collapsed, and the same text folded; both have the same number of chars
  messages: Vec<String>,
  folded: Vec<String>,
  /// The turn each message belongs to, numbered as `read_session` numbers them
  turns: Vec<u32>,
}

/// Lower-cases one char to one char, so a match position in folded text is the same char position in the original
pub fn fold(s: &str) -> String {
  s.chars().map(|c| c.to_lowercase().next().unwrap_or(c)).collect()
}

fn collapse(s: &str) -> String {
  s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn entry_of(stamp: Stamp, raw: &[u8]) -> Entry {
  let Ok(r) = serde_json::from_slice::<LiteRecord>(raw) else {
    // An unreadable record is remembered empty under its stamp, so it is not parsed again until it changes
    return Entry { stamp, title: String::new(), messages: vec![], folded: vec![], turns: vec![] };
  };
  let mut numbered: Vec<(u32, String)> = vec![];
  let mut counter = TurnCounter::default();
  for turn in r.turns {
    match turn {
      LiteTurn::User { text, auto } => {
        let n = counter.user(auto == Some(true));
        numbered.push((n, collapse(&text)));
      }
      LiteTurn::Agent { blocks } => {
        let n = counter.agent();
        for b in blocks {
          match b {
            LiteBlock::Text { markdown } => numbered.push((n, collapse(&markdown))),
            LiteBlock::Steer { text } => numbered.push((n, collapse(&text))),
            LiteBlock::Other => {}
          }
        }
      }
      // A role this build does not know is not numbered: read_session cannot read it either
      LiteTurn::Other => {}
    }
  }
  numbered.retain(|(_, m)| !m.is_empty());
  let (turns, messages): (Vec<u32>, Vec<String>) = numbered.into_iter().unzip();
  let folded = messages.iter().map(|m| fold(m)).collect();
  Entry { stamp, title: fold(&r.title), messages, folded, turns }
}

/// The text from a short lead before the first occurrence of `term` in `message`, `pos` being its byte offset in the
/// folded copy. The match always sits near the start, even at the end of a long message, since the row cuts the tail
fn snippet(message: &str, folded: &str, pos: usize, term: &str) -> String {
  let chars: Vec<char> = message.chars().collect();
  let at = folded.get(..pos).unwrap_or_default().chars().count();
  let term_len = term.chars().count();
  let mut start = at;
  let mut width = 0;
  while start > 0 && width + col_width(chars[start - 1]) <= SNIPPET_BEFORE {
    start -= 1;
    width += col_width(chars[start]);
  }
  // A cut through a word moves forward to the next word start, as long as one comes before the match
  if start > 0
    && !chars[start - 1].is_whitespace()
    && !chars[start].is_whitespace()
    && let Some(space) = chars[start..at].iter().position(|c| c.is_whitespace())
  {
    start += space + 1;
  }
  let end = (start + SNIPPET_LEN).max(at + term_len).min(chars.len());
  let mut out = String::new();
  if start > 0 {
    out.push('…');
  }
  out.extend(&chars[start..end]);
  if end < chars.len() {
    out.push('…');
  }
  out
}

/// Display columns of a character: two for East Asian wide and fullwidth forms and emoji, one otherwise
fn col_width(c: char) -> usize {
  match c as u32 {
    0x1100..=0x115F | 0x2E80..=0x303E | 0x3041..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE4F
    | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 | 0x1F300..=0x1FAFF | 0x20000..=0x3FFFD => 2,
    _ => 1,
  }
}

/// Every term must occur in the title or the conversation; the snippet comes from the first message holding any term.
/// A session whose terms all sit in its title is left to the webview's own title match
fn hit_of(id: &str, e: &Entry, terms: &[String]) -> Option<SessionHit> {
  numbered_hit(id, e, terms).map(|(_, h)| h)
}

/// `hit_of` with the turn of the message the snippet comes from
fn numbered_hit(id: &str, e: &Entry, terms: &[String]) -> Option<(u32, SessionHit)> {
  if !terms.iter().all(|t| e.title.contains(t.as_str()) || e.folded.iter().any(|m| m.contains(t.as_str()))) {
    return None;
  }
  for ((message, folded), turn) in e.messages.iter().zip(&e.folded).zip(&e.turns) {
    let first = terms.iter().filter_map(|t| folded.find(t.as_str()).map(|pos| (pos, t))).min_by_key(|(pos, _)| *pos);
    if let Some((pos, term)) = first {
      return Some((*turn, SessionHit { id: id.to_owned(), snippet: snippet(message, folded, pos, term) }));
    }
  }
  None
}

/// The distinct folded terms of a query, cut to `QUERY_MAX` chars
pub fn terms_of(query: &str) -> Vec<String> {
  let cut: String = query.chars().take(QUERY_MAX).collect();
  let mut seen = HashSet::new();
  fold(&cut).split_whitespace().filter(|t| seen.insert(t.to_string())).map(str::to_owned).collect()
}

/// One per manager: the cache outlives single searches and is shared by every view of this sidecar
#[derive(Default)]
pub struct SessionSearch {
  cache: Arc<Mutex<HashMap<String, Entry>>>,
}

impl SessionSearch {
  /// Search the records under `dir`. File reads and parsing run on the blocking pool; concurrent searches take turns
  /// on the cache, so a burst of keystrokes never parses the same record twice
  pub async fn search(&self, dir: PathBuf, query: &str) -> Vec<SessionHit> {
    let terms = terms_of(query);
    if terms.is_empty() {
      return vec![];
    }
    let cache = self.cache.clone();
    tokio::task::spawn_blocking(move || {
      let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
      refresh(&mut cache, &dir);
      let mut hits: Vec<(Option<SystemTime>, SessionHit)> =
        cache.iter().filter_map(|(id, e)| hit_of(id, e, &terms).map(|h| (e.stamp.0, h))).collect();
      hits.sort_by_key(|h| std::cmp::Reverse(h.0));
      hits.truncate(HITS_MAX);
      hits.into_iter().map(|(_, h)| h).collect()
    })
    .await
    .unwrap_or_default()
  }
}

impl SessionSearch {
  /// The conversation hits among `ids` (every record when None) for an agent's `search_sessions`: id → (turn, snippet).
  /// Same matching as the history list; a session whose terms all sit in its title is the caller's to add
  pub async fn tool_hits(&self, dir: PathBuf, query: &str, ids: Option<HashSet<String>>) -> HashMap<String, (u32, String)> {
    let terms = terms_of(query);
    if terms.is_empty() {
      return HashMap::new();
    }
    let cache = self.cache.clone();
    tokio::task::spawn_blocking(move || {
      let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
      refresh(&mut cache, &dir);
      cache
        .iter()
        .filter(|(id, _)| ids.as_ref().is_none_or(|ids| ids.contains(*id)))
        .filter_map(|(id, e)| numbered_hit(id, e, &terms).map(|(turn, h)| (id.clone(), (turn, h.snippet))))
        .collect()
    })
    .await
    .unwrap_or_default()
  }
}

/// Bring the cache in line with the directory: new or changed records are read, vanished ones dropped
fn refresh(cache: &mut HashMap<String, Entry>, dir: &Path) {
  let Ok(rd) = std::fs::read_dir(dir) else {
    cache.clear();
    return;
  };
  let mut present = HashSet::new();
  for e in rd.flatten() {
    let name = e.file_name().to_string_lossy().into_owned();
    let Some(id) = name.strip_suffix(".json").filter(|id| is_session_id(id)) else { continue };
    let Ok(meta) = e.metadata() else { continue };
    if !meta.is_file() {
      continue;
    }
    let stamp = (meta.modified().ok(), meta.len());
    present.insert(id.to_owned());
    if cache.get(id).is_some_and(|c| c.stamp == stamp) {
      continue;
    }
    let Ok(raw) = std::fs::read(e.path()) else { continue };
    cache.insert(id.to_owned(), entry_of(stamp, &raw));
  }
  cache.retain(|id, _| present.contains(id));
}

#[cfg(test)]
mod tests {
  use super::*;

  fn entry(title: &str, raw: &str) -> Entry {
    entry_of((None, 0), format!(r#"{{"title":{},"turns":{raw}}}"#, serde_json::to_string(title).unwrap()).as_bytes())
  }

  #[test]
  fn only_the_conversation_is_indexed() {
    let e = entry(
      "t",
      r#"[{"role":"user","text":"hello\n\n  world"},{"role":"agent","blocks":[
        {"type":"thought","text":"secret thinking"},
        {"type":"tool_call","id":"1","title":"grep needle"},
        {"type":"text","markdown":"The **answer**"},
        {"type":"steer","id":"s","text":"steer me"},
        {"type":"future_block","x":1}]},{"role":"narrator"}]"#,
    );
    assert_eq!(e.messages, vec!["hello world", "The **answer**", "steer me"]);
    assert_eq!(e.turns, vec![1, 1, 1]);
    assert!(hit_of("a", &e, &terms_of("secret")).is_none());
    assert!(hit_of("a", &e, &terms_of("needle")).is_none());
    assert_eq!(hit_of("a", &e, &terms_of("ANSWER")).unwrap().snippet, "The **answer**");
  }

  #[test]
  fn every_term_must_match_across_title_and_messages() {
    let e = entry("Refactor parser", r#"[{"role":"user","text":"make the lexer faster"}]"#);
    assert_eq!(hit_of("a", &e, &terms_of("parser lexer")).unwrap().snippet, "make the lexer faster");
    assert!(hit_of("a", &e, &terms_of("lexer missing")).is_none());
    // Title-only matches belong to the webview's title filter
    assert!(hit_of("a", &e, &terms_of("refactor")).is_none());
  }

  #[test]
  fn snippets_keep_context_and_mark_cuts() {
    let long = format!("{}needle{}", "a".repeat(100), "b".repeat(300));
    let e = entry("t", &format!(r#"[{{"role":"user","text":"{long}"}}]"#));
    let s = hit_of("a", &e, &terms_of("NEEDLE")).unwrap().snippet;
    assert!(s.starts_with('…') && s.ends_with('…'));
    assert_eq!(s.chars().count(), SNIPPET_LEN + 2);
    assert_eq!(s.split_once("needle").unwrap().0.trim_start_matches('…').chars().count(), SNIPPET_BEFORE);
  }

  #[test]
  fn snippets_lead_with_the_match_however_late_it_comes() {
    // Wide characters spend two columns of the lead, and a match at the end of a long message is not pulled back
    let e = entry("t", &format!(r#"[{{"role":"user","text":"{}标题"}}]"#, "会话".repeat(100)));
    assert_eq!(hit_of("a", &e, &terms_of("标题")).unwrap().snippet, "…会话会话会话标题");
    // A lead that starts inside a word skips to the next word
    let e = entry("t", r#"[{"role":"user","text":"the quick brownish fox needle"}]"#);
    assert_eq!(hit_of("a", &e, &terms_of("needle")).unwrap().snippet, "…fox needle");
  }

  #[test]
  fn folding_keeps_char_positions_for_wide_text() {
    let e = entry("t", r#"[{"role":"agent","blocks":[{"type":"text","markdown":"İstanbul 会话搜索 ÄBC"}]}]"#);
    assert_eq!(hit_of("a", &e, &terms_of("会话")).unwrap().snippet, "İstanbul 会话搜索 ÄBC");
    assert_eq!(hit_of("a", &e, &terms_of("äbc")).unwrap().snippet, "…会话搜索 ÄBC");
  }

  // The numbers must be read_session's: session_digest::rounds over the same record
  #[test]
  fn messages_carry_the_turn_numbers_read_session_uses() {
    let raw = r#"[{"role":"agent","blocks":[{"type":"text","markdown":"hi"}]},{"role":"user","text":"one"},
      {"role":"agent","blocks":[{"type":"text","markdown":"first"}]},{"role":"user","text":"go on","auto":true,"autoReason":"retry"},
      {"role":"agent","blocks":[{"type":"text","markdown":"more"}]},{"role":"user","text":"two"},{"role":"agent","blocks":[{"type":"steer","id":"s","text":"wait"}]}]"#;
    let e = entry("t", raw);
    assert_eq!(e.turns, vec![1, 2, 2, 2, 2, 3, 3]);
    assert_eq!(numbered_hit("a", &e, &terms_of("more")).unwrap().0, 2);
    assert_eq!(numbered_hit("a", &e, &terms_of("wait")).unwrap().0, 3);
  }

  #[test]
  fn unreadable_records_are_skipped() {
    let e = entry_of((None, 0), b"{not json");
    assert!(e.messages.is_empty());
  }
}
