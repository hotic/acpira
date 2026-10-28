//! The chunk's items as the numbered lines `history_grep` / `history_list` read, and the user's own words in them

use std::path::Path;

use super::{Chunk, LINE_CHARS};
use crate::llm::{Item, Part};

/// A user message's text without the notes the agent put in front of it (mode, MCP servers, sidekick, goal), in
/// whatever order they were stacked
pub(crate) fn strip_notes(mut t: &str) -> &str {
  const NOTES: [(&str, &str); 4] = [("<mode>\n", "</mode>"), ("<mcp>\n", "</mcp>"), ("<sidekick>\n", "</sidekick>"), ("<goal>\n", "</goal>")];
  while let Some(after) = NOTES.iter().find_map(|(open, close)| t.strip_prefix(open)?.split_once(close).map(|(_, a)| a)) {
    t = after.trim_start_matches('\n');
  }
  t
}

/// The user's own words in a prompt: the notes the agent put in front (`strip_notes`), and the skill bodies it put
/// behind, left out; images named. A goal turn's are the objective a set turn carries, and none for a continuation
fn prompt_words(parts: &[Part]) -> String {
  let mut out = vec![];
  for p in parts {
    match p {
      Part::Text(t) => {
        let t = strip_notes(t);
        let t = t.split("\n\n<skill name=\"").next().unwrap_or(t);
        if !t.trim().is_empty() {
          out.push(t.to_owned());
        }
      }
      Part::Image { mime, .. } => out.push(format!("[Attached {mime}]")),
    }
  }
  out.join("\n\n")
}

/// The user's words in an item: a prompt's, or a message steered into the turn (a note steered in alone, the goal's
/// budget note, holds none)
pub(crate) fn user_words(item: &Item) -> Option<String> {
  if let Some(t) = crate::steer::text_of(item) {
    return Some(t).filter(|t| !t.is_empty());
  }
  match item {
    Item::User(parts) => Some(prompt_words(parts)).filter(|t| !t.trim().is_empty()),
    _ => None,
  }
}

/// One line of the history the tools read: its number in the chunk, where it came from, its text
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
  pub n: usize,
  pub source: &'static str,
  pub text: String,
}

/// The chunk's items as numbered lines (`USER`, `ASSISTANT`, `ASSISTANT_TOOL`, `TOOL_OUTPUT`, `TOOL_ERROR`); reasoning
/// is never shown. Numbers are local to the chunk, so they stay the same whatever else the session holds
pub fn transcript(region: &[Item]) -> Vec<Line> {
  transcript_with_starts(region).0
}

/// The lines, and the first line of each user message in order
pub(super) fn transcript_with_starts(region: &[Item]) -> (Vec<Line>, Vec<usize>) {
  let mut blocks: Vec<(&'static str, String)> = vec![];
  for item in region {
    if let Some(words) = user_words(item) {
      blocks.push(("USER", words));
      continue;
    }
    match item {
      Item::User(_) => {}
      Item::Assistant { text, tool_calls, .. } => {
        if !text.trim().is_empty() {
          blocks.push(("ASSISTANT", text.clone()));
        }
        for c in tool_calls {
          blocks.push(("ASSISTANT_TOOL", format!("{}({}) [call={}]", c.name, c.arguments, c.id)));
        }
      }
      Item::ToolResult { content, is_error, images, .. } => {
        let mut text = content.clone();
        if !images.is_empty() {
          text.push_str(&format!("\n[{} image(s)]", images.len()));
        }
        blocks.push((if *is_error { "TOOL_ERROR" } else { "TOOL_OUTPUT" }, text));
      }
    }
  }
  let (mut lines, mut starts) = (vec![], vec![]);
  for (source, text) in blocks {
    if source == "USER" {
      starts.push(lines.len());
    }
    for line in text.replace("\r\n", "\n").split('\n') {
      let mut rest = line;
      loop {
        let cut = if rest.len() <= LINE_CHARS { rest.len() } else { crate::budget::cut(rest, LINE_CHARS).len() };
        let (head, tail) = rest.split_at_checked(cut).unwrap_or((rest, ""));
        lines.push(Line { n: lines.len(), source, text: head.to_owned() });
        rest = tail;
        if rest.is_empty() {
          break;
        }
      }
    }
  }
  (lines, starts)
}

/// Lines as the history tools print them: the source once per run of lines from it
pub fn format_lines(lines: &[Line], indent: &str) -> String {
  lines
    .iter()
    .enumerate()
    .map(|(i, l)| if i > 0 && lines[i - 1].source == l.source && lines[i - 1].n + 1 == l.n { format!("{indent}{}: {}", l.n, l.text) } else { format!("{indent}{} {}: {}", l.n, l.source, l.text) })
    .collect::<Vec<_>>()
    .join("\n")
}

/// One chunk's lines, for the history tools
fn chunk_lines<'a>(raw: &'a [Item], c: &Chunk) -> Option<&'a [Item]> {
  raw.get(c.from..c.to.min(raw.len()))
}

