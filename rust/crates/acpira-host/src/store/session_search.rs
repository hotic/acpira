//! Full-text search over the saved session records, for the history list's search box. Only the conversation is
//! indexed: user prompts, steered messages and the agent's reply text; tool output, thoughts, plans and attachments stay
//! out, so a hit reads like a line of the chat. Titles are matched by the webview itself and only join here so a
//! multi-word query may split its terms between the title and the conversation.
//!
//! The extracted text is cached per record file and re-read only when the file's modification time or size changes,
//! so a query typed one key at a time parses each record once. The records directory is shared by every window, which
//! is why the cache revalidates against the disk on every search instead of listening to this host's own writes.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use serde::Deserialize;

use acpira_shared::protocol::SessionHit;

use crate::store::transcript_store::is_session_id;

/// Characters of context kept before the first match, and the whole snippet's length; the row truncates the rest
const SNIPPET_BEFORE: usize = 24;
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
    return Entry { stamp, title: String::new(), messages: vec![], folded: vec![] };
  };
  let mut messages = vec![];
  for turn in r.turns {
    match turn {
      LiteTurn::User { text } => messages.push(collapse(&text)),
      LiteTurn::Agent { blocks } => {
        for b in blocks {
          match b {
            LiteBlock::Text { markdown } => messages.push(collapse(&markdown)),
            LiteBlock::Steer { text } => messages.push(collapse(&text)),
            LiteBlock::Other => {}
          }
        }
      }
      LiteTurn::Other => {}
    }
  }
  messages.retain(|m| !m.is_empty());
  let folded = messages.iter().map(|m| fold(m)).collect();
  Entry { stamp, title: fold(&r.title), messages, folded }
}

/// The text around the first occurrence of `term` in `message`, `pos` being its byte offset in the folded copy
fn snippet(message: &str, folded: &str, pos: usize, term: &str) -> String {
  let chars: Vec<char> = message.chars().collect();
  let at = folded[..pos].chars().count();
  let term_len = term.chars().count();
  let mut start = at.saturating_sub(SNIPPET_BEFORE);
  let end = (start + SNIPPET_LEN).max(at + term_len).min(chars.len());
  // A match near the end of a message pulls the window back so the snippet still carries its full length of context
  if end - start < SNIPPET_LEN {
    start = end.saturating_sub(SNIPPET_LEN);
  }
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

/// Every term must occur in the title or the conversation; the snippet comes from the first message holding any term.
/// A session whose terms all sit in its title is left to the webview's own title match
fn hit_of(id: &str, e: &Entry, terms: &[String]) -> Option<SessionHit> {
  if !terms.iter().all(|t| e.title.contains(t.as_str()) || e.folded.iter().any(|m| m.contains(t.as_str()))) {
    return None;
  }
  for (message, folded) in e.messages.iter().zip(&e.folded) {
    let first = terms.iter().filter_map(|t| folded.find(t.as_str()).map(|pos| (pos, t))).min_by_key(|(pos, _)| *pos);
    if let Some((pos, term)) = first {
      return Some(SessionHit { id: id.to_owned(), snippet: snippet(message, folded, pos, term) });
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
    assert!(s[..s.find("needle").unwrap()].trim_start_matches('…').chars().count() == SNIPPET_BEFORE);
  }

  #[test]
  fn folding_keeps_char_positions_for_wide_text() {
    let e = entry("t", r#"[{"role":"agent","blocks":[{"type":"text","markdown":"İstanbul 会话搜索 ÄBC"}]}]"#);
    assert_eq!(hit_of("a", &e, &terms_of("搜索")).unwrap().snippet, "İstanbul 会话搜索 ÄBC");
    assert_eq!(hit_of("a", &e, &terms_of("äbc")).unwrap().snippet, "İstanbul 会话搜索 ÄBC");
  }

  #[test]
  fn unreadable_records_are_skipped() {
    let e = entry_of((None, 0), b"{not json");
    assert!(e.messages.is_empty());
  }
}
