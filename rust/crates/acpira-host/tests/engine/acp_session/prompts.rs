//! Prompt turns: start-up, streaming, attachments and staging, cancel, stop reasons, startup banners

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn start_session_receives_modes_and_config_options_with_model_first() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  let ids = |k: &str| vw["controls"][k].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  assert_eq!(ids("modes"), ["agent", "plan"]);
  assert_eq!(vw["controls"]["modeId"], "agent");
  assert_eq!(ids("options"), ["model", "effort"]);
  expect_match(&vw["controls"]["options"][0], json!({ "category": "model", "value": "m1" }));
  assert_eq!(vw["controls"]["options"][0]["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("m1"), json!("m2")]);
  expect_match(&vw["controls"]["options"][1], json!({ "name": "Reasoning", "category": "thought_level", "value": "high" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn one_prompt_turn_merges_thought_plan_and_text_and_updates_title_and_commands() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "hi").await;
  let vw = view(&s);
  assert!(!vw["running"].as_bool().unwrap());
  assert_eq!(turns_in(&vw), 2);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "hi" }));
  let agent = &vw["turns"][1];
  assert_eq!(agent["role"], "agent");
  assert!(agent["endedAt"].as_i64().unwrap() >= agent["startedAt"].as_i64().unwrap());
  assert_eq!(agent["blocks"].as_array().unwrap().iter().map(|b| b["type"].clone()).collect::<Vec<_>>(), [json!("thought"), json!("plan"), json!("text")]);
  expect_match(&agent["blocks"][0], json!({ "type": "thought", "text": "thinking hard", "streaming": false }));
  expect_match(&agent["blocks"][2], json!({ "type": "text", "markdown": "hello world", "streaming": false }));
  expect_absent(agent, "activity");
  assert_eq!(vw["title"], "Fake title");
  expect_eq(&vw["commands"], json!([{ "name": "compact", "description": "compact it" }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_are_stored_as_blobs_and_sent_as_image_resource_and_link_blocks() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let png = base64::engine::general_purpose::STANDARD.encode("fake-png-bytes");
  s.prompt("echo blocks".into(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": png, "name": "shot.png" },
    { "kind": "text", "name": "notes.md", "text": "# notes" },
    { "kind": "file", "uri": "file:///repo/src/a.ts", "name": "src/a.ts" },
  ])), false, None, None).await;
  let vw = view(&s);
  let user = &vw["turns"][0];
  expect_match(user, json!({ "role": "user", "text": "echo blocks", "attachments": [
    { "kind": "image", "mimeType": "image/png", "name": "shot.png" },
    { "kind": "text", "name": "notes.md" },
    { "kind": "file", "uri": "file:///repo/src/a.ts", "name": "src/a.ts" },
  ] }));
  let img = user["attachments"][0]["blob"].as_str().unwrap();
  let txt = user["attachments"][1]["blob"].as_str().unwrap();
  assert_eq!(blob(&h, &s.id, img).unwrap(), b"fake-png-bytes");
  assert_eq!(blob(&h, &s.id, txt).unwrap(), b"# notes");
  // the fake agent echoes the block types and key fields it received
  let txt_path = h.dir.path().canonicalize().unwrap().join("sessions").join(&s.id).join(txt);
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], format!("text · image:image/png · resource:file://{}:# notes · resource_link:file:///repo/src/a.ts:src/a.ts", txt_path.display()));
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_only_omit_the_text_block_and_title_the_session_from_what_was_attached() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt(String::new(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": "AAAA" },
    { "kind": "file", "uri": "file:///repo/README.md", "name": "README.md" },
  ])), false, None, None).await;
  let vw = view(&s);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "" }));
  assert_eq!(vw["title"], "1 images, README.md");
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], "image:image/png · resource_link:file:///repo/README.md:README.md");
  // the echoed user_message_chunk (Grok sends the image back too) must not create a second user turn
  assert_eq!(vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_file_draft_is_sent_as_pixels_and_an_oversized_image_is_dropped_with_a_note() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let png = dir.path().join("shot.png");
  std::fs::write(&png, "real-png-bytes").unwrap();
  let mut h = Harness::new(&fake, json!({}));
  let notes = Arc::new(Mutex::new(Vec::<String>::new()));
  let n = notes.clone();
  h.deps.notify = Some(Arc::new(move |t: &str| n.lock().unwrap().push(t.to_owned())));
  let s = started(&h, "/tmp").await;
  let huge = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_IMAGE_BYTES + 1]);
  s.prompt("echo blocks".into(), drafts(json!([
    { "kind": "file", "uri": format!("file://{}", png.display()), "name": "shot.png" },
    { "kind": "image", "mimeType": "image/png", "data": huge, "name": "huge.png" },
  ])), false, None, None).await;
  let vw = view(&s);
  let shot = &vw["turns"][0]["attachments"][0];
  expect_match(shot, json!({ "kind": "image", "mimeType": "image/png", "name": "shot.png" }));
  assert_eq!(blob(&h, &s.id, shot["blob"].as_str().unwrap()).unwrap(), b"real-png-bytes");
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], "text · image:image/png");
  assert_eq!(*notes.lock().unwrap(), [format!("huge.png exceeds {} MB, skipped", MAX_IMAGE_BYTES >> 20)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_blob_store_does_not_lose_the_prompt() {
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  // The sessions directory cannot be created: every blob write fails
  std::fs::write(h.dir.path().join("sessions"), "not a directory").unwrap();
  let notes = Arc::new(Mutex::new(Vec::<String>::new()));
  let n = notes.clone();
  h.deps.notify = Some(Arc::new(move |t: &str| n.lock().unwrap().push(t.to_owned())));
  let s = started(&h, "/tmp").await;
  s.prompt("echo blocks".into(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": "AAAA", "name": "shot.png" },
    { "kind": "text", "name": "n.md", "text": "x" },
  ])), false, None, None).await;
  let vw = view(&s);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "echo blocks", "attachments": [{ "kind": "image", "mimeType": "image/png", "name": "shot.png" }, { "kind": "text", "name": "n.md" }] }));
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], "text · image:image/png · resource:attachment:///n.md:x");
  let notes = notes.lock().unwrap();
  assert_eq!(notes.len(), 2);
  assert!(notes[0].contains("shot.png"), "{notes:?}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn cancel_while_staging_drops_the_prompt_and_a_send_meanwhile_goes_out_afterwards() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let gate = StagingGate::new();
  let first = tokio::spawn(s.prompt("echo blocks".into(), gate.draft(), false, None, None));
  until(|| view(&s)["running"] == true, 5000).await;
  s.cancel().await;
  prompt(&s, "hi").await;
  let queued: Vec<Value> = view(&s)["queued"].as_array().cloned().unwrap_or_default();
  assert_eq!(queued.iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("hi")]);
  gate.release();
  first.await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 2, 5000).await;
  let vw = view(&s);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "hi" }));
  expect_absent(&vw, "queued");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn dispose_while_staging_appends_and_sends_nothing_afterwards() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let gate = StagingGate::new();
  let p = tokio::spawn(s.prompt("echo blocks".into(), gate.draft(), false, None, None));
  until(|| view(&s)["running"] == true, 5000).await;
  s.dispose();
  gate.release();
  p.await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["turns"], json!([]));
  assert_eq!(vw["running"], false);
}

