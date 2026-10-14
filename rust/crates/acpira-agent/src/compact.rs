//! Context compaction, the two kinds OpenCode has. Pruning clears the output of old tool calls: walking back from the
//! end past the last two user turns, the results seen are kept until they hold `PRUNE_PROTECT` tokens, every older one
//! is cleared, and only when that frees `PRUNE_MINIMUM` (otherwise the prefix, and the provider's cache, is left alone).
//! Summary compaction replaces everything but a tail of recent turns with a summary written to a fixed template, by the
//! fast model when one is set (the session's model when it is not, or when the fast one fails); a later compaction updates the previous summary instead of starting over. It runs on `/compact`, inside a
//! turn when a call's usage reaches the threshold (the host's `compactAtTokens`, sent as `_meta` on every prompt, capped
//! at the model's window less room for its output), and when the provider refuses a request as too long. The session
//! log keeps every item: a `compaction` event records the summary and how many items the tail kept, a `prune` event
//! the calls cleared, and `store::load` projects both

use std::time::Instant;

use serde_json::{Value, json};

use acpira_shared::model_catalog::Cost;
use acpira_shared::providers::{ProviderModel, Thinking};

use crate::acp::{Server, Session};
use crate::llm::{self, Endpoint, Event, Item, LlmError, Part, Request, Usage};
use crate::tools;

/// Tokens of recent tool output a prune leaves untouched
pub const PRUNE_PROTECT: u64 = 40_000;
/// A prune happens only when it frees this many tokens
pub const PRUNE_MINIMUM: u64 = 20_000;
/// What a cleared tool result reads as
pub const PRUNED: &str = "[Old tool result content cleared]";
/// Tools whose results a prune never clears: a skill's instructions are needed for the rest of the task
const PRUNE_PROTECTED: &[&str] = &[tools::SKILL];
/// Turns kept as they were after a summary
const TAIL_TURNS: usize = 2;
/// The tail's budget: a quarter of the usable window, within these bounds
const MIN_TAIL_TOKENS: u64 = 2_000;
const MAX_TAIL_TOKENS: u64 = 8_000;
/// Room left below the window for the model's answer (OpenCode caps the reserved output at this)
const RESERVED_OUTPUT: u64 = 32_000;
/// A tool result in the transcript the summary is written from is cut to this
const TOOL_OUTPUT_CHARS: usize = 2_000;
/// A user message in that transcript keeps this much of its start and of its end. A pasted file can make one message
/// most of the window: before this cap a 5.4 MB message of attachments filled the excerpt, whose oldest part was then
/// left out, the instruction with it, and the summary call read 600k tokens to report "no task was stated" (2026-10-13)
const USER_EDGE_CHARS: usize = 8_000;
/// The summary call's output limit
const SUMMARY_TOKENS: u64 = 4_096;

/// About four bytes a token: OpenCode's estimate, used for the tail budget and the prune
pub fn estimate(text: &str) -> u64 {
  (text.len() as u64).div_ceil(4)
}

fn item_tokens(item: &Item) -> u64 {
  match item {
    Item::User(parts) => parts
      .iter()
      .map(|p| match p {
        Part::Text(t) => estimate(t),
        Part::Image { .. } => 1000,
      })
      .sum(),
    Item::Assistant { text, reasoning, tool_calls, .. } => estimate(text) + estimate(reasoning) + tool_calls.iter().map(|c| estimate(&c.name) + estimate(&c.arguments)).sum::<u64>(),
    Item::ToolResult { content, images, .. } => estimate(content) + 1000 * images.len() as u64,
  }
}

/// The host's auto-compaction setting, read off `_meta["acpira/compaction"]` of a prompt; absent fields keep the
/// defaults (automatic, the window's own limit)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
  pub auto: bool,
  pub at_tokens: Option<u64>,
}

impl Default for Policy {
  fn default() -> Policy {
    Policy { auto: true, at_tokens: None }
  }
}

