//! Windowed session views (mirror of packages/shared/sessionWindow.ts). A page that said `window` in `ready` gets a view
//! that starts at its tail: the last turns that fit a byte budget, the first of them cut inside its blocks when it alone
//! is larger. `SessionView.earlier` says what is left out, and the page asks for the part right ahead of what it holds
//! (`loadEarlier`, answered with one `sessionEarlier`). Nothing sent to such a page grows with the transcript, so a paired
//! phone opens a session of any size, over a link whose send queue is bounded.
//!
//! Patches stay as they are for the page: the engine moves their indices into the window (`window_patch`). Indices
//! elsewhere (`editTurn`, `forkSession`, `SubagentSummary.turnIndex`, search hits) keep counting from the session's first
//! turn; the page adds `earlier.turns` to its own.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json::value::RawValue;

use crate::protocol::RawJson;
use crate::session_patch::{PatchRef, ViewPartsData, diff, raw, splice_array};

/// Encoded bytes of turns a windowed view opens with, and one `sessionEarlier` carries. A single block or an uncut turn
/// larger than this still goes whole: something must be shown
pub const VIEW_WINDOW_BYTES: usize = 512 << 10;

/// What a windowed view leaves out ahead of its first turn: `turns` whole turns, and the first `blocks` blocks of the
/// first turn it does hold (an agent turn)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Earlier {
  pub turns: usize,
  pub blocks: usize,
}

impl Earlier {
  pub fn is_whole(self) -> bool {
    self == Earlier::default()
  }
}

/// An agent turn taken apart: the turn with `blocks` emptied, and every block on its own
struct Split<'a> {
  head: Cow<'a, RawValue>,
  blocks: Cow<'a, [Box<RawValue>]>,
}

/// Turn `i` taken apart, or None when it has no blocks to cut at (a user turn) or cannot be put back together by
/// splicing (`splice_array` needs its one empty `blocks` key)
fn split(parts: &ViewPartsData, i: usize) -> Option<Split<'_>> {
  let spliceable = |head: &RawValue| head.get().matches("\"blocks\":[]").count() == 1;
  // The last agent turn is already encoded block by block
  if i + 1 == parts.turns.len()
    && let Some(last) = &parts.last
  {
    return spliceable(&last.head).then(|| Split { head: Cow::Borrowed(&*last.head), blocks: Cow::Borrowed(&last.blocks) });
  }
  let mut turn: Value = serde_json::from_str(parts.turns.get(i)?.get()).ok()?;
  if turn["role"] != "agent" {
    return None;
  }
  let blocks: Vec<Box<RawValue>> = turn.get_mut("blocks")?.as_array_mut()?.drain(..).map(|b| raw(&b)).collect();
  let head = raw(&turn);
  spliceable(&head).then_some(Split { head: Cow::Owned(head), blocks: Cow::Owned(blocks) })
}

/// The first of the trailing blocks that fit `left` bytes; with `at_least_one`, the last block goes whatever its size
fn tail_start(blocks: &[Box<RawValue>], left: usize, at_least_one: bool) -> usize {
  let mut left = left;
  let mut start = blocks.len();
  while start > 0 {
    let size = blocks[start - 1].get().len();
    if size > left && !(at_least_one && start == blocks.len()) {
      break;
    }
    left = left.saturating_sub(size);
    start -= 1;
  }
  start
}

/// Where a view opened at its tail starts: turns from the end while they fit `budget`. A turn larger than the whole
/// budget is cut inside its blocks when it is the last one (the view must show its end); ahead of others the window
/// stops, and the next `earlier_chunk` cuts it
pub fn open(parts: &ViewPartsData, budget: usize) -> Earlier {
  let n = parts.turns.len();
  let mut left = budget;
  let mut i = n;
  while i > 0 {
    let size = parts.turns[i - 1].get().len();
    if size <= left {
      left -= size;
      i -= 1;
      continue;
    }
    if i == n {
      if let Some(s) = split(parts, i - 1) {
        let start = tail_start(&s.blocks, left, true);
        return Earlier { turns: i - 1, blocks: start };
      }
      // Not a turn that can be cut: it goes whole
      i -= 1;
    }
    break;
  }
  Earlier { turns: i, blocks: 0 }
}

/// `head` naming what the view leaves out; unchanged for a whole view
fn stamp(head: &RawValue, at: Earlier) -> Cow<'_, RawValue> {
  if at.is_whole() {
    return Cow::Borrowed(head);
  }
  let stamped = head.get().strip_prefix('{').map(|rest| {
    let comma = if rest.trim_start().starts_with('}') { "" } else { "," };
    format!("{{\"earlier\":{{\"turns\":{},\"blocks\":{}}}{comma}{rest}", at.turns, at.blocks)
  });
  match stamped.and_then(|s| RawValue::from_string(s).ok()) {
    Some(s) => Cow::Owned(s),
    None => Cow::Borrowed(head),
  }
}

