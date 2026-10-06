//! Session view patches (mirror of src/shared/sessionPatch.ts). A view is encoded turn by turn once per change; a viewer that
//! already holds the previous encoding gets only the turns from the first one that differs, and for a changed last agent turn
//! only the blocks from the first one that differs. Streaming into a long conversation then costs the size of the tail, not
//! the size of the whole transcript, on every hop (sidecar stdout, the extension host, Remote-SSH, the webview)

use std::sync::Arc;

use serde::Serialize;
use serde_json::value::RawValue;

use crate::protocol::RawJson;
use crate::transcript::Turn;

/// The last visible turn when it is an agent turn: the turn with `blocks` emptied, and every block on its own
#[derive(Debug)]
pub struct LastTurnParts {
  pub head: Box<RawValue>,
  pub blocks: Vec<Box<RawValue>>,
}

/// One session view in fragments
#[derive(Debug)]
pub struct ViewPartsData {
  pub id: String,
  /// The line of views `rev` counts in (one per opened session instance, shared by a mirror's copies); never sent. A patch
  /// is only computed between two views of the same epoch
  pub epoch: u64,
  pub rev: i64,
  /// The view without its turns (an empty `turns` array)
  pub head: Box<RawValue>,
  /// Every visible turn, in order
  pub turns: Vec<Box<RawValue>>,
  pub last: Option<LastTurnParts>,
}

/// Shared between the viewers of one change; compared by identity only (the encoding is what counts)
#[derive(Debug, Clone)]
pub struct ViewParts(pub Arc<ViewPartsData>);

impl PartialEq for ViewParts {
  fn eq(&self, other: &Self) -> bool {
    Arc::ptr_eq(&self.0, &other.0)
  }
}

impl ViewPartsData {
  /// Bytes the encoding holds, what a cache of sent views budgets by
  pub fn size(&self) -> usize {
    let last = self.last.as_ref().map_or(0, |l| l.head.get().len() + l.blocks.iter().map(|b| b.get().len()).sum::<usize>());
    self.head.get().len() + self.turns.iter().map(|t| t.get().len()).sum::<usize>() + last
  }
}

pub fn raw<T: Serialize + ?Sized>(v: &T) -> Box<RawValue> {
  serde_json::value::to_raw_value(v).expect("serializable")
}

/// Every visible turn encoded on its own: `before` are the turns ahead of `last`; `last`, when an agent turn, is also split
/// into its head and blocks (`blocks` is emptied for the head encoding and handed back before this returns)
pub fn encode_turns(before: &[&Turn], last: Option<&mut Turn>) -> (Vec<Box<RawValue>>, Option<LastTurnParts>) {
  let mut encoded: Vec<Box<RawValue>> = before.iter().map(|t| raw(*t)).collect();
  let Some(t) = last else { return (encoded, None) };
  let Turn::Agent(a) = t else {
    encoded.push(raw(&*t));
    return (encoded, None);
  };
  let blocks: Vec<Box<RawValue>> = a.blocks.iter().map(raw).collect();
  let kept = std::mem::take(&mut a.blocks);
  let head = raw(&*t);
  if let Turn::Agent(a) = t {
    a.blocks = kept;
  }
  // The whole turn is the head with its blocks spliced in, so the live turn (often the largest) is encoded once
  encoded.push(splice_blocks(&head, &blocks).unwrap_or_else(|| raw(&*t)));
  (encoded, Some(LastTurnParts { head, blocks }))
}

/// `head` with its `"blocks":[]` replaced by the encoded blocks. Inside a JSON string every quote is escaped, so the marker
/// can only be a key; None when it is not unique (a nested object with an empty `blocks` key), and the caller encodes the turn
fn splice_blocks(head: &RawValue, blocks: &[Box<RawValue>]) -> Option<Box<RawValue>> {
  const MARK: &str = "\"blocks\":[]";
  let h = head.get();
  let at = h.find(MARK)?;
  if h[at + MARK.len()..].contains(MARK) {
    return None;
  }
  let mut out = String::with_capacity(h.len() + blocks.iter().map(|b| b.get().len() + 1).sum::<usize>());
  out.push_str(&h[..at]);
  out.push_str("\"blocks\":[");
  for (i, b) in blocks.iter().enumerate() {
    if i > 0 {
      out.push(',');
    }
    out.push_str(b.get());
  }
  out.push(']');
  out.push_str(&h[at + MARK.len()..]);
  RawValue::from_string(out).ok()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PatchRef<'a> {
  id: &'a str,
  base: i64,
  view: &'a RawValue,
  keep: usize,
  turns: Vec<&'a RawValue>,
  #[serde(skip_serializing_if = "Option::is_none")]
  keep_blocks: Option<usize>,
  #[serde(skip_serializing_if = "Option::is_none")]
  blocks: Option<Vec<&'a RawValue>>,
}

