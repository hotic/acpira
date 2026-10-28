//! What a closed chunk shows: its status, the user's messages, and the whole chunk or a bounded excerpt of it

use std::collections::HashSet;
use std::path::Path;

use super::calls::{calls, changed_files, state};
use super::history::{transcript_with_starts, user_words};
use super::size::{one_line, preview, preview_lines, tokens};
use super::{LONG_USER_TOKENS, Shown, Status};
use crate::llm::Item;

/// A completed chunk ends on a reply without tool calls
pub(super) fn status_of(region: &[Item]) -> Status {
  match region.last() {
    Some(Item::Assistant { text, tool_calls, .. }) if tool_calls.is_empty() && !text.trim().is_empty() => Status::Completed,
    _ => Status::Interrupted,
  }
}

/// The user's messages of a chunk, a long one as its start and end with where the whole text is
pub(super) fn user_blocks(region: &[Item], id: &str) -> Vec<String> {
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
pub(super) fn closed_content(region: &[Item], id: &str, status: Status, cwd: &Path, excerpt: bool) -> Shown {
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
