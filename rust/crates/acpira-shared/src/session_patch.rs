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
/// session, or nothing in common)
pub fn session_patch(prev: &ViewPartsData, next: &ViewPartsData) -> Option<RawJson> {
  if prev.id != next.id {
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
    ViewPartsData { id: "s".into(), rev, head: raw(&json!({ "id": "s", "rev": rev, "turns": [] })), turns, last }
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

  #[test]
  fn another_session_or_nothing_in_common_is_not_patched() {
    let prev = parts(1, vec![user("u1")]);
    let mut other = parts(2, vec![user("u1")]);
    other.id = "t".into();
    assert!(session_patch(&prev, &other).is_none());
    assert!(session_patch(&prev, &parts(2, vec![user("u9")])).is_none());
    // an unchanged transcript still patches (controls, usage …), with no turns at all
    let p = value(&session_patch(&prev, &parts(2, vec![user("u1")])).unwrap());
    assert_eq!((p["keep"].clone(), p["turns"].clone()), (json!(1), json!([])));
  }
}
