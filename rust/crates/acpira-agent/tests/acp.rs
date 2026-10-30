//! The ACP surface without a model: handshake, modes and the config options built from providers.json
#![allow(clippy::unwrap_used, reason = "test code")]

mod support;

use serde_json::json;
use support::Harness;

#[tokio::test]
async fn session_new_offers_the_configured_models_and_their_efforts() {
  let h = Harness::start().await;
  let empty = h.new_session().await;
  assert_eq!(empty["modes"]["currentModeId"], "agent");
  // Raw mode stays out of the picker until the settings page offers it
  let modes = |v: &serde_json::Value| v["modes"]["availableModes"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  assert!(!modes(&empty).contains(&"raw".to_owned()), "{:?}", modes(&empty));
  let ids = |v: &serde_json::Value| v["configOptions"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  assert_eq!(ids(&empty), ["approval"], "no source, no model select");

  h.providers("http://127.0.0.1:1", json!([{ "id": "plain" }, { "id": "thinker", "name": "Thinker", "efforts": ["low", "high"] }]));
  let s = h.new_session().await;
  let sid = s["sessionId"].as_str().unwrap();
  let opts = s["configOptions"].as_array().unwrap();
  assert_eq!(ids(&s), ["model", "approval"], "the approval level is the session's, last");
  let approval = &s["configOptions"][1];
  assert_eq!((approval["category"].as_str(), approval["currentValue"].as_str()), (Some("_permission_mode"), Some("ask")));
  assert_eq!(approval["options"][2], json!({ "value": "full", "name": "Full access", "_meta": { "kind": "full_access" } }));
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
  // A new session starts on the settings file's level, and the settings page's switch offers Raw mode
  let mut file: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(h.home().join("providers.json")).unwrap()).unwrap();
  file["approval"] = json!("auto-edit");
  file["rawMode"] = json!(true);
  std::fs::write(h.home().join("providers.json"), file.to_string()).unwrap();
  let next = h.new_session().await;
  assert_eq!(next["configOptions"][1]["currentValue"], "auto-edit");
  assert!(modes(&next).contains(&"raw".to_owned()), "{:?}", modes(&next));
}

#[tokio::test]
async fn a_model_added_by_id_takes_its_levels_from_the_catalogue() {
  let h = Harness::start().await;
  // Entered by hand, so without the levels discovery would have filled in
  h.providers(
    "http://127.0.0.1:1",
    json!([{ "id": "claude-sonnet-5-5" }, { "id": "glm-5.3", "thinking": "off" }, { "id": "gpt-6.1-sol" }, { "id": "gemini-3.8-flash" }]),
  );
  let s = h.new_session().await;
  let sid = s["sessionId"].as_str().unwrap();
  let effort = &s["configOptions"][1];
  // Claude starts at medium, Gemini at low, the other families at high (the family's preferred level)
  assert_eq!((effort["id"].as_str(), effort["currentValue"].as_str()), (Some("effort"), Some("medium")));
  let levels: Vec<&str> = effort["options"].as_array().unwrap().iter().map(|o| o["value"].as_str().unwrap()).collect();
  assert!(levels.contains(&"max"), "{levels:?}");
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "effort", "value": "max" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["currentValue"], "max");
  // Thinking switched off keeps the select away
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/glm-5.3" })).await.unwrap();
  assert_eq!(r["configOptions"].as_array().unwrap().len(), 2, "model and approval");
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/gemini-3.8-flash" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["currentValue"], "low");
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/glm-5.3" })).await.unwrap();
  assert_eq!(r["configOptions"].as_array().unwrap().len(), 2, "model and approval");
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/gpt-6.1-sol" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["currentValue"], "high");
}

#[tokio::test]
async fn a_guessed_scale_takes_the_catalogues_and_a_saved_one_is_kept() {
  let h = Harness::start().await;
  // A gateway's own id the catalogue only knows by its name, still holding the default levels guessed when it was
  // added, beside the same model whose levels were saved in the settings dialog
  h.providers(
    "http://127.0.0.1:1",
    json!([
      { "id": "k3-gw", "name": "Kimi K3", "efforts": ["low", "medium", "high"], "estimated": ["efforts"] },
      { "id": "kimi-k3", "efforts": ["low", "medium", "high"] },
    ]),
  );
  let levels = |r: &serde_json::Value| -> Vec<String> {
    r["configOptions"][1]["options"].as_array().unwrap().iter().map(|o| o["value"].as_str().unwrap().to_owned()).collect()
  };
  let s = h.new_session().await;
  let sid = s["sessionId"].as_str().unwrap();
  assert_eq!(levels(&s), ["low", "high", "max"]);
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "effort", "value": "max" })).await.unwrap();
  assert_eq!(r["configOptions"][1]["currentValue"], "max");
  let r = h.conn.request("session/set_config_option", json!({ "sessionId": sid, "configId": "model", "value": "mock/kimi-k3" })).await.unwrap();
  assert_eq!(levels(&r), ["low", "medium", "high"]);
}
