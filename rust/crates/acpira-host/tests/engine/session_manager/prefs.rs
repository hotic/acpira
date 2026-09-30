//! Remembered picks and per-model parameter shapes in prefs.json

use super::*;

fn option_values(view: &Value) -> Vec<Value> {
  view["controls"]["options"].as_array().unwrap().iter().map(|c| c["value"].clone()).collect()
}

// Every pick saves prefs.json in the background: a burst of picks must leave the last one on disk once dispose returns, never an
// older snapshot that finished writing late
#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_picks_leaves_the_last_one_on_disk_after_dispose() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  for i in 0..20 {
    let effort = if i % 2 == 0 { "low" } else { "high" };
    m.handle(json!({ "type": "setConfig", "configId": "effort", "value": effort })).await;
  }
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "low" })).await;
  m.dispose().await;
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  assert_eq!(store.load_prefs().await.last_settings["fake"].config["effort"], "low");
}

// The fake agent's process starts every session on model m1 / effort high / mode agent; the option values and the mode picked last
// come back on the next new session. Only the user's own picks count: a mode the agent switches by itself is not a choice
#[tokio::test(flavor = "multi_thread")]
async fn the_last_chosen_config_values_and_picked_mode_are_remembered_per_agent_and_replayed() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::fake(&fake));
  m.init().await;
  m.new_session(None).await;
  assert_eq!(option_values(&m.active().unwrap()), [json!("m1"), json!("high")]);
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "low" })).await;
  assert_eq!(v(m.m.last_settings("fake")), json!({ "config": { "model": "m2", "effort": "low" } }));
  m.handle(json!({ "type": "setMode", "id": "plan" })).await;
  assert_eq!(v(m.m.last_settings("fake")), json!({ "modeId": "plan", "config": { "model": "m2", "effort": "low" } }));
  // a mode the agent switches by itself is that session's business — the memory keeps what the user picked, and a later config
  // pick must not overwrite it either
  m.handle(json!({ "type": "send", "text": "mode:agent" })).await;
  assert_eq!(m.active().unwrap()["controls"]["modeId"], "agent");
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "high" })).await;
  assert_eq!(v(m.m.last_settings("fake")), json!({ "modeId": "plan", "config": { "model": "m2", "effort": "high" } }));
  m.new_session(None).await;
  assert_eq!(option_values(&m.active().unwrap()), [json!("m2"), json!("high")]);
  assert_eq!(m.active().unwrap()["controls"]["modeId"], "plan");
  // the applied choices are what the new session's first turn records
  m.handle(json!({ "type": "send", "text": "inspect-history" })).await;
  let markdown = last_turn(&m.active().unwrap())["blocks"].to_string();
  assert!(markdown.contains("\\\"model\\\":\\\"m2\\\"") && markdown.contains("\\\"mode\\\":\\\"plan\\\""), "{markdown}");
  m.dispose().await;

  // reload: the memory is on disk; a stale value (no longer in the agent's list) is passed over while the others still apply
  let store = TranscriptStore::new(dir.path().to_path_buf(), Arc::new(|_: &str| {}), None);
  let mut prefs = store.load_prefs().await;
  assert_eq!(v(&prefs.last_settings["fake"]), json!({ "modeId": "plan", "config": { "model": "m2", "effort": "high" } }));
  prefs.last_settings.insert("fake".into(), serde_json::from_value(json!({ "modeId": "plan", "config": { "model": "gone", "effort": "low" } })).unwrap());
  store.save_prefs(&prefs, &["fake".into()]).await.unwrap();
  let m2 = Mgr::new(dir.path(), Opts::fake(&fake));
  m2.init().await;
  m2.new_session(None).await;
  assert_eq!(option_values(&m2.active().unwrap()), [json!("m1"), json!("low")]);
  assert_eq!(m2.active().unwrap()["controls"]["modeId"], "plan");
  // a remembered mode the agent no longer offers is skipped like any other stale value
  m2.dispose().await;
  prefs.last_settings.insert("fake".into(), serde_json::from_value(json!({ "modeId": "gone", "config": {} })).unwrap());
  store.save_prefs(&prefs, &["fake".into()]).await.unwrap();
  let m3 = Mgr::new(dir.path(), Opts::fake(&fake));
  m3.init().await;
  m3.new_session(None).await;
  assert_eq!(m3.active().unwrap()["controls"]["modeId"], "agent");
  m3.dispose().await;
}

