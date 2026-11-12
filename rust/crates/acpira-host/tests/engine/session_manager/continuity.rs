//! Foreign-copy standby, live updates and ownership committed only after a successful local start.

use super::*;
use acpira_host::store::record::SessionRecord;
use acpira_shared::transcript::Turn;
use std::time::Duration;

fn copy(id: &str, cwd: &str) -> SessionRecord {
  serde_json::from_value(json!({
    "id": id, "agent": "fake", "cwd": cwd, "title": "Copied title", "acpSessionId": "foreign-native",
    "createdAt": "2026-01-01T00:00:00.000Z", "updatedAt": "2026-01-01T00:00:00.000Z",
    "turns": [{ "role": "user", "text": "earlier question" },
      { "role": "agent", "blocks": [{ "type": "text", "markdown": "earlier answer" }], "stop": "end_turn" }],
    "controls": { "modeId": "agent", "modes": [{ "id": "agent", "name": "Agent" }, { "id": "plan", "name": "Plan" }],
      "options": [{ "id": "model", "name": "Model", "value": "m1", "options": [{ "id": "m1", "name": "First" }, { "id": "m2", "name": "Second" }] }] },
    "copyOf": { "machine": "other-machine", "name": "Laptop" }
  })).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn standby_streams_new_copies_keeps_picks_and_promotes_on_send() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let work = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  let mut record = copy("standby", work.path().to_str().unwrap());
  m.store.flush(Arc::new(record.clone())).await.unwrap();
  m.init().await;
  m.handle(json!({ "type": "selectSession", "id": record.id })).await;
  let revision = m.active().unwrap()["rev"].as_i64().unwrap();
  assert!(revision < 0);
  assert_eq!(m.active().unwrap()["status"], "ready");
  assert!(m.active().unwrap()["error"].is_null());
  assert!(m.m.live_session(&record.id).is_none());
  m.handle(json!({ "type": "setMode", "id": "plan" })).await;
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  record.updated_at = "2026-01-02T00:00:00.000Z".into();
  record.turns.extend(serde_json::from_value::<Vec<Turn>>(json!([
    { "role": "user", "text": "new remote question" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "new remote answer" }], "stop": "end_turn" }
  ])).unwrap());
  assert!(m.store.put_copy(&record, None).await.unwrap());
  until(|| m.active().is_some_and(|s| turns_in(&s) == 4), 5000).await;
  assert_eq!(m.active().unwrap()["controls"]["modeId"], "plan");
  assert!(m.m.live_session(&record.id).is_none());
  m.handle(json!({ "type": "send", "text": "inspect-history" })).await;
  let session = m.m.live_session(&record.id).unwrap();
  let saved = session.to_record();
  assert_eq!(saved.controls.mode_id.as_deref(), Some("plan"));
  assert_eq!(saved.controls.options.iter().find(|c| c.id == "model").unwrap().value.as_deref(), Some("m2"));
  assert!(saved.copy_of.is_none() && !saved.history_pending && saved.history_context);
  assert!(m.active().unwrap()["rev"].as_i64().unwrap() >= 0);
  assert!(m.active().unwrap()["turns"][5].to_string().contains("new remote answer"));
  m.store.flush(session).await.unwrap();
  assert!(m.store.load(&record.id).await.unwrap().copy_of.is_none());
  record.updated_at = "2999-01-01T00:00:00.000Z".into();
  assert!(!m.store.put_copy(&record, None).await.unwrap());
  assert!(m.store.load(&record.id).await.unwrap().copy_of.is_none());
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_promotion_never_claims_the_disk_record_even_when_the_manager_closes() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let work = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::with_agents(fake.setting(json!({ "command": "/missing-continuity-agent" })), "fake"));
  let record = copy("no-agent", work.path().to_str().unwrap());
  m.store.flush(Arc::new(record.clone())).await.unwrap();
  m.init().await;
  m.handle(json!({ "type": "selectSession", "id": record.id })).await;
  m.handle(json!({ "type": "send", "text": "do not lose this record" })).await;
  assert_eq!(m.active().unwrap()["status"], "error");
  assert!(m.store.load(&record.id).await.unwrap().copy_of.is_some());
  m.dispose().await;
  assert!(m.store.load(&record.id).await.unwrap().copy_of.is_some());
}

