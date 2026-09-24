//! test/msgBatch.test.ts, the batching half of test/subagents.test.ts and the SettingsCenter half of test/settings.test.ts

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::acp::agent_registry::AgentRegistry;
use acpira_host::bridge_core::{MsgBatch, Pushed};
use acpira_host::settings::{SettingsCenter, SettingsDeps};
use acpira_shared::protocol::{HostMsg, RawJson};

use crate::support::v;

fn session(running: bool, rev: i64, text: &str) -> HostMsg {
  let view = json!({ "id": "s", "agent": "kimi", "title": "t", "cwd": "/w", "status": "ready", "running": running, "rev": rev,
    "createdAt": "a", "updatedAt": "b", "commands": [], "controls": { "modes": [], "options": [] },
    "turns": [{ "role": "user", "id": "u1", "text": "hi" }, { "role": "agent", "blocks": [{ "type": "text", "markdown": text }] }] });
  HostMsg::Session { session: RawJson::new(&view), running }
}

fn subagent(session: &str, id: &str, rev: i64) -> HostMsg {
  HostMsg::Subagent { session_id: session.into(), subagent_id: id.into(), rev, running: true, turns: RawJson::new(&json!([])) }
}

fn posted(p: Pushed) -> Vec<Value> {
  match p {
    Pushed::Flush(ms) => ms.iter().map(v).collect(),
    _ => vec![],
  }
}

#[test]
fn only_the_latest_session_view_survives_inside_the_window() {
  let mut b = MsgBatch::default();
  assert!(matches!(b.push(session(true, 1, "a")), Pushed::Arm));
  assert!(matches!(b.push(session(true, 2, "ab")), Pushed::Wait));
  let out: Vec<Value> = b.flush().iter().map(v).collect();
  assert_eq!(out.len(), 1);
  assert_eq!(out[0]["session"]["rev"], 2);
  assert_eq!(out[0]["session"]["running"], true);
}

#[test]
fn an_idle_session_flushes_at_once_and_replaces_the_coalesced_running_view() {
  let mut b = MsgBatch::default();
  b.push(session(true, 3, "hi"));
  let out = posted(b.push(session(false, 4, "hi")));
  assert_eq!(out.len(), 1);
  assert_eq!((out[0]["session"]["rev"].clone(), out[0]["session"]["running"].clone()), (json!(4), json!(false)));
  // The window timer that was armed finds nothing left
  assert!(b.flush().is_empty());
}

#[test]
fn a_pending_sessions_list_goes_out_with_the_idle_edge() {
  let mut b = MsgBatch::default();
  b.push(HostMsg::Sessions { sessions: vec![] });
  let out = posted(b.push(session(false, 2, "hi")));
  assert_eq!(out.iter().map(|m| m["type"].clone()).collect::<Vec<_>>(), [json!("sessions"), json!("session")]);
}

#[test]
fn subagent_messages_coalesce_per_session_and_subagent_not_per_type() {
  let mut b = MsgBatch::default();
  b.push(subagent("s", "a", 1));
  b.push(subagent("s", "b", 1));
  b.push(subagent("s", "a", 2));
  b.push(subagent("other", "a", 1));
  let mut keys: Vec<String> = b.flush().iter().map(v).map(|m| format!("{}:{}:{}", m["sessionId"].as_str().unwrap(), m["subagentId"].as_str().unwrap(), m["rev"])).collect();
  keys.sort();
  assert_eq!(keys, ["other:a:1", "s:a:2", "s:b:1"]);
}

type Store = Arc<Mutex<HashMap<String, Value>>>;

fn center(store: Store) -> SettingsCenter {
  let (r, w) = (store.clone(), store);
  SettingsCenter::new(SettingsDeps {
    read: Arc::new(move |k| r.lock().unwrap().get(k).cloned()),
    write: Arc::new(move |k, x| {
      w.lock().unwrap().insert(k, x);
      Box::pin(async { Ok(()) })
    }),
    host_language: Arc::new(|| "en".into()),
    registry: Arc::new(|| Arc::new(AgentRegistry::new(&json!({})))),
    runtime_info: Arc::new(|_| None),
    health: Arc::new(|_| None),
    home: Arc::new(|| "/home".into()),
    cwd: Arc::new(|| "/cwd".into()),
  })
}

#[tokio::test]
async fn session_list_placement_round_trips_and_unknown_positions_are_rejected() {
  let store: Store = Default::default();
  let c = center(store.clone());
  let positions = Arc::new(Mutex::new(Vec::<Value>::new()));
  let p = positions.clone();
  c.subscribe(Arc::new(move |view, _| p.lock().unwrap().push(v(view)["sessionListPosition"].clone())));
  for value in ["left", "right", "hidden", "outside"] {
    c.set("sessionListPosition", &json!(value)).await.ok();
  }
  assert_eq!(*positions.lock().unwrap(), [json!("left"), json!("right"), json!("hidden"), json!("hidden")]);
  c.set("sessionListPosition", &json!("right")).await.unwrap();
  assert_eq!(v(center(store).view())["sessionListPosition"], "right");
}

#[test]
fn the_view_carries_appearance_defaults_when_nothing_is_configured() {
  let view = v(center(Default::default()).view());
  assert_eq!(view["theme"], "auto");
  assert_eq!(view["diffMarkers"], "color");
  assert_eq!(view["fontSmoothing"], false);
  assert!(view["uiFontSize"].is_number() && view["codeFontSize"].is_number());
}

#[tokio::test]
async fn set_appearance_writes_only_values_the_axis_declares() {
  let store: Store = Default::default();
  let c = center(store.clone());
  c.set_appearance("motion", "none").await.ok();
  c.set_appearance("motion", "hyper").await.ok();
  c.set_appearance("nonsense", "none").await.ok();
  let written: Vec<(String, Value)> = store.lock().unwrap().iter().map(|(k, x)| (k.clone(), x.clone())).collect();
  assert_eq!(written, [("appearance.motion".to_owned(), json!("none"))]);
}
