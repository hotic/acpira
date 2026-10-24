//! Chunk compaction: the `chunk` strategy of Bkm016/opencode (MIT, `packages/opencode/src/session/chunk.ts` and the
//! loop in `session/prompt.ts`, read 2026-10-14), picked on the settings page (`compaction: "chunk"` in providers.json)
//! instead of the model-written summary of `compact.rs`. No model is called, so it takes no time:
//! - Every finished turn becomes a chunk: the user's own words and the assistant's final reply stay, the steps between
//!   them (reasoning, tool calls and their results) are folded away. The folded items stay in the session log, and
//!   `history_grep` / `history_list` (`tools/history.rs`) read them back by the chunk's id.
//! - A chunk is a range of the log's `item` events (`from..to`, ordinals). It is `completed` when it ends on a reply
//!   without tool calls, else `interrupted`, and then also keeps its last tool outcomes, so resumed work does not redo
//!   their side effects.
//! - The view shows the newest chunks whole up to `Budget::target`, older ones as bounded excerpts up to `Budget::hard`,
//!   and the rest as one line of their request each in the checkpoint's index (the first request gets more room, since
//!   it often states the session's constraints).
//! - A turn that alone grows past the limit is folded while it runs: its earlier steps become a `partial` chunk holding
//!   a work-state index built from the tool calls (files changed with their diffs, errors and whether a later step fixed
//!   them, reads, searches, commands), and the later steps stay verbatim behind it. When the turn ends, the partial chunk
//!   becomes the whole turn.
//! - Nothing to fold, or a log that does not match the history, returns to the caller, which falls back to the summary.
//!
//! The view is `[summary item] + render(chunks) + the items after the last chunk`; the `compaction` event (`strategy:
//! "chunk"`) records the chunks it added or changed, how many items the tail kept and the budget, so `store::load`
//! renders the same view again.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::path::Path;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::acp::{Server, Session};
use crate::compact::Reason;
use crate::llm::{Item, Part};

/// The value of `ProvidersFile::compaction` that picks this strategy (absent or anything else is the summary)
pub const STRATEGY: &str = "chunk";

/// Newest chunks are shown whole up to this many tokens (OpenCode's `target_tokens` default)
const TARGET_TOKENS: u64 = 20_000;
/// A user message longer than this is shown as its start and end with a reference to the whole text
const LONG_USER_TOKENS: u64 = 2_000;
/// The archived requests' index as a whole
const ARCHIVED_REQUESTS_TOKENS: u64 = 1_500;
/// One archived request's line; the first one gets more, as it often states the session's constraints
const REQUEST_TOKENS: u64 = 80;
const FIRST_REQUEST_TOKENS: u64 = 240;
/// History lines longer than this are split, so a one-line JSON file still pages
const LINE_CHARS: usize = 2_000;
/// The work-state index of a folded turn: at most this, and at most `INDEX_SHARE` of the usable window
const INDEX_TOKENS: u64 = 8_000;
const INDEX_SHARE: u64 = 20;
/// A running turn is folded when the history after sealing still takes this share of the limit (tenths)
const FOLD_TRIGGER_TENTHS: u64 = 7;
/// ... down to this share of it (tenths): half the limit, so a long turn does not fold every step
const FOLD_TARGET_TENTHS: u64 = 5;

const ID_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz123456789";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
  Completed,
  Interrupted,
  /// The running turn's earlier steps, folded into a work-state index
  Partial,
}

impl Status {
  fn wire(self) -> &'static str {
    match self {
      Status::Completed => "completed",
      Status::Interrupted => "interrupted",
      Status::Partial => "partial",
    }
  }
}

/// What a chunk shows the model: its user messages and its summary, with their estimated size
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shown {
  pub input: String,
  pub summary: String,
  pub tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
  /// Eight characters, unique in the session; the history tools take it
  pub id: String,
  pub seq: u32,
  pub status: Status,
  /// The log's `item` events it covers, as ordinals
  pub from: usize,
  pub to: usize,
  pub full: Shown,
  /// The bounded version, for older chunks when the whole ones no longer fit
  pub excerpt: Shown,
  /// One line of its user messages, for the archived index
  pub request: String,
}

/// How much of the window the shown chunks may take
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
  pub target: u64,
  pub hard: u64,
}

impl Budget {
  /// OpenCode's 20k / 24k, a fifth of a smaller usable window
  pub fn for_window(usable: u64) -> Budget {
    let target = (usable / 5).clamp(2_000, TARGET_TOKENS);
    Budget { target, hard: target * 6 / 5 }
  }
}

impl Default for Budget {
  fn default() -> Budget {
    Budget::for_window(u64::MAX)
  }
}

/// Size estimate in quarter tokens of one character: ASCII four to a token, CJK one, other text two (OpenCode's; bytes
/// / 4 counts a Chinese character as 0.75 and underestimates Chinese sessions)
fn quarters(c: char) -> u64 {
  let code = c as u32;
  if code <= 0x7f {
    1
  } else if matches!(code, 0x4e00..=0x9fff | 0x3000..=0x303f | 0xff00..=0xffef | 0x20000..=0x2a6df) {
    4
  } else {
    2
  }
}

pub fn tokens(text: &str) -> u64 {
  text.chars().map(quarters).sum::<u64>().div_ceil(4)
}

fn item_tokens(item: &Item) -> u64 {
  match item {
    Item::User(parts) => parts
      .iter()
      .map(|p| match p {
        Part::Text(t) => tokens(t),
        Part::Image { .. } => 1000,
      })
      .sum::<u64>()
      + 4,
    Item::Assistant { text, reasoning, tool_calls, .. } => tokens(text) + tokens(reasoning) + tool_calls.iter().map(|c| tokens(&c.name) + tokens(&c.arguments)).sum::<u64>() + 4,
    Item::ToolResult { content, images, .. } => tokens(content) + 1000 * images.len() as u64 + 4,
  }
}

/// The longest start of `text` within `budget` tokens
fn head_within(text: &str, budget: u64) -> &str {
  let mut q = 0;
  for (i, c) in text.char_indices() {
    q += quarters(c);
    if q > budget * 4 {
      return &text[..i];
    }
  }
  text
}

/// The longest end of `text` within `budget` tokens
fn tail_within(text: &str, budget: u64) -> &str {
  let mut q = 0;
  for (i, c) in text.char_indices().rev() {
    q += quarters(c);
    if q > budget * 4 {
      return &text[i + c.len_utf8()..];
    }
  }
  text
}

/// The text within `max` tokens: whole, else its start and its end around a note
fn preview(text: &str, max: u64) -> String {
  if tokens(text) <= max {
    return text.to_owned();
  }
  const MARK: &str = "\n[... omitted; use the history tools for the full text ...]\n";
  let budget = max.saturating_sub(tokens(MARK)) / 2;
  if budget == 0 {
    return String::new();
  }
  format!("{}{MARK}{}", head_within(text, budget), tail_within(text, budget))
}

/// One line within `max` tokens (whitespace folded), for lists with an entry a line
fn one_line(text: &str, max: u64) -> String {
  let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
  if tokens(&line) <= max {
    return line;
  }
  format!("{}…", head_within(&line, max.saturating_sub(1)))
}

