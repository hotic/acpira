//! Rebound local sessions retain their transcript, title and first-prompt history across retries and edits.

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_without_restore_capabilities_gets_a_fresh_peer_and_retained_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_NO_RESTORE": "1" } }));
  let mut record = ran_once(&h, "/tmp").await;
  // Native ids are scoped to the peer; the fake starts its counter at s1 in each process.
  record.acp_session_id = Some("previous-native".into());
  let s = reopened(&h, record.clone()).await;
  expect_match(view(&s), json!({ "status": "ready" }));
  assert_ne!(s.to_record().acp_session_id, record.acp_session_id);
  assert!(s.to_record().history_pending);
  assert!(!h.logs().iter().any(|l| l.contains("session/resume") || l.contains("session/load")));
  prompt(&s, "inspect-history").await;
  let wire = wire_prompt(&last_turn(&view(&s)))["prompt"].clone();
  assert_eq!(wire.as_array().unwrap().len(), 2);
  assert!(wire[0]["resource"]["text"].as_str().unwrap().contains("hi"));
  assert_eq!(wire[1]["text"], "inspect-history");
  assert!(!s.to_record().history_pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rebound_session_closed_before_its_first_prompt_persists_the_handoff_without_bumping_updated_at() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let mut record = ran_once(&h, "/tmp").await;
  record.acp_session_id = Some("native-gone".into());
  record.updated_at = iso_of_ms(5000);
  let s = reopened(&h, record.clone()).await;
  assert!(s.to_record().history_pending);
  h.deps.blobs.flush(s.0.clone()).await.unwrap();
  let saved = h.deps.blobs.load(&s.id).await.unwrap();
  assert!(saved.history_pending && saved.history_context);
  assert_eq!(saved.updated_at, record.updated_at);
  if let (_, Some(closing)) = s.shutdown() {
    closing.await;
  }
  // This time the newly created peer exists and resumes successfully, but still owes the original transcript.
  let again = reopened(&h, saved).await;
  assert!(again.to_record().history_pending);
  assert_eq!(again.to_record().updated_at, record.updated_at);
  prompt(&again, "inspect-history").await;
  let wire = wire_prompt(&last_turn(&view(&again)))["prompt"].clone();
  assert!(wire[0]["resource"]["text"].as_str().unwrap().contains("hi"));
  assert!(!again.to_record().history_pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_partial_output_after_rebinding_hands_over_the_partial_turn_before_continuing() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let mut record = interrupted(&h, false, false);
  record.turns[3].as_agent_mut().unwrap().blocks = serde_json::from_value(json!([
    { "type": "text", "markdown": "partial work must survive" }
  ])).unwrap();
  let s = reopened(&h, record).await;
  s.retry_turn().await.unwrap();
  let text = agent_text(&last_turn(&view(&s)));
  assert!(text.contains("first question") && text.contains("partial work must survive"), "{text}");
  assert_eq!(view(&s)["turns"][4]["text"], acpira_host::i18n::t("host.retryContinuePrompt"));
  assert!(!s.to_record().history_pending);
  assert_eq!(turn_count(&s), 6);
}

/// A recorded failure whose message has never reached the replacement peer.
fn interrupted(h: &Harness, fork: bool, edited: bool) -> SessionRecord {
  let s = Disposing(h.session("/tmp"));
  let mut record = s.to_record();
  record.title = "Keep this title".into();
  record.acp_session_id = Some("native-gone".into());
  record.turns = turns(json!([
    { "role": "user", "text": "first question" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "first answer" }], "stop": "end_turn" },
    { "role": "user", "text": "inspect-history", "edited": edited },
    { "role": "agent", "blocks": [], "stop": "error", "error": { "message": "failed before output", "retryable": true } },
  ]));
  if fork {
    record.forked_from = serde_json::from_value(json!({ "sessionId": "source", "turnIndex": 3 })).unwrap();
  }
  record
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_immediately_after_rebinding_sends_the_prefix_before_the_failed_message() {
  let fake = fake_or_skip!();
  for edited in [false, true] {
    let h = Harness::new(&fake, json!({}));
    let s = reopened(&h, interrupted(&h, false, edited)).await;
    let peer = s.to_record().acp_session_id;
    assert!(s.to_record().history_pending);
    s.retry_turn().await.unwrap();
    until(|| !s.is_running() && last_turn(&view(&s))["stop"] == "end_turn", 5000).await;
    let wire = wire_prompt(&last_turn(&view(&s)))["prompt"].clone();
    assert_eq!(wire.as_array().unwrap().len(), 2, "{wire}");
    let history = wire[0]["resource"]["text"].as_str().unwrap();
    assert!(history.contains("first question") && history.contains("first answer"), "{history}");
    assert!(!history.contains("inspect-history") && !history.contains("failed before output"), "{history}");
    assert_eq!(wire[1]["text"], "inspect-history");
    assert_eq!(s.to_record().acp_session_id, peer);
    assert_eq!(turn_count(&s), 4);
    assert!(!s.to_record().history_pending);
    assert_eq!(s.title(), "Keep this title");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_immediately_after_rebinding_hands_over_only_the_prefix_for_regular_sessions_and_forks() {
  let fake = fake_or_skip!();
  for fork in [false, true] {
    for idx in [0, 2] {
      let h = Harness::new(&fake, json!({}));
      let s = reopened(&h, interrupted(&h, fork, false)).await;
      let peer = s.to_record().acp_session_id;
      s.edit_turn(history_edit(&s, idx, "edited inspect-history")).await.unwrap();
      until(|| !s.is_running() && last_turn(&view(&s))["stop"] == "end_turn", 5000).await;
      let wire = wire_prompt(&last_turn(&view(&s)))["prompt"].clone();
      assert_eq!(wire.as_array().unwrap().len(), if idx == 0 { 1 } else { 2 }, "{wire}");
      if idx > 0 {
        let history = wire[0]["resource"]["text"].as_str().unwrap();
        assert!(history.contains("first question") && history.contains("first answer"), "{history}");
        assert!(!history.contains("inspect-history") && !history.contains("failed before output"), "{history}");
      }
      assert_eq!(wire.as_array().unwrap().last().unwrap()["text"], "edited inspect-history");
      assert_eq!(s.to_record().acp_session_id, peer);
      assert_eq!(turn_count(&s), idx + 2);
      assert!(!s.to_record().history_pending);
      assert!(!h.logs().iter().any(|l| l.contains("context too large")));
      assert_eq!(s.title(), "Keep this title");
      if fork {
        assert_eq!(s.to_record().forked_from.unwrap().turn_index, idx as i64 - 1);
      }
    }
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_trimmed_rebound_history_points_at_self_read_and_reclaims_an_empty_first_failure() {
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  h.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new("/opt/acpira/bin/acpira"));
  let mut record = interrupted(&h, false, false);
  record.turns = turns(json!([
    { "role": "user", "text": "old question" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "a".repeat(200_000) }], "stop": "end_turn" },
    { "role": "user", "text": "recent question" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "b".repeat(200_000) }], "stop": "end_turn" },
  ]));
  let s = reopened(&h, record).await;
  prompt(&s, "please fail").await;
  assert!(s.to_record().history_pending);
  assert!(s.to_record().history_context);
  let tried = s.to_record().acp_session_id;
  s.retry_turn().await.unwrap();
  until(|| !s.is_running() && last_turn(&view(&s))["stop"] == "end_turn", 5000).await;
  let text = agent_text(&last_turn(&view(&s)));
  assert!(text.contains("1 earlier turns were omitted") && text.contains("read_session {session:"), "{}", &text[..text.len().min(1000)]);
  assert!(text.contains("recent question"));
  assert!(s.to_record().history_context && !s.to_record().history_pending);
  assert_ne!(s.to_record().acp_session_id, tried);
  assert!(h.logs().iter().any(|l| l.contains("earlier turns behind an outline")));
}
