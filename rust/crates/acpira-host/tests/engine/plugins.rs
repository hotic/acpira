//! User plugins (`plugins/`, docs/dev/plugins.md) through the loopback hub, as Acpira's MCP server reaches them: the
//! script tools a session's agent lists and calls, the tool groups switched off, and `plugin_activate`'s card in the
//! calling session, answered in time and late

use std::sync::Arc;

use serde_json::{Value, json};

use acpira_host::plugins::host::PluginHost;
use acpira_host::relay::hub::{RelayHub, call};
use acpira_host::relay::roster::Roster;
use acpira_host::relay::wire::{HubOp, HubReply, HubRequest};
use acpira_shared::plugins::PluginAction;

use crate::fake_or_skip;
use crate::session_manager::{Mgr, Opts};
use crate::support::until;

/// Every reply line of one hub call
async fn op(addr: &str, token: &str, op: HubOp) -> Vec<HubReply> {
  let (addr, req) = (addr.to_owned(), HubRequest { token: token.into(), op });
  tokio::task::spawn_blocking(move || {
    let mut lines = vec![];
    let _ = call(&addr, &req, |r| {
      let more = matches!(r, HubReply::Progress(_));
      lines.push(r);
      more
    });
    lines
  })
  .await
  .unwrap()
}

fn done(lines: &[HubReply]) -> Result<String, String> {
  match lines.last() {
    Some(HubReply::Done(t)) => Ok(t.clone()),
    Some(HubReply::Error(t)) => Err(t.clone()),
    other => panic!("no final line: {other:?}"),
  }
}

fn write_plugin(root: &std::path::Path, id: &str, script: &str) {
  let dir = root.join(id);
  std::fs::create_dir_all(&dir).unwrap();
  let m = json!({ "schema": 1, "id": id, "version": "0.1.0", "description": "Demo",
    "contributes": { "tools": [{ "name": "where", "description": "Where the call runs", "command": ["sh", "go.sh"], "readOnly": true }] } });
  std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  std::fs::write(dir.join("go.sh"), script).unwrap();
}