/// Whole lines within `max` tokens, from both ends, so the first step and the latest ones both show
fn preview_lines(lines: &[String], max: u64) -> String {
  let text = lines.join("\n");
  if tokens(&text) <= max {
    return text;
  }
  let mark = |n: usize| format!("- [... {n} steps omitted; use the history tools for the full text ...]");
  let budget = max.saturating_sub(tokens(&mark(lines.len())));
  let (mut head, mut tail) = (vec![], vec![]);
  let (mut used, mut i, mut j) = (0u64, 0usize, lines.len());
  while i < j {
    let from_head = head.len() <= tail.len();
    let line = if from_head { &lines[i] } else { &lines[j - 1] };
    let cost = tokens(line) + 1;
    if used + cost > budget {
      break;
    }
    used += cost;
    if from_head {
      head.push(line.clone());
      i += 1;
    } else {
      tail.insert(0, line.clone());
      j -= 1;
    }
  }
  if j > i {
    head.push(mark(j - i));
  }
  head.extend(tail);
  head.join("\n")
}

/// A new display id, unique among `taken`
fn new_id(taken: &[Chunk]) -> String {
  loop {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    let mut n = h.finish();
    let id: String = (0..8)
      .map(|_| {
        let c = ID_ALPHABET[(n % ID_ALPHABET.len() as u64) as usize] as char;
        n /= ID_ALPHABET.len() as u64;
        c
      })
      .collect();
    if !taken.iter().any(|c| c.id == id) {
      return id;
    }
  }
}

/// A user message that opens a turn (not one steered into a running turn)
fn opens_turn(item: &Item) -> bool {
  matches!(item, Item::User(_)) && !item.is_steer()
}

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
fn transcript_with_starts(region: &[Item]) -> (Vec<Line>, Vec<usize>) {
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
        lines.push(Line { n: lines.len(), source, text: rest[..cut].to_owned() });
        rest = &rest[cut..];
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

/// The calls of a region with their results, in order
struct Call<'a> {
  name: &'a str,
  args: Value,
  raw_args: &'a str,
  result: Option<(&'a str, bool)>,
  /// The assistant item that made it (an index into the region)
  at: usize,
}

fn calls(region: &[Item]) -> Vec<Call<'_>> {
  // Providers reuse ids across replies: a result answers the latest open call with its id
  let mut out: Vec<Call> = vec![];
  let mut open: HashMap<&str, usize> = HashMap::new();
  for (i, item) in region.iter().enumerate() {
    match item {
      Item::Assistant { tool_calls, .. } => {
        for c in tool_calls {
          open.insert(c.id.as_str(), out.len());
          out.push(Call { name: &c.name, args: serde_json::from_str(&c.arguments).unwrap_or(Value::Null), raw_args: &c.arguments, result: None, at: i });
        }
      }
      Item::ToolResult { call_id, content, is_error, .. } => {
        if let Some(k) = open.remove(call_id.as_str()) {
          out[k].result = Some((content.as_str(), *is_error));
        }
      }
      Item::User(_) => {}
    }
  }
  out
}

fn arg<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
  keys.iter().find_map(|k| args.get(*k).and_then(Value::as_str))
}

/// A path as the index shows it: relative to the session folder
fn rel(path: &str, cwd: &Path) -> String {
  crate::tools::shown(&crate::tools::resolve(path, cwd), cwd)
}

/// A result without the to-do note a call that carried the list gets in front (`tools::todo::note`)
fn own_result(output: &str) -> &str {
  match output.strip_prefix(crate::tools::todo::NOTE_PREFIX).and_then(|rest| rest.split_once("\n\n")) {
    Some((_, rest)) => rest,
    None => output,
  }
}

/// What the command an edit or a write ran after its change (`then_run`) printed: the section after its `$ command`
/// line, when it ran
fn then_run_output<'a>(output: &'a str, command: &str) -> Option<&'a str> {
  let marker = format!("\n\n$ {command}\n");
  let at = output.rfind(&marker)?;
  let rest = &output[at + marker.len()..];
  (!rest.starts_with("Not run: ")).then_some(rest)
}

/// The exit code a `bash` result ends with (`[exit code N]`)
fn exit_code(output: &str) -> Option<i64> {
  output.trim_end().rsplit('\n').next()?.strip_prefix("[exit code ")?.strip_suffix(']')?.parse().ok()
}

/// A call's state as the history shows it
fn state(call: &Call) -> &'static str {
  match call.result {
    Some((_, true)) => "error",
    Some(_) => "completed",
    None => "no result",
  }
}

/// A failed call: an error result, or a command that exited non-zero
fn failed(call: &Call) -> bool {
  match call.result {
    Some((_, true)) => true,
    Some((out, false)) => call.name == crate::tools::BASH && exit_code(out).is_some_and(|c| c != 0),
    None => false,
  }
}

/// The files a call wrote, with the changed lines (`-` / `+`, a few of them); an edit's items may name their own files
fn file_changes(call: &Call, cwd: &Path) -> Vec<(String, Vec<String>)> {
  let Some(path) = arg(&call.args, &["path", "file_path"]) else { return vec![] };
  let file = rel(path, cwd);
  match call.name {
    crate::tools::WRITE => vec![(file, edit_lines("", arg(&call.args, &["content"]).unwrap_or("")))],
    crate::tools::EDIT => match call.args.get("edits").and_then(Value::as_array).filter(|a| !a.is_empty()) {
      Some(edits) => {
        let mut out: Vec<(String, Vec<String>)> = vec![];
        for e in edits {
          let f = arg(e, &["path", "file_path"]).filter(|p| !p.trim().is_empty()).map_or_else(|| file.clone(), |p| rel(p.trim(), cwd));
          let lines = edit_lines(arg(e, &["old_string"]).unwrap_or(""), arg(e, &["new_string"]).unwrap_or(""));
          match out.iter_mut().find(|(name, _)| *name == f) {
            Some((_, l)) => l.extend(lines),
            None => out.push((f, lines)),
          }
        }
        out
      }
      None => vec![(file, edit_lines(arg(&call.args, &["old_string"]).unwrap_or(""), arg(&call.args, &["new_string"]).unwrap_or("")))],
    },
    _ => vec![],
  }
}

/// The lines an edit took out and put in, six at most, with a line of context for a pure insertion
fn edit_lines(before: &str, after: &str) -> Vec<String> {
  let old: Vec<&str> = before.split('\n').collect();
  let new: Vec<&str> = after.split('\n').collect();
  let mut lines: Vec<String> = old.iter().filter(|l| !l.trim().is_empty() && !new.contains(l)).map(|l| format!("- {}", one_line(l, 40))).collect();
  lines.extend(new.iter().filter(|l| !l.trim().is_empty() && !old.contains(l)).map(|l| format!("+ {}", one_line(l, 40))));
  let anchor = old.iter().find(|l| !l.trim().is_empty() && new.contains(l));
  let mut out: Vec<String> = lines.iter().take(6).cloned().collect();
  if lines.len() > 6 {
    out.push(format!("… {} more changed lines", lines.len() - 6));
  }
  if let Some(a) = anchor
    && lines.iter().all(|l| l.starts_with('+'))
  {
    out.push(format!("@ {}", one_line(a, 40)));
  }
  out
}

