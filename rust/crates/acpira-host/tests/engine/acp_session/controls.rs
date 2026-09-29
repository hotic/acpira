//! Config options, modes, efforts and optimistic picks

use super::*;

// ACP RFD boolean-config-option: the fake only offers its `fast` toggle to clients advertising
// clientCapabilities.session.configOptions.boolean, and the set request must carry a real boolean
#[tokio::test(flavor = "multi_thread")]
async fn a_boolean_config_option_arrives_gated_and_goes_out_as_a_real_boolean() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let log_file = dir.path().join("config.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_BOOL": "1", "FAKE_CONFIG_LOG": log_file } }));
  let s = started(&h, "/tmp").await;
  let fast = view(&s)["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == "fast").cloned().unwrap();
  expect_match(&fast, json!({ "type": "boolean", "name": "Fast mode", "value": "false" }));
  assert_eq!(fast["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("false"), json!("true")]);
  s.set_config("fast".into(), "true".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "fast"), "true");
  s.set_config("fast".into(), "false".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "fast"), "false");
  let wire: Vec<Value> = std::fs::read_to_string(&log_file).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
  expect_eq(wire, json!([{ "configId": "fast", "type": "boolean", "value": true }, { "configId": "fast", "type": "boolean", "value": false }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn synthesized_modes_backfill_from_the_registry_and_go_through_set_mode() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-no-modes").unwrap();
  let h = Harness::new(&fake, json!({ "modes": syn_modes() }));
  let s = started(&h, "/tmp/acpira-no-modes").await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  let ids = |k: &str| vw["controls"][k].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  assert_eq!(ids("modes"), ["default", "plan", "yolo"]);
  assert_eq!(vw["controls"]["modeId"], "default");
  // configOptions unaffected, still land in controls.options
  assert_eq!(ids("options"), ["model", "effort"]);
  s.set_mode("plan".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "plan");
  s.set_mode("default".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "default");
}

fn agent_value(s: &AcpSession, id: &str) -> Option<String> {
  s.agent_controls().options.iter().find(|o| o.id == id).and_then(|o| o.value.clone())
}

fn record_value(s: &AcpSession, id: &str) -> Option<String> {
  s.to_record().controls.options.iter().find(|o| o.id == id).and_then(|o| o.value.clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_optimistic_config_pick_moves_the_view_at_once_while_the_record_waits_for_the_agent() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150" } }));
  let s = started(&h, "/tmp").await;
  let before = h.changes();
  let p = tokio::spawn({
    let s = s.0.clone();
    async move { s.select_config("effort".into(), "low".into()).await }
  });
  until(|| option_value(&view(&s), "effort") == "low", 1000).await;
  assert!(h.changes() > before);
  assert_eq!(record_value(&s, "effort").as_deref(), Some("high"));
  p.await.unwrap().unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  assert_eq!(record_value(&s, "effort").as_deref(), Some("low"));
  assert_eq!(agent_value(&s, "effort").as_deref(), Some("low"));
}

#[tokio::test(flavor = "multi_thread")]
async fn rapid_config_picks_collapse_to_the_last_value_without_flicker() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150" } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(agent_value(&s, "model").as_deref(), Some("m1"));
  // Both clicks land before anything is observed, like two synchronous TS calls
  let (s1, s2) = (s.0.clone(), s.0.clone());
  let p1 = claimed(async move { s1.select_config("model".into(), "m1".into()).await });
  let p2 = claimed(async move { s2.select_config("model".into(), "m2".into()).await });
  let seen = h.sample(&s, |vw| vw.controls.options.iter().find(|o| o.id == "model").and_then(|o| o.value.clone()));
  p1.await.ok();
  p2.await.ok();
  until(|| !seen.lock().unwrap().is_empty(), 5000).await;
  let seen = seen.lock().unwrap().clone();
  assert!(seen.iter().all(|x| x.as_deref() == Some("m2")), "{seen:?}");
  assert_eq!(agent_value(&s, "model").as_deref(), Some("m2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_optimistic_value_reverts_the_view_to_agent_truth() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150", "FAKE_MODELS": "unavailable" } }));
  let s = started(&h, "/tmp").await;
  let p = tokio::spawn({
    let s = s.0.clone();
    async move { s.select_config("model".into(), "unavailable".into()).await }
  });
  until(|| option_value(&view(&s), "model") == "unavailable", 1000).await;
  assert_eq!(record_value(&s, "model").as_deref(), Some("m1"));
  assert!(p.await.unwrap().is_err());
  assert_eq!(option_value(&view(&s), "model"), "m1");
  assert_eq!(record_value(&s, "model").as_deref(), Some("m1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_optimistic_mode_pick_moves_the_view_at_once_and_agent_truth_follows_the_wire() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150" } }));
  let s = started(&h, "/tmp").await;
  let p = tokio::spawn({
    let s = s.0.clone();
    async move { s.select_mode("plan".into()).await }
  });
  until(|| view(&s)["controls"]["modeId"] == "plan", 1000).await;
  assert_eq!(s.agent_controls().mode_id.as_deref(), Some("agent"));
  p.await.unwrap().unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "plan");
  assert_eq!(s.agent_controls().mode_id.as_deref(), Some("plan"));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_effort_survives_a_fusion_sidekick_change_on_the_wire() {
  let fake = fake_or_skip!();
  let first = "Fusion (GPT-6 Astra High Thinking + SWE-2 Medium)";
  let second = "Fusion (GPT-6 Astra High Thinking + SWE-2 High)";
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODELS": format!("{first},{second}"), "FAKE_MODEL_RESETS_EFFORT": "1" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("model".into(), first.into()).await.unwrap();
  s.set_config("effort".into(), "low".into()).await.unwrap();
  s.set_config("model".into(), second.into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  assert_eq!(option_value(&view(&s), "model"), second);
  // Another round trip proves the restored value belongs to the agent, not only the view
  s.set_config("model".into(), first.into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  s.set_config("model".into(), "m2".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_switch_keeps_the_chosen_effort_and_never_shows_the_interim_reset() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODEL_RESETS_EFFORT": "1", "FAKE_CONFIG_DELAY_MS": "60" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("effort".into(), "low".into()).await.unwrap();
  let seen = h.sample(&s, |vw| vw.controls.options.iter().find(|o| o.id == "effort").and_then(|o| o.value.clone()));
  s.select_config("model".into(), "m2".into()).await.unwrap();
  until(|| !seen.lock().unwrap().is_empty(), 5000).await;
  let seen = seen.lock().unwrap().clone();
  assert!(seen.iter().all(|x| x.as_deref() == Some("low")), "{seen:?}");
  assert_eq!(agent_value(&s, "model").as_deref(), Some("m2"));
  assert_eq!(agent_value(&s, "effort").as_deref(), Some("low"));
}

#[tokio::test(flavor = "multi_thread")]
async fn replaying_remembered_controls_does_not_reset_the_previous_effort_between_model_and_effort() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODEL_RESETS_EFFORT": "1" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("effort".into(), "low".into()).await.unwrap();
  s.adopt_controls(serde_json::from_value(json!({ "config": { "model": "m2" } })).unwrap()).await;
  // Nothing remembered for effort: the agent's own value for the new model stands
  assert_eq!(agent_value(&s, "effort").as_deref(), Some("high"));
}

#[tokio::test(flavor = "multi_thread")]
async fn ignore_modes_drops_protocol_modes_and_a_pushed_mode_update() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "ignoreModes": true }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["controls"]["modes"], json!([]));
  expect_absent(&view(&s)["controls"], "modeId");
  prompt(&s, "mode:plan").await;
  expect_absent(&view(&s)["controls"], "modeId");
  assert_eq!(view(&s)["controls"]["modes"], json!([]));
}