/// The open question card of a session's view
fn card(view: &Value) -> Option<Value> {
  view["turns"].as_array()?.iter().rev().flat_map(|t| t["blocks"].as_array().into_iter().flatten()).find(|b| b["type"] == "question" && b["outcome"].is_null()).cloned()
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_session_lists_and_calls_plugin_tools_and_its_agent_asks_to_enable_one() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let project = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake).cwd(project.path().to_str().unwrap()));
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "send", "text": "hello" })).await;
  let id = m.active_id().unwrap();
  let session = m.m.live_session(&id).expect("open");

  let plugins_root = dir.path().join("plugins");
  let plugins = PluginHost::new(plugins_root.clone(), false, Arc::new(|_: &str| {}));
  plugins.start().await;
  let hub = RelayHub::start(Arc::new(Roster::new(dir.path())), Arc::new(|_: &str| {})).await.unwrap();
  hub.set_plugins(Arc::downgrade(&plugins));
  let token = hub.env(&session, None, 0)[1]["value"].as_str().unwrap().to_owned();
  let addr = hub.addr().to_owned();

  // Nothing enabled: nothing listed, nothing callable
  let listed = op(&addr, &token, HubOp::Plugins).await;
  assert!(matches!(&listed[..], [HubReply::Plugins { tools, off }] if tools.is_empty() && off.is_empty()), "{listed:?}");

  // Enabled on the page: listed with its served name, run in its frozen copy with the session's folder
  write_plugin(&plugins_root, "demo", "read call; echo \"ws=$ACPIRA_WORKSPACE\"; echo \"$call\"");
  plugins.action(PluginAction::Enable { id: "demo".into() }).await.unwrap();
  plugins.action(PluginAction::ToolGroup { id: "browser".into(), on: false }).await.unwrap();
  let listed = op(&addr, &token, HubOp::Plugins).await;
  let [HubReply::Plugins { tools, off }] = &listed[..] else { panic!("{listed:?}") };
  assert_eq!((tools[0]["name"].as_str(), off.as_slice()), (Some("demo_where"), &["browser".to_owned()][..]));
  let out = done(&op(&addr, &token, HubOp::PluginCall { name: "demo_where".into(), arguments: json!({ "x": 1 }) }).await).unwrap();
  assert!(out.contains(&format!("ws={}", project.path().display())), "{out}");
  assert!(out.contains(&format!("\"session\":\"{id}\"")) && out.contains("\"arguments\":{\"x\":1}"), "{out}");

  // The agent asks to enable its new version: a card in its session, answered in time
  std::fs::write(plugins_root.join("demo/go.sh"), "echo v2").unwrap();
  let waiting = {
    let (addr, token) = (addr.clone(), token.clone());
    tokio::spawn(async move { op(&addr, &token, HubOp::PluginActivate { id: "demo".into(), wait_secs: Some(30) }).await })
  };
  until(|| m.view_of(&id).and_then(|v| card(&v)).is_some(), 5000).await;
  let shown = card(&m.view_of(&id).unwrap()).unwrap();
  let text = shown.to_string();
  assert!(text.contains("demo_where") && text.contains("go.sh"), "{text}");
  m.handle(json!({ "type": "answer", "sessionId": id, "blockId": shown["id"], "answers": { "plugin": "enable" } })).await;
  let answer = done(&waiting.await.unwrap()).unwrap();
  assert!(answer.contains("The user enabled demo") && answer.contains("demo_where"), "{answer}");
  assert_eq!(done(&op(&addr, &token, HubOp::PluginCall { name: "demo_where".into(), arguments: json!({}) }).await).unwrap().trim(), "v2");
  let state = plugins.store().load().unwrap();
  assert_eq!((state.plugins["demo"].provenance.by.as_str(), state.plugins["demo"].provenance.session.as_deref()), ("session", Some(id.as_str())));

  // A broken plugin is never offered: the error comes back for the agent to fix
  std::fs::write(plugins_root.join("demo/plugin.json"), json!({ "schema": 1, "id": "demo", "contributes": {} }).to_string()).unwrap();
  let err = done(&op(&addr, &token, HubOp::PluginActivate { id: "demo".into(), wait_secs: Some(30) }).await).unwrap_err();
  assert!(err.contains("contributes nothing") && card(&m.view_of(&id).unwrap()).is_none(), "{err}");

  // Answered after the call gave up: still enabled, and the agent is told in a message of its own
  write_plugin(&plugins_root, "late", "echo late");
  let gave_up = done(&op(&addr, &token, HubOp::PluginActivate { id: "late".into(), wait_secs: Some(1) }).await).unwrap();
  assert!(gave_up.contains("still open"), "{gave_up}");
  let shown = card(&m.view_of(&id).unwrap()).expect("the card stays");
  m.handle(json!({ "type": "answer", "sessionId": id, "blockId": shown["id"], "answers": { "plugin": "enable" } })).await;
  until(|| m.view_of(&id).is_some_and(|v| v.to_string().contains("[Acpira] The user enabled late.")), 10_000).await;
  assert!(plugins.loaded().tool("late_where").is_some());

  // Switched off: the next call is refused although the client may still list it
  plugins.action(PluginAction::Disable { id: "late".into() }).await.unwrap();
  assert!(done(&op(&addr, &token, HubOp::PluginCall { name: "late_where".into(), arguments: json!({}) }).await).is_err());
  m.dispose().await;
}

