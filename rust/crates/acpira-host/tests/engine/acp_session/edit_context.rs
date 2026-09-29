//! Historical edits and continues: native resends, oversized payloads, usage during rebuilds

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_edit_right_after_compaction_goes_out_as_one_request() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let first = started(&h, "/tmp").await;
  prompt(&first, "big").await;
  let (s, _) = grow_history(&h, &first, "archived output ".repeat(300_000)).await;
  s.compact(false).await.unwrap();
  prompt(&s, "cancel-empty-once").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 4, "inspect-history")).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null() && turn_count(&s) > before.as_array().unwrap().len(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(json!(turns[..turns.len() - 2]), before);
  expect_eq(&wire_prompt(&turns[turns.len() - 1])["prompt"], json!([{ "type": "text", "text": "inspect-history" }]));
  expect_absent(&turns[turns.len() - 2], "edited");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expanded_payload_over_the_cap_falls_back_to_one_native_prompt() {
  let fake = fake_or_skip!();
  for historical in [false, true] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    let image = json!({ "kind": "image", "name": "large.png", "mimeType": "image/png", "data": "a".repeat(400_000) });
    s.prompt("earlier".into(), if historical { drafts(json!([image])) } else { vec![] }, false, None, None).await;
    prompt(&s, "original").await;
    let before = view(&s)["turns"].clone();
    let peer = s.to_record().acp_session_id;
    let mut edit = history_edit(&s, 2, "inspect-history");
    if !historical {
      edit.attachments = drafts(json!([image]));
    }
    s.edit_turn(edit).await.unwrap();
    until(|| !s.is_running() && turn_count(&s) == 6 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
    assert_eq!(s.to_record().acp_session_id, peer, "historical={historical}");
    let vw = view(&s);
    let turns = vw["turns"].as_array().unwrap();
    assert_eq!(json!(turns[..4]), before);
    expect_absent(&turns[4], "edited");
    let expected = if historical {
      json!([{ "type": "text", "text": "inspect-history" }])
    } else {
      json!([{ "type": "text", "text": "inspect-history" }, { "type": "image", "mimeType": "image/png", "data": "a".repeat(400_000) }])
    };
    expect_eq(&wire_prompt(&turns[5])["prompt"], expected);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_failed_message_retries_natively_even_when_the_edit_changed_settings() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let first = started(&h, "/tmp").await;
  prompt(&first, "big").await;
  let (s, _) = grow_history(&h, &first, "archived output ".repeat(300_000)).await;
  s.compact(false).await.unwrap();
  prompt(&s, "cancel-empty-once").await;
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 4, "cancel-empty-once");
  edit.settings.mode_id = Some("plan".into());
  edit.settings.config.insert("model".into(), "m2".into());
  edit.settings.config.insert("effort".into(), "low".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 6 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  expect_match(&vw["turns"][4], json!({ "text": "cancel-empty-once" }));
  expect_absent(&vw["turns"][4], "edited");
  assert_eq!(vw["controls"]["modeId"], "plan");
  assert_eq!(option_value(&vw, "model"), "m2");
  assert_eq!(option_value(&vw, "effort"), "low");
  prompt(&s, "inspect-history").await;
  let wire = wire_prompt(&last_turn(&view(&s)));
  assert_eq!(wire["mode"], "plan");
  expect_match(&wire["config"], json!({ "model": "m2", "effort": "low" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continue_from_an_earlier_turn_keeps_and_adds_attachments_and_leaves_later_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([
    { "kind": "image", "name": "old.png", "mimeType": "image/png", "data": "aGVsbG8=" },
    { "kind": "text", "name": "remove.txt", "text": "removed attachment content" },
  ])), false, None, None).await;
  s.prompt("later".into(), drafts(json!([{ "kind": "text", "name": "unrelated.txt", "text": "unrelated payload" }])), false, None, None).await;
  let lost = view(&s)["turns"][2]["attachments"][0]["blob"].as_str().unwrap().to_owned();
  std::fs::remove_file(h.dir.path().join("sessions").join(&s.id).join(&lost)).unwrap();
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.retained_attachments = vec![0];
  edit.attachments = drafts(json!([{ "kind": "text", "name": "new.txt", "text": "new attachment content" }]));
  edit.settings.mode_id = Some("plan".into());
  edit.settings.config.insert("model".into(), "m2".into());
  edit.settings.config.insert("effort".into(), "low".into());
  edit.intent = Some(serde_json::from_value(json!("continue")).unwrap());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 6 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(json!(turns[..4]), before);
  expect_absent(&turns[4], "edited");
  expect_match(&turns[4]["attachments"], json!([{ "kind": "image", "name": "old.png" }, { "kind": "text", "name": "new.txt" }]));
  let wire = wire_prompt(&turns[5]);
  let p = wire["prompt"].as_array().unwrap();
  assert_eq!(p.len(), 3);
  expect_eq(&p[0], json!({ "type": "text", "text": "inspect-history" }));
  expect_match(&p[1], json!({ "type": "image", "mimeType": "image/png", "data": "aGVsbG8=" }));
  expect_match(&p[2], json!({ "type": "resource", "resource": { "text": "new attachment content" } }));
  let text = wire["prompt"].to_string();
  assert!(!text.contains("removed attachment content") && !text.contains("Conversation before") && !text.contains("unrelated payload"));
  assert_eq!(wire["mode"], "plan");
  expect_match(&wire["config"], json!({ "model": "m2", "effort": "low" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_native_retry_over_a_context_length_failure_is_still_refused() {
  let fake = fake_or_skip!();
  for changed in [false, true] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    prompt(&s, "context-too-long").await;
    expect_match(last_turn(&view(&s)), json!({ "stop": "error" }));
    let before = view(&s)["turns"].clone();
    let peer = s.to_record().acp_session_id;
    let mut edit = history_edit(&s, 0, "context-too-long");
    if changed {
      edit.settings.mode_id = Some("plan".into());
      edit.settings.config.insert("model".into(), "m2".into());
    }
    let err = s.edit_turn(edit).await.expect_err("refused");
    assert!(err.to_string().to_lowercase().contains("compact") || err.to_string().contains("压缩"), "{err}");
    assert_eq!(view(&s)["turns"], before);
    assert_eq!(s.to_record().acp_session_id, peer);
    assert!(!s.is_running());
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continue_with_an_unavailable_selection_touches_neither_the_session_nor_the_controls() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODELS": "unavailable" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "original").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.settings.config.insert("model".into(), "unavailable".into());
  edit.intent = Some(serde_json::from_value(json!("continue")).unwrap());
  assert!(s.edit_turn(edit).await.is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert_eq!(option_value(&view(&s), "model"), "m1");
  assert!(!s.is_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continue_with_a_missing_blob_or_a_stale_or_malformed_request_is_rejected() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([{ "kind": "text", "name": "lost.txt", "text": "payload" }])), false, None, None).await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let lost = view(&s)["turns"][0]["attachments"][0]["blob"].as_str().unwrap().to_owned();
  let lost_path = h.dir.path().join("sessions").join(&s.id).join(&lost);
  std::fs::remove_file(&lost_path).unwrap();
  let cont = || {
    let mut e = history_edit(&s, 0, "inspect-history");
    e.intent = Some(serde_json::from_value(json!("continue")).unwrap());
    e
  };
  assert!(s.edit_turn(cont()).await.is_err());
  std::fs::write(&lost_path, "payload").unwrap();
  let mut stale = cont();
  stale.turn_count += 2;
  assert!(s.edit_turn(stale).await.is_err());
  let mut wrong_session = cont();
  wrong_session.session_id = "other".into();
  assert!(s.edit_turn(wrong_session).await.is_err());
  let mut wrong_turn = cont();
  wrong_turn.turn_id = Some("other".into());
  assert!(s.edit_turn(wrong_turn).await.is_err());
  let mut dup = cont();
  dup.retained_attachments = vec![0, 0];
  assert!(s.edit_turn(dup).await.is_err());
  // An unknown intent never gets past decoding
  let mut bogus = serde_json::to_value(cont()).unwrap();
  bogus["intent"] = json!("bogus");
  assert!(serde_json::from_value::<acpira_shared::protocol::EditTurnRequest>(bogus).is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert!(!s.is_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn native_usage_reported_while_applying_editor_settings_is_retained() {
  let fake = fake_or_skip!();
  for rejected in [false, true] {
    // The rejected effort is offered and refused on the wire; an effort the model does not offer would just yield
    let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_USAGE": "1", "FAKE_EFFORTS": "unavailable" } }));
    let s = started(&h, "/tmp").await;
    prompt(&s, "big").await;
    let peer = s.to_record().acp_session_id;
    // usage.context on the last agent turn tracks every usage_update, including ones the settings apply triggers — the
    // equality check is about the rejected edit not rewriting the transcript, so usage snapshots are left out of it
    let sans_usage = |s: &AcpSession| {
      let mut t = view(s)["turns"].clone();
      for turn in t.as_array_mut().unwrap() {
        turn.as_object_mut().unwrap().remove("usage");
      }
      t
    };
    let before = sans_usage(&s);
    let mut edit = history_edit(&s, 0, "inspect-history");
    edit.intent = Some(serde_json::from_value(json!("continue")).unwrap());
    edit.settings.config.insert("model".into(), "m2".into());
    if rejected {
      edit.settings.config.insert("effort".into(), "unavailable".into());
      assert!(s.edit_turn(edit).await.is_err());
    } else {
      s.edit_turn(edit).await.unwrap();
      until(|| !s.is_running() && turn_count(&s) == 4 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
    }
    expect_match(&view(&s)["usage"], json!({ "used": 24_000, "size": 200_000 }));
    assert_eq!(option_value(&view(&s), "model"), "m2", "rejected={rejected}");
    assert_eq!(s.to_record().acp_session_id, peer);
    if rejected {
      assert_eq!(sans_usage(&s), before);
    }
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_reported_on_the_fresh_peer_during_a_rebuild_is_ignored() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_USAGE": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "original").await;
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.settings.config.insert("model".into(), "m2".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_ne!(s.to_record().acp_session_id, peer);
  expect_absent(view(&s), "usage");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_edit_after_native_style_compaction_keeps_context_and_applies_the_mode() {
  let fake = fake_or_skip!();
  for (agent, mode_id) in [("grok", "plan"), ("grok", "yolo"), ("kimi", "plan")] {
    let cwd = tempfile::Builder::new().prefix(if agent == "grok" { "acpira-grok-no-modes-" } else { "acpira-kimi-edit-" }).tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let extra = if agent == "grok" {
      json!({ "modes": syn_modes(), "env": { "FAKE_GROK_USAGE": "context", "FAKE_SESSION_DIR": native.path() } })
    } else {
      json!({ "env": { "FAKE_COMPACTION": "kimi", "FAKE_SESSION_DIR": native.path() } })
    };
    let h = Harness::for_agent(&fake, agent, extra);
    let first = started(&h, cwd.path().to_str().unwrap()).await;
    prompt(&first, "big").await;
    until(|| view(&first)["commands"].as_array().is_some_and(|c| c.iter().any(|c| c["name"] == "compact")), 5000).await;
    let compact = claimed({
      let s = first.0.clone();
      async move { s.compact(false).await }
    });
    if agent == "kimi" {
      until(|| h.logs().iter().any(|l| l.contains("waiting for compaction completion")), 5000).await;
      assert!(first.is_running());
      first.set_config("effort".into(), "high".into()).await.ok();
    }
    compact.await.unwrap().unwrap();
    assert!(!first.is_running());
    let (s, _) = grow_history(&h, &first, "archived output ".repeat(100_000)).await;
    s.prompt("original".into(), drafts(json!([{ "kind": "image", "name": "kept.png", "mimeType": "image/png", "data": "aGVsbG8=" }])), false, None, None).await;
    let before = view(&s)["turns"].clone();
    let peer = s.to_record().acp_session_id;
    let mut edit = history_edit(&s, 4, "inspect-history");
    edit.settings.mode_id = Some(mode_id.into());
    edit.settings.config.insert("model".into(), "m2".into());
    edit.settings.config.insert("effort".into(), "low".into());
    edit.attachments = drafts(json!([{ "kind": "text", "name": "new.txt", "text": "new attachment content" }]));
    s.edit_turn(edit).await.unwrap();
    until(|| !s.is_running() && turn_count(&s) == 8 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
    assert_eq!(s.to_record().acp_session_id, peer, "{agent} {mode_id}");
    let vw = view(&s);
    let turns = vw["turns"].as_array().unwrap();
    assert_eq!(json!(turns[..6]), before);
    assert_eq!(vw["controls"]["modeId"], mode_id);
    let wire = wire_prompt(&turns[7]);
    assert_eq!(wire["mode"], if mode_id == "yolo" { "default" } else { mode_id }, "{agent} {mode_id}");
    expect_match(&wire["config"], json!({ "model": "m2", "effort": "low" }));
    let p = wire["prompt"].as_array().unwrap();
    assert_eq!(p.len(), 3);
    expect_eq(&p[0], json!({ "type": "text", "text": "inspect-history" }));
    expect_match(&p[1], json!({ "type": "image", "data": "aGVsbG8=" }));
    expect_match(&p[2], json!({ "type": "resource", "resource": { "text": "new attachment content" } }));
    expect_absent(&turns[6], "edited");
  }
}
