//! A session another machine drives over SSH reads the built-in agent settings that machine brought
//! (`acpira/clientSettings`, `ConfigCache::of_client`) instead of the data root's own, and takes them at a turn's start
#![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]

mod support;

use std::time::Duration;

use acpira_agent::mock::{self, MockModel};
use serde_json::{Value, json};
use support::Harness;

const META: &str = "acpira/clientSettings";

/// Settings `machine` brought: one source with these models, and these other fields of the file
fn bring(h: &Harness, machine: &str, base_url: &str, models: &[&str], fields: Value) {
  let dir = h.home().join("remote").join("clients").join(machine);
  std::fs::create_dir_all(&dir).unwrap();
  let mut file = json!({ "version": 1, "fastModel": "off", "providers": [{
    "id": "cs", "name": "Client", "format": "openai-chat", "baseUrl": base_url, "models": models.iter().map(|m| json!({ "id": m })).collect::<Vec<_>>(),
  }] });
  file.as_object_mut().unwrap().extend(fields.as_object().cloned().unwrap_or_default());
  std::fs::write(dir.join("secrets.json"), json!({ "acpira.provider.cs": "sk-client" }).to_string()).unwrap();
  std::fs::write(dir.join("providers.json"), file.to_string()).unwrap();
}

/// The model select's values, and the approval select's current value
fn picker(options: &Value) -> (Vec<String>, String) {
  let opts = options.as_array().cloned().unwrap_or_default();
  let find = |id: &str| opts.iter().find(|o| o["id"] == id).cloned().unwrap_or(Value::Null);
  let models = find("model")["options"].as_array().cloned().unwrap_or_default().iter().filter_map(|o| o["value"].as_str().map(str::to_owned)).collect();
  (models, find("approval")["currentValue"].as_str().unwrap_or("").to_owned())
}

/// The last model list pushed to `sid`
fn pushed(h: &Harness, sid: &str) -> Option<Vec<String>> {
  h.updates_of(sid).iter().rev().find(|u| u["sessionUpdate"] == "config_option_update").map(|u| picker(&u["configOptions"]).0)
}

async fn until(what: &str, mut pred: impl FnMut() -> bool) {
  let t0 = std::time::Instant::now();
  while !pred() {
    assert!(t0.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

/// Opened for a machine, a session lists that machine's models and starts on its approval level, and follows its
/// edits; a session of this machine's own reads the data root's settings, and neither hears of the other's changes
#[tokio::test]
async fn a_session_reads_the_settings_of_the_machine_that_drives_it() {
  let h = Harness::start().await;
  h.providers("http://127.0.0.1:9/v1", json!([{ "id": "h1" }]));
  bring(&h, "mac", "http://127.0.0.1:9/v1", &["c1", "c2"], json!({ "approval": "full" }));

  let theirs = h.conn.request("session/new", json!({ "cwd": h.cwd(), "mcpServers": [], "_meta": { META: "mac" } })).await.unwrap();
  let own = h.new_session().await;
  assert_eq!(picker(&theirs["configOptions"]), (vec!["cs/c1".to_owned(), "cs/c2".to_owned()], "full".to_owned()));
  assert_eq!(picker(&own["configOptions"]), (vec!["mock/h1".to_owned()], "ask".to_owned()));
  let (theirs, own) = (theirs["sessionId"].as_str().unwrap().to_owned(), own["sessionId"].as_str().unwrap().to_owned());

  // A model switched off over there reaches its session, not this machine's
  bring(&h, "mac", "http://127.0.0.1:9/v1", &["c2"], json!({}));
  until("the edit on the machine's session", || pushed(&h, &theirs).is_some_and(|m| m == ["cs/c2"])).await;
  assert!(pushed(&h, &own).is_none(), "the data root's own session did not change");

  // The settings gone (that machine turned its setting off): the data root's own
  std::fs::remove_dir_all(h.home().join("remote").join("clients").join("mac")).unwrap();
  until("the data root's settings", || pushed(&h, &theirs).is_some_and(|m| m == ["mock/h1"])).await;
}

/// A turn reads the settings of the machine that sent it, switching the session's model when that machine has no such
/// model; a turn sent here goes back to the data root's own
#[tokio::test]
async fn a_turn_takes_the_settings_of_the_machine_that_sent_it() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "h1" }]));
  bring(&h, "mac", &server.base_url(), &["c1"], json!({}));
  let sid = h.new_session().await["sessionId"].as_str().unwrap().to_owned();
  let send = |meta: Value| {
    let mut params = json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "hi" }] });
    if !meta.is_null() {
      params["_meta"] = meta;
    }
    h.conn.request("session/prompt", params)
  };

  server.push(mock::text("one"));
  assert_eq!(send(json!({ META: "mac" })).await.unwrap()["stopReason"], "end_turn");
  assert_eq!(server.requests()[0].body["model"], "c1");
  assert_eq!(pushed(&h, &sid).unwrap(), ["cs/c1"], "the picker followed before the turn ran");

  server.push(mock::text("two"));
  assert_eq!(send(Value::Null).await.unwrap()["stopReason"], "end_turn");
  assert_eq!(server.requests()[1].body["model"], "h1");
  assert_eq!(pushed(&h, &sid).unwrap(), ["mock/h1"]);
}
