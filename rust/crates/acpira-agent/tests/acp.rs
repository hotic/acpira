//! The ACP surface without a model: handshake, modes and the config options built from providers.json

mod support;

use serde_json::json;
use support::Harness;

#[tokio::test]
async fn session_new_offers_the_configured_models_and_their_efforts() {
  let h = Harness::start().await;
  let empty = h.new_session().await;
  assert_eq!(empty["modes"]["currentModeId"], "agent");
  let ids = |v: &serde_json::Value| v["configOptions"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  assert_eq!(ids(&empty), ["approval"], "no source, no model select");

  h.providers("http://127.0.0.1:1", json!([{ "id": "plain" }, { "id": "thinker", "name": "Thinker", "efforts": ["low", "high"] }]));
  let s = h.new_session().await;
  let sid = s["sessionId"].as_str().unwrap();
  let opts = s["configOptions"].as_array().unwrap();
  assert_eq!(ids(&s), ["model", "approval"]);
  assert_eq!(opts[0]["currentValue"], "mock/plain");
  assert_eq!(opts[0]["options"][1], json!({ "value": "mock/thinker", "name": "Thinker", "description": "Mock" }));

  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/thinker" })).await.unwrap();
  let effort = &r["configOptions"][1];
  assert_eq!((effort["id"].as_str(), effort["currentValue"].as_str()), (Some("effort"), Some("high")));
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "effort", "value": "low" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["currentValue"], "low");
  let bad = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/gone" })).await;
  assert!(bad.is_err());
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "approval", "value": "full" })).await.unwrap();
  assert_eq!(r["configOptions"][2]["currentValue"], "full");
  assert!(h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "approval", "value": "yolo" })).await.is_err());
}
