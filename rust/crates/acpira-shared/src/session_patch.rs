//! Session view patches (mirror of packages/shared/sessionPatch.ts). A view is encoded turn by turn once per change; a viewer that
//! already holds the previous encoding gets only the turns from the first one that differs, and for a changed last agent turn
//! only the blocks from the first one that differs. Streaming into a long conversation then costs the size of the tail, not
//! the size of the whole transcript, on every hop (sidecar stdout, the extension host, Remote-SSH, the webview)

use std::sync::Arc;

use serde::Serialize;
use serde_json::value::RawValue;

use crate::protocol::RawJson;
use crate::session_window::{self, Earlier, window_patch};
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

#[expect(clippy::expect_used, reason = "only the protocol's own types come here; one that cannot serialize is a bug of the build")]
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

/// What the last encoding of a session's turns was made of, so the next one encodes only what changed. Every turn, and
/// every block of the live turn, is compared with its copy here and keeps its encoding while equal: a streamed chunk
/// then costs a comparison of the transcript (a fraction of encoding it) plus the blocks that changed, where
/// `encode_turns` alone encodes every turn again on every change
#[derive(Default)]
pub struct Reuse {
  /// The turns as they were last encoded, in order; the live turn with its blocks
  seen: Vec<Turn>,
  /// The view those encodings went into (`keep`); None until then, and nothing is reused without it
  parts: Option<ViewParts>,
}

impl Reuse {
  /// `encode_turns`, byte for byte, reusing what `keep` was last given
  pub fn encode(&mut self, before: &[&Turn], last: Option<&mut Turn>) -> (Vec<Box<RawValue>>, Option<LastTurnParts>) {
    fn put<T>(held: &mut Vec<T>, i: usize, v: T) {
      match held.get_mut(i) {
        Some(slot) => *slot = v,
        None => held.push(v),
      }
    }
    // Taken, so an encoding that is never kept leaves nothing a later one could wrongly reuse
    let prev = self.parts.take();
    let prev = prev.as_ref().map(|p| &*p.0);
    let n = before.len() + usize::from(last.is_some());
    let mut encoded: Vec<Box<RawValue>> = Vec::with_capacity(n);
    for (i, t) in before.iter().enumerate() {
      match prev.and_then(|p| p.turns.get(i)).filter(|_| self.seen.get(i) == Some(*t)) {
        Some(same) => encoded.push(same.clone()),
        None => {
          encoded.push(raw(*t));
          put(&mut self.seen, i, (*t).clone());
        }
      }
    }
    let i = before.len();
    let Some(t) = last else {
      self.seen.truncate(n);
      return (encoded, None);
    };
    let Turn::Agent(a) = &*t else {
      match prev.and_then(|p| p.turns.get(i)).filter(|_| self.seen.get(i) == Some(&*t)) {
        Some(same) => encoded.push(same.clone()),
        None => {
          encoded.push(raw(&*t));
          put(&mut self.seen, i, t.clone());
        }
      }
      self.seen.truncate(n);
      return (encoded, None);
    };
    // The live turn's blocks have encodings of their own only where the last view had the same turn live
    let live = prev.filter(|p| p.turns.len() == n).and_then(|p| p.last.as_ref());
    let mut seen_blocks = match self.seen.get_mut(i) {
      Some(Turn::Agent(s)) => std::mem::take(&mut s.blocks),
      _ => Vec::new(),
    };
    let mut blocks: Vec<Box<RawValue>> = Vec::with_capacity(a.blocks.len());
    for (j, b) in a.blocks.iter().enumerate() {
      match live.and_then(|l| l.blocks.get(j)).filter(|_| seen_blocks.get(j) == Some(b)) {
        Some(same) => blocks.push(same.clone()),
        None => {
          blocks.push(raw(b));
          put(&mut seen_blocks, j, b.clone());
        }
      }
    }
    seen_blocks.truncate(a.blocks.len());
    // The head, and the copy kept of it, are taken with the blocks out: neither encodes nor clones them again
    let kept = match t {
      Turn::Agent(a) => std::mem::take(&mut a.blocks),
      Turn::User(_) => Vec::new(),
    };
    let head = raw(&*t);
    let mut copy = t.clone();
    if let Turn::Agent(a) = t {
      a.blocks = kept;
    }
    if let Turn::Agent(c) = &mut copy {
      c.blocks = seen_blocks;
    }
    put(&mut self.seen, i, copy);
    self.seen.truncate(n);
    encoded.push(splice_blocks(&head, &blocks).unwrap_or_else(|| raw(&*t)));
    (encoded, Some(LastTurnParts { head, blocks }))
  }