/// Turn `at.turns` without its first `at.blocks` blocks
fn cut_turn(parts: &ViewPartsData, at: Earlier) -> Option<Box<RawValue>> {
  let s = split(parts, at.turns)?;
  splice_array(&s.head, "blocks", s.blocks.get(at.blocks..)?)
}

/// The view from `at` on, as the page takes it whole; None when it cannot be put together (the caller sends the whole
/// view, and the page holds everything)
pub fn window_view(parts: &ViewPartsData, at: Earlier) -> Option<RawJson> {
  let mut turns: Vec<Cow<'_, RawValue>> = parts.turns.get(at.turns..)?.iter().map(|t| Cow::Borrowed(&**t)).collect();
  if at.blocks > 0 {
    *turns.first_mut()? = Cow::Owned(cut_turn(parts, at)?);
  }
  let head = stamp(&parts.head, at);
  let mark = "\"turns\":[]";
  let (before, after) = head.get().split_once(mark)?;
  if after.contains(mark) {
    return None;
  }
  let mut out = String::with_capacity(head.get().len() + turns.iter().map(|t| t.get().len() + 1).sum::<usize>());
  out.push_str(before);
  out.push_str("\"turns\":[");
  for (i, t) in turns.iter().enumerate() {
    if i > 0 {
      out.push(',');
    }
    out.push_str(t.get());
  }
  out.push(']');
  out.push_str(after);
  RawValue::from_string(out).ok().map(|v| RawJson(std::sync::Arc::from(v)))
}

/// The patch from `prev` to `next` for a page that holds `prev` from `at` on, its indices counted inside the window.
/// None when the change reaches ahead of the window or the patch is larger than a fresh window: the page then gets
/// `next` opened at its tail again
pub fn window_patch(prev: &ViewPartsData, next: &ViewPartsData, at: Earlier, budget: usize) -> Option<RawJson> {
  let head = stamp(&next.head, at);
  let mut p: PatchRef<'_> = diff(prev, next)?;
  if p.keep < at.turns {
    return None;
  }
  // The cut turn changed: only a block-level change behind the cut reaches the page as one
  if p.keep == at.turns && at.blocks > 0 {
    let kept = p.keep_blocks?;
    p.keep_blocks = Some(kept.checked_sub(at.blocks)?);
  }
  p.keep -= at.turns;
  p.view = &head;
  let size = p.turns.iter().map(|t| t.get().len()).sum::<usize>() + p.blocks.iter().flatten().map(|b| b.get().len()).sum::<usize>();
  (size <= budget).then(|| RawJson::new(&p))
}

/// What comes right ahead of a view held from `from` on, one budget's worth: the rest of the cut turn's blocks first, then
/// whole turns, a turn larger than the budget in pieces of its own
#[derive(Debug)]
pub struct Chunk {
  /// Blocks to put ahead of the first held turn's
  pub blocks: Vec<Box<RawValue>>,
  /// Turns to put ahead of the held ones, in order; the first is cut when `earlier.blocks` is not 0
  pub turns: Vec<Box<RawValue>>,
  /// What is still left out after this chunk
  pub earlier: Earlier,
}

/// None when `from` does not fit `parts` (the page and the engine's record of it disagree)
pub fn earlier_chunk(parts: &ViewPartsData, from: Earlier, budget: usize) -> Option<Chunk> {
  if from.turns > parts.turns.len() {
    return None;
  }
  let mut left = budget;
  let mut blocks = Vec::new();
  let mut at = from;
  let mut took = false;
  if from.blocks > 0 {
    let s = split(parts, from.turns)?;
    let ahead = s.blocks.get(..from.blocks)?;
    let start = tail_start(ahead, left, true);
    blocks = ahead[start..].to_vec();
    left = left.saturating_sub(blocks.iter().map(|b| b.get().len()).sum());
    at = Earlier { turns: from.turns, blocks: start };
    if start > 0 {
      return Some(Chunk { blocks, turns: Vec::new(), earlier: at });
    }
    took = true;
  }
  let mut turns = Vec::new();
  let mut i = at.turns;
  while i > 0 {
    let turn = &parts.turns[i - 1];
    let size = turn.get().len();
    if size <= left {
      turns.push(turn.clone());
      left -= size;
      i -= 1;
      took = true;
      continue;
    }
    // Larger than what is left: a chunk of its own, in pieces when it can be cut
    if !took {
      let piece = Earlier { turns: i - 1, blocks: split(parts, i - 1).map_or(0, |s| tail_start(&s.blocks, left, true)) };
      if piece.blocks > 0
        && let Some(cut) = cut_turn(parts, piece)
      {
        turns.push(cut);
        return Some(Chunk { blocks, turns, earlier: piece });
      }
      turns.push(turn.clone());
      i -= 1;
    }
    break;
  }
  turns.reverse();
  Some(Chunk { blocks, turns, earlier: Earlier { turns: i, blocks: 0 } })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChunkRef<'a> {
  id: &'a str,
  from: Earlier,
  turns: Vec<&'a RawValue>,
  #[serde(skip_serializing_if = "Vec::is_empty")]
  blocks: Vec<&'a RawValue>,
  #[serde(skip_serializing_if = "Option::is_none")]
  earlier: Option<Earlier>,
}

