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

use std::hash::{BuildHasher, Hasher};
use std::path::Path;
use std::time::Instant;

use serde_json::json;

use crate::acp::{Server, Session};
pub use crate::chunk_data::{Budget, Chunk, STRATEGY, Shown, Status, merge, render, tokens};
use crate::chunk_data::{item_tokens, one_line};
#[cfg(test)]
use crate::chunk_data::{preview, select};
pub use crate::chunk_history::{grep, list};
use crate::compact_data::Reason;
pub(crate) use crate::defs::user_words;
use crate::llm::Item;

mod calls;
mod closed;
mod work;

use self::closed::{closed_content, status_of, user_blocks};
use self::work::work_state;

/// A user message longer than this is shown as its start and end with a reference to the whole text
const LONG_USER_TOKENS: u64 = 2_000;
/// One archived request's line; the first one gets more, as it often states the session's constraints
const REQUEST_TOKENS: u64 = 80;
const FIRST_REQUEST_TOKENS: u64 = 240;
/// The work-state index of a folded turn: at most this, and at most `INDEX_SHARE` of the usable window
const INDEX_TOKENS: u64 = 8_000;
const INDEX_SHARE: u64 = 20;
/// A running turn is folded when the history after sealing still takes this share of the limit (tenths)
const FOLD_TRIGGER_TENTHS: u64 = 7;
/// ... down to this share of it (tenths): half the limit, so a long turn does not fold every step
const FOLD_TARGET_TENTHS: u64 = 5;

const ID_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghjkmnpqrstuvwxyz123456789";

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
  // Ordinals read back from the session file: a chunk ending before it starts is not one to continue
  let partial = chunks.last().filter(|c| c.status == Status::Partial && c.to == base && c.from <= c.to).cloned();
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

#[cfg(test)]
mod tests {
  use serde_json::Value;

  use super::*;
  use crate::chunk_history::transcript;
  use crate::llm::{Part, ToolCall};

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