impl Policy {
  pub fn from_prompt(params: &Value) -> Policy {
    let meta = params.pointer("/_meta/acpira~1compaction").unwrap_or(&Value::Null);
    Policy {
      auto: meta.get("auto").and_then(Value::as_bool).unwrap_or(true),
      at_tokens: meta.get("atTokens").and_then(Value::as_f64).filter(|t| *t > 0.0).map(|t| t as u64),
    }
  }

  /// The usage at which a turn compacts: the setting, never above what the window leaves for an answer
  pub fn threshold(&self, context: u64, output: Option<u64>) -> u64 {
    let usable = usable(context, output);
    self.at_tokens.map_or(usable, |t| t.min(usable))
  }
}

/// The window less the room reserved for the model's output
pub fn usable(context: u64, output: Option<u64>) -> u64 {
  context.saturating_sub(output.unwrap_or(RESERVED_OUTPUT).min(RESERVED_OUTPUT))
}

/// What providers say when a request is over the window, as seen through the gateway on 2026-10-13 (`scripts/` probe
/// notes in `docs/dev/builtin-agent.md`): OpenAI "Your input exceeds the context window of this model", DeepSeek and
/// GLM "The prompt is too long: N, model maximum context length: M", MiniMax "context window exceeds limit", Gemini
/// "The input token count exceeds the maximum number of tokens allowed (N)"; plus the wordings OpenAI's and
/// Anthropic's documentation give ("maximum context length", "prompt is too long")
const OVERFLOW_WORDS: &[&str] = &[
  "context length", "context window", "context_length", "too many tokens", "too long", "maximum context", "token limit", "exceeds the limit",
  "too large", "input length", "maximum number of tokens", "input token count",
];

/// A request the provider refused for its size: compacting and sending again is the answer, a retry alone is not
pub fn is_overflow(e: &LlmError) -> bool {
  let message = match e {
    LlmError::Http { status: 400 | 413 | 422, message, .. } => message,
    LlmError::Api(m) | LlmError::Rejected(m) => m,
    _ => return false,
  };
  let m = message.to_ascii_lowercase();
  OVERFLOW_WORDS.iter().any(|p| m.contains(p))
}

/// The provider refused the request itself, whatever it said: the turn takes this for an overflow when the request is
/// near the window, since a gateway's Claude route answers an oversized request with only "There is an issue with this
/// request, please try a different model." (2026-10-13, every format)
pub fn is_refusal(e: &LlmError) -> bool {
  matches!(e, LlmError::Http { status: 400 | 413 | 422, .. } | LlmError::Rejected(_))
}

/// A user message that opens a turn: a prompt (a summary counts), not a message steered into a running turn
fn opens_turn(item: &Item) -> bool {
  matches!(item, Item::User(_)) && !item.is_steer()
}

/// The items whose results a prune clears (indices), by OpenCode's rule
pub fn prune_plan(items: &[Item]) -> Vec<usize> {
  let (mut turns, mut total, mut freed) = (0usize, 0u64, 0u64);
  let mut out = vec![];
  for (i, item) in items.iter().enumerate().rev() {
    match item {
      Item::User(_) if opens_turn(item) => turns += 1,
      Item::ToolResult { name, content, is_error, images, .. } if turns >= TAIL_TURNS && !*is_error && !PRUNE_PROTECTED.contains(&name.as_str()) => {
        // Everything older was cleared by an earlier prune
        if content == PRUNED {
          break;
        }
        let n = estimate(content) + 1000 * images.len() as u64;
        total += n;
        if total > PRUNE_PROTECT {
          freed += n;
          out.push(i);
        }
      }
      _ => {}
    }
  }
  if freed > PRUNE_MINIMUM { out } else { vec![] }
}

