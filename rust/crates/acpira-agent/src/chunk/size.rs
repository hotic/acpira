//! Size estimates in tokens, and the trimming built on them: a text whole, or its start and end within a budget

use crate::llm::{Item, Part};

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

pub(super) fn item_tokens(item: &Item) -> u64 {
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
      return text.get(..i).unwrap_or_default();
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
      return text.get(i + c.len_utf8()..).unwrap_or_default();
    }
  }
  text
}

/// The text within `max` tokens: whole, else its start and its end around a note
pub(super) fn preview(text: &str, max: u64) -> String {
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
pub(super) fn one_line(text: &str, max: u64) -> String {
  let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
  if tokens(&line) <= max {
    return line;
  }
  format!("{}…", head_within(&line, max.saturating_sub(1)))
}

/// Whole lines within `max` tokens, from both ends, so the first step and the latest ones both show
pub(super) fn preview_lines(lines: &[String], max: u64) -> String {
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
