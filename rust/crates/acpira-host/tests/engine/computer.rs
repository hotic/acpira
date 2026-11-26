//! Computer use through the loopback hub, as Acpira's MCP server reaches it (`computer.rs`, docs/dev/computer-use.md):
//! off until its group is switched on, then every call goes straight to the driver with the never-list, and the
//! driver's reply is the tool's result. The driver here is a script that speaks its line protocol

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

/// Answers like the driver, and says what it was sent: no allow list ever, and the never-list every time
const DRIVER: &str = r##"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"allow"'*) printf '%s\n' '{"ok":false,"text":"an allow list came"}' ;;
    *'"deny":['*'"com.apple.keychainaccess"'*) printf '%s\n' '{"ok":true,"text":"# Notes\n","image":{"mime":"image/jpeg","data":"QUJD"}}' ;;
    *) printf '%s\n' '{"ok":false,"text":"no never-list"}' ;;
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
async fn an_app_is_driven_once_the_switch_is_on_without_a_question() {
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
  hub.set_computer(ComputerHost::new(driver.to_str().unwrap(), Arc::new(|_: &str| {})));
  let token = hub.env(&session, None, 0)[1]["value"].as_str().unwrap().to_owned();
  let addr = hub.addr().to_owned();

  // Off until the user switches the group on, whatever the caller asks
  let refused = op(&addr, &token, observe("Notes")).await;
  assert!(matches!(&refused, HubReply::Error(e) if e.contains("switched off")), "{refused:?}");
  plugins.action(PluginAction::ToolGroup { id: "computer".into(), on: true }).await.unwrap();

  // Then the call is answered at once: the caller's own allow list and op are dropped, the never-list goes along
  let answered = op(&addr, &token, observe("Notes")).await;
  let HubReply::Tool(result) = answered else { panic!("no tool result: {answered:?}") };
  assert_eq!(result["isError"], false, "{result}");
  assert_eq!(result["content"], json!([{ "type": "text", "text": "# Notes\n" }, { "type": "image", "data": "QUJD", "mimeType": "image/jpeg" }]));
  assert!(questions(&m.view_of(&id).unwrap()).is_empty());
}