// codex-acp / claude-agent-acp send images as message chunks and tool content items; the payload lands in the
// session's blob store under its content-hash name (replay writes the same file, no duplicates)
#[tokio::test(flavor = "multi_thread")]
async fn agent_emitted_images_land_in_the_blob_store() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "image").await;
  let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==").unwrap();
  let name = blob_name(".png", &png);
  // Agent images are written off the update path
  until(|| blob(&h, &s.id, &name).is_some(), 5000).await;
  assert_eq!(blob(&h, &s.id, &name).unwrap(), png);
  let agent = view(&s)["turns"][1].clone();
  let blocks = agent["blocks"].as_array().unwrap();
  expect_match(blocks.iter().find(|b| b["type"] == "image").unwrap(), json!({ "type": "image", "mimeType": "image/png", "blob": name }));
  assert_eq!(blocks.iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].clone()).collect::<Vec<_>>(), [json!("here is "), json!("the red dot")]);
  let tool = blocks.iter().find(|b| b["id"] == "im1").unwrap();
  expect_eq(&tool["contents"], json!([
    { "type": "text", "text": "Revised prompt: red dot" },
    { "type": "image", "mimeType": "image/png", "blob": name, "uri": "/repo/red.png" },
  ]));
  expect_eq(&tool["content"], json!({ "type": "text", "text": "Revised prompt: red dot" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn session_views_carry_a_monotonic_rev_and_leave_running_false_after_the_prompt() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = Disposing(h.session("/tmp"));
  let before = s.view().rev.unwrap_or(0);
  s.start().await;
  let ready = s.view().rev.unwrap_or(0);
  assert!(ready > before);
  assert_eq!(s.view().rev.unwrap_or(0), ready);
  prompt(&s, "hi").await;
  let done = view(&s);
  assert_eq!(done["running"], false);
  assert!(done["rev"].as_i64().unwrap_or(0) > ready);
  assert_eq!(last_turn(&done)["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_text_midway_wraps_up_the_turn_and_allows_another_send() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["turns"][1]["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["type"] == "text")), 5000).await;
  s.cancel().await;
  p.await.unwrap();
  assert_eq!(view(&s)["running"], false);
  prompt(&s, "hi").await;
  assert_eq!(turn_count(&s), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn short_stops_record_refusal_and_max_tokens_and_a_normal_turn_records_end_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "refuse this").await;
  prompt(&s, "truncate this").await;
  prompt(&s, "hi").await;
  let vw = view(&s);
  let (refused, truncated, ok) = (&vw["turns"][1], &vw["turns"][3], &vw["turns"][5]);
  assert_eq!(refused["stop"], "refusal");
  assert_eq!(refused["blocks"], json!([]));
  assert_eq!(truncated["stop"], "max_tokens");
  expect_match(truncated["blocks"].as_array().unwrap().last().unwrap(), json!({ "type": "text", "markdown": "once upon a", "streaming": false }));
  assert_eq!(ok["stop"], "end_turn");
  expect_absent(ok, "error");
}

// The session list sorts by updatedAt: only the user's message may move a session, never the stream that follows it
#[tokio::test(flavor = "multi_thread")]
async fn updated_at_is_bumped_once_by_the_prompt_then_stable_across_the_stream() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let before = s.view().updated_at.clone();
  tokio::time::sleep(std::time::Duration::from_millis(5)).await;
  let seen = h.sample(&s, |vw| vw.updated_at.clone());
  prompt(&s, "hi").await;
  until(|| seen.lock().unwrap().len() > 2, 5000).await;
  let seen = seen.lock().unwrap().clone();
  assert_ne!(seen[0], before);
  assert_eq!(seen.iter().collect::<std::collections::HashSet<_>>().len(), 1, "{seen:?}");
  assert_eq!(s.view().updated_at, seen[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_mode_model_and_effort_and_rename_and_pin_leave_updated_at_untouched() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.set_mode("plan".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "plan");
  s.set_config("model".into(), "m2".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "model"), "m2");
  s.set_config("effort".into(), "low".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  assert_eq!(option_value(&view(&s), "model"), "m2");
  s.set_config("nope".into(), "x".into()).await.ok();
  let before = s.view().updated_at.clone();
  s.rename("  改个名  ");
  s.set_pinned(true);
  assert_eq!(view(&s)["title"], "改个名");
  assert_eq!(s.to_record().pinned, Some(true));
  assert_eq!(s.view().updated_at, before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_strict_prompt_capabilities_agent_receives_dropped_text_as_marked_up_text() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
  let s = started(&h, "/tmp").await;
  s.prompt("echo-blocks".into(), drafts(json!([{ "kind": "text", "name": "notes.txt", "text": "payload" }])), false, None, None).await;
  let block = last_turn(&view(&s))["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(block["markdown"], "text,text");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_startup_banner_streamed_before_session_new_returns_is_ignored() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STARTUP_BANNER": "early" } }));
  let s = started(&h, "/tmp").await;
  // The banner carried no session id (none existed yet): it must not open a ghost agent turn
  assert_eq!(turn_count(&s), 0);
  assert!(h.logs().iter().any(|l| l.contains("startup agent_message_chunk ignored")));
  prompt(&s, "hi").await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "end_turn" }));
}

