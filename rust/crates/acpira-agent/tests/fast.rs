//! The Fast switch: offered only for a model with a fast tier, sent as the format's own parameter, recorded in the
//! session log, and kept across a reopen

mod support;

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel};
use support::Harness;

fn option<'a>(r: &'a Value, id: &str) -> Option<&'a Value> {
  r["configOptions"].as_array().unwrap().iter().find(|o| o["id"] == id)
}

async fn set(h: &Harness, sid: &str, id: &str, value: Value) -> Result<Value, acpira_rpc::rpc::RpcError> {
  h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": id, "value": value })).await
}

async fn prompt(h: &Harness, sid: &str) {
  let r = h.conn.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "hi" }] })).await.unwrap();
  assert_eq!(r["stopReason"], "end_turn");
}

fn events(h: &Harness, sid: &str, kind: &str) -> Vec<Value> {
  let log = std::fs::read_to_string(h.home().join("agent").join("sessions").join(format!("{sid}.jsonl"))).unwrap();
  log.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()).filter(|e| e["type"] == kind).collect()
}

#[tokio::test]
async fn a_fast_model_offers_the_switch_and_its_calls_ask_for_the_fast_tier() {
  let h = Harness::start().await;
  let server = MockModel::start();
  h.providers(&server.base_url(), json!([{ "id": "gpt-6.1-sol" }, { "id": "m1" }]));
  let s = h.new_session().await;
  let sid = s["sessionId"].as_str().unwrap().to_owned();
  // The harness's client does not take boolean options: the switch is a Standard / Fast select, off at first
  let fast = option(&s, "fast").expect("gpt-6.1-sol has a fast tier");
  assert_eq!((fast["type"].as_str(), fast["category"].as_str(), fast["currentValue"].as_str()), (Some("select"), Some("model_config"), Some("standard")));
  let r = set(&h, &sid, "fast", json!("fast")).await.unwrap();
  assert_eq!(option(&r, "fast").unwrap()["currentValue"], "fast");

  server.push(mock::text("quick"));
  prompt(&h, &sid).await;
  assert_eq!(server.requests()[0].body["service_tier"], "priority");
  let reqs = events(&h, &sid, "request");
  assert!(reqs[0].get("fast").is_some(), "the record says the call asked for the fast tier");

  // Off again: the next call is a standard one, and the log names the switch as what changed the request
  set(&h, &sid, "fast", json!("standard")).await.unwrap();
  server.push(mock::text("slow"));
  prompt(&h, &sid).await;
  assert!(server.requests()[1].body.get("service_tier").is_none());
  let reqs = events(&h, &sid, "request");
  assert!(reqs[1].get("fast").is_none());
  // Same scripted usage both times: the fast call's estimate is at twice the list price
  let cost = |r: &Value| r["cost"].as_f64().expect("the catalogue prices gpt-6.1-sol");
  assert!((cost(&reqs[0]) - 2.0 * cost(&reqs[1])).abs() < 1e-12, "{} vs {}", reqs[0]["cost"], reqs[1]["cost"]);
  let views = events(&h, &sid, "view");
  assert_eq!(views.last().unwrap()["changed"], json!(["fast"]));

  // A model without a fast tier shows no switch and refuses one
  let r = set(&h, &sid, "model", json!("mock/m1")).await.unwrap();
  assert!(option(&r, "fast").is_none());
  assert!(set(&h, &sid, "fast", json!("fast")).await.is_err());
}

#[tokio::test]
async fn the_switch_is_a_boolean_for_a_client_that_takes_them_and_survives_a_reopen() {
  let h = Harness::start().await;
  h.conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": { "session": { "configOptions": { "boolean": {} } } } })).await.unwrap();
  // Claude Opus 5.5 on Anthropic Messages, and a gateway alias the user marked as having a fast tier
  let server = MockModel::start();
  h.providers_in("anthropic", &server.base_url(), json!([{ "id": "claude-opus-5-5" }, { "id": "house-model", "fast": true }, { "id": "claude-sonnet-5-5" }]));
  let s = h.new_session().await;
  let sid = s["sessionId"].as_str().unwrap().to_owned();
  assert_eq!(option(&s, "fast").unwrap()["type"], "boolean");
  assert_eq!(option(&s, "fast").unwrap()["currentValue"], false);
  let r = set(&h, &sid, "fast", json!(true)).await.unwrap();
  assert_eq!(option(&r, "fast").unwrap()["currentValue"], true);
  // The switch stays on through a model without a fast tier and applies again on the next one that has it
  let r = set(&h, &sid, "model", json!("mock/claude-sonnet-5-5")).await.unwrap();
  assert!(option(&r, "fast").is_none());
  let r = set(&h, &sid, "model", json!("mock/house-model")).await.unwrap();
  assert_eq!(option(&r, "fast").unwrap()["currentValue"], true);
  // On Anthropic Messages: `speed` in the body and the beta header
  server.push(mock::messages(&[json!({ "type": "text", "text": "quick" })], "end_turn", 0));
  prompt(&h, &sid).await;
  let req = &server.requests()[0];
  assert_eq!((req.body["model"].as_str(), req.body["speed"].as_str()), (Some("house-model"), Some("fast")));
  assert_eq!(req.header("anthropic-beta"), Some("fast-mode-2026-02-01"));

  // A new agent process on the same data root resumes the session with the switch as it was
  let Harness { home, cwd, conn, .. } = h;
  drop(conn);
  let h = Harness::start_in(home, cwd).await;
  h.conn.request("initialize", json!({ "protocolVersion": 1, "clientCapabilities": { "session": { "configOptions": { "boolean": {} } } } })).await.unwrap();
  let r = h.conn.request("session/resume", json!({ "sessionId": sid, "cwd": h.cwd(), "mcpServers": [] })).await.unwrap();
  assert_eq!(option(&r, "fast").unwrap()["currentValue"], true);
}