  /// The view the last `encode` went into: what the next one reuses
  pub fn keep(&mut self, parts: &ViewParts) {
    self.parts = Some(parts.clone());
  }
}

/// `head` with its `"blocks":[]` replaced by the encoded blocks. Inside a JSON string every quote is escaped, so the marker
/// can only be a key; None when it is not unique (a nested object with an empty `blocks` key), and the caller encodes the turn
fn splice_blocks(head: &RawValue, blocks: &[Box<RawValue>]) -> Option<Box<RawValue>> {
  splice_array(head, "blocks", blocks)
}

/// `head` with its one empty `"<key>":[]` replaced by `items`; None when that key is missing or not unique
pub fn splice_array(head: &RawValue, key: &str, items: &[Box<RawValue>]) -> Option<Box<RawValue>> {
  let mark = format!("\"{key}\":[]");
  let h = head.get();
  let (before, after) = h.split_once(&mark)?;
  if after.contains(&mark) {
    return None;
  }
  let mut out = String::with_capacity(h.len() + items.iter().map(|b| b.get().len() + 1).sum::<usize>());
  out.push_str(before);
  out.push('"');
  out.push_str(key);
  out.push_str("\":[");
  for (i, b) in items.iter().enumerate() {
    if i > 0 {
      out.push(',');
    }
    out.push_str(b.get());
  }
  out.push(']');
  out.push_str(after);
  RawValue::from_string(out).ok()
}

/// The whole view of `parts`: its head with every turn in. Spliced as text; through a parse when the head's empty
/// `turns` is not the only one in it
pub fn whole_view(parts: &ViewPartsData) -> RawJson {
  let whole = splice_array(&parts.head, "turns", &parts.turns).or_else(|| {
    let mut v: serde_json::Value = serde_json::from_str(parts.head.get()).ok()?;
    let turns: Result<Vec<serde_json::Value>, _> = parts.turns.iter().map(|t| serde_json::from_str(t.get())).collect();
    v.as_object_mut()?.insert("turns".to_owned(), serde_json::Value::Array(turns.ok()?));
    serde_json::value::to_raw_value(&v).ok()
  });
  // A head that is not an object holding `turns` has no turns to show
  RawJson(Arc::from(whole.unwrap_or_else(|| parts.head.clone())))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PatchRef<'a> {
  pub(crate) id: &'a str,
  pub(crate) base: i64,
  pub(crate) view: &'a RawValue,
  pub(crate) keep: usize,
  pub(crate) turns: Vec<&'a RawValue>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(crate) keep_blocks: Option<usize>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub(crate) blocks: Option<Vec<&'a RawValue>>,
}

pub fn common_prefix(a: &[Box<RawValue>], b: &[Box<RawValue>]) -> usize {
  a.iter().zip(b).take_while(|(x, y)| x.get() == y.get()).count()
}

/// The patch that turns `prev` (what the viewer holds) into `next`; None when the whole view is no larger (another
/// session or another instance of it, or nothing in common)
pub fn session_patch(prev: &ViewPartsData, next: &ViewPartsData) -> Option<RawJson> {
  diff(prev, next).map(|patch| RawJson::new(&patch))
}

/// `session_patch` before it is encoded (`session_window.rs` moves its indices into a page's window)
pub(crate) fn diff<'a>(prev: &'a ViewPartsData, next: &'a ViewPartsData) -> Option<PatchRef<'a>> {
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
  Some(patch)
}