// Editing a historical turn rebuilds the peer through session/new, whose modes used to skip ignoreModes (Pi, 2026-09-26)
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_keeps_ignored_modes_hidden_on_the_rebuilt_peer() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "ignoreModes": true }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  let old_peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 2, "inspect-history");
  // A turn recorded while the modes leaked still carries a mode id; with no modes it is moot, not an error
  edit.settings.mode_id = Some("plan".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  assert_ne!(s.to_record().acp_session_id, old_peer);
  let vw = view(&s);
  assert_eq!(vw["controls"]["modes"], json!([]));
  expect_absent(&vw["controls"], "modeId");
  // The fresh peer was never asked to switch modes
  let reply: Value = serde_json::from_str(vw["turns"][3]["blocks"][0]["markdown"].as_str().unwrap()).unwrap();
  expect_absent(&reply, "mode");
}

#[tokio::test(flavor = "multi_thread")]
async fn modes_mirroring_the_reasoning_select_stay_hidden_through_picks_and_edits() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MIRROR_MODES": "1" } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["controls"]["modes"], json!([]));
  expect_absent(&view(&s)["controls"], "modeId");
  // The adapter echoes the pick as current_mode_update, which must not bring a mode back
  s.set_config("effort".into(), "low".into()).await.unwrap();
  prompt(&s, "earlier-context").await;
  let vw = view(&s);
  assert_eq!(option_value(&vw, "effort"), "low");
  expect_absent(&vw["controls"], "modeId");
  expect_absent(&vw["turns"][0]["settings"], "modeId");
  prompt(&s, "original").await;
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(vw["controls"]["modes"], json!([]));
  expect_absent(&vw["controls"], "modeId");
}