/// Clear old tool output in the session's history and record it; the number of results cleared
pub fn prune(session: &Session) -> usize {
  let mut st = session.state.lock();
  let plan = prune_plan(&st.items);
  if plan.is_empty() {
    return 0;
  }
  let mut calls = vec![];
  let mut tokens = 0u64;
  for i in &plan {
    if let Item::ToolResult { call_id, content, images, .. } = &mut st.items[*i] {
      tokens += estimate(content) + 1000 * images.len() as u64;
      *content = PRUNED.to_owned();
      images.clear();
      calls.push(call_id.clone());
    }
  }
  session.store.append(json!({ "type": "prune", "calls": calls, "tokens": tokens }));
  plan.len()
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

/// Where the kept tail starts: the last `TAIL_TURNS` turns that fit the budget, the oldest of them split at a message
/// boundary (never in front of a tool result, which must follow its call) when it does not fit whole. None when
/// nothing fits, or when the tail would be the whole conversation: then everything is summarized. `first` is the index
/// of the first real item (1 behind a summary item)
pub fn tail_start(items: &[Item], first: usize, budget: u64) -> Option<usize> {
  let starts: Vec<usize> = (first..items.len()).filter(|&i| opens_turn(&items[i])).collect();
  let recent = &starts[starts.len().saturating_sub(TAIL_TURNS)..];
  let mut total = 0u64;
  let mut keep: Option<usize> = None;
  for (k, &start) in recent.iter().enumerate().rev() {
    let end = recent.get(k + 1).copied().unwrap_or(items.len());
    let size: u64 = items[start..end].iter().map(item_tokens).sum();
    if total + size <= budget {
      total += size;
      keep = Some(start);
      continue;
    }
    let remaining = budget - total;
    let split = (start + 1..end).filter(|&i| !matches!(items[i], Item::ToolResult { .. })).find(|&i| items[i..end].iter().map(item_tokens).sum::<u64>() <= remaining);
    if split.is_some() {
      keep = split;
    }
    break;
  }
  keep.filter(|&s| s > first)
}

/// The summary as the model sees it from then on: a user message in front of the kept tail. `request` is the running
/// turn's prompt when the summary took it in (a compaction inside a turn whose tail starts after it): its own words
/// come back verbatim, as OpenCode replays the last user message after an overflow, so the task is not left to a
/// paraphrase
pub fn summary_item(summary: &str, request: Option<&str>) -> Item {
  let mut text = format!(
    "<conversation-summary>\n{}\n</conversation-summary>\n\nThe conversation up to here was compacted into the summary above. Continue from it: the messages after it, if any, are the most recent ones kept as they were. If work was in progress, carry on with it.",
    summary.trim()
  );
  if let Some(r) = request {
    text.push_str(&format!("\n\nThe request being worked on, in the user's own words:\n<request>\n{}\n</request>", r.trim()));
  }
  Item::User(vec![Part::Text(text)])
}

/// The longest request text carried verbatim behind a summary; a longer one (a pasted file) is cut, with a note
const REQUEST_CHARS: usize = 32 * 1024;

/// The running turn's prompt as `summary_item` carries it, with the messages the user steered in after it that the
/// summary took in too (`turn` runs from the prompt up to the kept tail): text parts, images left out, cut to
/// `REQUEST_CHARS`
fn request_text(turn: &[Item]) -> Option<String> {
  let Some(Item::User(parts)) = turn.first() else { return None };
  let mut text = parts
    .iter()
    .filter_map(|p| match p {
      Part::Text(t) => Some(t.as_str()),
      Part::Image { .. } => None,
    })
    .collect::<Vec<_>>()
    .join("\n\n");
  for added in turn[1..].iter().filter_map(crate::steer::text_of).filter(|t| !t.is_empty()) {
    text.push_str(&format!("\n\n[Added while the work was in progress]\n{added}"));
  }
  let text = text.trim();
  if text.is_empty() {
    return None;
  }
  Some(if text.len() > REQUEST_CHARS {
    format!("{}\n[{} more characters of the request left out]", crate::budget::cut(text, REQUEST_CHARS), text.len() - REQUEST_CHARS)
  } else {
    text.to_owned()
  })
}

/// Is this item a summary written by a compaction
pub fn is_summary(item: &Item) -> bool {
  matches!(item, Item::User(parts) if matches!(parts.first(), Some(Part::Text(t)) if t.starts_with("<conversation-summary>")))
}

/// One item of the transcript the summary is written from
fn serialize(item: &Item) -> String {
  // A long result is cut for the excerpt only; said so, since a bare "[truncated]" read as the tool having returned
  // less (seen 2026-10-13: summaries claiming "the code is not yet visible" after the model had read the whole file)
  let cut = |s: &str| {
    if s.len() <= TOOL_OUTPUT_CHARS {
      s.to_owned()
    } else {
      format!("{}\n[{} more characters left out of this excerpt; the assistant saw the whole result]", crate::budget::cut(s, TOOL_OUTPUT_CHARS), s.len() - TOOL_OUTPUT_CHARS)
    }
  };
  if let Some(words) = crate::steer::text_of(item) {
    return format!("[User, while the assistant was working]: {}", user_excerpt(&words));
  }
  match item {
    Item::User(parts) => {
      let text: Vec<String> = parts
        .iter()
        .map(|p| match p {
          Part::Text(t) => t.clone(),
          Part::Image { mime, .. } => format!("[Attached {mime}]"),
        })
        .collect();
      format!("[User]: {}", user_excerpt(&text.join("\n")))
    }
    Item::Assistant { text, tool_calls, .. } => {
      let mut lines = vec![];
      if !text.trim().is_empty() {
        lines.push(format!("[Assistant]: {text}"));
      }
      for c in tool_calls {
        lines.push(format!("[Assistant tool call]: {}({})", c.name, cut(&c.arguments)));
      }
      lines.join("\n")
    }
    Item::ToolResult { content, is_error, images, .. } => {
      let shown = if images.is_empty() { String::new() } else { format!(" [{} image(s) not shown]", images.len()) };
      format!("[{}]: {}{shown}", if *is_error { "Tool error" } else { "Tool result" }, cut(content))
    }
  }
}

/// A user message for the transcript: whole when short, else its start and its end around a note, since the request
/// may come before or after what was pasted with it
fn user_excerpt(text: &str) -> String {
  if text.len() <= 2 * USER_EDGE_CHARS {
    return text.to_owned();
  }
  let head = crate::budget::cut(text, USER_EDGE_CHARS);
  let mut from = text.len() - USER_EDGE_CHARS;
  while !text.is_char_boundary(from) {
    from += 1;
  }
  let tail = &text[from..];
  format!("{head}\n[{} characters of this message left out of the excerpt]\n{tail}", text.len() - head.len() - tail.len())
}

/// The template the summary follows (OpenCode's), and the intros for a first summary and an update of the last one
const TEMPLATE: &str = "Output exactly the Markdown structure shown inside <template> and keep the section order unchanged. Do not include the <template> tags in your response.
<template>
## Objective
- [one or two brief sentences describing what the user is trying to accomplish]

## Important Details
- [constraints/preferences, decisions and why, important facts/assumptions, exact context needed to continue, or \"(none)\"]

## Work State
### Completed
- [finished work, verified facts, or changes made; otherwise \"(none)\"]

### Active
- [current work, partial changes, or investigation state; otherwise \"(none)\"]

### Blocked
- [blockers, failing commands, or unknowns; otherwise \"(none)\"]

## Next Move
1. [immediate concrete action, or \"(none)\"]
2. [next action if known, or \"(none)\"]

## Relevant Files
- [file or directory path: why it matters, or \"(none)\"]
</template>

Rules:
- Keep every section, even when empty.
- Use terse bullets, not prose paragraphs.
- Preserve exact file paths, symbols, commands, error strings, URLs, and identifiers when known.
- Do not mention the summary process or that context was compacted.";

const FRESH_INTRO: &str = "Create a new anchored summary from the conversation history above.";
const UPDATE_INTRO: &str = "Update the anchored summary below using the conversation history above.\nPreserve still-true details, remove stale details, and merge in the new facts.";
const SYSTEM: &str = "You write handoff summaries of coding sessions, so that the work can continue from the summary alone.";
const OMITTED: &str = "[The earliest part of the conversation was left out of this excerpt; the previous summary covers it.]\n";

/// The summary call's user message: the transcript, then the instructions. `room` is how many bytes of transcript the
/// window takes; an older part beyond it is left out, noted at the top
pub fn summary_prompt(head: &[Item], previous: Option<&str>, room: usize) -> String {
  let mut transcript = head.iter().map(serialize).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("\n\n");
  if transcript.len() > room {
    let mut at = transcript.len() - room;
    while !transcript.is_char_boundary(at) {
      at += 1;
    }
    let at = transcript[at..].find('\n').map_or(at, |n| at + n + 1);
    transcript = format!("{OMITTED}{}", &transcript[at..]);
  }
  let intro = match previous {
    Some(p) => format!("{UPDATE_INTRO}\n<previous-summary>\n{}\n</previous-summary>", p.trim()),
    None => FRESH_INTRO.to_owned(),
  };
  format!("<conversation>\n{transcript}\n</conversation>\n\n{intro}\n\n{TEMPLATE}")
}

/// A model a summary can be written with
pub struct Writer {
  pub endpoint: Endpoint,
  pub model: ProviderModel,
  pub pick: String,
  pub cost: Option<Cost>,
}

/// What a compaction is made with: the session's model sets the tail, the writers are tried in order for the summary
/// (the fast model first when one is set, then the session's model), each without tools
pub struct Call<'a> {
  pub http: ureq::Agent,
  /// The session's model: its window decides how much of the recent history stays verbatim
  pub model: &'a ProviderModel,
  pub writers: Vec<Writer>,
}

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
  fn wire(self) -> &'static str {
    match self {
      Reason::Manual => "manual",
      Reason::Threshold => "threshold",
      Reason::Overflow => "overflow",
    }
  }
}

/// Summarize the history and replace it with the summary and the tail. The client sees `compaction_update`
/// in_progress, then completed or failed. Ok(false) when there was nothing to compact; Err carries the failure already
/// reported to the client
pub async fn run(server: &Server, session: &Session, call: &Call<'_>, reason: Reason) -> Result<bool, String> {
  let context = call.model.context.unwrap_or(crate::turn::DEFAULT_CONTEXT);
  let usable = usable(context, call.model.output);
  let budget = (usable / 4).clamp(MIN_TAIL_TOKENS, MAX_TAIL_TOKENS);
  let (items, summarized) = {
    let st = session.state.lock();
    (st.items.clone(), st.summarized)
  };
  let first = usize::from(summarized);
  // Inside a turn the last user item that opens a turn is its prompt (messages steered in after it are not)
  let current = if reason == Reason::Manual { None } else { (first..items.len()).rev().find(|&i| opens_turn(&items[i])) };
  let mut tail = tail_start(&items, first, budget).unwrap_or(items.len());
  // After an overflow the running turn is kept whole when it fits half the window, as OpenCode replays the last user
  // message: the history before it is what made the request too big, and the prompt (a pasted file, say) is still needed
  if reason == Reason::Overflow
    && let Some(cur) = current
    && cur > first
    && cur < tail
    && items[cur..].iter().map(item_tokens).sum::<u64>() <= usable / 2
  {
    tail = cur;
  }
  let previous = summarized.then(|| match &items[0] {
    Item::User(parts) => match parts.first() {
      Some(Part::Text(t)) => previous_summary(t),
      _ => String::new(),
    },
    _ => String::new(),
  });
  let head = &items[first..tail];
  if head.is_empty() {
    return Ok(false);
  }
  let id = session.next_compaction_id();
  server.update(&session.id, json!({ "sessionUpdate": "compaction_update", "compactionId": id, "status": "in_progress" }));
  let started = Instant::now();
  let mut ev = json!({ "type": "compaction", "reason": reason.wire() });
  let mut result = Err(LlmError::Protocol("no model to write the summary with".into()));
  for (n, writer) in call.writers.iter().enumerate() {
    if n > 0
      && let Err(e) = &result
    {
      // The fast model failed: the next writer (the session's model) tries again, and the log says why
      eprintln!("[acpira agent] summary with {} failed, falling back: {e}", ev["model"].as_str().unwrap_or("?"));
      ev["fallback"] = json!({ "model": ev["model"].clone(), "error": e.to_string() });
    }
    ev["model"] = Value::String(writer.pick.clone());
    result = summarize(call.http.clone(), writer.endpoint.clone(), summary_request(writer, head, previous.as_deref())).await;
    if matches!(&result, Ok((text, _)) if text.trim().is_empty()) {
      result = Err(LlmError::Protocol("the model returned an empty summary".into()));
    }
    if let Ok((_, Some(u))) = &result {
      ev["usage"] = json!({ "input": u.input, "output": u.output, "cacheRead": u.cache_read, "cacheWrite": u.cache_write, "reasoning": u.reasoning });
      if let Some(c) = &writer.cost {
        ev["cost"] = json!(c.estimate(u.input, u.output, u.cache_read, u.cache_write));
      }
    }
    if result.is_ok() {
      break;
    }
  }
  ev["ms"] = json!(started.elapsed().as_millis() as u64);
  let summary = match result {
    Ok((text, _)) => text,
    Err(e) => {
      let message = e.to_string();
      ev["error"] = Value::String(message.clone());
      session.store.append(ev);
      server.update(&session.id, json!({ "sessionUpdate": "compaction_update", "compactionId": id, "status": "failed", "error": message }));
      return Err(message);
    }
  };
  let kept = items.len() - tail;
  // When the summary took the running turn's prompt in, its words come back verbatim
  let request = current.filter(|&i| i < tail).and_then(|i| request_text(&items[i..tail]));
  {
    let mut st = session.state.lock();
    // The turn is the only writer of the history while it runs, so the snapshot is still the history
    let rest = st.items.split_off(tail);
    st.items = vec![summary_item(&summary, request.as_deref())];
    st.items.extend(rest);
    st.summarized = true;
    // The mode note was in what the summary replaced: the next prompt announces it again
    st.announced.clear();
  }
  ev["summary"] = Value::String(summary);
  ev["tail"] = json!(kept);
  if let Some(r) = request {
    ev["request"] = Value::String(r);
  }
  session.store.append(ev);
  server.update(&session.id, json!({ "sessionUpdate": "compaction_update", "compactionId": id, "status": "completed" }));
  Ok(true)
}

/// The summary call for one writer: the transcript cut to what its window takes beside the answer and the instructions
fn summary_request(writer: &Writer, head: &[Item], previous: Option<&str>) -> Request {
  let context = writer.model.context.unwrap_or(crate::turn::DEFAULT_CONTEXT);
  let room = (context.saturating_sub(SUMMARY_TOKENS + estimate(TEMPLATE) + 500) * 4) as usize;
  Request {
    model: writer.model.id.clone(),
    system: SYSTEM.to_owned(),
    items: vec![Item::User(vec![Part::Text(summary_prompt(head, previous, room))])],
    tools: vec![],
    max_tokens: Some(writer.model.output.map_or(SUMMARY_TOKENS, |o| o.min(SUMMARY_TOKENS))),
    output_limit: Some(SUMMARY_TOKENS),
    sampling: Default::default(),
    thinking: Thinking::Off,
    effort: None,
    cache_key: None,
    fast: false,
  }
}

/// The summary text inside a summary item
fn previous_summary(text: &str) -> String {
  text.strip_prefix("<conversation-summary>\n").and_then(|t| t.split_once("\n</conversation-summary>")).map(|(s, _)| s.to_owned()).unwrap_or_default()
}

async fn summarize(http: ureq::Agent, endpoint: Endpoint, request: Request) -> Result<(String, Option<Usage>), LlmError> {
  let mut rx = llm::stream(http, endpoint, request);
  while let Some(ev) = rx.recv().await {
    if let Event::Done { reply, usage, .. } = ev? {
      return Ok((reply.text, usage));
    }
  }
  Err(LlmError::Protocol("the stream ended without a result".into()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::llm::ToolCall;

  fn user(t: &str) -> Item {
    Item::User(vec![Part::Text(t.into())])
  }

  fn call(id: &str, name: &str) -> Item {
    Item::Assistant { text: String::new(), reasoning: String::new(), tool_calls: vec![ToolCall { id: id.into(), name: name.into(), arguments: "{}".into() }], native: None }
  }

  fn result(id: &str, name: &str, bytes: usize) -> Item {
    Item::ToolResult { call_id: id.into(), name: name.into(), content: "x".repeat(bytes), is_error: false, images: vec![] }
  }

  #[test]
  fn a_prune_clears_old_results_past_the_protected_recent_ones() {
    // 100 KB a result = 25k tokens; the last two turns are skipped, then 40k tokens of results are protected
    let big = 100 * 1024;
    let items = vec![
      user("a"), call("1", "read"), result("1", "read", big),
      user("b"), call("2", "read"), result("2", "read", big),
      user("c"), call("3", "skill"), result("3", "skill", big), call("4", "read"), result("4", "read", big),
      user("d"), call("5", "read"), result("5", "read", big),
      user("e"), call("6", "read"), result("6", "read", big),
    ];
    // Walking back: e and d are the last two turns; c's read (25k) is protected, its skill result always; b's read
    // takes the total past 40k and is cleared, so is a's: 50k freed, over the minimum
    assert_eq!(prune_plan(&items), vec![5, 2]);
    // Below the minimum nothing is cleared
    let few = vec![user("a"), call("1", "read"), result("1", "read", big), user("b"), user("c"), call("2", "read"), result("2", "read", 200 * 1024)];
    assert_eq!(prune_plan(&few), Vec::<usize>::new());
    // An earlier prune ends the walk
    let mut pruned = items.clone();
    apply_prune(&mut pruned, &["1".to_owned()]);
    assert_eq!(prune_plan(&pruned), vec![5]);
    assert!(matches!(&pruned[2], Item::ToolResult { content, .. } if content == PRUNED));
  }

  #[test]
  fn the_tail_keeps_the_last_two_turns_that_fit_and_splits_the_oldest() {
    let items = vec![user(&"a".repeat(400)), user(&"b".repeat(400)), call("1", "read"), result("1", "read", 4000), user(&"c".repeat(400))];
    // Turns b and c fit 2000 tokens together (100 + 1002 + 100): the tail starts at b
    assert_eq!(tail_start(&items, 0, 2000), Some(1));
    // With 1200, c fits and b's turn (1102) does not: it is split after its user message, where the rest (1002) fits
    assert_eq!(tail_start(&items, 0, 1200), Some(2));
    // With 500 nothing of b fits, since the split never lands in front of a tool result: the tail is c alone
    assert_eq!(tail_start(&items, 0, 500), Some(4));
    // With everything fitting the tail would be the whole conversation: none
    assert_eq!(tail_start(&items[1..], 0, 100_000), None);
    // Behind a summary item the same rule counts from the first real item
    let mut with_summary = vec![summary_item("old", None)];
    with_summary.extend(items[1..].iter().cloned());
    assert_eq!(tail_start(&with_summary, 1, 100_000), None);
    assert_eq!(tail_start(&with_summary, 1, 1200), Some(2));
    assert_eq!(tail_start(&with_summary, 1, 500), Some(4));
    assert!(is_summary(&with_summary[0]) && !is_summary(&items[0]));
    assert_eq!(previous_summary(match &with_summary[0] { Item::User(p) => match &p[0] { Part::Text(t) => t, _ => "" }, _ => "" }), "old");
  }

  #[test]
  fn a_steered_message_belongs_to_the_turn_it_was_sent_into() {
    let steer = crate::steer::item(&[json!([{ "type": "text", "text": "use printf" }])], true, &[]);
    let items = vec![user("a"), call("1", "bash"), result("1", "bash", 10), user("b"), call("2", "bash"), result("2", "bash", 10), steer.clone(), call("3", "bash"), result("3", "bash", 10)];
    // Two turns, not three: a tail holding both would be the whole conversation
    assert_eq!(tail_start(&items, 0, 100_000), None);
    // Summarized with the prompt it followed, it comes back verbatim behind the request
    assert_eq!(request_text(&items[3..]).as_deref(), Some("b\n\n[Added while the work was in progress]\nuse printf"));
    assert_eq!(serialize(&steer), "[User, while the assistant was working]: use printf");
  }

  #[test]
  fn the_summary_prompt_holds_the_transcript_and_the_previous_summary() {
    let head = vec![user("fix it"), call("1", "bash"), result("1", "bash", 10), Item::ToolResult { call_id: "2".into(), name: "read".into(), content: "no".into(), is_error: true, images: vec![] }];
    let p = summary_prompt(&head, None, 1 << 20);
    assert!(p.starts_with("<conversation>\n[User]: fix it\n\n[Assistant tool call]: bash({})\n\n[Tool result]: xxxxxxxxxx\n\n[Tool error]: no\n</conversation>"), "{p}");
    assert!(p.contains(FRESH_INTRO) && p.contains("## Next Move"));
    let p = summary_prompt(&head, Some("earlier"), 1 << 20);
    assert!(p.contains("<previous-summary>\nearlier\n</previous-summary>") && !p.contains(FRESH_INTRO));
    // A pasted file keeps the message's start and end, so the instruction survives on either side of it
    let pasted = vec![user(&format!("Summarize the attachment.\n<attachment>{}</attachment>\nThen stop.", "hello ".repeat(100_000)))];
    let p = summary_prompt(&pasted, None, 1 << 20);
    assert!(p.contains("[User]: Summarize the attachment.") && p.contains("Then stop.") && p.contains("characters of this message left out"), "{}", p.len());
    assert!(p.len() < 2 * USER_EDGE_CHARS + 4000);
    // Over the room the oldest part is left out at a line boundary
    let long: Vec<Item> = (0..50).map(|i| user(&format!("message {i} {}", "y".repeat(100)))).collect();
    let p = summary_prompt(&long, None, 2000);
    assert!(p.contains(OMITTED) && !p.contains("message 0 ") && p.contains("message 49 "), "{p}");
  }

  #[test]
  fn the_policy_comes_from_the_prompts_meta_and_the_window_caps_it() {
    let p = Policy::from_prompt(&json!({ "_meta": { "acpira/compaction": { "auto": false, "atTokens": 400000 } } }));
    assert_eq!(p, Policy { auto: false, at_tokens: Some(400_000) });
    assert_eq!(Policy::from_prompt(&json!({})), Policy::default());
    assert_eq!(p.threshold(1_050_000, Some(128_000)), 400_000);
    assert_eq!(p.threshold(128_000, Some(16_000)), 112_000);
    assert_eq!(Policy::default().threshold(128_000, None), 96_000);
    assert!(is_overflow(&LlmError::Http { status: 400, message: "This model's maximum context length is 131072 tokens".into(), retry_after: None }));
    assert!(is_overflow(&LlmError::Api("prompt is too long: 213462 tokens > 200000 maximum".into())));
    // Seen on the gateway, 2026-10-13
    for m in [
      "Your input exceeds the context window of this model. Please adjust your input and try again.",
      "Ollama upstream rejected the request: The prompt is too long: 1150041, model maximum context length: 1048576",
      "Ollama upstream rejected the request: invalid params, context window exceeds limit",
      "The input token count exceeds the maximum number of tokens allowed (1048576).",
    ] {
      assert!(is_overflow(&LlmError::Http { status: 400, message: m.into(), retry_after: None }), "{m}");
      assert!(is_overflow(&LlmError::Rejected(m.into())), "{m}");
    }
    let vague = LlmError::Rejected("There is an issue with this request, please try a different model.".into());
    assert!(!is_overflow(&vague) && is_refusal(&vague));
    assert!(!is_refusal(&LlmError::Api("upstream overloaded".into())));
    assert!(!is_overflow(&LlmError::Http { status: 500, message: "context".into(), retry_after: None }));
    assert!(!is_overflow(&LlmError::Http { status: 400, message: "invalid tool schema".into(), retry_after: None }));
  }
}
