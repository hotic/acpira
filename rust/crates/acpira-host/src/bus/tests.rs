use super::*;
use acpira_shared::session_patch::{encode_turns, raw};
use acpira_shared::transcript::Turn;
use serde_json::json;

fn turn(v: Value) -> Turn {
  serde_json::from_value(v).unwrap()
}

fn user(id: &str) -> Turn {
  turn(json!({ "role": "user", "id": id, "text": id }))
}

fn agent(texts: &[&str]) -> Turn {
  turn(json!({ "role": "agent", "blocks": texts.iter().map(|t| json!({ "type": "text", "markdown": t })).collect::<Vec<_>>() }))
}

fn parts(rev: i64, mut turns: Vec<Turn>) -> ViewPartsData {
  let mut last = turns.pop();
  let before: Vec<&Turn> = turns.iter().collect();
  let (turns, last) = encode_turns(&before, last.as_mut());
  ViewPartsData { id: "s".into(), epoch: 1, rev, head: raw(&json!({ "id": "s", "rev": rev, "turns": [] })), turns, last }
}

// What the receiver rebuilds from a frame against what it held, as plain JSON per turn
fn rebuilt(prev: Option<&ViewPartsData>, f: &ViewFrame) -> Vec<Value> {
  let (turns, _) = apply_frame(prev, f).expect("applies");
  turns.iter().map(|t| serde_json::from_str(t.get()).unwrap()).collect()
}

fn plain(p: &ViewPartsData) -> Vec<Value> {
  p.turns.iter().map(|t| serde_json::from_str(t.get()).unwrap()).collect()
}

#[test]
fn a_streamed_chunk_sends_only_the_changed_blocks_and_rebuilds_the_same_view() {
  let a = parts(1, vec![user("u1"), agent(&["a", "b"])]);
  let b = parts(2, vec![user("u1"), agent(&["a", "b", "c"])]);
  let whole = view_frame(None, &a);
  assert_eq!((whole.keep, whole.turns.len()), (0, 1), "the user turn whole, the live turn as parts");
  let (turns, last) = apply_frame(None, &whole).unwrap();
  let held = ViewPartsData { turns, last, ..parts(1, vec![]) };
  assert_eq!(plain(&held), plain(&a));
  let step = view_frame(Some(&a), &b);
  assert_eq!(step.keep, 1);
  assert!(step.turns.is_empty());
  let l = step.last.as_ref().unwrap();
  assert_eq!((l.keep_blocks, l.blocks.len()), (2, 1));
  assert_eq!(rebuilt(Some(&held), &step), plain(&b));
}

#[test]
fn a_new_turn_after_the_live_one_keeps_the_finished_turn_and_sends_the_rest() {
  let a = parts(1, vec![user("u1"), agent(&["a"])]);
  let b = parts(2, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["x"])]);
  let step = view_frame(Some(&a), &b);
  assert_eq!(step.keep, 2);
  assert_eq!(step.turns.len(), 1);
  assert_eq!(step.last.as_ref().unwrap().keep_blocks, 0);
  assert_eq!(rebuilt(Some(&a), &step), plain(&b));
}

#[test]
fn a_rewind_and_a_frame_beyond_what_is_held_are_handled() {
  let a = parts(1, vec![user("u1"), agent(&["a"]), user("u2"), agent(&["b"])]);
  let b = parts(2, vec![user("u1"), agent(&["a"])]);
  let step = view_frame(Some(&a), &b);
  assert_eq!(rebuilt(Some(&a), &step), plain(&b));
  // Nothing held: a step that keeps turns cannot apply, the receiver asks for the whole view
  assert!(apply_frame(None, &step).is_none());
  // Another epoch is another instance of the session: the whole view
  let other = ViewPartsData { epoch: 2, ..parts(3, vec![user("u1")]) };
  assert_eq!(view_frame(Some(&a), &other).keep, 0);
}

#[test]
fn a_head_with_a_nested_empty_key_still_splices() {
  let head = raw(&json!({ "role": "agent", "blocks": [], "meta": { "blocks": [] } }));
  let out = splice(&head, "blocks", &[raw(&json!({ "type": "text", "markdown": "x" }))]).unwrap();
  let v: Value = serde_json::from_str(out.get()).unwrap();
  assert_eq!(v["blocks"][0]["markdown"], "x");
  assert_eq!(v["meta"]["blocks"], json!([]));
}