/// The files the region's calls changed, each once
fn changed_files(region: &[Item], cwd: &Path) -> Vec<String> {
  let mut seen = HashSet::new();
  calls(region)
    .iter()
    .filter(|c| matches!(c.result, Some((_, false))))
    .flat_map(|c| file_changes(c, cwd))
    .map(|(f, _)| f)
    .filter(|f| seen.insert(f.clone()))
    .collect()
}

/// A completed chunk ends on a reply without tool calls
fn status_of(region: &[Item]) -> Status {
  match region.last() {
    Some(Item::Assistant { text, tool_calls, .. }) if tool_calls.is_empty() && !text.trim().is_empty() => Status::Completed,
    _ => Status::Interrupted,
  }
}

/// The user's messages of a chunk, a long one as its start and end with where the whole text is
fn user_blocks(region: &[Item], id: &str) -> Vec<String> {
  // Where each user message starts in the chunk's lines, in order
  let mut starts = transcript_with_starts(region).1.into_iter();
  region
    .iter()
    .filter_map(user_words)
    .enumerate()
    .map(|(k, text)| {
      let line = starts.next().unwrap_or(0);
      let n = tokens(&text);
      let shown = if n <= LONG_USER_TOKENS {
        text
      } else {
        let header = format!(
          "<user-text-reference chunk_id=\"{id}\" line=\"{line}\">\nThe complete text is too long to include here ({} bytes, {} lines, about {n} tokens). Read it with history_list (chunk_id=\"{id}\", offset={line}) or search it with history_grep (chunk_id=\"{id}\").",
          text.len(),
          text.lines().count()
        );
        format!("{header}\n{}\n</user-text-reference>", preview(&text, LONG_USER_TOKENS.saturating_sub(tokens(&header))))
      };
      format!("<user-message sequence=\"{}\">\n{shown}\n</user-message>", k + 1)
    })
    .collect()
}

fn reference(id: &str) -> String {
  format!("Use history_list with chunk_id=\"{id}\" or history_grep to recover omitted details.")
}

/// What a closed chunk shows, whole or as an excerpt
fn closed_content(region: &[Item], id: &str, status: Status, cwd: &Path, excerpt: bool) -> Shown {
  let blocks = user_blocks(region, id);
  let mark = if excerpt { " excerpt=\"true\"" } else { "" };
  let body = if blocks.is_empty() {
    "(No user message in this chunk.)".to_owned()
  } else if excerpt {
    let words: Vec<String> = region.iter().filter_map(user_words).collect();
    format!("{}\n{}", preview(&words.join("\n\n"), 600), reference(id))
  } else {
    blocks.join("\n\n")
  };
  let input = format!("<chunk-input id=\"{id}\" status=\"{}\"{mark}>\n{body}\n</chunk-input>", status.wire());
  let last = match region.last() {
    Some(Item::Assistant { text, .. }) => text.trim().to_owned(),
    _ => String::new(),
  };
  let mut parts = vec![if last.is_empty() {
    "No final assistant response was recorded.".to_owned()
  } else if excerpt {
    preview(&last, 600)
  } else {
    last
  }];
  if status != Status::Completed {
    // Not completed work: the last outcomes are evidence for resuming, so side effects are not repeated
    let all = calls(region);
    let recent = &all[all.len().saturating_sub(3)..];
    let recent_at: HashSet<usize> = recent.iter().map(|c| c.at).collect();
    let mut log = vec![];
    for (i, item) in region.iter().enumerate() {
      let Item::Assistant { text, .. } = item else { continue };
      if recent_at.contains(&i) {
        continue;
      }
      if i + 1 < region.len() && !text.trim().is_empty() {
        log.push(format!("- ASSISTANT: {}", one_line(text, 50)));
      }
      for c in all.iter().filter(|c| c.at == i) {
        log.push(format!("- {}({}) [{}]", c.name, one_line(c.raw_args, 40), state(c)));
      }
    }
    parts.push("This chunk did not complete. Recent tool outcomes below are historical evidence, not new tool calls. Check omitted outcomes before repeating side effects.".to_owned());
    parts.push(reference(id));
    if !log.is_empty() {
      parts.push(format!("Earlier steps, oldest first:\n{}", preview_lines(&log, if excerpt { 300 } else { 800 })));
    }
    for c in recent {
      if let Item::Assistant { text, .. } = &region[c.at]
        && c.at + 1 < region.len()
        && !text.trim().is_empty()
      {
        parts.push(format!("ASSISTANT: {}", one_line(text, 50)));
      }
      parts.push(format!("ASSISTANT_TOOL: {}", preview(&format!("{}({}) [{}]", c.name, c.raw_args, state(c)), 180)));
      if let Some((out, err)) = c.result {
        parts.push(format!("{}: {}", if err { "TOOL_ERROR" } else { "TOOL_OUTPUT" }, preview(out, 360)));
      }
    }
  }
  let files = changed_files(region, cwd);
  if !files.is_empty() {
    parts.push(format!("Recorded file changes:\n{}", preview(&files.join("\n"), 250)));
  }
  if excerpt && status == Status::Completed {
    parts.push(reference(id));
  }
  let summary = format!("<chunk-summary id=\"{id}\" status=\"{}\" folded-messages=\"{}\"{mark}>\n{}\n</chunk-summary>", status.wire(), region.len(), parts.join("\n"));
  let tokens = tokens(&input) + tokens(&summary) + 8;
  Shown { input, summary, tokens }
}

/// A changed file in the work-state index
struct FileEntry {
  count: usize,
  first: usize,
  last: usize,
  /// The last three changes: (step, changed lines)
  diffs: Vec<(usize, Vec<String>)>,
}

/// A failed call, and the later step that fixed it (the same file written, the same command run without an error)
struct ErrorEntry {
  step: usize,
  key: String,
  what: String,
  detail: String,
  resolved: Option<usize>,
}

struct CommandEntry {
  step: usize,
  command: String,
  exit: Option<i64>,
  lines: usize,
  tail: Vec<String>,
}

/// The index of a running turn's folded steps, built from its tool calls without a model
#[derive(Default)]
struct WorkIndex {
  steps: usize,
  files: BTreeMap<String, FileEntry>,
  file_order: Vec<String>,
  errors: Vec<ErrorEntry>,
  narration: Vec<String>,
  reads: Vec<(String, Vec<usize>)>,
  /// (key, steps, first line of the latest output)
  searches: Vec<(String, Vec<usize>, String)>,
  commands: Vec<CommandEntry>,
  other: Vec<(String, Vec<usize>)>,
}

fn push_step(list: &mut Vec<(String, Vec<usize>)>, key: String, step: usize) {
  match list.iter_mut().find(|(k, _)| *k == key) {
    Some((_, steps)) => steps.push(step),
    None => list.push((key, vec![step])),
  }
}

