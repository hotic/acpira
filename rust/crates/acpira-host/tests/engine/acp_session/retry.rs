//! Failed turns, retries, reconnects and empty completions

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_error_ends_the_turn_with_a_typed_error_and_retry_turn_sends_it_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "please fail").await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  assert_eq!(vw["running"], false);
  expect_absent(&vw, "error");
  assert_eq!(turns_in(&vw), 2);
  assert_eq!(vw["turns"][1]["stop"], "error");
  expect_eq(&vw["turns"][1]["error"], json!({ "message": "Upstream error: quota exhausted", "code": -32603, "kind": "upstream_error", "retryable": true }));
  s.retry_turn().await.unwrap();
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 2);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "please fail" }));
  assert_eq!(vw["turns"][1]["stop"], "end_turn");
  assert!(vw["turns"][1]["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "text"));
}

/// A failed turn that already did work is carried on by a hidden continue, never by sending the message again; a continue
/// that fails too is retried over a fresh connection on the same native session
#[tokio::test(flavor = "multi_thread")]
async fn retry_preserves_output_completed_tools_and_the_native_session_after_quota_errors() {
  let fake = fake_or_skip!();
  for edited in [false, true] {
    let native = tempfile::tempdir().unwrap();
    let h = Harness::new(&fake, json!({ "env": { "FAKE_SESSION_DIR": native.path() } }));
    let s = started(&h, "/tmp").await;
    prompt(&s, "earlier-context").await;
    let text = "fail-after-output fail-twice";
    if edited {
      prompt(&s, "original").await;
      s.edit_turn(history_edit(&s, 2, text)).await.unwrap();
      until(|| !s.is_running(), 5000).await;
    } else {
      s.prompt(text.into(), drafts(json!([{ "kind": "text", "name": "plan.txt", "text": "Retain this plan on retry." }])), false, None, None).await;
    }
    let peer = s.to_record().acp_session_id;
    let continue_text = acpira_host::i18n::t("host.retryContinuePrompt");
    for (expected, reconnects) in [("error", 0), ("end_turn", 1)] {
      let before = v(&s.to_record().turns);
      let n = before.as_array().unwrap().len();
      expect_match(before.as_array().unwrap().last().unwrap(), json!({ "stop": "error", "blocks": [
        { "type": "thought" }, { "type": "text" }, { "type": "tool_call", "status": "completed" },
      ] }));
      let (a, b) = tokio::join!(s.retry_turn(), s.retry_turn());
      // One of the racing calls may be refused; the state below is what counts
      let _ = (a, b);
      until(|| !s.is_running(), 5000).await;
      assert_eq!(s.to_record().acp_session_id, peer, "edited={edited}");
      let vw = view(&s);
      assert_eq!(json!(vw["turns"].as_array().unwrap()[..n]), before, "edited={edited}");
      assert_eq!(turns_in(&vw), n + 2);
      expect_match(turn_at(&vw, -2), json!({ "role": "user", "auto": true, "autoReason": "retry", "text": continue_text }));
      expect_match(last_turn(&vw), json!({ "stop": expected }));
      // The first retry stays on the live process; retrying the continue that failed reconnects first
      let logs = h.logs();
      assert_eq!(logs.iter().filter(|l| l.contains("retried turn failed again")).count(), reconnects, "edited={edited}: {logs:#?}");
      let restored = Disposing(AcpSession::new(s.to_record(), h.deps.clone()));
      assert_eq!(json!(view(&restored)["turns"].as_array().unwrap()[..n]), before);
    }
    if !edited {
      expect_match(turn_at(&view(&s), 2), json!({ "role": "user", "text": text, "attachments": [{ "kind": "text", "name": "plan.txt" }] }));
    }
    // The native session received the message once, then only the continues
    prompt(&s, "inspect-native-history").await;
    let echoed: Value = serde_json::from_str(last_turn(&view(&s))["blocks"][0]["markdown"].as_str().unwrap()).unwrap();
    let texts: Vec<String> = echoed["prompts"]
      .as_array()
      .unwrap()
      .iter()
      .map(|p| p.as_array().unwrap().iter().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("
"))
      .collect();
    assert_eq!(texts.iter().filter(|t| t.contains(text)).count(), 1, "edited={edited}: {texts:#?}");
    assert_eq!(texts.iter().filter(|t| **t == continue_text).count(), 2, "edited={edited}: {texts:#?}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_turn_rebuilds_attachments_from_their_blobs_and_does_nothing_after_a_normal_end() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("fail with picture".into(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode("png!"), "name": "shot.png" },
  ])), false, None, None).await;
  assert_eq!(std::fs::read_dir(h.dir.path().join("sessions").join(&s.id)).unwrap().count(), 1);
  s.retry_turn().await.unwrap();
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 2);
  let user = &vw["turns"][0];
  expect_match(user, json!({ "role": "user", "text": "fail with picture", "attachments": [{ "kind": "image", "mimeType": "image/png", "name": "shot.png" }] }));
  // The re-sent image is byte-for-byte the original
  assert_eq!(blob(&h, &s.id, user["attachments"][0]["blob"].as_str().unwrap()).unwrap(), b"png!");
  assert_eq!(vw["turns"][1]["stop"], "end_turn");
  expect_match(&vw["turns"][1]["blocks"][0], json!({ "type": "text", "markdown": "text · image:image/png" }));
  s.retry_turn().await.ok();
  assert_eq!(turn_count(&s), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_replaces_the_process_and_resumes_the_same_native_session() {
  let fake = fake_or_skip!();
  // A cwd containing "flaky-resume" makes the fake agent resume any known-or-not sessionId while no resume.lock sits in it
  let cwd = tempfile::Builder::new().prefix("acpira-flaky-resume-").tempdir().unwrap();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, cwd.path().to_str().unwrap()).await;
  s.set_config("model".into(), "m2".into()).await.unwrap();
  prompt(&s, "please fail").await;
  let record = s.to_record();
  assert_eq!(view(&s)["status"], "ready");
  expect_match(&view(&s)["turns"][1], json!({ "role": "agent", "stop": "error" }));
  s.reconnect().await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  assert_eq!(s.to_record().acp_session_id, record.acp_session_id);
  let logs = h.logs();
  assert_eq!(logs.iter().filter(|l| l.contains("spawn ")).count(), 2, "{logs:#?}");
  assert!(logs.iter().any(|l| l.contains("session/resume ok")));
  assert_eq!(turns_in(&vw), 2);
  expect_match(&vw["turns"][1], json!({ "role": "agent", "stop": "error" }));
  assert_eq!(option_value(&vw, "model"), "m2");
  // Not retryTurn: the fake's per-process "fail once" map resets on the respawn, so resending 'please fail' would fail again
  prompt(&s, "hi").await;
  assert_eq!(turn_count(&s), 4);
  expect_match(&view(&s)["turns"][3], json!({ "role": "agent", "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_is_refused_while_a_turn_runs_and_the_process_is_kept() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["running"] == true, 5000).await;
  assert!(s.reconnect().await.is_err());
  assert_eq!(h.logs().iter().filter(|l| l.contains("spawn ")).count(), 1);
  assert_eq!(view(&s)["running"], true);
  s.cancel().await;
  p.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_completion_reports_missing_output_and_retries_on_the_same_native_session() {
  let fake = fake_or_skip!();
  for text in ["empty-response", "empty-response-whitespace"] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    let native = s.to_record().acp_session_id;
    prompt(&s, text).await;
    assert!(h.logs().iter().any(|l| l.contains("prompt done: end_turn")), "{text}");
    expect_match(view(&s), json!({ "status": "ready", "running": false }));
    let last = last_turn(&view(&s));
    expect_match(&last, json!({ "stop": "error", "error": { "kind": "empty_response", "retryable": true } }));
    assert!(last["error"]["message"].as_str().unwrap().contains("no reply"));
    expect_absent(&last, "error.code");
    expect_match(s.to_record().turns.last().unwrap(), json!({ "stop": "error" }));
    s.retry_turn().await.unwrap();
    assert_eq!(s.to_record().acp_session_id, native);
    assert_eq!(turn_count(&s), 2);
    expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  }
}

/// Kimi answers a failed turn with an empty end_turn and logs the cause in its own session store: the card shows it
#[tokio::test(flavor = "multi_thread")]
async fn a_kimi_empty_completion_reports_the_failure_from_its_session_log() {
  let fake = fake_or_skip!();
  let kimi_home = tempfile::tempdir().unwrap();
  let home = kimi_home.path().to_string_lossy().into_owned();
  let h = Harness::for_agent(&fake, "kimi", json!({ "env": { "KIMI_CODE_HOME": home } }));
  let s = started(&h, "/tmp").await;
  let native = s.to_record().acp_session_id.unwrap();
  let dir = kimi_home.path().join("sessions").join("wd_tmp_0123abcd").join(&native).join("agents").join("main");
  std::fs::create_dir_all(&dir).unwrap();
  // Stamped ahead of the prompt so it counts as this turn's end; an older line would belong to the previous turn
  let ended = json!({
    "type": "turn.ended", "agentId": "main", "turnId": 0, "reason": "failed",
    "error": { "code": "provider.api_error", "message": "400 unsupported Ollama model: deepseek-v4-flash", "name": "APIStatusError", "retryable": false },
    "time": acpira_host::util::now_ms() + 60_000,
  });
  std::fs::write(dir.join("wire.jsonl"), format!("{ended}\n")).unwrap();
  prompt(&s, "empty-response").await;
  expect_match(view(&s), json!({ "status": "ready", "running": false }));
  let last = last_turn(&view(&s));
  expect_match(
    &last,
    json!({ "stop": "error", "error": { "message": "400 unsupported Ollama model: deepseek-v4-flash", "kind": "provider.api_error", "retryable": false } }),
  );
  assert!(h.logs().iter().any(|l| l.contains("from the CLI's session log")));
  // Nothing logged for a later turn (only an older line): the generic card
  std::fs::write(dir.join("wire.jsonl"), format!("{}\n", json!({ "type": "turn.ended", "agentId": "main", "reason": "failed", "time": 1 }))).unwrap();
  prompt(&s, "empty-response-whitespace").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "error", "error": { "kind": "empty_response", "retryable": true } }));
}

/// pi-acp settles a turn whose model call failed with an empty end_turn; pi wrote the failed reply to its session file
#[tokio::test(flavor = "multi_thread")]
async fn a_pi_empty_completion_reports_the_error_from_its_session_file() {
  let fake = fake_or_skip!();
  let agent_dir = tempfile::tempdir().unwrap();
  let dir = agent_dir.path().to_string_lossy().into_owned();
  let h = Harness::for_agent(&fake, "pi", json!({ "env": { "PI_CODING_AGENT_DIR": dir } }));
  let s = started(&h, "/tmp").await;
  let native = s.to_record().acp_session_id.unwrap();
  // pi's default directory for cwd /tmp
  let sessions = agent_dir.path().join("sessions").join("--tmp--");
  std::fs::create_dir_all(&sessions).unwrap();
  let reply = json!({
    "type": "message", "id": "b", "parentId": "a",
    "message": { "role": "assistant", "content": [], "stopReason": "error", "errorMessage": "502: upstream connection closed", "timestamp": acpira_host::util::now_ms() + 60_000 },
  });
  std::fs::write(sessions.join(format!("2026-10-09T00-00-00-000Z_{native}.jsonl")), format!("{reply}\n")).unwrap();
  prompt(&s, "empty-response").await;
  expect_match(view(&s), json!({ "status": "ready", "running": false }));
  let last = last_turn(&view(&s));
  expect_match(&last, json!({ "stop": "error", "error": { "message": "502: upstream connection closed" } }));
  expect_absent(&last, "error.kind");
}
