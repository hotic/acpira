//! Claude's host-made Ultra effort level: the last option of the effort select, applied as ultracode through
//! `_meta.claudeCode.options.settings` on a same-process session/resume plus wire effort `xhigh` (fake `FAKE_REBUILD`
//! mimics claude-agent-acp 0.87.0 recreating the query and resetting model / effort / Fast)

use super::*;

use acpira_shared::transcript::TurnSettings;

const ULTRA: &str = "ultra";

/// A claude-vendor harness whose workflow settings come from an empty config dir, plus the meta and config logs. The
/// effort select offers low, high, xhigh and max unless `extra_env` says otherwise
fn claude(fake: &FakeAgent, dir: &std::path::Path, extra_env: Value) -> Harness {
  let config_dir = dir.join("claude-config");
  std::fs::create_dir_all(&config_dir).unwrap();
  let mut env = json!({
    "FAKE_META_LOG": dir.join("meta.log"), "FAKE_CONFIG_LOG": dir.join("config.log"), "FAKE_REBUILD": "1",
    "FAKE_EFFORTS": "xhigh,max", "CLAUDE_CONFIG_DIR": config_dir, "CLAUDE_CODE_WORKFLOWS": "1",
  });
  for (k, v) in extra_env.as_object().into_iter().flatten() {
    env[k] = v.clone();
  }
  Harness::for_agent(fake, "claude", json!({ "env": env }))
}