fn work_index(region: &[Item], cwd: &Path) -> WorkIndex {
  let mut ix = WorkIndex::default();
  let all = calls(region);
  let mut step_of: HashMap<usize, usize> = HashMap::new();
  for (i, item) in region.iter().enumerate() {
    if let Item::Assistant { text, .. } = item {
      ix.steps += 1;
      step_of.insert(i, ix.steps);
      if !text.trim().is_empty() {
        ix.narration.push(format!("- {}. {}", ix.steps, one_line(text, 60)));
      }
    }
  }
  for c in &all {
    let step = step_of[&c.at];
    let key = match (arg(&c.args, &["path", "file_path"]), arg(&c.args, &["command", "cmd"])) {
      (Some(p), _) if c.name == crate::tools::WRITE || c.name == crate::tools::EDIT => format!("file:{}", rel(p, cwd)),
      (_, Some(cmd)) => format!("command:{cmd}"),
      _ => format!("{}:{}", c.name, c.raw_args),
    };
    if failed(c) {
      let what = match arg(&c.args, &["command", "cmd"]) {
        Some(cmd) => format!("{} `{}`", c.name, one_line(cmd, 40)),
        None => format!("{} {}", c.name, key.strip_prefix("file:").map(str::to_owned).unwrap_or_else(|| one_line(c.raw_args, 40))),
      };
      let raw = c.result.map_or("", |(o, _)| o).trim();
      let detail = preview(&raw.lines().take(8).collect::<Vec<_>>().join("\n"), 200);
      ix.errors.push(ErrorEntry { step, key, what, detail, resolved: None });
      continue;
    }
    if c.result.is_none() {
      continue;
    }
    for e in ix.errors.iter_mut().filter(|e| e.resolved.is_none() && e.key == key) {
      e.resolved = Some(step);
    }
    match c.name {
      crate::tools::WRITE | crate::tools::EDIT => {
        for (file, lines) in file_changes(c, cwd) {
          if !ix.files.contains_key(&file) {
            ix.file_order.push(file.clone());
          }
          let entry = ix.files.entry(file).or_insert(FileEntry { count: 0, first: step, last: step, diffs: vec![] });
          entry.count += 1;
          entry.last = step;
          entry.diffs.push((step, lines));
          if entry.diffs.len() > 3 {
            entry.diffs.remove(0);
          }
        }
        // The command the change ran (`then_run`) counts like a bash call of its own
        if let Some(command) = arg(&c.args, &["then_run"]).map(str::trim)
          && let Some(out) = then_run_output(c.result.map_or("", |(o, _)| o), command)
        {
          let key = format!("command:{command}");
          match exit_code(out) {
            Some(code) if code != 0 => {
              let detail = preview(&out.trim().lines().take(8).collect::<Vec<_>>().join("\n"), 200);
              ix.errors.push(ErrorEntry { step, key, what: format!("then_run `{}`", one_line(command, 40)), detail, resolved: None });
            }
            _ => {
              for e in ix.errors.iter_mut().filter(|e| e.resolved.is_none() && e.key == key) {
                e.resolved = Some(step);
              }
            }
          }
          ix.commands.push(command_entry(step, command, out));
        }
      }
      crate::tools::READ => {
        for p in crate::tools::read::targets(&c.args, cwd) {
          push_step(&mut ix.reads, crate::tools::shown(&p, cwd), step);
        }
      }
      crate::tools::GREP | crate::tools::GLOB | crate::tools::LIST => {
        let target = arg(&c.args, &["path"]).map(|p| format!(" in {}", rel(p, cwd))).unwrap_or_default();
        let include = arg(&c.args, &["include"]).map(|i| format!(" ({i})")).unwrap_or_default();
        let key = one_line(&format!("{} {}{target}{include}", c.name, arg(&c.args, &["pattern"]).unwrap_or("")), 60);
        let head = one_line(own_result(c.result.map_or("", |(o, _)| o)).trim().lines().next().unwrap_or(""), 25);
        match ix.searches.iter_mut().find(|s| s.0 == key) {
          Some(s) => {
            s.1.push(step);
            s.2 = head;
          }
          None => ix.searches.push((key, vec![step], head)),
        }
      }
      crate::tools::BASH if arg(&c.args, &["command", "cmd"]).is_some() => {
        ix.commands.push(command_entry(step, arg(&c.args, &["command", "cmd"]).unwrap_or(""), own_result(c.result.map_or("", |(o, _)| o))));
      }
      _ => push_step(&mut ix.other, format!("{} {}", c.name, one_line(c.raw_args, 40)), step),
    }
  }
  ix
}

/// A command's line in the index: its exit code, how many lines it printed and the last few
fn command_entry(step: usize, command: &str, output: &str) -> CommandEntry {
  let out = output.trim();
  let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with("[exit code ")).collect();
  let tail = lines[lines.len().saturating_sub(3)..].iter().map(|l| one_line(l, 40)).collect();
  CommandEntry { step, command: one_line(command, 40), exit: exit_code(out), lines: lines.len(), tail }
}

fn step_list(steps: &[usize]) -> String {
  let list = steps.iter().map(usize::to_string).collect::<Vec<_>>().join(",");
  if steps.len() > 1 { format!("steps {list}; {} times", steps.len()) } else { format!("step {list}") }
}

/// How much of the index is spelled out: narration and list budgets in tokens (None: all), files with their diffs
#[derive(Clone, Copy)]
struct Fit {
  narration: Option<u64>,
  lists: Option<u64>,
  diff_files: usize,
}

fn render_work(ix: &WorkIndex, id: &str, fit: Fit) -> String {
  let cap = |lines: Vec<String>, budget: Option<u64>| -> Vec<String> {
    match budget {
      Some(b) if !lines.is_empty() => preview_lines(&lines, b).split('\n').map(str::to_owned).collect(),
      _ => lines,
    }
  };
  let mut out = vec![
    format!("<work-state chunk=\"{id}\" status=\"partial\" folded-steps=\"1-{}\">", ix.steps),
    // OpenCode found that models answer from memory about folded output unless told it is gone
    format!(
      "This turn is still in progress. Steps 1-{} were folded to free context: their tool outputs (file contents, search hits, command output) are no longer in your context, only this index is. The messages after it are the later steps, verbatim. Continue the current task from the latest step and do not redo folded steps. Do not answer from memory about folded output: when you need its exact text, call history_grep or history_list with chunk_id=\"{id}\", or run the tool again.",
      ix.steps
    ),
    String::new(),
    format!("## Modified files ({})", ix.file_order.len()),
  ];
  for (k, file) in ix.file_order.iter().enumerate() {
    let f = &ix.files[file];
    let span = if f.last != f.first { format!("{}-{}", f.first, f.last) } else { f.first.to_string() };
    out.push(format!("- {file}: {} change{} (step {span})", f.count, if f.count > 1 { "s" } else { "" }));
    // The list itself is never shortened; only the most recently changed files keep their diffs when it is tight.
    // Counted from the end: `k + diff_files` overflowed at `usize::MAX` (all diffs) from the second file on
    if ix.file_order.len() - k <= fit.diff_files {
      for (step, lines) in &f.diffs {
        out.push(format!("  step {step}:"));
        out.extend(lines.iter().map(|l| format!("    {l}")));
      }
    }
  }
  out.push(String::new());
  out.push(format!("## Errors ({})", ix.errors.len()));
  for e in &ix.errors {
    let detail = e.detail.replace('\n', "\n    ");
    let (step, what) = (e.step, &e.what);
    out.push(match e.resolved {
      Some(r) if fit.lists.is_some() => format!("- step {step} {what} (resolved at step {r})"),
      Some(r) => format!("- step {step} {what} (resolved at step {r}): {detail}"),
      None => format!("- step {step} {what} (UNRESOLVED): {detail}"),
    });
  }
  out.push(String::new());
  out.push("## Steps".to_owned());
  out.extend(cap(ix.narration.clone(), fit.narration));
  out.push(String::new());
  out.push(format!("## Files read ({}; contents folded, read again or use history_grep before relying on them)", ix.reads.len()));
  out.extend(cap(ix.reads.iter().map(|(f, s)| format!("- {f} @ {}", s.iter().map(usize::to_string).collect::<Vec<_>>().join(","))).collect(), fit.lists));
  out.push(String::new());
  out.push(format!("## Searches ({})", ix.searches.len()));
  out.extend(cap(ix.searches.iter().map(|(k, s, h)| format!("- {k} → {} ({})", if h.is_empty() { "(no output)" } else { h }, step_list(s))).collect(), fit.lists));
  out.push(String::new());
  out.push(format!("## Commands ({})", ix.commands.len()));
  out.extend(cap(
    ix.commands
      .iter()
      .map(|c| {
        let mut s = format!("- {}. `{}` exit={}, {} output lines, last {}:", c.step, c.command, c.exit.map_or("?".to_owned(), |e| e.to_string()), c.lines, c.tail.len());
        for l in &c.tail {
          s.push_str(&format!("\n    {l}"));
        }
        s
      })
      .collect(),
    fit.lists,
  ));
  if !ix.other.is_empty() {
    out.push(String::new());
    out.push(format!("## Other tool calls ({})", ix.other.len()));
    out.extend(cap(ix.other.iter().map(|(k, s)| format!("- {k} ({})", step_list(s))).collect(), fit.lists));
  }
  out.push("</work-state>".to_owned());
  out.join("\n")
}