fn common_prefix(a: &[Box<RawValue>], b: &[Box<RawValue>]) -> usize {
  a.iter().zip(b).take_while(|(x, y)| x.get() == y.get()).count()
}

/// The patch that turns `prev` (what the viewer holds) into `next`; None when the whole view is no larger (another
/// session or another instance of it, or nothing in common)
pub fn session_patch(prev: &ViewPartsData, next: &ViewPartsData) -> Option<RawJson> {
  if prev.id != next.id || prev.epoch != next.epoch {
    return None;
  }
  let keep = common_prefix(&prev.turns, &next.turns);
  let mut patch = PatchRef {
    id: &next.id,
    base: prev.rev,
    view: &next.head,
    keep,
    turns: next.turns[keep..].iter().map(|t| &**t).collect(),
    keep_blocks: None,
    blocks: None,
  };
  // The same last agent turn changed (a streamed chunk, a tool update): send its head and the blocks from the first change
  if keep + 1 == next.turns.len()
    && keep + 1 == prev.turns.len()
    && let (Some(p), Some(n)) = (&prev.last, &next.last)
  {
    let kb = common_prefix(&p.blocks, &n.blocks);
    if kb > 0 {
      patch.turns = vec![&*n.head];
      patch.keep_blocks = Some(kb);
      patch.blocks = Some(n.blocks[kb..].iter().map(|b| &**b).collect());
    }
  }
  if keep == 0 && patch.keep_blocks.is_none() && !next.turns.is_empty() {
    return None;
  }
  Some(RawJson::new(&patch))
}

/// Sessions whose last sent view a page keeps (`src/shared/sessionPatch.ts` `VIEW_CACHE_ENTRIES` keeps at least as many):
/// switching back to one of them is sent as a patch against that view instead of the whole transcript again
pub const SENT_VIEWS_MAX: usize = 8;
/// Encoded bytes the kept views may hold per page; the view on screen stays even when it alone is larger
pub const SENT_VIEWS_BUDGET: usize = 32 << 20;

/// How one view goes to a page
pub enum Delivery {
  /// Older than the view on screen (two flushes racing, a batch after a resync): nothing to send
  Stale,
  Whole,
  Patch(RawJson),
}

/// What a page holds, as far as patches go: the view on screen and the last view sent of each recent session. Kept in step
/// with the page's own cache by sending through it in order; a page that cannot apply a patch asks for the whole view
/// (`forget` then makes the next one whole)
#[derive(Default)]
pub struct SentViews {
  /// The view on screen; None after anything without parts replaced it (a ChatGPT mirror, a page that does not patch)
  current: Option<ViewParts>,
  /// Least recently sent first, with each encoding's size
  kept: Vec<(ViewParts, usize)>,
  bytes: usize,
}

impl SentViews {
  /// The page starts over (init): it holds nothing
  pub fn clear(&mut self) {
    *self = SentViews::default();
  }

  /// The page could not apply a patch for `id`: whatever it holds of that session is unknown
  pub fn forget(&mut self, id: &str) {
    if self.current.as_ref().is_some_and(|c| c.0.id == id) {
      self.current = None;
    }
    if let Some(i) = self.kept.iter().position(|(v, _)| v.0.id == id) {
      self.bytes -= self.kept.remove(i).1;
    }
  }

