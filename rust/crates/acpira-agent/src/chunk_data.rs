//! Pure data types, token sizing, and rendering for chunk compaction.

use serde::{Deserialize, Serialize};

use crate::llm::{Item, Part};

pub use crate::defs::CHUNK_STRATEGY as STRATEGY;

/// Newest chunks are shown whole up to this many tokens (OpenCode's `target_tokens` default)
const TARGET_TOKENS: u64 = 20_000;
/// The archived requests' index as a whole
const ARCHIVED_REQUESTS_TOKENS: u64 = 1_500;
/// History lines longer than this are split, so a one-line JSON file still pages
pub(crate) const LINE_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
  Completed,
  Interrupted,
  /// The running turn's earlier steps, folded into a work-state index
  Partial,
}

impl Status {
  pub(crate) fn wire(self) -> &'static str {
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

/// Size estimate in quarter tokens of one character: ASCII four to a token, CJK one, other text two
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

pub(crate) fn item_tokens(item: &Item) -> u64 {
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

fn head_within(text: &str, budget: u64) -> &str {
  let mut q = 0;
  for (i, c) in text.char_indices() {
    q += quarters(c);
    if q > budget * 4 {
      return text.get(..i).unwrap_or_default();
    }
  }
  text
}

fn tail_within(text: &str, budget: u64) -> &str {
  let mut q = 0;
  for (i, c) in text.char_indices().rev() {
    q += quarters(c);
    if q > budget * 4 {
      return text.get(i + c.len_utf8()..).unwrap_or_default();
    }
  }
  text
}

/// The text within `max` tokens: whole, else its start and its end around a note
pub(crate) fn preview(text: &str, max: u64) -> String {
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
pub(crate) fn one_line(text: &str, max: u64) -> String {
  let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
  if tokens(&line) <= max {
    return line;
  }
  format!("{}…", head_within(&line, max.saturating_sub(1)))
}

/// Whole lines within `max` tokens, from both ends, so the first step and the latest ones both show
pub(crate) fn preview_lines(lines: &[String], max: u64) -> String {
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

pub(crate) fn select(chunks: &[Chunk], budget: Budget) -> (Vec<(usize, bool)>, u64) {
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

/// The chunks as the model sees them
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

/// Put chunks a `compaction` event recorded into the list: one with a known id replaces it, others are appended
pub fn merge(chunks: &mut Vec<Chunk>, changed: Vec<Chunk>) {
  for c in changed {
    match chunks.iter_mut().find(|k| k.id == c.id) {
      Some(k) => *k = c,
      None => chunks.push(c),
    }
  }
}