/// `history_grep`: a fixed string in the folded history (one chunk, or all of them), with two lines around each hit
pub fn grep(log: &Path, pattern: &str, chunk: Option<&str>, case_sensitive: bool, limit: usize) -> Result<String, String> {
  let (raw, chunks) = crate::store::history(log).map_err(|e| format!("Cannot read the session history: {e}"))?;
  if chunks.is_empty() {
    return Ok("No folded history available (no chunk checkpoints in this session).".to_owned());
  }
  if let Some(id) = chunk
    && !chunks.iter().any(|c| c.id == id)
  {
    return Ok(format!("Unknown chunk ID \"{id}\" in this session."));
  }
  let needle = if case_sensitive { pattern.to_owned() } else { pattern.to_lowercase() };
  let mut hits = vec![];
  'chunks: for c in chunks.iter().filter(|c| chunk.is_none_or(|id| c.id == id)) {
    let Some(region) = chunk_lines(&raw, c) else { continue };
    let lines = transcript(region);
    for (i, l) in lines.iter().enumerate() {
      let hay = if case_sensitive { l.text.clone() } else { l.text.to_lowercase() };
      if !hay.contains(&needle) {
        continue;
      }
      let around: Vec<Line> = lines[i.saturating_sub(2)..i].iter().chain(lines.iter().skip(i + 1).take(2)).cloned().collect();
      let mut hit = format!("chunk {} (sequence {}) {} line {}:\n  {}", c.id, c.seq, l.source, l.n, l.text);
      if !around.is_empty() {
        hit.push_str(&format!("\n  context:\n{}", format_lines(&around, "  ")));
      }
      hits.push(hit);
      if hits.len() >= limit {
        break 'chunks;
      }
    }
  }
  Ok(if hits.is_empty() { "No matches in folded history.".to_owned() } else { hits.join("\n\n") })
}

/// `history_list`: lines `offset..offset + limit` of one chunk
pub fn list(log: &Path, chunk: &str, offset: usize, limit: usize) -> Result<String, String> {
  let (raw, chunks) = crate::store::history(log).map_err(|e| format!("Cannot read the session history: {e}"))?;
  let Some(c) = chunks.iter().find(|c| c.id == chunk) else {
    let known = chunks.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ");
    return Ok(format!("Unknown chunk ID \"{chunk}\" in this session. Known chunks: {}.", if known.is_empty() { "none" } else { &known }));
  };
  let lines = chunk_lines(&raw, c).map(transcript).unwrap_or_default();
  let page: Vec<Line> = lines.iter().skip(offset).take(limit).cloned().collect();
  if page.is_empty() {
    return Ok(format!("Chunk {} has {} lines; offset {offset} is past the end.", c.id, lines.len()));
  }
  let end = offset + page.len();
  let more = if end < lines.len() { format!(" Continue with offset={end}.") } else { String::new() };
  Ok(format!("Chunk {} (sequence {}, {}), lines {offset}..{end} of {}:{more}\n{}", c.id, c.seq, c.status.wire(), lines.len(), format_lines(&page, "")))
}