// pi-acp's real timing: the prelude text rides session/new's _meta.piAcp.startupInfo and is re-sent as one
// agent_message_chunk a tick after the response — past the no-session guard, so the exact text does the match
#[tokio::test(flavor = "multi_thread")]
async fn a_startup_banner_sent_right_after_session_new_is_matched_and_dropped() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STARTUP_BANNER": "1" } }));
  let s = started(&h, "/tmp").await;
  until(|| h.logs().iter().any(|l| l.contains("startup banner ignored")), 5000).await;
  // The banner must not open a ghost agent turn
  assert_eq!(turn_count(&s), 0);
  prompt(&s, "hi").await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_prompt_queued_during_start_is_not_prefixed_by_the_late_banner() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STARTUP_BANNER": "1" } }));
  let s = Disposing(h.session("/tmp"));
  let started = claimed({
    let s = s.0.clone();
    async move { s.start().await }
  });
  // enqueue resolves once the prompt is parked — the turn runs after start flushes the queue
  prompt(&s, "hello").await;
  started.await.unwrap();
  until(|| last_turn(&view(&s))["stop"] == "end_turn", 5000).await;
  let text: String = last_turn(&view(&s))["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].as_str().unwrap().to_owned()).collect();
  // The observed wire shape was "pi v0.86.0 --- ## Skills …pong" — the banner prepended to the first reply chunk
  assert!(!text.contains("pi v0.0 banner"), "{text}");
  assert!(text.contains("hello world"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn per_prompt_token_usage_lands_on_the_agent_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "usage-devin").await;
  expect_match(&last_turn(&view(&s))["usage"], json!({ "input": 100, "output": 20, "cachedRead": 64, "requestId": "req-devin-1", "context": { "used": 5000, "size": 100_000 } }));
  prompt(&s, "usage-grok").await;
  expect_match(&last_turn(&view(&s))["usage"], json!({ "input": 38_140, "output": 20, "modelCalls": 2, "model": "grok-4.6", "requestId": "req-grok-1" }));
}
