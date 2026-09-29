//! Historical edits: rebuilt peers, settings, retained attachments and stale requests

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_empty_cancelled_turn_resends_natively_even_with_multi_megabyte_history() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let (s, _) = with_ui_history(&h, "old output ".repeat(400_000)).await;
  prompt(&s, "cancel-empty-once").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "cancelled", "blocks": [] }));
  let peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 2, "cancel-empty-once")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  expect_absent(&view(&s)["turns"][2], "edited");
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_historical_edit_continues_without_replacing_the_native_session_or_history() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let (s, _) = with_ui_history(&h, "archived output ".repeat(300_000)).await;
  prompt(&s, "original").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(json!(turns[..turns.len() - 2]), before);
  assert_eq!(turns.len(), 6);
  expect_absent(&turns[4], "edited");
  expect_eq(&wire_prompt(&turns[5])["prompt"], json!([{ "type": "text", "text": "inspect-history" }]));
  prompt(&s, "ordinary follow-up").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  assert_eq!(s.to_record().acp_session_id, peer);
}

#[tokio::test(flavor = "multi_thread")]
async fn resending_an_unchanged_message_after_empty_failures_keeps_native_compacted_context() {
  let fake = fake_or_skip!();
  for attempts in [1, 2] {
    let (h, _native) = native_harness(&fake, json!({}));
    // The UI retains old tool output even after native compaction. It must never be injected into an unchanged failed-message retry
    let (s, earlier) = with_ui_history(&h, "archived output ".repeat(250_000)).await;
    let text = if attempts == 2 { "fail-twice" } else { "please fail" };
    for _ in 0..attempts {
      prompt(&s, text).await;
    }
    expect_match(last_turn(&view(&s)), json!({ "stop": "error", "blocks": [] }));
    let native = s.to_record().acp_session_id;
    s.edit_turn(history_edit(&s, 2, text)).await.unwrap();
    until(|| !s.is_running(), 5000).await;
    assert_eq!(s.to_record().acp_session_id, native, "attempts={attempts}");
    let vw = view(&s);
    assert_eq!(turns_in(&vw), 4);
    assert_eq!(vw["turns"][1], earlier);
    expect_absent(&vw["turns"][2], "edited");
    expect_match(&vw["turns"][3], json!({ "stop": "end_turn" }));
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_starts_a_fresh_peer_with_only_earlier_context_and_applies_mode_and_effort_first() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "replaced-original").await;
  prompt(&s, "discarded-future").await;
  let old_peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 2, "inspect-history");
  edit.settings.mode_id = Some("plan".into());
  edit.settings.config.insert("effort".into(), "low".into());
  edit.settings.config.insert("model".into(), "m2".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  assert_ne!(s.to_record().acp_session_id, old_peer);
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 4);
  expect_match(&vw["turns"][0], json!({ "text": "earlier-context", "settings": { "config": { "effort": "high" } } }));
  expect_match(&vw["turns"][2], json!({ "text": "inspect-history", "settings": { "modeId": "plan", "config": { "effort": "low", "model": "m2" } } }));
  let reply = vw["turns"][3].to_string();
  assert!(reply.contains("earlier-context") && !reply.contains("replaced-original") && !reply.contains("discarded-future"), "{reply}");
  assert!(reply.contains("low") && reply.contains("m2") && reply.contains("plan"));
}