// A rename claims the copy for this machine and nothing more: no agent starts when the standby next reads its record,
// and what was picked on it is still there for the first send
#[tokio::test(flavor = "multi_thread")]
async fn a_list_edit_claims_a_standby_copy_without_an_agent_and_keeps_its_picks() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let work = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  let record = copy("claimed", work.path().to_str().unwrap());
  m.store.flush(Arc::new(record.clone())).await.unwrap();
  m.init().await;
  m.handle(json!({ "type": "selectSession", "id": record.id })).await;
  m.handle(json!({ "type": "setMode", "id": "plan" })).await;
  m.handle(json!({ "type": "renameSession", "id": record.id, "title": "Renamed here" })).await;
  assert!(m.store.load(&record.id).await.unwrap().copy_of.is_none());
  // Past the standby's next looks at the record it follows
  tokio::time::sleep(Duration::from_millis(2500)).await;
  assert!(m.m.live_session(&record.id).is_none(), "{:#?}", m.logs());
  let shown = m.active().unwrap();
  assert!(shown["rev"].as_i64().unwrap() < 0);
  assert_eq!((&shown["title"], &shown["controls"]["modeId"]), (&json!("Renamed here"), &json!("plan")));
  m.handle(json!({ "type": "send", "text": "inspect-history" })).await;
  let saved = m.m.live_session(&record.id).unwrap().to_record();
  assert_eq!(saved.controls.mode_id.as_deref(), Some("plan"));
  assert_eq!(saved.title, "Renamed here");
  m.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_synced_builtin_log_promotes_by_resuming_native_context() {
  let root = tempfile::tempdir().unwrap();
  let work = tempfile::tempdir().unwrap();
  let home = root.path().join("lease-home");
  let server = acpira_agent::mock::MockModel::start();
  std::fs::create_dir_all(home.join("agent/sessions")).unwrap();
  std::fs::write(home.join("providers.json"), json!({ "version": 1, "fastModel": "off", "providers": [{
    "id": "mock", "name": "Mock", "format": "openai-chat", "baseUrl": server.base_url(),
    "models": [{ "id": "m1", "name": "Mock", "context": 64000 }]
  }] }).to_string()).unwrap();
  std::fs::write(home.join("secrets.json"), json!({ "acpira.provider.mock": "test" }).to_string()).unwrap();
  let native = "native-lossless";
  let log = [
    json!({ "type": "session", "id": native, "cwd": work.path(), "format": 1 }),
    json!({ "type": "item", "item": { "role": "user", "parts": [{ "text": "native-only secret context" }] } }),
    json!({ "type": "item", "item": { "role": "assistant", "text": "native answer", "reasoning": "", "calls": [] } }),
  ].iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n";
  std::fs::write(home.join("agent/sessions").join(format!("{native}.jsonl")), &log).unwrap();
  let opts = Opts::with_agents(json!({ "acpira": { "name": "Acpira", "command": env!("CARGO_BIN_EXE_acpira"), "args": ["agent", "--home", home] } }), "acpira");
  let m = Mgr::new(root.path(), opts);
  let mut record = copy("builtin-copy", work.path().to_str().unwrap());
  record.agent = "acpira".into();
  record.acp_session_id = Some(native.into());
  record.controls = Default::default();
  m.store.flush(Arc::new(record)).await.unwrap();
  m.init().await;
  m.handle(json!({ "type": "selectSession", "id": "builtin-copy" })).await;
  server.push(acpira_agent::mock::text("resumed"));
  m.handle(json!({ "type": "send", "text": "continue here" })).await;
  let session = m.m.live_session("builtin-copy").unwrap();
  assert_eq!(session.to_record().acp_session_id.as_deref(), Some(native));
  assert!(!session.to_record().history_pending);
  assert!(!session.to_record().history_context);
  let requests = server.requests();
  let sent = requests[0].body["messages"].to_string();
  assert!(sent.contains("native-only secret context") && !sent.contains("acpira://history"), "{sent}");
  assert!(m.store.load("builtin-copy").await.unwrap().copy_of.is_none());
  m.dispose().await;
}