fn shape_ids(shape: &Value) -> Vec<String> {
  shape.as_array().map(|a| a.iter().map(|c| format!("{}:{}", c["id"].as_str().unwrap(), c["options"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect::<Vec<_>>().join("|"))).collect()).unwrap_or_default()
}

async fn shape_keys(dir: &Path) -> Vec<String> {
  let store = TranscriptStore::new(dir.to_path_buf(), Arc::new(|_: &str| {}), None);
  let mut keys: Vec<String> = store.load_prefs().await.model_shapes.and_then(|s| s.get("fake").map(|x| v(x).as_object().unwrap().keys().cloned().collect())).unwrap_or_default();
  keys.sort();
  keys
}

// The history editor switches models locally; what each model really offers (Devin: SWE-2 drops `speed`) is learned from live
// switches, carried on the view and kept in prefs.json for the next host
#[tokio::test(flavor = "multi_thread")]
async fn each_models_parameters_are_learned_from_live_switches_and_carried_on_the_view() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let opts = || Opts::with_agents(fake.setting(json!({ "env": { "FAKE_SPEED": "1", "FAKE_MODELS": "x3" } })), "fake");
  let m = Mgr::new(dir.path(), opts());
  m.init().await;
  m.new_session(None).await;
  // The shape is learned from the ready session on its next flush
  until(|| m.active().is_some_and(|a| !a["modelShapes"]["m1"].is_null()), 2000).await;
  assert_eq!(shape_ids(&m.active().unwrap()["modelShapes"]["m1"]), ["effort:low|high", "speed:standard|fast"]);
  let before = m.active().unwrap()["modelShapes"].clone();
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "low" })).await;
  // A value change is not a new shape
  assert_eq!(m.active().unwrap()["modelShapes"], before);
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  until(|| m.active().is_some_and(|a| !a["modelShapes"]["m2"].is_null()), 2000).await;
  assert_eq!(shape_ids(&m.active().unwrap()["modelShapes"]["m2"]), ["effort:high"]);
  // prefs writes are fire-and-forget
  let t0 = std::time::Instant::now();
  while shape_keys(dir.path()).await != ["m1", "m2"] {
    assert!(t0.elapsed() < std::time::Duration::from_secs(5), "shapes never reached prefs.json");
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
  }
  m.dispose().await;
  let next = Mgr::new(dir.path(), opts());
  let other = Mgr::new(dir.path(), opts());
  next.init().await;
  next.new_session(None).await;
  until(|| next.active().is_some_and(|a| a["modelShapes"].as_object().is_some_and(|o| o.len() >= 2)), 2000).await;
  let mut keys: Vec<String> = next.active().unwrap()["modelShapes"].as_object().unwrap().keys().cloned().collect();
  keys.sort();
  assert_eq!(keys, ["m1", "m2"]);
  // Another window that was already running picks up a shape learned here on focus / webview ready, with a newer rev
  other.init().await;
  other.new_session(None).await;
  let rev = other.active().unwrap()["rev"].as_i64().unwrap();
  next.handle(json!({ "type": "setConfig", "configId": "model", "value": "x3" })).await;
  let t0 = std::time::Instant::now();
  while !shape_keys(dir.path()).await.contains(&"x3".to_owned()) {
    assert!(t0.elapsed() < std::time::Duration::from_secs(5));
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
  }
  other.m.refresh_index().await;
  until(|| other.active().is_some_and(|a| !a["modelShapes"]["x3"].is_null()), 2000).await;
  assert_eq!(shape_ids(&other.active().unwrap()["modelShapes"]["x3"]), ["effort:high"]);
  assert!(other.active().unwrap()["rev"].as_i64().unwrap() > rev);
  next.dispose().await;
  other.dispose().await;
}

// A new session shows its remembered choices from the first frame: session/new's defaults (m1 / high / agent) never reach the
// screen while a slow agent replays them one request at a time, and a prompt sent meanwhile waits for the replay instead of
// running under the defaults
#[tokio::test(flavor = "multi_thread")]
async fn remembered_choices_show_from_the_first_frame_and_a_prompt_waits_for_the_replay() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let m = Mgr::new(dir.path(), Opts::with_agents(fake.setting(json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150" } })), "fake"));
  m.init().await;
  m.new_session(None).await;
  m.handle(json!({ "type": "setConfig", "configId": "model", "value": "m2" })).await;
  m.handle(json!({ "type": "setConfig", "configId": "effort", "value": "low" })).await;
  m.handle(json!({ "type": "setMode", "id": "plan" })).await;
  // a turn keeps the first session out of the empty-session cleanup, so its controls stay known
  m.handle(json!({ "type": "send", "text": "hello" })).await;
  let first = m.active_id().unwrap();
  m.events.lock().unwrap().clear();

  let (mgr, viewer) = (m.m.clone(), m.v.clone());
  let opening = tokio::spawn(async move { mgr.new_session_for(&viewer, None, None).await.unwrap() });
  until(|| m.active_id().is_some_and(|id| id != first) && m.active().is_some_and(|a| a["status"] == "ready"), 5000).await;
  // the replay is still in flight here: the prompt queues behind it
  m.handle(json!({ "type": "send", "text": "inspect-history" })).await;
  opening.await.unwrap();
  until(|| m.active().is_some_and(|a| a["running"] == false && turns_len(Some(a.clone())) == 2), 5000).await;

  let id = m.active_id().unwrap();
  let frames: Vec<Value> =
    m.events.lock().unwrap().iter().filter(|e| e["type"] == "session" && e["session"]["id"] == id.as_str()).map(|e| e["session"].clone()).collect();
  assert!(frames.iter().any(|f| f["status"] == "starting" && f["controls"]["modeId"] == "plan"), "the mode was not painted before session/new");
  for f in &frames {
    let options = f["controls"]["options"].as_array().cloned().unwrap_or_default();
    if !options.is_empty() {
      assert_eq!(option_values(f), [json!("m2"), json!("low")], "status {}", f["status"]);
    }
    if f["controls"]["modes"].as_array().is_some_and(|m| !m.is_empty()) {
      assert_eq!(f["controls"]["modeId"], "plan", "status {}", f["status"]);
    }
  }
  // the queued prompt ran under the replayed choices
  let markdown = last_turn(&m.active().unwrap())["blocks"].to_string();
  assert!(markdown.contains("\\\"model\\\":\\\"m2\\\"") && markdown.contains("\\\"mode\\\":\\\"plan\\\""), "{markdown}");
  m.dispose().await;
}