  /// Record that `next` goes to the page and say how: a patch against the last view of that session the page was sent,
  /// or the whole view. None is a view with nothing to patch against (no parts), which replaces the one on screen
  pub fn deliver(&mut self, next: Option<&ViewParts>) -> Delivery {
    let Some(next) = next else {
      self.current = None;
      return Delivery::Whole;
    };
    if let Some(cur) = &self.current
      && cur.0.id == next.0.id
      && next.0.rev < cur.0.rev
    {
      return Delivery::Stale;
    }
    let patch = self.kept.iter().find(|(v, _)| v.0.id == next.0.id).and_then(|(prev, _)| session_patch(&prev.0, &next.0));
    self.forget(&next.0.id);
    let size = next.0.size();
    self.kept.push((next.clone(), size));
    self.bytes += size;
    self.current = Some(next.clone());
    // The oldest go first; the one just sent stays even alone over budget, it is what the next push patches against
    while self.kept.len() > 1 && (self.kept.len() > SENT_VIEWS_MAX || self.bytes > SENT_VIEWS_BUDGET) {
      self.bytes -= self.kept.remove(0).1;
    }
    patch.map_or(Delivery::Whole, Delivery::Patch)
  }

  #[cfg(test)]
  fn kept_ids(&self) -> Vec<&str> {
    self.kept.iter().map(|(v, _)| v.0.id.as_str()).collect()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::{Value, json};

  fn turn(v: Value) -> Turn {
    serde_json::from_value(v).unwrap()
  }

  fn parts(rev: i64, mut turns: Vec<Turn>) -> ViewPartsData {
    let mut last = turns.pop();
    let before: Vec<&Turn> = turns.iter().collect();
    let (turns, last) = encode_turns(&before, last.as_mut());
    ViewPartsData { id: "s".into(), epoch: 1, rev, head: raw(&json!({ "id": "s", "rev": rev, "turns": [] })), turns, last }
  }

  fn user(id: &str) -> Turn {
    turn(json!({ "role": "user", "id": id, "text": id }))
  }

  fn agent(texts: &[&str]) -> Turn {
    turn(json!({ "role": "agent", "blocks": texts.iter().map(|t| json!({ "type": "text", "markdown": t })).collect::<Vec<_>>() }))
  }

  fn value(r: &RawJson) -> Value {
    serde_json::from_str(r.get()).unwrap()
  }

  #[test]
  fn every_turn_is_encoded_and_the_last_agent_turn_also_block_by_block() {
    let p = parts(3, vec![user("u1"), agent(&["a", "b"])]);
    assert_eq!(p.turns.len(), 2);
    let whole: Value = serde_json::from_str(p.turns[1].get()).unwrap();
    assert_eq!(whole["blocks"][1]["markdown"], "b");
    let last = p.last.as_ref().unwrap();
    assert_eq!(serde_json::from_str::<Value>(last.head.get()).unwrap(), json!({ "role": "agent", "blocks": [] }));
    assert_eq!(last.blocks.len(), 2);
    assert!(parts(1, vec![agent(&["a"]), user("u2")]).last.is_none());
  }

  #[test]
  fn the_spliced_last_turn_is_the_turn_as_serde_writes_it() {
    // a block whose text holds the marker, and an empty turn
    for t in [agent(&["a", "\"blocks\":[]", "c"]), agent(&[])] {
      let expected = raw(&t).get().to_owned();
      let mut last = t.clone();
      let (turns, _) = encode_turns(&[], Some(&mut last));
      assert_eq!(turns[0].get(), expected);
      assert_eq!(last, t);
    }
  }

  #[test]
  fn a_streamed_chunk_carries_only_the_changed_blocks_of_the_last_turn() {
    let prev = parts(1, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x", "y"])]);
    let next = parts(2, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x", "yz"])]);
    let p = value(&session_patch(&prev, &next).unwrap());
    assert_eq!((p["base"].clone(), p["keep"].clone(), p["keepBlocks"].clone()), (json!(1), json!(3), json!(1)));
    assert_eq!(p["turns"], json!([{ "role": "agent", "blocks": [] }]));
    assert_eq!(p["blocks"], json!([{ "type": "text", "markdown": "yz" }]));
    assert_eq!(p["view"]["rev"], 2);
  }

  #[test]
  fn a_new_turn_is_sent_whole_after_the_kept_ones() {
    let prev = parts(1, vec![user("u1"), agent(&["a"])]);
    let next = parts(2, vec![user("u1"), agent(&["a"]), user("u2")]);
    let p = value(&session_patch(&prev, &next).unwrap());
    assert_eq!(p["keep"], 2);
    assert_eq!(p["turns"], json!([{ "role": "user", "id": "u2", "text": "u2" }]));
    assert!(p.get("keepBlocks").is_none());
    // dropped turns (a retry) shrink the list from `keep`
    let p = value(&session_patch(&next, &parts(3, vec![user("u1")])).unwrap());
    assert_eq!((p["keep"].clone(), p["turns"].clone()), (json!(1), json!([])));
  }

  fn sent(id: &str, epoch: u64, rev: i64, turns: Vec<Turn>) -> ViewParts {
    let mut p = parts(rev, turns);
    p.id = id.into();
    p.epoch = epoch;
    ViewParts(std::sync::Arc::new(p))
  }

  fn kind(d: &Delivery) -> &'static str {
    match d {
      Delivery::Stale => "stale",
      Delivery::Whole => "whole",
      Delivery::Patch(_) => "patch",
    }
  }

  #[test]
  fn switching_back_to_a_kept_session_is_a_patch_against_its_last_sent_view() {
    let mut v = SentViews::default();
    let long = vec![user("u1"), agent(&["a", "b"])];
    assert_eq!(kind(&v.deliver(Some(&sent("a", 1, 1, long.clone())))), "whole");
    assert_eq!(kind(&v.deliver(Some(&sent("b", 2, 1, vec![user("x")])))), "whole");
    // back to a: only what changed since the page last saw it
    let d = v.deliver(Some(&sent("a", 1, 2, long.clone())));
    let Delivery::Patch(p) = &d else { panic!("expected a patch, got {}", kind(&d)) };
    let p = value(p);
    assert_eq!((p["base"].clone(), p["keep"].clone(), p["turns"].clone()), (json!(1), json!(2), json!([])));
    // a reopened instance (another epoch) is sent whole even at a lower rev: it is not the view on screen
    assert_eq!(kind(&v.deliver(Some(&sent("b", 3, 0, vec![user("x")])))), "whole");
    // the view on screen never goes back to an older rev
    assert_eq!(kind(&v.deliver(Some(&sent("b", 3, -1, vec![user("x")])))), "stale");
  }

  #[test]
  fn kept_views_are_bounded_by_count_and_bytes_but_the_view_on_screen_stays() {
    let mut v = SentViews::default();
    for i in 0..SENT_VIEWS_MAX + 2 {
      v.deliver(Some(&sent(&format!("s{i}"), i as u64, 1, vec![user("u")])));
    }
    assert_eq!(v.kept_ids().len(), SENT_VIEWS_MAX);
    assert_eq!(v.kept_ids()[0], "s2");
    let huge = "x".repeat(SENT_VIEWS_BUDGET + 1);
    v.deliver(Some(&sent("big", 99, 1, vec![agent(&[huge.as_str()])])));
    assert_eq!(v.kept_ids(), vec!["big"]);
    // forget (a failed patch on the page) makes the next view of that session whole
    v.forget("big");
    assert_eq!(kind(&v.deliver(Some(&sent("big", 99, 2, vec![agent(&[huge.as_str()])])))), "whole");
    // a view without parts replaces the one on screen but keeps the rest
    v.deliver(Some(&sent("s9", 9, 1, vec![user("u")])));
    assert_eq!(kind(&v.deliver(None)), "whole");
    assert_eq!(kind(&v.deliver(Some(&sent("s9", 9, 2, vec![user("u")])))), "patch");
  }

  #[test]
  fn another_session_or_nothing_in_common_is_not_patched() {
    let prev = parts(1, vec![user("u1")]);
    let mut other = parts(2, vec![user("u1")]);
    other.id = "t".into();
    assert!(session_patch(&prev, &other).is_none());
    // the same session reopened (a new instance) counts its revs again: never patched across
    let mut reopened = parts(2, vec![user("u1")]);
    reopened.epoch = 2;
    assert!(session_patch(&prev, &reopened).is_none());
    assert!(session_patch(&prev, &parts(2, vec![user("u9")])).is_none());
    // an unchanged transcript still patches (controls, usage …), with no turns at all
    let p = value(&session_patch(&prev, &parts(2, vec![user("u1")])).unwrap());
    assert_eq!((p["keep"].clone(), p["turns"].clone()), (json!(1), json!([])));
  }
}