fn lines(path: &std::path::Path) -> Vec<Value> {
  std::fs::read_to_string(path).unwrap_or_default().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn settings_of(entry: &Value) -> Value {
  entry["meta"]["claudeCode"]["options"]["settings"].clone()
}

fn ids(view: &Value) -> Vec<String> {
  view["controls"]["options"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap().to_owned()).collect()
}

fn efforts(view: &Value) -> Vec<String> {
  let effort = view["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == "effort").cloned().unwrap_or(Value::Null);
  effort["options"].as_array().into_iter().flatten().map(|o| o["id"].as_str().unwrap().to_owned()).collect()
}

/// The effort values that went out as session/set_config_option
fn wire_efforts(dir: &std::path::Path) -> Vec<String> {
  lines(&dir.join("config.log")).iter().filter(|w| w["configId"] == "effort").map(|w| w["value"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn ultra_is_the_last_effort_level_and_rebuilds_the_live_session_with_xhigh() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({}));
  let cwd = dir.path().to_str().unwrap();
  let s = started(&h, cwd).await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  // No control of its own: Ultra is the effort select's last option, named like Codex's real level
  assert_eq!(ids(&vw), ["model", "effort"]);
  assert_eq!(efforts(&vw), ["low", "high", "xhigh", "max", ULTRA]);
  let effort = vw["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == "effort").cloned().unwrap();
  assert_eq!(effort["options"].as_array().unwrap().last().unwrap()["name"], "Ultra");
  let meta_log = dir.path().join("meta.log");
  let first = lines(&meta_log);
  assert_eq!(first.len(), 1);
  assert_eq!(first[0]["method"], "new");
  assert!(settings_of(&first[0]).is_null(), "a session without Ultra sends no settings");
  assert!(first[0]["meta"]["claudeCode"]["options"]["thinking"].is_object(), "thinking still rides along");

  s.select_config("effort".into(), "low".into()).await.unwrap();
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  let vw = view(&s);
  assert_eq!(option_value(&vw, "effort"), ULTRA);
  assert_eq!(vw["controls"]["modeId"], "agent");
  let meta = lines(&meta_log);
  assert_eq!(meta.len(), 2);
  assert_eq!(meta[1]["method"], "resume");
  assert_eq!(settings_of(&meta[1])["ultracode"], true);
  assert!(meta[1]["meta"]["claudeCode"]["options"]["thinking"].is_object());
  assert!(meta[1]["meta"]["claudeCode"]["emitRawSDKMessages"].is_array());
  // Ultra never went out; the rebuild reset the effort to high and the replay set Ultra's wire effort
  assert_eq!(wire_efforts(dir.path()), ["low", "xhigh"]);
  // The agent answered xhigh, yet the select still shows Ultra, and that is what a record or remembered prefs keep
  assert_eq!(s.agent_controls().options.iter().find(|o| o.id == "effort").and_then(|o| o.value.clone()).as_deref(), Some(ULTRA));
  assert_eq!(s.to_record().controls.options.iter().find(|o| o.id == "effort").and_then(|o| o.value.clone()).as_deref(), Some(ULTRA));

  // Ultra again changes nothing
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  assert_eq!(lines(&meta_log).len(), 2);
  assert_eq!(wire_efforts(dir.path()).len(), 2);

  // Any other level leaves Ultra: a rebuild without the settings, then that level on the wire
  s.select_config("effort".into(), "low".into()).await.unwrap();
  let meta = lines(&meta_log);
  assert_eq!(meta.len(), 3);
  assert_eq!(meta[2]["method"], "resume");
  assert!(settings_of(&meta[2]).is_null(), "leaving Ultra sends no settings");
  assert_eq!(wire_efforts(dir.path()), ["low", "xhigh", "low"]);
  let vw = view(&s);
  assert_eq!(option_value(&vw, "effort"), "low");
  assert_eq!(efforts(&vw), ["low", "high", "xhigh", "max", ULTRA], "Ultra stays on offer");
  // A level while Ultra is off is a plain set_config_option
  s.select_config("effort".into(), "high".into()).await.unwrap();
  assert_eq!(lines(&meta_log).len(), 3);
  assert_eq!(wire_efforts(dir.path()).last().map(String::as_str), Some("high"));
  prompt(&s, "hello").await;
  assert_eq!(last_turn(&view(&s))["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_xhigh_ultra_takes_the_highest_level_the_model_offers() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({ "FAKE_EFFORTS": "max" }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  assert_eq!(efforts(&view(&s)), ["low", "high", "max", ULTRA]);
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), ULTRA);
  assert_eq!(wire_efforts(dir.path()), ["max"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pick_during_a_turn_waits_for_the_turn_to_end_and_goes_before_the_queue() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({ "FAKE_SLOW_STEP_MS": "10" }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  let meta_log = dir.path().join("meta.log");
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["running"] == true, 5000).await;
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), ULTRA, "shown at once");
  assert_eq!(lines(&meta_log).len(), 1, "no rebuild while the turn runs");
  assert!(wire_efforts(dir.path()).is_empty(), "the wire effort waits for the rebuild too");
  prompt(&s, "after").await;
  p.await.unwrap();
  until(|| lines(&meta_log).len() == 2, 5000).await;
  assert_eq!(settings_of(&lines(&meta_log)[1])["ultracode"], true);
  until(|| turn_count(&s) == 4 && view(&s)["running"] == false, 5000).await;
  assert_eq!(option_value(&view(&s), "effort"), ULTRA);
  assert_eq!(wire_efforts(dir.path()), ["xhigh"]);

  // Leaving Ultra mid-turn: the picked level shows at once and the rebuild sets it, a later pick replacing it
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["running"] == true, 5000).await;
  s.select_config("effort".into(), "high".into()).await.unwrap();
  s.select_config("effort".into(), "low".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  assert_eq!(lines(&meta_log).len(), 2);
  p.await.unwrap();
  prompt(&s, "after").await;
  let meta = lines(&meta_log);
  assert_eq!(meta.len(), 3);
  assert!(settings_of(&meta[2]).is_null());
  assert_eq!(wire_efforts(dir.path()), ["xhigh", "low"]);
  assert_eq!(option_value(&view(&s), "effort"), "low");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_remembered_ultra_rides_the_first_request_without_a_rebuild() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({ "CLAUDE_MODEL_CONFIG": r#"{"availableModels":["m1"],"modelOverrides":{"m1":"m1-x"}}"# }));
  let s = Disposing(h.session(dir.path().to_str().unwrap()));
  let settings = TurnSettings { mode_id: None, config: [("effort".to_owned(), ULTRA.to_owned())].into() };
  s.hold_settings(&settings);
  s.start().await;
  s.adopt_controls(settings).await;
  let meta = lines(&dir.path().join("meta.log"));
  assert_eq!(meta.len(), 1, "matching value: no rebuild");
  assert_eq!(meta[0]["method"], "new");
  // The settings tier replaces the adapter's CLAUDE_MODEL_CONFIG settings, so its keys travel along
  expect_eq(settings_of(&meta[0]), json!({ "ultracode": true, "availableModels": ["m1"], "modelOverrides": { "m1": "m1-x" } }));
  assert_eq!(option_value(&view(&s), "effort"), ULTRA);
  assert_eq!(wire_efforts(dir.path()), ["xhigh"], "the replay sets Ultra's wire effort, never Ultra itself");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reopened_session_asks_for_its_own_ultracode_on_resume() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({ "FAKE_SESSION_DIR": dir.path().join("native") }));
  std::fs::create_dir_all(dir.path().join("native")).unwrap();
  let s = started(&h, dir.path().to_str().unwrap()).await;
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  let record = s.to_record();
  drop(s);
  let again = Disposing(AcpSession::new(record, h.deps.clone()));
  again.start().await;
  assert_eq!(view(&again)["status"], "ready");
  let meta = lines(&dir.path().join("meta.log"));
  let last = meta.last().unwrap();
  assert_ne!(last["method"], "new");
  assert_eq!(settings_of(last)["ultracode"], true);
  assert_eq!(option_value(&view(&again), "effort"), ULTRA);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_switch_keeps_ultra_while_the_model_takes_an_effort_and_leaves_it_otherwise() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({ "FAKE_MODEL_RESETS_EFFORT": "1", "FAKE_NO_EFFORT_MODEL": "m3", "FAKE_MODELS": "m3" }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  let meta_log = dir.path().join("meta.log");
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  assert_eq!(lines(&meta_log).len(), 2);
  // m2 takes an effort too: Ultra stays, and its wire effort is set again over the switch's reset
  s.select_config("model".into(), "m2".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), ULTRA);
  assert_eq!(lines(&meta_log).len(), 2, "no rebuild");
  assert_eq!(wire_efforts(dir.path()), ["xhigh", "xhigh"]);
  // m3 takes none: ultracode goes off with a rebuild, and the model stays m3
  s.select_config("model".into(), "m3".into()).await.unwrap();
  let meta = lines(&meta_log);
  assert_eq!(meta.len(), 3);
  assert!(settings_of(&meta[2]).is_null());
  let vw = view(&s);
  assert_eq!(option_value(&vw, "model"), "m3");
  assert!(!ids(&vw).iter().any(|id| id == "effort"));
  // Back on a model with an effort: its own level shows, Ultra is on offer again
  s.select_config("model".into(), "m1".into()).await.unwrap();
  let vw = view(&s);
  assert_eq!(option_value(&vw, "effort"), "high");
  assert_eq!(efforts(&vw).last().map(String::as_str), Some(ULTRA));
  assert_eq!(lines(&meta_log).len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_ultra_without_claude_or_with_workflows_off() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let other = Harness::new(&fake, json!({ "env": { "FAKE_EFFORTS": "xhigh" } }));
  let s = started(&other, "/tmp").await;
  assert_eq!(efforts(&view(&s)), ["low", "high", "xhigh"]);
  let off = claude(&fake, dir.path(), json!({ "CLAUDE_CODE_WORKFLOWS": "0" }));
  let s = started(&off, dir.path().to_str().unwrap()).await;
  assert_eq!(view(&s)["status"], "ready");
  assert_eq!(efforts(&view(&s)), ["low", "high", "xhigh", "max"]);
  // A stray Ultra pick changes nothing and sends nothing
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  s.set_config("effort".into(), ULTRA.into()).await.unwrap();
  assert_eq!(lines(&dir.path().join("meta.log")).len(), 1);
  assert!(wire_efforts(dir.path()).is_empty());
}

fn async_task(s: &AcpSession) -> Value {
  agent_blocks(&view(s)).into_iter().find(|b| b["type"] == "tool_call" && b["id"] == "bg-1").map(|b| b["asyncTask"].clone()).unwrap_or(Value::Null)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pick_waits_while_background_work_runs_and_lands_before_the_next_prompt() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({}));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  let meta_log = dir.path().join("meta.log");
  // The turn returns with a background task still running, the way a workflow outlives its prompt
  prompt(&s, "async-stop").await;
  assert_eq!(async_task(&s)["state"], "running");
  s.select_config("effort".into(), ULTRA.into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), ULTRA, "shown at once");
  assert_eq!(lines(&meta_log).len(), 1, "a rebuild would tear the running task down");
  // A prompt while the task still runs goes out without the change
  prompt(&s, "while busy").await;
  assert_eq!(lines(&meta_log).len(), 1);
  s.stop_async_task("task-1").await.unwrap();
  until(|| async_task(&s)["state"] == "stopped", 8000).await;
  assert_eq!(lines(&meta_log).len(), 1, "nothing rebuilds until a prompt needs it");
  prompt(&s, "after").await;
  let meta = lines(&meta_log);
  assert_eq!(meta.len(), 2);
  assert_eq!(meta[1]["method"], "resume");
  assert_eq!(settings_of(&meta[1])["ultracode"], true);
  assert_eq!(wire_efforts(dir.path()), ["xhigh"]);
  assert_eq!(last_turn(&view(&s))["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_continuing_natively_rebuilds_with_the_edited_ultra_first() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({}));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  let meta_log = dir.path().join("meta.log");
  prompt(&s, "first").await;
  let mut edit = history_edit(&s, 0, "first again");
  edit.intent = Some(serde_json::from_value(json!("continue")).unwrap());
  edit.settings.config.insert("effort".into(), ULTRA.into());
  s.edit_turn(edit).await.unwrap();
  until(|| view(&s)["turns"][0]["text"] == "first again" && view(&s)["running"] == false && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_eq!(turn_count(&s), 2, "the edited prompt replaces the original on screen");
  let meta = lines(&meta_log);
  assert_eq!(meta.len(), 2, "the native session is rebuilt once, ahead of the edited prompt");
  assert_eq!(meta[1]["method"], "resume");
  assert_eq!(settings_of(&meta[1])["ultracode"], true);
  assert_eq!(option_value(&view(&s), "effort"), ULTRA);
  let wire = wire_efforts(dir.path());
  assert!(!wire.is_empty() && wire.iter().all(|v| v == "xhigh"), "only Ultra's wire effort went out: {wire:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_rebuild_reverts_the_effort_and_brings_the_native_session_back() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({ "FAKE_REBUILD_FAIL": "ultracode" }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  s.select_config("effort".into(), "low".into()).await.unwrap();
  assert!(s.select_config("effort".into(), ULTRA.into()).await.is_err());
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  assert_eq!(option_value(&vw, "effort"), "low", "the effort goes back, and the restored session gets it again");
  let meta = lines(&dir.path().join("meta.log"));
  assert_eq!(meta.iter().map(|m| m["method"].as_str().unwrap()).collect::<Vec<_>>(), ["new", "resume", "resume"]);
  assert_eq!(settings_of(&meta[1])["ultracode"], true);
  assert!(settings_of(&meta[2]).is_null(), "the restore asks for the old settings");
  assert_eq!(wire_efforts(dir.path()), ["low", "low"]);
  prompt(&s, "hello").await;
  assert_eq!(last_turn(&view(&s))["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rebuild_whose_restore_fails_too_leaves_the_session_for_retry() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = claude(&fake, dir.path(), json!({ "FAKE_REBUILD_FAIL": "always" }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  assert!(s.select_config("effort".into(), ULTRA.into()).await.is_err());
  let vw = view(&s);
  assert_eq!(vw["status"], "error");
  assert_eq!(lines(&dir.path().join("meta.log")).len(), 3);
}
