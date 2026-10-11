//! Regressions from the review after v1.9.1 (commit 1ac9ddc); every model response comes from the local scripted server.
mod support;

use std::time::Duration;
use acpira_agent::mock::{self, MockModel};
use serde_json::{Value, json};
use support::Harness;

async fn prompt(h: &Harness, sid: &str) -> Value {
  h.conn.request("session/prompt", json!({"sessionId":sid,"prompt":[{"type":"text","text":"review fixture"}]})).await.unwrap()
}

async fn set(h: &Harness, sid: &str, id: &str, value: &str) {
  h.conn.request("session/set_config_option", json!({"sessionId":sid,"configId":id,"value":value})).await.unwrap();
}

async fn reopen(h: Harness, sid: &str) -> (Harness, Value) {
  let Harness { conn, home, cwd, .. } = h;
  conn.close();
  drop(conn);
  let h = Harness::start_in(home, cwd).await;
  let r = h.conn.request("session/resume", json!({"sessionId":sid,"cwd":h.cwd(),"mcpServers":[]})).await.unwrap();
  (h, r)
}

fn control<'a>(r: &'a Value, id: &str) -> &'a Value {
  &r["configOptions"].as_array().unwrap().iter().find(|v| v["id"] == id).unwrap()["currentValue"]
}

#[tokio::test]
async fn a_control_changed_after_resume_survives_the_next_resume() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{"id":"m1"},{"id":"m2"}]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  set(&h, &sid, "model", "mock/m2").await;
  server.push(mock::text("first"));
  prompt(&h, &sid).await;
  let (h, _) = reopen(h, &sid).await;
  set(&h, &sid, "model", "mock/m1").await;
  server.push(mock::text("second"));
  prompt(&h, &sid).await;
  let (_h, r) = reopen(h, &sid).await;
  assert_eq!(control(&r, "model"), &json!("mock/m1"));
}

#[tokio::test]
async fn a_default_model_is_persisted_after_the_first_prompt() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{"id":"m1"},{"id":"m2"}]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  server.push(mock::text("first"));
  prompt(&h, &sid).await;
  h.providers(&server.base_url(), json!([{"id":"m2"},{"id":"m1"}]));
  let (_h, r) = reopen(h, &sid).await;
  assert_eq!(control(&r, "model"), &json!("mock/m1"), "the session followed the changed provider default");
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_a_reused_provider_call_id_still_writes_its_result() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{"id":"m1"}]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  std::fs::write(h.cwd().join("a.txt"), "fixture").unwrap();
  server.push(mock::tools(&[("c1", "read", json!({"path":"a.txt"}))]));
  server.push(mock::tools(&[("c1", "bash", json!({"command":"sleep 5"}))]));
  let (conn, id) = (h.conn.clone(), sid.clone());
  let turn = tokio::spawn(async move {
    conn.request("session/prompt", json!({"sessionId":id,"prompt":[{"type":"text","text":"review"}]})).await.unwrap()
  });
  tokio::time::timeout(Duration::from_secs(5), async {
    while !h.updates().iter().any(|u| u["toolCallId"] == "call-2" && u["status"] == "in_progress") {
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  }).await.unwrap();
  h.conn.notify("session/cancel", json!({"sessionId":sid}));
  assert_eq!(turn.await.unwrap()["stopReason"], "cancelled");
  server.push(mock::text("next"));
  prompt(&h, &sid).await;
  let requests = server.requests();
  let messages = requests.last().unwrap().body["messages"].as_array().unwrap();
  let last_call = messages.iter().rposition(|m| m.get("tool_calls").is_some()).unwrap();
  assert_eq!(messages[last_call + 1]["role"], "tool", "cancelled call has no result: {messages:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn allowing_a_command_prefix_does_not_allow_a_second_shell_command() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{"id":"m1"}]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  *h.client.answer.lock() = support::Answer::Always;
  server.push(mock::tools(&[("c1", "bash", json!({"command":"echo review"}))]));
  server.push(mock::tools(&[("c2", "bash", json!({"command":"echo review && printf unapproved > marker"}))]));
  server.push(mock::text("done"));
  prompt(&h, &sid).await;
  assert_eq!(h.client.permissions.lock().len(), 2, "a second command ran without approval; marker exists: {}", h.cwd().join("marker").exists());
}

#[cfg(unix)]
#[test]
fn protected_config_stays_protected_through_a_symlink() {
  let home = tempfile::tempdir().unwrap();
  let cwd = tempfile::tempdir().unwrap();
  std::fs::write(home.path().join("secrets.json"), "{}").unwrap();
  std::os::unix::fs::symlink(home.path(), cwd.path().join("alias")).unwrap();
  let guard = acpira_agent::permission::Guard::new(home.path(), cwd.path(), None);
  assert!(guard.protects(&cwd.path().join("alias/secrets.json")));
}

#[cfg(unix)]
#[tokio::test]
async fn a_protected_config_write_through_a_symlink_still_asks() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{"id":"m1"}]));
  std::os::unix::fs::symlink(h.home(), h.cwd().join("alias")).unwrap();
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  h.approval("full");
  server.push(mock::tools(&[("c1", "write", json!({"path":"alias/secrets.json","content":"{}"}))]));
  server.push(mock::text("done"));
  prompt(&h, &sid).await;
  assert_eq!(h.client.permissions.lock().len(), 1, "config was changed without asking: {}", std::fs::read_to_string(h.home().join("secrets.json")).unwrap());
}

#[test]
fn a_full_responses_endpoint_can_derive_its_model_list() {
  let p: acpira_shared::providers::Provider = serde_json::from_value(json!({
    "id":"fixture","format":"openai-responses","baseUrl":"https://example.test/v1/responses","fullUrl":true
  })).unwrap();
  assert_eq!(acpira_agent::llm::discover::models_url(&p).as_deref(), Some("https://example.test/v1/models"));
}

#[tokio::test]
async fn a_saved_model_source_updates_the_open_session_options() {
  let h = Harness::start().await;
  h.providers("http://127.0.0.1:9/v1", json!([{"id":"m1"}]));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  h.providers("http://127.0.0.1:9/v1", json!([{"id":"m1"},{"id":"m2"}]));
  let models = |u: &Value| -> Option<Vec<String>> {
    let opts = u.get("configOptions")?.as_array()?;
    let model = opts.iter().find(|o| o["id"] == "model")?;
    Some(model["options"].as_array()?.iter().filter_map(|o| o["value"].as_str().map(str::to_owned)).collect())
  };
  let pushed = tokio::time::timeout(Duration::from_secs(5), async {
    loop {
      if let Some(m) = h.updates().iter().filter(|u| u["sessionUpdate"] == "config_option_update").find_map(models) {
        return m;
      }
      tokio::time::sleep(Duration::from_millis(20)).await;
    }
  }).await.expect("no config_option_update after the source changed");
  assert_eq!(pushed, ["mock/m1", "mock/m2"]);
  // Sent once per change, and only to sessions whose options moved
  tokio::time::sleep(Duration::from_millis(1500)).await;
  assert_eq!(h.updates().iter().filter(|u| u["sessionUpdate"] == "config_option_update").count(), 1, "{sid}");
}