/// Sessions whose last sent view a page keeps (`packages/shared/sessionPatch.ts` `VIEW_CACHE_ENTRIES` keeps at least as many):
/// switching back to one of them is sent as a patch against that view instead of the whole transcript again
pub const SENT_VIEWS_MAX: usize = 12;
/// Encoded bytes the kept views may hold per page; the view on screen stays even when it alone is larger
pub const SENT_VIEWS_BUDGET: usize = 64 << 20;

/// How one view goes to a page
pub enum Delivery {
  /// Older than the view on screen in the same epoch (two flushes racing, a batch after a resync): nothing to send. A view
  /// of another epoch (the session reopened, or moved to another engine and followed from here) is never stale: its revs
  /// count on their own line, often below the old one (a follower's start at -2^40)
  Stale,
  Whole,
  Patch(RawJson),
  /// Only for a page that takes windowed views: the view from its tail on, in place of the whole one
  Window(RawJson),
}

/// One session's last view sent to a page
struct Kept {
  view: ViewParts,
  /// The encoding's size
  size: usize,
  /// Where the page's copy starts (`session_window.rs`); the whole view unless the page takes windows
  at: Earlier,
}

/// What a page holds, as far as patches go: the view on screen and the last view sent of each recent session. Kept in step
/// with the page's own cache by sending through it in order; a page that cannot apply a patch asks for the whole view
/// (`forget` then makes the next one whole)
#[derive(Default)]
pub struct SentViews {
  /// The view on screen; None after anything without parts replaced it (a page that does not patch)
  current: Option<ViewParts>,
  /// Least recently sent first
  kept: Vec<Kept>,
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
    if let Some(i) = self.kept.iter().position(|k| k.view.0.id == id) {
      self.bytes -= self.kept.remove(i).size;
    }
  }

  /// Record that `next` goes to the page and say how: a patch against the last view of that session the page was sent,
  /// or the whole view. None is a view with nothing to patch against (no parts), which replaces the one on screen
  pub fn deliver(&mut self, next: Option<&ViewParts>) -> Delivery {
    let Some(next) = next else {
      self.current = None;
      return Delivery::Whole;
    };
    if self.stale(next) {
      return Delivery::Stale;
    }
    let patch = self.kept.iter().find(|k| k.view.0.id == next.0.id).and_then(|prev| session_patch(&prev.view.0, &next.0));
    self.record(next, Earlier::default());
    patch.map_or(Delivery::Whole, Delivery::Patch)
  }

  /// `deliver` for a page that takes windowed views: a patch counted inside the window the page holds, else the view
  /// opened at its tail again (`Whole` when that is all of it)
  pub fn deliver_window(&mut self, next: &ViewParts, budget: usize) -> Delivery {
    if self.stale(next) {
      return Delivery::Stale;
    }
    let patch = self.kept.iter().find(|k| k.view.0.id == next.0.id).and_then(|prev| Some((window_patch(&prev.view.0, &next.0, prev.at, budget)?, prev.at)));
    if let Some((patch, at)) = patch {
      self.record(next, at);
      return Delivery::Patch(patch);
    }
    let at = session_window::open(&next.0, budget);
    match (!at.is_whole()).then(|| session_window::window_view(&next.0, at)).flatten() {
      Some(view) => {
        self.record(next, at);
        Delivery::Window(view)
      }
      None => {
        self.record(next, Earlier::default());
        Delivery::Whole
      }
    }
  }

  /// The part right ahead of what the page holds of `id`, as `sessionEarlier` carries it, and the page's window moved
  /// over it. None when the page holds no view of that session, or all of it
  pub fn earlier(&mut self, id: &str, budget: usize) -> Option<RawJson> {
    let kept = self.kept.iter_mut().find(|k| k.view.0.id == id)?;
    if kept.at.is_whole() {
      return None;
    }
    let chunk = session_window::earlier_chunk(&kept.view.0, kept.at, budget)?;
    let json = session_window::chunk_json(id, kept.at, &chunk);
    kept.at = chunk.earlier;
    Some(json)
  }

  /// The page was sent a view of `id` it may still hold
  pub fn holds(&self, id: &str) -> bool {
    self.kept.iter().any(|k| k.view.0.id == id)
  }

  /// Where the page's copy of `id` starts
  pub fn window_of(&self, id: &str) -> Option<Earlier> {
    self.kept.iter().find(|k| k.view.0.id == id).map(|k| k.at)
  }

  fn stale(&self, next: &ViewParts) -> bool {
    self.current.as_ref().is_some_and(|cur| cur.0.id == next.0.id && cur.0.epoch == next.0.epoch && next.0.rev < cur.0.rev)
  }

  fn record(&mut self, next: &ViewParts, at: Earlier) {
    self.forget(&next.0.id);
    let size = next.0.size();
    self.kept.push(Kept { view: next.clone(), size, at });
    self.bytes += size;
    self.current = Some(next.clone());
    // The oldest go first; the one just sent stays even alone over budget, it is what the next push patches against
    while self.kept.len() > 1 && (self.kept.len() > SENT_VIEWS_MAX || self.bytes > SENT_VIEWS_BUDGET) {
      self.bytes -= self.kept.remove(0).size;
    }
  }

  #[cfg(test)]
  fn kept_ids(&self) -> Vec<&str> {
    self.kept.iter().map(|k| k.view.0.id.as_str()).collect()
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
      Delivery::Window(_) => "window",
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
  fn the_view_on_screen_moving_to_another_epoch_is_sent_whole_even_at_a_lower_rev() {
    let mut v = SentViews::default();
    let turns = vec![user("u1"), agent(&["a"])];
    assert_eq!(kind(&v.deliver(Some(&sent("s", 1, 50, turns.clone())))), "whole");
    // another engine took the session over: this engine now shows a follower view whose revs start far below 0
    let follower = -(1_i64 << 40);
    assert_eq!(kind(&v.deliver(Some(&sent("s", 2, follower, turns.clone())))), "whole");
    // and patches from there on, within its own epoch
    assert_eq!(kind(&v.deliver(Some(&sent("s", 2, follower + 1, turns.clone())))), "patch");
    assert_eq!(kind(&v.deliver(Some(&sent("s", 2, follower, turns)))), "stale");
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
  fn a_windowed_page_gets_the_tail_then_patches_and_chunks_in_step() {
    let mut v = SentViews::default();
    let long = "x".repeat(200);
    let turns = |last: &str| vec![user("u1"), agent(&[&long]), user("u2"), agent(&[&long]), user("u3"), agent(&["a", last])];
    let d = v.deliver_window(&sent("s", 1, 1, turns("b")), 300);
    let Delivery::Window(w) = &d else { panic!("expected a window, got {}", kind(&d)) };
    let w = value(w);
    let at = w["earlier"]["turns"].as_u64().unwrap();
    assert!(at >= 3);
    // the streamed tail patches inside the window
    let d = v.deliver_window(&sent("s", 1, 2, turns("bc")), 300);
    let Delivery::Patch(p) = &d else { panic!("expected a patch, got {}", kind(&d)) };
    let p = value(p);
    assert_eq!(p["keep"].as_u64().unwrap(), 5 - at);
    assert_eq!(p["view"]["earlier"]["turns"].as_u64().unwrap(), at);
    // the page asks for more: the window moves, and the next patch counts from there
    let c = value(&v.earlier("s", 300).unwrap());
    assert_eq!(c["from"]["turns"].as_u64().unwrap(), at);
    let moved = c.get("earlier").map_or(0, |e| e["turns"].as_u64().unwrap());
    assert!(moved < at);
    let d = v.deliver_window(&sent("s", 1, 3, turns("bcd")), 300);
    let Delivery::Patch(p) = &d else { panic!("expected a patch, got {}", kind(&d)) };
    assert_eq!(value(p)["keep"].as_u64().unwrap(), 5 - moved);
    // a page that lost its copy starts at the tail again; a short view is whole, with nothing earlier to ask for
    v.forget("s");
    assert_eq!(kind(&v.deliver_window(&sent("s", 1, 4, turns("bcd")), 300)), "window");
    assert_eq!(kind(&v.deliver_window(&sent("t", 2, 1, vec![user("u")]), 300)), "whole");
    assert!(v.earlier("t", 300).is_none());
    assert!(v.earlier("unknown", 300).is_none());
  }

  /// One view through `Reuse`, checked against `encode_turns` byte for byte
  fn reused(r: &mut Reuse, rev: i64, turns: &[Turn]) {
    let fresh = parts(rev, turns.to_vec());
    let mut owned = turns.to_vec();
    let mut last = owned.pop();
    let before: Vec<&Turn> = owned.iter().collect();
    let (encoded, last_parts) = r.encode(&before, last.as_mut());
    assert_eq!(last.as_ref(), turns.last(), "the live turn gets its blocks back");
    assert_eq!(encoded.iter().map(|t| t.get()).collect::<Vec<_>>(), fresh.turns.iter().map(|t| t.get()).collect::<Vec<_>>());
    assert_eq!(last_parts.as_ref().map(|l| l.head.get()), fresh.last.as_ref().map(|l| l.head.get()));
    let blocks = |l: &Option<LastTurnParts>| l.as_ref().map(|l| l.blocks.iter().map(|b| b.get().to_owned()).collect::<Vec<_>>());
    assert_eq!(blocks(&last_parts), blocks(&fresh.last));
    assert_eq!(r.seen, turns, "the copies are the turns as encoded");
    r.keep(&ViewParts(Arc::new(ViewPartsData { id: "s".into(), epoch: 1, rev, head: fresh.head, turns: encoded, last: last_parts })));
  }

  #[test]
  fn reused_encodings_are_what_a_fresh_encoding_gives() {
    let mut r = Reuse::default();
    let mut turns = vec![user("u1"), agent(&["a", "b"])];
    reused(&mut r, 1, &turns);
    // a streamed chunk, then a new block
    turns[1] = agent(&["a", "bc"]);
    reused(&mut r, 2, &turns);
    turns[1] = agent(&["a", "bc", "d"]);
    reused(&mut r, 3, &turns);
    // the live turn ends and a prompt follows: it is now a turn ahead
    turns.push(user("u2"));
    reused(&mut r, 4, &turns);
    turns.push(agent(&["x"]));
    reused(&mut r, 5, &turns);
    // an earlier turn rewritten in place, the live one growing
    turns[1] = agent(&["a", "rewritten", "d"]);
    turns[3] = agent(&["x", "y"]);
    reused(&mut r, 6, &turns);
    // a block of the live turn changed ahead of the others, and one dropped
    turns[3] = agent(&["x2", "y"]);
    reused(&mut r, 7, &turns);
    turns[3] = agent(&["x2"]);
    reused(&mut r, 8, &turns);
    // turns dropped (a retry), then the transcript ending on a user turn, then empty
    turns.truncate(3);
    reused(&mut r, 9, &turns);
    turns.truncate(2);
    reused(&mut r, 10, &turns);
    turns[1] = agent(&[]);
    reused(&mut r, 11, &turns);
    reused(&mut r, 12, &[]);
    reused(&mut r, 13, &[agent(&["only"])]);
    // a block whose text holds the splice marker, then nothing changed at all
    reused(&mut r, 14, &[agent(&["only", "\"blocks\":[]"])]);
    reused(&mut r, 15, &[agent(&["only", "\"blocks\":[]"])]);
    // another conversation in the same place: a user turn where the live agent turn was
    reused(&mut r, 16, &[user("z1"), user("z2")]);
    reused(&mut r, 17, &[user("z1"), agent(&["q"]), user("z2")]);
  }

  #[test]
  fn an_encoding_never_kept_is_not_reused() {
    let mut r = Reuse::default();
    let turns = vec![user("u1"), agent(&["a"])];
    reused(&mut r, 1, &turns);
    // encoded but dropped: the copies moved on while no view holds their encodings
    let mut owned = vec![user("u1"), agent(&["a", "b"])];
    let mut last = owned.pop();
    r.encode(&owned.iter().collect::<Vec<_>>(), last.as_mut());
    assert!(r.parts.is_none());
    reused(&mut r, 3, &[user("u1"), agent(&["a", "b"])]);
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