fn option_ids(view: &Value, id: &str) -> Vec<String> {
  let control = view["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == id).cloned().unwrap_or(Value::Null);
  control["options"].as_array().map(|a| a.iter().map(|o| o["id"].as_str().unwrap_or("").to_owned()).collect()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pi_custom_model_offers_only_the_efforts_pi_and_the_catalogue_both_allow() {
  let fake = fake_or_skip!();
  // pi's own model data: a gateway provider whose models set `reasoning` but no thinkingLevelMap (pi clamps to off…high)
  let pi = tempfile::tempdir().unwrap();
  let models = json!({ "providers": { "asgard": { "baseUrl": "https://gateway.example/v1", "api": "openai-completions", "models": [
    { "id": "gemini-3.8-flash", "reasoning": true }, { "id": "unlisted-9", "reasoning": true }
  ] } } });
  std::fs::write(pi.path().join("models.json"), models.to_string()).unwrap();
  let h = Harness::for_agent(&fake, "pi", json!({ "env": {
    "PI_CODING_AGENT_DIR": pi.path().to_string_lossy(),
    "FAKE_MODELS": "asgard/gemini-3.8-flash,asgard/unlisted-9",
    "FAKE_EFFORTS": "off,minimal,medium,xhigh,max",
  } }));
  let s = started(&h, "/tmp").await;
  // m1 is in neither pi's data nor the catalogue: the adapter's list stands
  assert_eq!(option_ids(&view(&s), "effort"), ["low", "high", "off", "minimal", "medium", "xhigh", "max"]);
  s.set_config("effort".into(), "max".into()).await.unwrap();
  s.select_config("model".into(), "asgard/gemini-3.8-flash".into()).await.unwrap();
  let vw = view(&s);
  let gemini = vw["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == "model").unwrap()["options"]
    .as_array()
    .unwrap()
    .iter()
    .find(|o| o["id"] == "asgard/gemini-3.8-flash")
    .cloned()
    .unwrap();
  expect_match(&gemini, json!({ "name": "gemini-3.8-flash", "source": { "id": "asgard", "kind": "custom" } }));
  // Google documents low / medium / high; max was narrowed away and corrected to a value the model offers
  assert_eq!(option_ids(&vw, "effort"), ["low", "high", "medium"]);
  assert_eq!(option_value(&vw, "effort"), "high");
  // Unknown to the catalogue: pi's own levels alone, so xhigh / max (which pi would clamp) are gone
  s.select_config("model".into(), "asgard/unlisted-9".into()).await.unwrap();
  assert_eq!(option_ids(&view(&s), "effort"), ["low", "high", "off", "minimal", "medium"]);
}