/// A stdio MCP server in node: answers initialize and tools/list, or exits at once when `broken`
fn write_mcp_plugin(root: &std::path::Path, id: &str, broken: bool) {
  let dir = root.join(id);
  std::fs::create_dir_all(&dir).unwrap();
  let m = json!({ "schema": 1, "id": id, "version": "0.1.0", "description": "Rates",
    "contributes": { "mcp": { "rates": { "command": "node", "args": ["${ACPIRA_PLUGIN_DIR}/srv.mjs"], "env": { "MODE": "x" } } } } });
  std::fs::write(dir.join("plugin.json"), m.to_string()).unwrap();
  let server = if broken {
    "process.exit(3);".to_owned()
  } else {
    r#"import { createInterface } from 'node:readline';
createInterface({ input: process.stdin }).on('line', (l) => {
  const m = JSON.parse(l);
  if (m.id === undefined) return;
  const result = m.method === 'initialize'
    ? { protocolVersion: '2025-06-18', capabilities: { tools: {} }, serverInfo: { name: 'rates', version: '1' } }
    : { tools: [{ name: 'rate', description: 'A rate', inputSchema: { type: 'object' } }] };
  process.stdout.write(JSON.stringify({ jsonrpc: '2.0', id: m.id, result }) + '\n');
});"#
      .to_owned()
  };
  std::fs::write(dir.join("srv.mjs"), server).unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_s_mcp_server_is_checked_and_reaches_local_sessions_only() {
  let _fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let root = dir.path().join("plugins");
  let plugins = PluginHost::new(root.clone(), false, Arc::new(|_: &str| {}));
  plugins.start().await;

  // A server that does not answer is refused on the dynamic check
  write_mcp_plugin(&root, "broken", true);
  let err = plugins.action(PluginAction::Enable { id: "broken".into() }).await.unwrap_err().to_string();
  assert!(err.contains("the check failed") && err.contains("mcp `rates`"), "{err}");
  assert!(plugins.mcp_servers().is_empty());

  // One that answers is enabled and runs from its frozen copy
  write_mcp_plugin(&root, "rates", false);
  plugins.action(PluginAction::Enable { id: "rates".into() }).await.unwrap();
  let servers = plugins.mcp_servers();
  assert_eq!(servers.len(), 1);
  let frozen = plugins.loaded().active[0].dir.clone();
  assert_eq!(servers[0].spec["args"], json!([frozen.join("srv.mjs").to_string_lossy()]));

  // A new local session gets it after the shared servers; a session on an SSH host does not
  let home = dir.path().join("home");
  std::fs::create_dir_all(&home).unwrap();
  let home_s = home.to_string_lossy().into_owned();
  let p = plugins.clone();
  let provider = acpira_host::shared_config::mcp_provider(Arc::new(move || home_s.clone()), Some(Arc::new(move || p.mcp_servers())));
  let cwd = home.to_string_lossy().into_owned();
  let (sent, _) = provider("grok".into(), cwd.clone(), None, true).await;
  assert_eq!(sent.iter().map(|s| s["name"].as_str().unwrap()).collect::<Vec<_>>(), ["rates"]);
  assert!(sent[0]["env"].as_array().unwrap().iter().any(|e| e["name"] == "ACPIRA_PLUGIN_DATA"));
  assert!(provider("grok".into(), cwd.clone(), None, false).await.0.is_empty());

  // A shared entry of the same name shadows it; safe mode takes it away
  std::fs::create_dir_all(home.join(".agents")).unwrap();
  std::fs::write(home.join(".agents/mcp.json"), r#"{ "mcpServers": { "rates": { "command": "/bin/true", "disabled": true } } }"#).unwrap();
  assert!(provider("grok".into(), cwd.clone(), None, true).await.0.is_empty());
  std::fs::remove_file(home.join(".agents/mcp.json")).unwrap();
  plugins.action(PluginAction::SafeMode { on: true }).await.unwrap();
  assert!(provider("grok".into(), cwd, None, true).await.0.is_empty());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_hears_sessions_made_renamed_and_deleted() {
  let fake = fake_or_skip!();
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  let plugins = tempfile::tempdir().unwrap();
  let rt = crate::acp_session::plugin_runtime::recorder(plugins.path(), &["session.*"]).await;
  let mut opts = Opts::fake(&fake);
  opts.plugin_runtime = Some(rt.clone());
  let m = Mgr::new(dir.path(), opts);
  m.init().await;
  m.new_session(None).await;
  let id = m.active_id().unwrap();
  m.handle(json!({ "type": "renameSession", "id": id, "title": "Chest wishes" })).await;
  m.handle(json!({ "type": "deleteSession", "id": id })).await;
  let heard = || crate::acp_session::plugin_runtime::heard(plugins.path());
  until(|| heard().iter().any(|e| e["type"] == "session.deleted"), 5000).await;
  let events = heard();
  let of = |t: &str| events.iter().find(|e| e["type"] == t).unwrap_or_else(|| panic!("no {t} in {events:?}"))["data"].clone();
  assert_eq!((of("session.created")["how"].as_str(), of("session.created")["session"].as_str()), (Some("new"), Some(id.as_str())));
  assert_eq!(of("session.renamed")["title"], "Chest wishes");
  assert_eq!(of("session.deleted")["session"], id.as_str());
  // Only the family it asked for
  assert!(events.iter().all(|e| e["type"].as_str().unwrap().starts_with("session.")), "{events:?}");
  m.dispose().await;
  rt.stop_all().await;
}
