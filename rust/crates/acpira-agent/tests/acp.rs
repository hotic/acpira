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

#[tokio::test]
async fn a_model_added_by_id_takes_its_levels_from_the_catalogue() {
  let h = Harness::start().await;
  // Entered by hand, so without the levels discovery would have filled in
  h.providers(
    "http://127.0.0.1:1",
    json!([{ "id": "claude-sonnet-5-5" }, { "id": "glm-5.3", "thinking": "off" }, { "id": "gpt-6.1-sol" }]),
  );
  let s = h.new_session().await;
  let sid = s["sessionId"].as_str().unwrap();
  let effort = &s["configOptions"][1];
  // Claude starts at medium, the other families at high (the family's preferred level)
  assert_eq!((effort["id"].as_str(), effort["currentValue"].as_str()), (Some("effort"), Some("medium")));
  let levels: Vec<&str> = effort["options"].as_array().unwrap().iter().map(|o| o["value"].as_str().unwrap()).collect();
  assert!(levels.contains(&"max"), "{levels:?}");
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "effort", "value": "max" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["currentValue"], "max");
  // Thinking switched off keeps the select away
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/glm-5.3" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["id"], "approval");
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/gpt-6.1-sol" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["currentValue"], "high");
}
