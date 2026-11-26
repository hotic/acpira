//! Pure data types and projection helpers for summary compaction and tool-output pruning.

use serde_json::{Value, json};

use crate::llm::{Item, Part};

/// What a cleared tool result reads as
pub const PRUNED: &str = "[Old tool result content cleared]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
  /// `/compact`
  Manual,
  /// A call's usage reached the threshold
  Threshold,
  /// The provider refused the request as too long
  Overflow,
}

impl Reason {
  pub fn wire(self) -> &'static str {
    match self {
      Reason::Manual => "manual",
      Reason::Threshold => "threshold",
      Reason::Overflow => "overflow",
    }
  }
}

/// What a summary item carries verbatim beside the summary, so none of it depends on the model that wrote it. Logged
/// with the `compaction` event, so a reopened session rebuilds the same item (and the provider's cache still matches)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Carry {
  /// The user's own messages up to the kept tail, oldest first, within `CARRIED_TOKENS`
  pub messages: Vec<String>,
  /// Earlier messages left out of `messages` for the budget, over every compaction so far
  pub omitted: usize,
  /// The last to-do list the summarized part wrote, when the tail does not write a later one
  pub todo: Option<String>,
  /// The files the summarized part worked on most (`restore_paths`), as they were on disk at the compaction
  pub files: Vec<RestoredFile>,
  /// The running turn's prompt, when the summary took it in (`request_text`)
  pub request: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RestoredFile {
  /// As tool titles show it (`tools::shown`)
  pub path: String,
  pub text: String,
}

impl Carry {
  /// The event fields this is logged as (`compaction` beside `summary`)
  pub(crate) fn log_into(&self, ev: &mut Value) {
    if !self.messages.is_empty() {
      ev["messages"] = json!(self.messages);
    }
    if self.omitted > 0 {
      ev["omitted"] = json!(self.omitted);
    }
    if let Some(t) = &self.todo {
      ev["todo"] = Value::String(t.clone());
    }
    if !self.files.is_empty() {
      ev["files"] = Value::Array(self.files.iter().map(|f| json!({ "path": f.path, "text": f.text })).collect());
    }
    if let Some(r) = &self.request {
      ev["request"] = Value::String(r.clone());
    }
  }

  /// Read back from a `compaction` event; an event written before these fields existed carries only its request
  pub fn from_event(ev: &Value) -> Carry {
    let s = |k: &str| ev.get(k).and_then(Value::as_str).map(str::to_owned);
    Carry {
      messages: ev.get("messages").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect(),
      omitted: ev.get("omitted").and_then(Value::as_u64).unwrap_or(0) as usize,
      todo: s("todo"),
      files: ev
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| Some(RestoredFile { path: f.get("path")?.as_str()?.to_owned(), text: f.get("text")?.as_str()?.to_owned() }))
        .collect(),
      request: s("request"),
    }
  }
}

/// Apply a logged prune to a loaded history
pub fn apply_prune(items: &mut [Item], calls: &[String]) {
  for item in items {
    if let Item::ToolResult { call_id, content, images, .. } = item
      && calls.contains(call_id)
    {
      *content = PRUNED.to_owned();
      images.clear();
    }
  }
}

/// The summary as the model sees it from then on: a user message in front of the kept tail
pub fn summary_item(summary: &str, carry: &Carry) -> Item {
  let mut text = format!(
    "<conversation-summary>\n{}\n</conversation-summary>\n\nThis session is being continued from a previous conversation that ran out of context. The summary above covers the earlier portion of the conversation. The messages after it, if any, are the most recent ones, kept as they were; the summary was written without seeing them, so something it says has not happened yet may already have happened in them.",
    summary.trim()
  );
  if !carry.messages.is_empty() {
    let omitted = match carry.omitted {
      0 => String::new(),
      n => format!(" ({n} earlier ones are left out; the summary covers them)"),
    };
    let list: Vec<String> = carry.messages.iter().map(|m| format!("<message>\n{}\n</message>", m.trim())).collect();
    text.push_str(&format!("\n\nThe user's messages up to here, in their own words, oldest first{omitted}:\n<user-messages>\n{}\n</user-messages>", list.join("\n")));
  }
  if let Some(t) = &carry.todo {
    text.push_str(&format!("\n\nThe to-do list as last written:\n<todo>\n{}\n</todo>", t.trim()));
  }
  if !carry.files.is_empty() {
    let files: Vec<String> = carry.files.iter().map(|f| format!("<file path=\"{}\">\n{}\n</file>", f.path, f.text.trim_end())).collect();
    text.push_str(&format!("\n\nThe files worked on most before this point, re-read from disk at the compaction (a long one is cut):\n{}", files.join("\n")));
  }
  if let Some(r) = &carry.request {
    text.push_str(&format!("\n\nThe request being worked on, in the user's own words:\n<request>\n{}\n</request>", r.trim()));
  }
  text.push_str(
    "\n\nIf work was in progress, continue it from where it left off without asking the user any further questions. Resume directly: do not acknowledge the summary, do not recap what was happening, do not preface with \"I'll continue\" or similar. Pick up the last task as if the break never happened.",
  );
  Item::User(vec![Part::Text(text)])
}
