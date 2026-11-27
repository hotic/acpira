//! Settings reaching the built-in agent: effort scales, raw settings, the machine that sent the turn

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn switching_between_disjoint_effort_scales_sends_the_level_shown_by_the_controls() {
  let env = Env::new();
  let path = env.root.path().join("providers.json");
  let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  file["providers"][0]["models"] = json!([
    { "id": "qwen-next", "efforts": ["medium", "xhigh"] },
    { "id": "gemini-3.8-flash", "efforts": ["low", "medium", "high"] }
  ]);
  std::fs::write(path, file.to_string()).unwrap();
  let s = env.session().await;
  let effort = || s.agent_controls().options.iter().find(|o| o.id == "effort").and_then(|o| o.value.clone()).unwrap();
  assert_eq!(effort(), "xhigh");
  // A stale High from the previous menu is harmless, not an Unknown effort RPC failure.
  s.select_config("effort".into(), "high".into()).await.unwrap();
  assert_eq!(effort(), "xhigh");
  s.select_config("model".into(), "mock/gemini-3.8-flash".into()).await.unwrap();
  assert_eq!(effort(), "low");
  s.select_config("effort".into(), "high".into()).await.unwrap();
  env.server.push(mock::text("Hello."));
  run(&s, "hello").await;
  assert_eq!(env.server.requests()[0].body["reasoning_effort"], "high");
  expect_match(&view(&s)["turns"][0]["settings"], json!({ "config": { "model": "mock/gemini-3.8-flash", "effort": "high" } }));
  s.select_config("effort".into(), "medium".into()).await.unwrap();
  s.select_config("model".into(), "mock/qwen-next".into()).await.unwrap();
  assert_eq!(effort(), "medium", "a supported effort survives a model switch");
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_settings_reach_existing_sessions_without_reopening_them() {
  let env = Env::new();
  let s = env.session().await;
  let other = env.session().await;
  let native_id = s.to_record().acp_session_id;
  let has_raw = |s: &AcpSession| view(s)["controls"]["modes"].as_array().unwrap().iter().any(|m| m["id"] == "raw");
  assert!(!has_raw(&s) && !has_raw(&other));
  let path = env.root.path().join("providers.json");
  let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
  file["rawMode"] = json!(true);
  file["rawSystemPrompt"] = json!("Answer briefly.");
  std::fs::write(&path, file.to_string()).unwrap();
  until(|| has_raw(&s) && has_raw(&other), 5000).await;
  assert_eq!(view(&s)["controls"]["modeId"], "agent", "showing Raw must not select it");
  s.set_mode("raw".into()).await.unwrap();
  env.server.push(mock::text("Hello."));
  run(&s, "hello").await;
  assert_eq!(env.server.requests()[0].body["messages"][0], json!({ "role": "system", "content": "Answer briefly." }));

  file["rawMode"] = json!(false);
  file["rawSystemPrompt"] = json!("Answer in French.");
  std::fs::write(&path, file.to_string()).unwrap();
  until(|| !has_raw(&other), 5000).await;
  assert!(has_raw(&s), "a session already in Raw keeps its mode");
  assert_eq!(view(&s)["controls"]["modeId"], "raw");
  env.server.push(mock::text("Bonjour."));
  run(&s, "again").await;
  let requests = env.server.requests();
  assert_eq!(requests[1].body["messages"][0], json!({ "role": "system", "content": "Answer in French." }));
  assert!(requests[1].body.get("tools").is_none());
  assert_eq!(s.to_record().acp_session_id, native_id, "the same native conversation continues");
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 4);
  s.set_mode("agent".into()).await.unwrap();
  until(|| !has_raw(&s), 5000).await;
  env.assert_prefix_stable(&env.log_of(&s));
}

/// "Whoever sends decides": a session started or last sent from another machine reads the built-in agent settings that
/// machine keeps here (`remote/host_models.rs`), its picker and its turns alike; a turn sent from a window here goes back
/// to this machine's own. The pick is saved with the record
#[tokio::test(flavor = "multi_thread")]
async fn a_session_reads_the_settings_of_the_machine_that_sent_its_turn() {
  use acpira_host::acp::session::SentFrom;
  let env = Env::new();
  let set = acpira_shared::providers::client_settings_dir(env.root.path(), "mac").unwrap();
  std::fs::create_dir_all(&set).unwrap();
  let theirs = json!({ "version": 1, "fastModel": "off", "approval": "full", "providers": [{ "id": "cs", "name": "Client", "format": "openai-chat",
    "baseUrl": env.server.base_url(), "models": [{ "id": "c1", "context": 64000 }] }] });
  std::fs::write(set.join("secrets.json"), json!({ "acpira.provider.cs": "sk-client" }).to_string()).unwrap();
  std::fs::write(set.join("providers.json"), theirs.to_string()).unwrap();
  let picker = |s: &AcpSession| {
    let controls = s.agent_controls();
    let find = |id: &str| controls.options.iter().find(|o| o.id == id).cloned();
    let models: Vec<String> = find("model").map(|m| m.options.iter().map(|o| o.id.clone()).collect()).unwrap_or_default();
    (models, find("approval").and_then(|a| a.value).unwrap_or_default())
  };

  // The manager's policy always names a compaction setting: it rides the same `_meta` as the sender
  let mut deps = env.deps.clone();
  deps.policy = Arc::new(crate::support::TestPolicy {
    compaction: Some(Arc::new(|| acpira_host::acp::session::CompactionPolicy { at_tokens: 0.0, auto: false })),
    ..Default::default()
  });
  let s = Disposing(AcpSession::fresh("acpira", &env.cwd().to_string_lossy(), deps, None));
  s.sent_by(&SentFrom::Machine("mac".into()));
  s.adopt_sender();
  s.start().await;
  let own = env.session().await;
  assert_eq!(picker(&s), (vec!["cs/c1".to_owned()], "full".to_owned()));
  assert_eq!(picker(&own).0, ["mock/m1"]);
  env.server.push(mock::text("From there."));
  run(&s, "hello").await;
  assert_eq!(env.server.requests()[0].body["model"], "c1");
  assert_eq!(s.to_record().settings_from.as_deref(), Some("mac"));

  // Sent from a window here: this machine's own, from the turn the user starts next. One Acpira starts on its own
  // (the /compact ahead of a prompt over budget) goes on with what the session reads
  s.sent_by(&SentFrom::Here);
  assert_eq!(s.to_record().settings_from.as_deref(), Some("mac"), "noted until a turn starts");
  env.server.push(mock::text("Summary."));
  s.compact(true).await.unwrap();
  assert_eq!(env.server.requests()[1].body["model"], "c1");
  assert_eq!(s.to_record().settings_from.as_deref(), Some("mac"), "an automatic turn leaves the sender pending");
  env.server.push(mock::text("From here."));
  run(&s, "again").await;
  assert_eq!(env.server.requests()[2].body["model"], "m1");
  until(|| picker(&s).0 == ["mock/m1"], 5000).await;
  assert_eq!(s.to_record().settings_from, None);
  // An action whose sender is not known changes nothing
  s.sent_by(&SentFrom::Unknown);
  assert_eq!(s.to_record().settings_from, None);
}