/// A chunk as `sessionEarlier` carries it: the page applies it only while its view of `id` still starts at `from`
pub fn chunk_json(id: &str, from: Earlier, chunk: &Chunk) -> RawJson {
  RawJson::new(&ChunkRef {
    id,
    from,
    turns: chunk.turns.iter().map(|t| &**t).collect(),
    blocks: chunk.blocks.iter().map(|b| &**b).collect(),
    earlier: (!chunk.earlier.is_whole()).then_some(chunk.earlier),
  })
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::session_patch::{encode_turns, session_patch};
  use crate::transcript::Turn;
  use serde_json::json;

  fn user(id: &str) -> Turn {
    serde_json::from_value(json!({ "role": "user", "id": id, "text": id })).unwrap()
  }

  fn agent(texts: &[&str]) -> Turn {
    serde_json::from_value(json!({ "role": "agent", "blocks": texts.iter().map(|t| json!({ "type": "text", "markdown": t })).collect::<Vec<_>>() })).unwrap()
  }

  fn parts(rev: i64, mut turns: Vec<Turn>) -> ViewPartsData {
    let mut last = turns.pop();
    let before: Vec<&Turn> = turns.iter().collect();
    let (turns, last) = encode_turns(&before, last.as_mut());
    ViewPartsData { id: "s".into(), epoch: 1, rev, head: raw(&json!({ "id": "s", "rev": rev, "turns": [] })), turns, last }
  }

  fn value(r: &RawJson) -> Value {
    serde_json::from_str(r.get()).unwrap()
  }

  fn whole(p: &ViewPartsData) -> Value {
    value(&window_view(p, Earlier::default()).unwrap())
  }

  /// What a page does with a chunk (sessionWindow.ts applySessionEarlier)
  fn apply_chunk(view: &mut Value, chunk: &Value) {
    assert_eq!(view.get("earlier").cloned().unwrap_or(json!({ "turns": 0, "blocks": 0 })), chunk["from"]);
    let held = view["turns"].as_array_mut().unwrap();
    if let Some(blocks) = chunk.get("blocks").and_then(Value::as_array) {
      let first = held[0]["blocks"].as_array_mut().unwrap();
      first.splice(0..0, blocks.iter().cloned());
    }
    held.splice(0..0, chunk["turns"].as_array().unwrap().iter().cloned());
    match chunk.get("earlier") {
      Some(e) => view["earlier"] = e.clone(),
      None => {
        view.as_object_mut().unwrap().remove("earlier");
      }
    }
  }

  /// A page that opens at the tail and asks until nothing is left ends up with the whole view, whatever the budget
  fn loads_back_to(p: &ViewPartsData, budget: usize) -> usize {
    let mut at = open(p, budget);
    let mut view = value(&window_view(p, at).unwrap());
    let mut asked = 0;
    while !at.is_whole() {
      let chunk = earlier_chunk(p, at, budget).unwrap();
      assert!(chunk.earlier != at, "a chunk always moves the window");
      apply_chunk(&mut view, &value(&chunk_json("s", at, &chunk)));
      at = chunk.earlier;
      asked += 1;
    }
    assert_eq!(view, whole(p));
    asked
  }

  #[test]
  fn a_view_within_the_budget_is_whole() {
    let p = parts(1, vec![user("u1"), agent(&["a", "b"])]);
    assert!(open(&p, 1 << 20).is_whole());
    assert!(whole(&p).get("earlier").is_none());
    assert_eq!(whole(&p)["turns"][1]["blocks"][1]["markdown"], "b");
  }

  #[test]
  fn a_long_view_opens_at_its_tail_and_loads_back_in_chunks() {
    let turns: Vec<Turn> = (0..20).flat_map(|i| [user(&format!("u{i}")), agent(&[&"x".repeat(100)])]).collect();
    let p = parts(1, turns);
    let at = open(&p, 400);
    assert!(at.turns > 0 && at.turns < 40 && at.blocks == 0);
    let v = value(&window_view(&p, at).unwrap());
    assert_eq!(v["earlier"], json!({ "turns": at.turns, "blocks": 0 }));
    assert_eq!(v["turns"].as_array().unwrap().len(), 40 - at.turns);
    assert_eq!(v["turns"].as_array().unwrap().last().unwrap()["role"], "agent");
    assert!(loads_back_to(&p, 400) > 3);
  }

  #[test]
  fn a_turn_larger_than_the_budget_is_cut_inside_its_blocks() {
    let big: Vec<String> = (0..30).map(|i| format!("{i}:{}", "y".repeat(60))).collect();
    let big: Vec<&str> = big.iter().map(String::as_str).collect();
    // the last turn (encoded block by block already) and one in the middle (taken apart on demand)
    for turns in [vec![user("u1"), agent(&big)], vec![user("u1"), agent(&big), user("u2"), agent(&["end"])]] {
      let p = parts(1, turns);
      let at = open(&p, 300);
      assert!(!at.is_whole());
      assert!(loads_back_to(&p, 300) > 5);
    }
    // the last turn alone opens cut, its tail blocks on screen
    let p = parts(1, vec![user("u1"), agent(&big)]);
    let at = open(&p, 300);
    assert_eq!(at.turns, 1);
    assert!(at.blocks > 0 && at.blocks < 30);
    let v = value(&window_view(&p, at).unwrap());
    assert_eq!(v["turns"][0]["blocks"].as_array().unwrap().len(), 30 - at.blocks);
    assert!(v["turns"][0]["blocks"][0]["markdown"].as_str().unwrap().starts_with(&format!("{}:", at.blocks)));
  }

  #[test]
  fn one_block_or_a_user_turn_larger_than_the_budget_still_goes() {
    let huge = "z".repeat(5000);
    let p = parts(1, vec![user(&huge), agent(&[&huge]), user(&huge)]);
    let at = open(&p, 100);
    assert_eq!(at, Earlier { turns: 2, blocks: 0 });
    assert_eq!(loads_back_to(&p, 100), 2);
    let p = parts(1, vec![user("u"), agent(&["a", &huge])]);
    assert_eq!(open(&p, 100), Earlier { turns: 1, blocks: 1 });
    loads_back_to(&p, 100);
  }

  #[test]
  fn a_streamed_patch_is_counted_inside_the_window() {
    let prev = parts(1, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x", "y"])]);
    let next = parts(2, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x", "yz"])]);
    let at = Earlier { turns: 2, blocks: 0 };
    let p = value(&window_patch(&prev, &next, at, 1 << 20).unwrap());
    assert_eq!((p["base"].clone(), p["keep"].clone(), p["keepBlocks"].clone()), (json!(1), json!(1), json!(1)));
    assert_eq!(p["view"]["earlier"], json!({ "turns": 2, "blocks": 0 }));
    assert_eq!(p["view"]["rev"], 2);
    // a whole view is patched as ever
    let plain = value(&window_patch(&prev, &next, Earlier::default(), 1 << 20).unwrap());
    assert_eq!(plain, value(&session_patch(&prev, &next).unwrap()));
    // the cut turn streams on: its kept blocks count from the cut
    let at = Earlier { turns: 3, blocks: 1 };
    let p = value(&window_patch(&prev, &next, at, 1 << 20).unwrap());
    assert_eq!((p["keep"].clone(), p["keepBlocks"].clone()), (json!(0), json!(0)));
    assert_eq!(p["blocks"], json!([{ "type": "text", "markdown": "yz" }]));
  }

  #[test]
  fn a_change_ahead_of_the_window_or_a_large_patch_opens_the_window_again() {
    let prev = parts(1, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x", "y"])]);
    let rewritten = parts(2, vec![user("u1"), agent(&["b"]), user("u2"), agent(&["x", "y"])]);
    assert!(window_patch(&prev, &rewritten, Earlier { turns: 2, blocks: 0 }, 1 << 20).is_none());
    // a change inside the cut part of the cut turn
    let cut = parts(2, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["w", "y"])]);
    assert!(window_patch(&prev, &cut, Earlier { turns: 3, blocks: 1 }, 1 << 20).is_none());
    // new turns larger than a window
    let long = "n".repeat(900);
    let grown = parts(2, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x", "y"]), user(&long)]);
    assert!(window_patch(&prev, &grown, Earlier { turns: 2, blocks: 0 }, 300).is_none());
    assert!(window_patch(&prev, &grown, Earlier { turns: 2, blocks: 0 }, 2000).is_some());
  }

  #[test]
  fn a_chunk_for_a_window_the_view_does_not_have_is_refused() {
    let p = parts(1, vec![user("u1"), agent(&["a", "b"])]);
    assert!(earlier_chunk(&p, Earlier { turns: 5, blocks: 0 }, 100).is_none());
    assert!(earlier_chunk(&p, Earlier { turns: 1, blocks: 9 }, 100).is_none());
    assert!(earlier_chunk(&p, Earlier { turns: 0, blocks: 1 }, 100).is_none());
  }
}