#[test]
fn frames_skip_unknown_fields_and_kinds() {
  let f: Frame = serde_json::from_str(r#"{"t":"later","session":"s","novel":1}"#).unwrap();
  assert_eq!(f.t, "later");
  let mut r = Router::default();
  r.handle(1, f, Arc::from("{}"));
}

#[cfg(unix)]
#[test]
fn long_homes_get_a_short_socket_path() {
  let (s, l) = paths(Path::new("/h"));
  assert_eq!(s, Path::new("/h/run/bus.sock"));
  assert_eq!(l, Path::new("/h/run/bus.lock"));
  let deep = PathBuf::from("/").join("x".repeat(120));
  let (s, _) = paths(&deep);
  assert!(s.as_os_str().len() <= SOCKET_PATH_MAX + 40 && s.starts_with(std::env::temp_dir()));
}

#[cfg(windows)]
#[test]
fn the_bus_is_a_pipe_named_after_the_home() {
  let (s, l) = paths(Path::new(r"C:\h"));
  assert!(s.to_string_lossy().starts_with(r"\\.\pipe\acpira-bus-"), "{}", s.display());
  assert_ne!(s, paths(Path::new(r"C:\other")).0);
  assert_eq!(l, Path::new(r"C:\h\run\bus.lock"));
}

#[test]
fn the_router_routes_watches_views_actions_and_departures() {
  let mut r = Router::default();
  let mut rx = HashMap::new();
  for id in [1u64, 2, 3] {
    let (tx, rx1) = mpsc::unbounded_channel();
    r.join(id, tx);
    rx.insert(id, rx1);
  }
  let mut take = |id: u64| -> Vec<Value> {
    let mut out = vec![];
    while let Ok(l) = rx.get_mut(&id).unwrap().try_recv() {
      out.push(serde_json::from_str(&l).unwrap());
    }
    out
  };
  let frame = |v: Value| -> (Frame, Arc<str>) {
    let s = v.to_string();
    (serde_json::from_str(&s).unwrap(), Arc::from(s))
  };
  // 2 subscribes before anyone owns the session; 1 then announces it and is asked to stream
  let (f, l) = frame(json!({ "t": "sub", "session": "s" }));
  r.handle(2, f, l);
  let (f, l) = frame(json!({ "t": "own", "sessions": ["s"] }));
  r.handle(1, f, l);
  assert_eq!(take(1), vec![json!({ "t": "watch", "session": "s", "on": true })]);
  // 3 joins: another whole view is asked for
  let (f, l) = frame(json!({ "t": "sub", "session": "s" }));
  r.handle(3, f, l);
  assert_eq!(take(1).len(), 1);
  // A view goes to both subscribers as sent
  let (f, l) = frame(json!({ "t": "view", "session": "s", "view": { "head": {}, "keep": 0, "turns": [] } }));
  r.handle(1, f, l);
  assert_eq!(take(2).len(), 1);
  assert_eq!(take(3).len(), 1);
  // An action reaches the owner with its sender; the answer goes back to that sender only
  let (f, l) = frame(json!({ "t": "act", "session": "s", "id": 7, "msg": { "type": "stop" } }));
  r.handle(3, f, l);
  let got = take(1);
  assert_eq!(got[0]["peer"], 3);
  let (f, l) = frame(json!({ "t": "done", "id": 7, "peer": 3 }));
  r.handle(1, f, l);
  assert_eq!(take(3), vec![json!({ "t": "done", "id": 7 })]);
  assert!(take(2).is_empty());
  // Nobody owns another session: an action on it is answered with an error at once
  let (f, l) = frame(json!({ "t": "act", "session": "x", "id": 8 }));
  r.handle(2, f, l);
  assert!(take(2)[0]["error"].is_string());
  // 2 and 3 leave the session: the owner stops streaming once the last one has gone
  let (f, l) = frame(json!({ "t": "unsub", "session": "s" }));
  r.handle(2, f, l);
  assert!(take(1).is_empty());
  r.leave(3);
  assert_eq!(take(1), vec![json!({ "t": "watch", "session": "s", "on": false })]);
  // The owner leaves while 2 subscribes again: 2 hears the session is gone
  let (f, l) = frame(json!({ "t": "sub", "session": "s" }));
  r.handle(2, f, l);
  take(1);
  r.leave(1);
  assert_eq!(take(2), vec![json!({ "t": "gone", "session": "s" })]);
}

fn take(rx: &mut mpsc::UnboundedReceiver<Arc<str>>) -> Vec<Value> {
  let mut out = vec![];
  while let Ok(l) = rx.try_recv() {
    out.push(serde_json::from_str(&l).unwrap());
  }
  out
}

fn send(r: &mut Router, from: u64, v: Value) {
  let line = v.to_string();
  r.handle(from, serde_json::from_str(&line).unwrap(), Arc::from(line));
}

// List entries (title, state …) reach every other engine stamped with their sender, a late joiner hears them at once,
// and an engine leaving clears its entries everywhere. A `touch` goes to everyone else, once
#[test]
fn list_entries_are_relayed_replayed_and_cleared_and_touches_broadcast() {
  let mut r = Router::default();
  let (tx1, mut rx1) = mpsc::unbounded_channel();
  let (tx2, mut rx2) = mpsc::unbounded_channel();
  r.join(1, tx1);
  r.join(2, tx2);
  let entry = json!({ "id": "s", "title": "t", "state": "working" });
  send(&mut r, 1, json!({ "t": "live", "summaries": [entry] }));
  assert!(take(&mut rx1).is_empty(), "not echoed to its sender");
  assert_eq!(take(&mut rx2), vec![json!({ "t": "live", "peer": 1, "summaries": [entry] })]);
  // A third engine joining later gets the entries already announced
  let (tx3, mut rx3) = mpsc::unbounded_channel();
  r.join(3, tx3);
  assert_eq!(take(&mut rx3), vec![json!({ "t": "live", "peer": 1, "summaries": [entry] })]);
  send(&mut r, 2, json!({ "t": "touch", "what": "unread" }));
  assert!(take(&mut rx2).is_empty());
  assert_eq!(take(&mut rx1), vec![json!({ "t": "touch", "what": "unread" })]);
  assert_eq!(take(&mut rx3), vec![json!({ "t": "touch", "what": "unread" })]);
  // The sender leaves: everyone hears its entries are gone
  r.leave(1);
  assert_eq!(take(&mut rx2), vec![json!({ "t": "live", "peer": 1 })]);
  assert_eq!(take(&mut rx3), vec![json!({ "t": "live", "peer": 1 })]);
}

// Observing a subagent of a session another engine owns: the owner is asked for it by every new observer, its
// transcripts reach the observers of that subagent only, it is told when the last observer goes (unsubscribed or gone),
// and a session that changes hands is asked of its new owner
#[test]
fn subagent_observers_are_routed_to_the_owner_and_back() {
  let mut r = Router::default();
  let (tx1, mut rx1) = mpsc::unbounded_channel();
  let (tx2, mut rx2) = mpsc::unbounded_channel();
  let (tx3, mut rx3) = mpsc::unbounded_channel();
  r.join(1, tx1);
  r.join(2, tx2);
  r.join(3, tx3);
  send(&mut r, 1, json!({ "t": "own", "sessions": ["s"] }));
  send(&mut r, 2, json!({ "t": "nsub", "session": "s", "node": "c1" }));
  assert_eq!(take(&mut rx1), vec![json!({ "t": "nwatch", "session": "s", "node": "c1", "on": true })]);
  send(&mut r, 3, json!({ "t": "nsub", "session": "s", "node": "c2" }));
  take(&mut rx1);
  let view = json!({ "t": "nview", "session": "s", "node": "c1", "rev": 3, "running": true, "turns": [] });
  send(&mut r, 1, view.clone());
  assert_eq!(take(&mut rx2), vec![view]);
  assert!(take(&mut rx3).is_empty(), "another subagent's observer");
  // The last observer of c1 goes: the owner stops; 3 leaving ends c2 the same way
  send(&mut r, 2, json!({ "t": "nunsub", "session": "s", "node": "c1" }));
  assert_eq!(take(&mut rx1), vec![json!({ "t": "nwatch", "session": "s", "node": "c1", "on": false })]);
  send(&mut r, 2, json!({ "t": "nsub", "session": "s", "node": "c1" }));
  take(&mut rx1);
  r.leave(3);
  assert_eq!(take(&mut rx1), vec![json!({ "t": "nwatch", "session": "s", "node": "c2", "on": false })]);
  // The owner leaves and another engine opens the session: it is asked for what 2 still observes
  r.leave(1);
  let (tx4, mut rx4) = mpsc::unbounded_channel();
  r.join(4, tx4);
  send(&mut r, 4, json!({ "t": "own", "sessions": ["s"] }));
  assert!(take(&mut rx4).contains(&json!({ "t": "nwatch", "session": "s", "node": "c1", "on": true })));
}