// The editor's picker switches models locally, so its settings still carry the previous model's Fast and effort
// (Devin: GPT-6 Luna Fast → SWE-2, which has no `speed` control and no low effort); the edit must still send
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_lets_the_agent_settle_dependent_controls_the_new_model_no_longer_offers() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_SPEED": "1" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("speed".into(), "fast".into()).await.unwrap();
  s.set_config("effort".into(), "low".into()).await.unwrap();
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  let mut edit = history_edit(&s, 2, "inspect-history");
  expect_match(&edit.settings.config, json!({ "model": "m1", "speed": "fast", "effort": "low" }));
  edit.settings.config.insert("model".into(), "m2".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(option_value(&vw, "model"), "m2");
  assert!(vw["controls"]["options"].as_array().unwrap().iter().all(|o| o["id"] != "speed"));
  assert_eq!(option_value(&vw, "effort"), "high");
  expect_match(&vw["turns"][2], json!({ "text": "inspect-history", "edited": true, "settings": { "config": { "model": "m2", "effort": "high" } } }));
  assert!(vw["turns"][3].to_string().contains("m2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_still_refuses_a_model_the_agent_no_longer_offers() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  let mut edit = history_edit(&s, 2, "inspect-history");
  edit.settings.config.insert("model".into(), "gone".into());
  let err = s.edit_turn(edit).await.expect_err("refused");
  assert!(err.to_string().contains("model"), "{err}");
  assert_eq!(turn_count(&s), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_ignores_the_rebuilt_peers_title_so_a_renamed_session_keeps_its_title() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  s.rename("Kept title");
  let old_peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  // The edit rebuilt the context through session/new (fresh peer), and its 'Fake title' update was ignored
  assert_ne!(s.to_record().acp_session_id, old_peer);
  assert_eq!(view(&s)["title"], "Kept title");
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_a_failed_edited_prompt_rebuilds_context_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  s.edit_turn(history_edit(&s, 2, "please fail")).await.unwrap();
  until(|| !s.is_running() && last_turn(&view(&s))["role"] == "agent" && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  expect_match(&view(&s)["turns"][3], json!({ "stop": "error" }));
  let failed_peer = s.to_record().acp_session_id;
  s.retry_turn().await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_ne!(s.to_record().acp_session_id, failed_peer);
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 4);
  expect_match(&vw["turns"][0], json!({ "text": "earlier-context" }));
  expect_match(&vw["turns"][2], json!({ "text": "please fail", "edited": true }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_keeps_retained_image_bytes_removes_selected_attachments_and_adds_new_ones() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([
    { "kind": "image", "name": "old.png", "mimeType": "image/png", "data": "aGVsbG8=" },
    { "kind": "text", "name": "remove.txt", "text": "removed attachment content" },
  ])), false, None, None).await;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.retained_attachments = vec![0];
  edit.attachments = drafts(json!([{ "kind": "text", "name": "new.txt", "text": "new attachment content" }]));
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 2);
  expect_match(&vw["turns"][0], json!({ "attachments": [{ "kind": "image", "name": "old.png" }, { "kind": "text", "name": "new.txt" }] }));
  let reply = vw["turns"][1].to_string();
  assert!(reply.contains("aGVsbG8=") && reply.contains("new attachment content") && !reply.contains("removed attachment content"), "{reply}");
}

#[tokio::test(flavor = "multi_thread")]
async fn under_strict_prompt_capabilities_a_kept_text_attachment_resends_as_text_in_the_rebuilt_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
  let s = started(&h, "/tmp").await;
  s.prompt("earlier-context".into(), drafts(json!([{ "kind": "text", "name": "keep.txt", "text": "kept payload" }])), false, None, None).await;
  prompt(&s, "original").await;
  // Editing turn 2 rebuilds the context through historyContext: turn 0's kept attachment must be re-encoded with the strict caps
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  let wire = wire_prompt(&last_turn(&view(&s)));
  assert!(wire["prompt"].as_array().unwrap().iter().all(|b| b["type"] == "text"));
  assert!(wire["prompt"].to_string().contains("[Attachment: keep.txt]"));
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_edits_or_unavailable_settings_preserve_the_transcript_and_peer() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODELS": "unavailable" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "original").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut stale = history_edit(&s, 0, "inspect-history");
  stale.turn_count += 2;
  assert!(s.edit_turn(stale).await.is_err());
  let mut invalid = history_edit(&s, 0, "inspect-history");
  invalid.settings.config.insert("model".into(), "unavailable".into());
  assert!(s.edit_turn(invalid).await.is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert_eq!(option_value(&view(&s), "model"), "m1");
  assert!(!s.is_running());
}

fn remove_blobs(h: &Harness, sid: &str) {
  for e in std::fs::read_dir(h.dir.path().join("sessions").join(sid)).unwrap() {
    std::fs::remove_file(e.unwrap().path()).unwrap();
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_retained_blob_does_not_replace_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([{ "kind": "text", "name": "lost.txt", "text": "payload" }])), false, None, None).await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  remove_blobs(&h, &s.id);
  assert!(s.edit_turn(history_edit(&s, 0, "inspect-history")).await.is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert!(!s.is_running());
}

/// A retained attachment whose blob is a FIFO: reading it blocks until released (the TS readBlob gate). It has its own name, so
/// the edit re-staging the same bytes under their content hash never opens the FIFO for writing
#[cfg(unix)]
struct BlobGate {
  path: std::path::PathBuf,
}

#[cfg(unix)]
impl BlobGate {
  const NAME: &'static str = "gate-blob.txt";

  fn create(h: &Harness, sid: &str) -> BlobGate {
    let path = h.dir.path().join("sessions").join(sid).join(Self::NAME);
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    BlobGate { path }
  }
  fn release(&self) {
    let path = self.path.clone();
    std::thread::spawn(move || std::fs::write(path, b"payload"));
  }
}

#[cfg(unix)]
async fn rejects_double_submission_then_cancels(intent: Option<&str>) {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let first = started(&h, "/tmp").await;
  first.prompt("original".into(), drafts(json!([{ "kind": "text", "name": "wait.txt", "text": "payload" }])), false, None, None).await;
  let mut record = first.to_record();
  first.dispose();
  if let Some(Turn::User(u)) = record.turns.get_mut(0) {
    let mut a = v(&u.attachments.as_ref().unwrap()[0]);
    a["blob"] = json!(BlobGate::NAME);
    u.attachments = Some(vec![serde_json::from_value(a).unwrap()]);
  }
  let gate = BlobGate::create(&h, &record.id);
  let s = reopened(&h, record).await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.intent = intent.map(|i| serde_json::from_value(json!(i)).unwrap());
  let pending = claimed({
    let (s, edit) = (s.0.clone(), edit.clone());
    async move { s.edit_turn(edit).await }
  });
  assert!(s.edit_turn(edit).await.is_err());
  // The cancel is requested while the blob read is still blocked; it completes once staging lets go
  let cancel = claimed({
    let s = s.0.clone();
    async move { s.cancel().await }
  });
  gate.release();
  cancel.await.unwrap();
  assert!(pending.await.unwrap().is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert!(!s.is_running());
  if intent.is_some() {
    prompt(&s, "ordinary follow-up").await;
    expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_double_submission_is_rejected_and_a_cancel_lands_before_history_is_replaced() {
  rejects_double_submission_then_cancels(None).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_continue_cancelled_while_attachments_stage_still_accepts_a_normal_prompt() {
  rejects_double_submission_then_cancels(Some("continue")).await;
}