/// The work-state index within `max` tokens: narration shortened first, then diffs kept for the 20 / 10 / 5 files
/// changed last, then the lists of reads, searches and commands; the list of changed files is never shortened
fn work_state(region: &[Item], id: &str, cwd: &Path, max: u64) -> String {
  let ix = work_index(region, cwd);
  let all = Fit { narration: None, lists: None, diff_files: usize::MAX };
  let full = render_work(&ix, id, all);
  if tokens(&full) <= max {
    return full;
  }
  let bare = WorkIndex { narration: vec![], ..work_index(region, cwd) };
  for diff_files in [usize::MAX, 20, 10, 5] {
    let fixed = tokens(&render_work(&bare, id, Fit { diff_files, ..all }));
    if fixed > max * 7 / 10 {
      continue;
    }
    return render_work(&ix, id, Fit { narration: Some((max - fixed).max(500)), lists: None, diff_files });
  }
  let lists = (max / 10).max(300);
  let fixed = tokens(&render_work(&bare, id, Fit { narration: None, lists: Some(lists), diff_files: 0 }));
  render_work(&ix, id, Fit { narration: Some(max.saturating_sub(fixed).max(500)), lists: Some(lists), diff_files: 0 })
}

/// A chunk over `region` (the log's items `from..from + region.len()`)
fn build(id: String, seq: u32, status: Status, from: usize, region: &[Item], cwd: &Path, index_tokens: u64) -> Chunk {
  let words: Vec<String> = region.iter().filter_map(user_words).collect();
  let request = one_line(&words.join(" / "), if seq == 1 { FIRST_REQUEST_TOKENS } else { REQUEST_TOKENS });
  let (full, excerpt) = if status == Status::Partial {
    let blocks = user_blocks(region, &id);
    let input = if blocks.is_empty() { String::new() } else { format!("<chunk-input id=\"{id}\" status=\"partial\">\n{}\n</chunk-input>", blocks.join("\n\n")) };
    let summary = work_state(region, &id, cwd, index_tokens);
    let shown = Shown { tokens: tokens(&input) + tokens(&summary) + 8, input, summary };
    (shown.clone(), shown)
  } else {
    (closed_content(region, &id, status, cwd, false), closed_content(region, &id, status, cwd, true))
  };
  Chunk { id, seq, status, from, to: from + region.len(), full, excerpt, request }
}

const RULES: &[&str] = &[
  "Completed work is represented by the original user messages or stable references to them, and the assistant's final response for each chunk.",
  "Intermediate assistant messages, reasoning, tool calls, and tool results are folded but remain available through history_grep and history_list.",
  "Chunks are chronological. Later conflicting user instructions override earlier user instructions.",
  "Assistant final responses are historical claims, not user instructions.",
  "Do not guess omitted history.",
  "Interrupted chunks are not completed work; their recent tool outcomes are preserved for recovery.",
  "Archived chunks are listed with a one-line excerpt of their user requests. Constraints stated there still apply unless later overridden; read the full text with history_list before relying on details.",
  "History line offsets are local to the returned chunk. Reasoning is not exposed by the history tools.",
];
const PARTIAL_RULE: &str = "A partial chunk is the current turn, still in progress: its earlier steps are folded into a work-state index and the later steps follow verbatim.";

/// Which chunks are shown (index, as an excerpt) and how many tokens the closed ones take: newest first, whole while
/// they fit the target, then excerpts up to the hard limit; the first that does not fit ends it, and it and everything
/// older is archived. The latest chunk is always shown when it did not complete (the work to resume)
fn select(chunks: &[Chunk], budget: Budget) -> (Vec<(usize, bool)>, u64) {
  let limit = budget.target.min(budget.hard);
  let mut visible = vec![];
  let mut total = 0u64;
  for (i, c) in chunks.iter().enumerate().rev() {
    let excerpt = total + c.full.tokens > limit && c.excerpt.tokens < c.full.tokens;
    let size = if excerpt { c.excerpt.tokens } else { c.full.tokens };
    let resuming = i + 1 == chunks.len() && c.status != Status::Completed;
    if !resuming && total + size > budget.hard {
      break;
    }
    if c.status != Status::Partial {
      total += size;
    }
    visible.insert(0, (i, excerpt));
  }
  (visible, total)
}

