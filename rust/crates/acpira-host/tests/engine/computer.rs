//! Computer use through the loopback hub, as Acpira's MCP server reaches it (`computer.rs`, docs/dev/computer-use.md):
//! off until its group is switched on, one question per app in the calling session, and the driver's reply as the
//! tool's result. The driver here is a script that speaks its line protocol

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use serde_json::{Value, json};

use acpira_host::computer::ComputerHost;
use acpira_host::plugins::host::PluginHost;
use acpira_host::relay::hub::{RelayHub, call};
use acpira_host::relay::roster::Roster;
use acpira_host::relay::wire::{HubOp, HubReply, HubRequest};
use acpira_shared::plugins::PluginAction;

use crate::fake_or_skip;
use crate::session_manager::{Mgr, Opts};
use crate::support::until;

/// Answers like the driver: an app outside the request's allow list needs approval, an allowed one is read
const DRIVER: &str = r##"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"app":"Keys"'*) printf '%s\n' '{"ok":false,"text":"needs approval","approval":{"name":"Keys","identity":"com.apple.keychainaccess","path":"/Applications/Keys.app"}}' ;;
    *'"app":"Mail"'*) printf '%s\n' '{"ok":false,"text":"needs approval","approval":{"name":"Mail","identity":"com.example.mail","path":"/Applications/Mail.app"}}' ;;
    *'"allow":['*'"com.example.notes"'*) printf '%s\n' '{"ok":true,"text":"# Notes\n","image":{"mime":"image/jpeg","data":"QUJD"}}' ;;
    *) printf '%s\n' '{"ok":false,"text":"needs approval","approval":{"name":"Notes","identity":"com.example.notes","path":"/Applications/Notes.app"}}' ;;
  esac
done
"##;

async fn op(addr: &str, token: &str, op: HubOp) -> HubReply {
  let (addr, req) = (addr.to_owned(), HubRequest { token: token.into(), op });
  tokio::task::spawn_blocking(move || {
    let mut last = None;
    let _ = call(&addr, &req, |r| {
      last = Some(r);
      false
    });
    last.expect("the hub answers")
  })
  .await
  .unwrap()
}

fn observe(app: &str) -> HubOp {
  HubOp::Computer { name: "computer_observe".into(), arguments: json!({ "app": app, "allow": ["com.example.mail"], "op": "act" }) }
}

fn questions(view: &Value) -> Vec<Value> {
  view["turns"].as_array().into_iter().flatten().flat_map(|t| t["blocks"].as_array().into_iter().flatten()).filter(|b| b["type"] == "question").cloned().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_is_driven_only_after_the_switch_and_the_user_s_yes() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hello" })).await;
  let id = m.active_id().unwrap();
  let session = m.m.live_session(&id).expect("open");

  let driver = dir.path().join("driver.sh");
  std::fs::write(&driver, DRIVER).unwrap();
  std::fs::set_permissions(&driver, std::fs::Permissions::from_mode(0o755)).unwrap();
  let plugins = PluginHost::new(dir.path().join("plugins"), false, Arc::new(|_: &str| {}));
  plugins.start().await;
  let hub = RelayHub::start(Arc::new(Roster::new(dir.path())), Arc::new(|_: &str| {})).await.unwrap();
  hub.set_plugins(Arc::downgrade(&plugins));
  hub.set_computer(ComputerHost::new(driver.to_str().unwrap(), dir.path().to_path_buf(), Arc::new(|_: &str| {})));
  let token = hub.env(&session, None, 0)[1]["value"].as_str().unwrap().to_owned();
  let addr = hub.addr().to_owned();

  // Off until the user switches the group on, whatever the caller asks
  let refused = op(&addr, &token, observe("Notes")).await;
  assert!(matches!(&refused, HubReply::Error(e) if e.contains("switched off")), "{refused:?}");
  plugins.action(PluginAction::ToolGroup { id: "computer".into(), on: true }).await.unwrap();

  // The first call about an app waits on a card in the calling session; the allow list the caller sent is dropped
  let call = { let (addr, token) = (addr.clone(), token.clone()); tokio::spawn(async move { op(&addr, &token, observe("Notes")).await }) };
  until(|| m.view_of(&id).is_some_and(|v| questions(&v).iter().any(|q| q["outcome"].is_null())), 10_000).await;
  let card = questions(&m.view_of(&id).unwrap()).pop().unwrap();
  assert!(card.to_string().contains("Let the agent see and control Notes?"), "{card}");
  m.handle(json!({ "type": "answer", "sessionId": id, "blockId": card["id"], "answers": { "access": "always" } })).await;
  let answered = call.await.unwrap();
  let HubReply::Tool(result) = answered else { panic!("no tool result: {answered:?}") };
  assert_eq!(result["isError"], false, "{result}");
  assert_eq!(result["content"], json!([{ "type": "text", "text": "# Notes\n" }, { "type": "image", "data": "QUJD", "mimeType": "image/jpeg" }]));
  // Allowed for good: kept in the data home, and the next call asks nobody
  let kept: Value = serde_json::from_slice(&std::fs::read(dir.path().join("computer.json")).unwrap()).unwrap();
  assert_eq!(kept["allowed"], json!([{ "id": "com.example.notes", "name": "Notes" }]));
  assert!(matches!(op(&addr, &token, observe("Notes")).await, HubReply::Tool(_)));
  assert_eq!(questions(&m.view_of(&id).unwrap()).len(), 1);

  // A refusal is told to the agent, and asking again at once puts no second card up
  let call = { let (addr, token) = (addr.clone(), token.clone()); tokio::spawn(async move { op(&addr, &token, observe("Mail")).await }) };
  until(|| m.view_of(&id).is_some_and(|v| questions(&v).len() == 2), 10_000).await;
  let card = questions(&m.view_of(&id).unwrap()).pop().unwrap();
  m.handle(json!({ "type": "answer", "sessionId": id, "blockId": card["id"], "answers": { "access": "deny" } })).await;
  let denied = call.await.unwrap();
  assert!(matches!(&denied, HubReply::Error(e) if e.contains("did not allow computer use of Mail")), "{denied:?}");
  assert!(matches!(op(&addr, &token, observe("Mail")).await, HubReply::Error(_)));
  assert_eq!(questions(&m.view_of(&id).unwrap()).len(), 2);

  // A password store is never offered to the user at all
  let never = op(&addr, &token, observe("Keys")).await;
  assert!(matches!(&never, HubReply::Error(e) if e.contains("cannot be used through computer use")), "{never:?}");
  assert_eq!(questions(&m.view_of(&id).unwrap()).len(), 2);
}