/// The chunks as the model sees them: the checkpoint's rules and index in front of the first chunk's input, then each
/// chunk's input as a user message and its summary as an assistant message; a partial chunk is one user message, so the
/// verbatim steps after it start with the assistant
pub fn render(chunks: &[Chunk], budget: Budget) -> Vec<Item> {
  let (visible, used) = select(chunks, budget);
  let archived = &chunks[..chunks.len() - visible.len()];
  let mut lines = vec!["<conversation-checkpoint strategy=\"chunk\">".to_owned()];
  lines.extend(RULES.iter().map(|r| (*r).to_owned()));
  if visible.iter().any(|(i, _)| chunks[*i].status == Status::Partial) {
    lines.push(PARTIAL_RULE.to_owned());
  }
  lines.push(String::new());
  lines.push(format!("visible chunks: {}", visible.len()));
  lines.push(format!("archived chunks: {}", archived.len()));
  lines.push(format!("archived chunk IDs: {}", if archived.is_empty() { "none".to_owned() } else { archived.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ") }));
  lines.push(format!(
    "visible sequences: {}",
    match (visible.first(), visible.last()) {
      (Some((a, _)), Some((b, _))) => format!("{}..{}", chunks[*a].seq, chunks[*b].seq),
      _ => "none".to_owned(),
    }
  ));
  lines.push(format!("history budget: {used}/{} tokens (hard {})", budget.target, budget.hard));
  let requests: Vec<String> = archived.iter().filter(|c| !c.request.is_empty()).map(|c| format!("- {} [{}]: {}", c.id, c.status.wire(), c.request)).collect();
  if !requests.is_empty() {
    lines.push("archived requests, oldest first:".to_owned());
    lines.push(preview(&requests.join("\n"), ARCHIVED_REQUESTS_TOKENS));
  }
  lines.push("</conversation-checkpoint>".to_owned());
  let mut checkpoint = Some(lines.join("\n"));
  let mut items = vec![];
  for (i, excerpt) in visible {
    let c = &chunks[i];
    let shown = if excerpt { &c.excerpt } else { &c.full };
    let mut input = shown.input.clone();
    if let Some(cp) = checkpoint.take() {
      input = if input.is_empty() { cp } else { format!("{cp}\n\n{input}") };
    }
    if c.status == Status::Partial {
      let text = if input.is_empty() { shown.summary.clone() } else { format!("{input}\n\n{}", shown.summary) };
      items.push(Item::User(vec![Part::Text(text)]));
    } else {
      items.push(Item::User(vec![Part::Text(input)]));
      items.push(Item::Assistant { text: shown.summary.clone(), reasoning: String::new(), tool_calls: vec![], native: None });
    }
  }
  if let Some(cp) = checkpoint {
    items.push(Item::User(vec![Part::Text(cp)]));
  }
  items
}

/// Put chunks a `compaction` event recorded into the list: one with a known id replaces it (a partial chunk extended),
/// others are appended
pub fn merge(chunks: &mut Vec<Chunk>, changed: Vec<Chunk>) {
  for c in changed {
    match chunks.iter_mut().find(|k| k.id == c.id) {
      Some(k) => *k = c,
      None => chunks.push(c),
    }
  }
}

/// What the caller knows about the limits: the usable window, the size it compacts towards (the threshold, or the
/// window after an overflow; None for `/compact`, which folds no running turn), and the system prompt and tools
pub struct Limits {
  pub usable: u64,
  pub goal: Option<u64>,
  /// After an overflow the running turn is folded whatever the estimate says: the provider counted more
  pub force_fold: bool,
  pub fixed: u64,
  /// The provider's count of the call that crossed the threshold, against which the estimates are scaled
  pub measured: Option<Measured>,
}

/// One call's prompt as the provider counted it, and how many of the shown items it carried
#[derive(Debug, Clone, Copy)]
pub struct Measured {
  pub input: u64,
  pub items: usize,
}

/// A goal in the estimate's units. The provider's count of the last call over this estimate of the same request
/// scales it, so a tokenizer that counts source code at about three characters a token still folds down to the goal:
/// Gemini 4 Argon counted 403k for a request estimated at about 280k, and a fold aimed at 200k kept 286k (2026-10-23).
/// The ratio stays within half to three times, and a count of a view that has changed since is ignored
fn in_estimate_units(goal: u64, limits: &Limits, items: &[Item]) -> u64 {
  let Some(m) = limits.measured.filter(|m| m.input > 0 && m.items <= items.len()) else { return goal };
  let estimated = limits.fixed + items[..m.items].iter().map(item_tokens).sum::<u64>();
  if estimated == 0 {
    return goal;
  }
  let ratio = (m.input as f64 / estimated as f64).clamp(0.5, 3.0);
  (goal as f64 / ratio) as u64
}

/// A step starts at an assistant item whose earlier calls all have their results
fn step_start(raw: &[Item], k: usize) -> bool {
  matches!(raw[k], Item::Assistant { .. }) && !matches!(&raw[k - 1], Item::Assistant { tool_calls, .. } if !tool_calls.is_empty())
}

/// Seal the finished turns into chunks and, inside a turn that still takes too much, fold its earlier steps. Ok(false)
/// when there was nothing to fold; Err when the log does not match the history (the caller falls back to the summary)
pub fn run(server: &Server, session: &Session, reason: Reason, limits: &Limits) -> Result<bool, String> {
  let started = Instant::now();
  let (raw, _) = crate::store::history(session.store.path()).map_err(|e| format!("cannot read the session log: {e}"))?;
  let mut st = session.state.lock();
  let logged = session.store.items_logged();
  let body = usize::from(st.summarized) + st.folded;
  let n = st.items.len().saturating_sub(body);
  if raw.len() != logged || n > logged {
    return Err(format!("the session log ({} items) does not match the history ({logged} logged, {n} shown)", raw.len()));
  }
  // The log's ordinal of the first shown item after the chunks
  let base = logged - n;
  let budget = Budget::for_window(limits.usable);
  let index_tokens = (limits.usable / INDEX_SHARE).clamp(500, INDEX_TOKENS);
  let mut chunks = st.chunks.clone();
  let partial = chunks.last().filter(|c| c.status == Status::Partial && c.to == base).cloned();
  let starts: Vec<usize> = (base..logged).filter(|&k| opens_turn(&raw[k])).collect();
  // The running turn's first item: the latest prompt, or the turn folded earlier when none came after it
  let running = match reason {
    Reason::Manual => None,
    _ => starts.last().copied().or(partial.as_ref().map(|p| p.from)),
  };
  let closed_end = running.map_or(logged, |r| r.max(base));
  let mut segments = vec![];
  let mut at = base;
  for &s in starts.iter().filter(|&&s| s < closed_end) {
    if s > at {
      segments.push((at, s));
    }
    at = s;
  }
  if at < closed_end {
    segments.push((at, closed_end));
  }
  let mut changed: Vec<Chunk> = vec![];
  let next_seq = |chunks: &[Chunk]| chunks.last().map_or(1, |c| c.seq + 1);
  for (a, b) in segments {
    // The steps after a fold belong to the folded turn: it is closed now, over the whole turn
    let (id, seq, from) = match &partial {
      Some(p) if a == base && !opens_turn(&raw[a]) => (p.id.clone(), p.seq, p.from),
      _ => (new_id(&chunks), next_seq(&chunks), a),
    };
    let region = &raw[from..b];
    let c = build(id, seq, status_of(region), from, region, &session.cwd, index_tokens);
    merge(&mut chunks, vec![c.clone()]);
    changed.push(c);
  }
  // A folded turn that ended with nothing after the fold: it is closed over what it had
  if let Some(p) = &partial
    && running != Some(p.from)
    && chunks.iter().any(|c| c.id == p.id && c.status == Status::Partial)
  {
    let region = &raw[p.from..p.to];
    let c = build(p.id.clone(), p.seq, status_of(region), p.from, region, &session.cwd, index_tokens);
    merge(&mut chunks, vec![c.clone()]);
    changed.push(c);
  }
  let mut tail_from = closed_end;
  // The running turn, when the history still takes too much: its earlier steps are folded, the latest stays verbatim
  if let (Some(r), Some(goal)) = (running, limits.goal.map(|g| in_estimate_units(g, limits, &st.items))) {
    let continuing = partial.as_ref().is_some_and(|p| p.from == r);
    let projection: u64 = render(&chunks, budget).iter().map(item_tokens).sum();
    let tail: u64 = st.items[body + (closed_end - base)..].iter().map(item_tokens).sum();
    let estimate = limits.fixed + projection + tail;
    if limits.force_fold || estimate > goal * FOLD_TRIGGER_TENTHS / 10 {
      let others = projection.saturating_sub(partial.as_ref().filter(|_| continuing).map_or(0, |p| p.full.tokens));
      let keep = (goal * FOLD_TARGET_TENTHS / 10).saturating_sub(limits.fixed + others + index_tokens);
      // The earliest step whose verbatim rest fits; the latest step stays verbatim whatever its size
      let lower = if continuing { base } else { r };
      // Cuts that fold at least one step: behind a fresh prompt the first step start would fold the prompt alone,
      // which helps only after an overflow (a pasted file in it may be what overflowed)
      let skip = usize::from(!continuing && !limits.force_fold);
      let steps: Vec<usize> = (lower + 1..logged).filter(|&k| step_start(&raw, k)).skip(skip).collect();
      // The verbatim rest from each shown item on, summed once from the end
      let view = &st.items[body + (lower - base)..];
      let mut rest = vec![0u64; view.len() + 1];
      for i in (0..view.len()).rev() {
        rest[i] = rest[i + 1] + item_tokens(&view[i]);
      }
      let cut = steps.iter().copied().find(|&k| rest[k - lower] <= keep).or(steps.last().copied());
      if let Some(cut) = cut.filter(|&c| c > lower) {
        let (id, seq) = match &partial {
          Some(p) if continuing => (p.id.clone(), p.seq),
          _ => (new_id(&chunks), next_seq(&chunks)),
        };
        let c = build(id, seq, Status::Partial, r, &raw[r..cut], &session.cwd, index_tokens);
        merge(&mut chunks, vec![c.clone()]);
        changed.push(c);
        tail_from = cut;
      }
    }
  }
  if changed.is_empty() {
    return Ok(false);
  }
  let id = session.next_compaction_id();
  server.update(&session.id, json!({ "sessionUpdate": "compaction_update", "compactionId": id, "status": "in_progress" }));
  let projection = render(&chunks, budget);
  let rest = st.items.split_off(body + (tail_from - base));
  let kept = rest.len();
  let summary = st.summarized.then(|| st.items[0].clone());
  st.folded = projection.len();
  st.items = summary.into_iter().chain(projection).chain(rest).collect();
  st.chunks = chunks;
  // The notes the model got with the folded prompts are announced again with the next one
  st.announced.clear();
  st.mcp_note = None;
  drop(st);
  session.store.append(json!({
    "type": "compaction", "strategy": STRATEGY, "reason": reason.wire(), "ms": started.elapsed().as_millis() as u64,
    "chunks": changed, "tail": kept, "budget": budget,
  }));
  server.update(&session.id, json!({ "sessionUpdate": "compaction_update", "compactionId": id, "status": "completed" }));
  Ok(true)
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

#[cfg(test)]
mod tests {
  use super::*;
  use crate::llm::ToolCall;

  fn user(t: &str) -> Item {
    Item::User(vec![Part::Text(t.into())])
  }
  fn reply(t: &str) -> Item {
    Item::Assistant { text: t.into(), reasoning: "hidden".into(), tool_calls: vec![], native: None }
  }
  fn call(id: &str, name: &str, args: Value) -> Item {
    Item::Assistant { text: String::new(), reasoning: String::new(), tool_calls: vec![ToolCall { id: id.into(), name: name.into(), arguments: args.to_string() }], native: None }
  }
  fn result(id: &str, name: &str, out: &str, err: bool) -> Item {
    Item::ToolResult { call_id: id.into(), name: name.into(), content: out.into(), is_error: err, images: vec![] }
  }
  fn cwd() -> &'static Path {
    Path::new("/w")
  }

  #[test]
  fn the_goal_is_scaled_by_the_providers_count() {
    // Two items of 100 estimated tokens each (384 characters + 4), 300 fixed: 500 estimated
    let items = vec![user(&"x".repeat(384)), user(&"x".repeat(384)), user("later")];
    let limits = |input, items| Limits { usable: 0, goal: None, force_fold: false, fixed: 300, measured: Some(Measured { input, items }) };
    assert_eq!(in_estimate_units(1000, &limits(750, 2), &items), 666, "counted 1.5 times the estimate");
    assert_eq!(in_estimate_units(1000, &limits(50_000, 2), &items), 333, "held at three times");
    assert_eq!(in_estimate_units(1000, &limits(100, 2), &items), 2000, "held at half");
    assert_eq!(in_estimate_units(1000, &limits(750, 9), &items), 1000, "a count of another view is ignored");
    assert_eq!(in_estimate_units(1000, &Limits { measured: None, ..limits(0, 0) }, &items), 1000);
  }

  #[test]
  fn the_estimate_counts_a_chinese_character_as_a_token() {
    assert_eq!(tokens("abcd"), 1);
    assert_eq!(tokens("压缩上下文"), 5);
    assert_eq!(tokens("é"), 1);
    let p = preview(&"x".repeat(4000), 100);
    assert!(p.contains("omitted") && tokens(&p) <= 100, "{}", tokens(&p));
    assert_eq!(one_line("a\n  b\tc", 10), "a b c");
    assert!(one_line(&"字".repeat(50), 10).ends_with('…'));
  }

  #[test]
  fn a_closed_turn_keeps_the_users_words_and_the_final_reply() {
    let region = vec![
      user("<mode>\nPlan mode is off.\n</mode>\n\nfix the build\n\n<skill name=\"x\" path=\"/s\">\nbody\n</skill>"),
      call("c1", "read", json!({ "path": "/w/src/a.rs" })),
      result("c1", "read", "fn a() {}", false),
      call("c2", "edit", json!({ "path": "src/a.rs", "old_string": "fn a() {}", "new_string": "fn a() { b() }" })),
      result("c2", "edit", "ok", false),
      reply("Fixed: a now calls b."),
    ];
    let c = build("AAAAAAAA".into(), 1, status_of(&region), 4, &region, cwd(), 8000);
    assert_eq!((c.status, c.from, c.to), (Status::Completed, 4, 10));
    assert!(c.full.input.contains("<user-message sequence=\"1\">\nfix the build\n</user-message>"), "{}", c.full.input);
    assert!(!c.full.input.contains("Plan mode") && !c.full.input.contains("body"));
    assert!(c.full.summary.contains("Fixed: a now calls b.") && c.full.summary.contains("Recorded file changes:\nsrc/a.rs"), "{}", c.full.summary);
    assert!(!c.full.summary.contains("fn a() {}"), "tool output is folded");
    assert_eq!(c.request, "fix the build");
    // The transcript the tools read holds everything but reasoning
    let lines = transcript(&region);
    assert!(lines.iter().any(|l| l.source == "TOOL_OUTPUT" && l.text == "fn a() {}"));
    assert!(lines.iter().all(|l| l.text != "hidden"));
  }

  #[test]
  fn an_interrupted_turn_keeps_its_last_outcomes() {
    let region = vec![
      user("deploy"),
      call("c1", "bash", json!({ "command": "make build" })),
      result("c1", "bash", "built\n[exit code 0]", false),
      call("c2", "bash", json!({ "command": "make deploy" })),
      result("c2", "bash", "Cancelled by the user before it finished.", true),
    ];
    let c = build("BBBBBBBB".into(), 2, status_of(&region), 0, &region, cwd(), 8000);
    assert_eq!(c.status, Status::Interrupted);
    assert!(c.full.summary.contains("did not complete") && c.full.summary.contains("make deploy") && c.full.summary.contains("TOOL_ERROR: Cancelled"), "{}", c.full.summary);
  }

  #[test]
  fn a_long_user_message_points_at_its_line() {
    let long = format!("start\n{}\nend", "数据".repeat(3000));
    let region = vec![user("first"), user(&long), reply("ok")];
    let c = build("CCCCCCCC".into(), 1, Status::Completed, 0, &region, cwd(), 8000);
    assert!(c.full.input.contains("history_list (chunk_id=\"CCCCCCCC\", offset=1)"), "{}", crate::budget::cut(&c.full.input, 400));
    assert!(c.full.input.contains("start") && c.full.input.contains("end") && tokens(&c.full.input) < 2_300);
  }

  fn closed(n: usize, size: usize) -> Chunk {
    let region = vec![user(&format!("task {n}")), reply(&"r".repeat(size))];
    build(format!("ID{n:06}"), n as u32, Status::Completed, 2 * n, &region, cwd(), 8000)
  }

  #[test]
  fn older_chunks_become_excerpts_then_archived_requests() {
    let chunks: Vec<Chunk> = (1..=10).map(|n| closed(n, 12_000)).collect();
    let budget = Budget { target: 8_000, hard: 10_000 };
    let (visible, used) = select(&chunks, budget);
    // About 3k each whole: the newest two fit the target, older ones are excerpts until the hard limit, the rest archived
    let first = visible[0].0;
    assert!(first > 0 && visible.iter().map(|v| v.0).eq(first..10), "{visible:?}");
    assert!(visible.iter().rev().take(2).all(|v| !v.1) && visible.iter().rev().skip(2).all(|v| v.1) && used <= 10_000, "{visible:?} {used}");
    let items = render(&chunks, budget);
    let Item::User(parts) = &items[0] else { panic!() };
    let Part::Text(head) = &parts[0] else { panic!() };
    assert!(head.starts_with("<conversation-checkpoint strategy=\"chunk\">") && head.contains("- ID000001 [completed]: task 1"), "{head}");
    assert!(head.contains(&format!("archived chunks: {first}")) && head.contains("excerpt=\"true\""));
    // User and assistant alternate
    for pair in items.chunks(2) {
      assert!(matches!(pair[0], Item::User(_)) && matches!(pair[1], Item::Assistant { .. }));
    }
  }

  #[test]
  fn a_running_turn_folds_into_a_work_state_index() {
    let region = vec![
      user("refactor the parser"),
      call("c1", "read", json!({ "path": "src/p.rs" })),
      result("c1", "read", "old parser", false),
      call("c2", "edit", json!({ "path": "src/p.rs", "old_string": "a\nb", "new_string": "a\nc" })),
      result("c2", "edit", "error: no match", true),
      call("c3", "edit", json!({ "path": "src/p.rs", "old_string": "a\nb", "new_string": "a\nc" })),
      result("c3", "edit", "ok", false),
      call("c4", "bash", json!({ "command": "cargo test" })),
      result("c4", "bash", "test a ... ok\ntest b ... FAILED\n[exit code 101]", false),
      call("c5", "grep", json!({ "pattern": "parse(", "path": "src" })),
      result("c5", "grep", "src/p.rs:3: fn parse(", false),
    ];
    let c = build("DDDDDDDD".into(), 3, Status::Partial, 0, &region, cwd(), 8000);
    let s = &c.full.summary;
    assert!(s.starts_with("<work-state chunk=\"DDDDDDDD\" status=\"partial\" folded-steps=\"1-5\">"), "{s}");
    assert!(s.contains("- src/p.rs: 1 change (step 3)") && s.contains("    - b") && s.contains("    + c"), "{s}");
    assert!(s.contains("- step 2 edit src/p.rs (resolved at step 3)") && s.contains("- step 4 bash `cargo test` (UNRESOLVED)"), "{s}");
    assert!(s.contains("- src/p.rs @ 1") && s.contains("- grep parse( in src → src/p.rs:3: fn parse( (step 5)"), "{s}");
    assert!(c.full.input.contains("refactor the parser"));
    // A tight budget keeps the file list and the errors
    let small = work_state(&region, "DDDDDDDD", cwd(), 300);
    assert!(small.contains("- src/p.rs: 1 change") && small.contains("UNRESOLVED"), "{small}");
  }

  #[test]
  fn the_work_index_reads_ranges_item_paths_and_commands_run_after_a_change() {
    let todos = |status: &str| json!([{ "content": "rename", "status": status }]);
    let region = vec![
      user("rename it"),
      call("c1", "read", json!({ "path": "src/a.rs:10-20", "paths": ["src/b.rs:5"] })),
      result("c1", "read", "...", false),
      call("c2", "edit", json!({ "path": "src/a.rs", "edits": [{ "old_string": "old_name", "new_string": "new_name" }, { "path": "src/b.rs", "old_string": "old_name()", "new_string": "new_name()" }], "then_run": "cargo test" })),
      result("c2", "edit", "Edited 2 files: src/a.rs, src/b.rs.\n\n$ cargo test\ntest a ... FAILED\n[exit code 101]", false),
      call("c3", "edit", json!({ "path": "src/b.rs", "old_string": "x", "new_string": "y", "then_run": "cargo test", "todos": todos("in_progress") })),
      result("c3", "edit", "To-do list updated (0 pending, 1 in progress, 0 completed).\n\nEdited src/b.rs.\n\n$ cargo test\ntest a ... ok", false),
      call("c4", "grep", json!({ "pattern": "old_name", "todos": todos("completed") })),
      result("c4", "grep", "To-do list updated (0 pending, 0 in progress, 1 completed).\n\nNo matches.", false),
    ];
    let s = work_state(&region, "EEEEEEEE", cwd(), 8000);
    assert!(s.contains("- src/a.rs: 1 change (step 2)") && s.contains("- src/b.rs: 2 changes (step 2-3)"), "an item's own path counts: {s}");
    assert!(s.contains("- src/a.rs @ 1") && s.contains("- src/b.rs @ 1"), "ranges left out: {s}");
    assert!(s.contains("- step 2 then_run `cargo test` (resolved at step 3)"), "{s}");
    assert!(s.contains("- 2. `cargo test` exit=101, 1 output lines") && s.contains("- 3. `cargo test` exit=?, 1 output lines"), "{s}");
    assert!(s.contains("- grep old_name → No matches. (step 4)"), "the to-do note is not the search's summary: {s}");
  }

  #[test]
  fn step_starts_never_split_a_call_from_its_result() {
    let raw = vec![user("go"), call("a", "read", json!({})), result("a", "read", "x", false), call("b", "read", json!({})), result("b", "read", "y", false), reply("done")];
    let starts: Vec<usize> = (1..raw.len()).filter(|&k| step_start(&raw, k)).collect();
    assert_eq!(starts, vec![1, 3, 5]);
  }
}
