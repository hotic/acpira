//! Context overflow, manual compaction and auto compaction

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn keeps_the_native_session_on_context_overflow_and_requires_compaction_before_retrying() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "seed-context").await;
  let peer = s.to_record().acp_session_id;
  // Polled in one go like the TS calls: the first claims the turn synchronously, so the second queues behind it
  tokio::join!(prompt(&s, "context-too-long"), prompt(&s, "queued-follow-up"));
  let queued: Vec<Value> = view(&s)["queued"].as_array().cloned().unwrap_or_default();
  assert_eq!(queued.iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("queued-follow-up")]);
  let before = view(&s)["turns"].clone();
  let err = s.retry_turn().await.expect_err("retry must require compaction");
  assert!(err.to_string().to_lowercase().contains("compact") || err.to_string().contains("压缩"), "{err}");
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  s.compact(false).await.unwrap();
  // The drained follow-up has run to its end (between dequeue and dispatch the view is briefly idle with the prompt pending)
  until(|| {
    let vw = view(&s);
    !s.is_running() && vw["queued"].as_array().is_none_or(|q| q.is_empty()) && turn_at(&vw, -2)["text"] == "queued-follow-up" && !last_turn(&vw)["stop"].is_null()
  }, 5000).await;
  expect_match(turn_at(&view(&s), -2), json!({ "role": "user", "text": "queued-follow-up" }));
  prompt(&s, "context-too-long").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  assert_eq!(s.to_record().acp_session_id, peer);
}

fn auto_compaction() -> Option<acpira_host::acp::session::CompactionPolicy> {
  Some(acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: true })
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_sends_an_auto_turn_over_the_threshold_and_does_not_repeat_without_growth() {
  let fake = fake_or_skip!();
  let h = Harness::with_compaction(&fake, json!({}), auto_compaction());
  let s = started(&h, "/tmp").await;
  prompt(&s, "big").await;
  // auto compaction is already queued (async) when prompt() returns; wait for it to finish
  until(|| turn_count(&s) == 4 && !s.is_running(), 5000).await;
  let vw = view(&s);
  expect_eq(&vw["turns"][2], json!({ "role": "user", "text": "/compact", "auto": true }));
  expect_eq(&vw["turns"][3]["blocks"], json!([{ "type": "compaction", "id": "cp1", "status": "completed" }]));
  assert!(vw["usage"]["used"].as_f64().unwrap() < 300_000.0);
  assert_eq!(vw["title"], "big");
  // grows again → compacts once more, but this time the fake agent can't compact (usage unchanged)
  prompt(&s, "big").await;
  until(|| turn_count(&s) == 8 && !s.is_running(), 5000).await;
  let used = view(&s)["usage"]["used"].as_f64().unwrap();
  assert!(used > 300_000.0);
  // usage didn't grow back much: the next turn end doesn't resend /compact
  prompt(&s, "hi").await;
  tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  assert_eq!(turn_count(&s), 10);
  assert_eq!(view(&s)["usage"]["used"].as_f64().unwrap(), used);
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_runs_before_a_follow_up_queued_during_the_over_threshold_turn() {
  let fake = fake_or_skip!();
  let h = Harness::with_compaction(&fake, json!({}), auto_compaction());
  let s = started(&h, "/tmp").await;
  tokio::join!(prompt(&s, "big"), prompt(&s, "follow-up"));
  until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().iter().any(|t| t["role"] == "user" && t["text"] == "follow-up"), 5000).await;
  let users: Vec<Value> = view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").cloned().collect();
  expect_match(users, json!([{ "text": "big" }, { "text": "/compact", "auto": true }, { "text": "follow-up" }]));
  assert!(view(&s)["usage"]["used"].as_f64().unwrap() < 300_000.0);
}

/// A compaction policy whose `auto` flag the test flips mid-way (the TS `let auto` captured by the deps closure)
fn switchable_compaction() -> (Arc<std::sync::atomic::AtomicBool>, Arc<dyn Fn() -> acpira_host::acp::session::CompactionPolicy + Send + Sync>) {
  let auto = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let a = auto.clone();
  (auto, Arc::new(move || acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: a.load(std::sync::atomic::Ordering::SeqCst) }))
}

fn user_texts(s: &AcpSession) -> Vec<String> {
  view(s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_runs_before_the_next_typed_prompt_when_usage_was_left_over_the_threshold() {
  let fake = fake_or_skip!();
  let (auto, policy) = switchable_compaction();
  let h = Harness::with_compaction_fn(&fake, json!({}), Some(policy));
  let s = started(&h, "/tmp").await;
  prompt(&s, "big").await;
  assert_eq!(turn_count(&s), 2);
  assert!(view(&s)["usage"]["used"].as_f64().unwrap() > 300_000.0);
  auto.store(true, std::sync::atomic::Ordering::SeqCst);
  prompt(&s, "hi").await;
  until(|| !s.is_running() && user_texts(&s).contains(&"hi".to_owned()), 5000).await;
  let users: Vec<Value> = view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").cloned().collect();
  expect_match(users, json!([{ "text": "big" }, { "text": "/compact", "auto": true }, { "text": "hi" }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_submitted_message_shows_below_automatic_compaction_while_the_peer_is_compacting() {
  let fake = fake_or_skip!();
  let (auto, policy) = switchable_compaction();
  let h = Harness::with_compaction_fn(&fake, json!({ "env": { "FAKE_COMPACTION": "structured" } }), Some(policy));
  let s = started(&h, "/tmp").await;
  prompt(&s, "big").await;
  auto.store(true, std::sync::atomic::Ordering::SeqCst);
  let sent = tokio::spawn(s.prompt("visible follow-up".into(), drafts(json!([{ "kind": "text", "name": "note.txt", "text": "attached note" }])), false, None, None));
  until(|| h.logs().iter().any(|l| l.contains("waiting for compaction completion")), 5000).await;
  // The peer has not received the follow-up yet, but its bubble and staged attachment must already be visible
  expect_match(last_turn(&view(&s)), json!({ "role": "user", "text": "visible follow-up", "attachments": [{ "name": "note.txt" }] }));
  expect_match(s.to_record().turns.last().unwrap(), json!({ "role": "user", "text": "visible follow-up" }));
  expect_match(turn_at(&view(&s), -2), json!({ "role": "agent", "blocks": [{ "type": "compaction", "status": "in_progress" }] }));
  prompt(&s, "later queued message").await;
  s.set_config("effort".into(), "high".into()).await.ok();
  sent.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].as_array().is_none_or(|q| q.is_empty()) && user_texts(&s).len() == 4, 5000).await;
  assert_eq!(user_texts(&s), ["big", "/compact", "visible follow-up", "later queued message"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_compaction_sends_compact_when_available_and_errors_when_not() {
  let fake = fake_or_skip!();
  let h = Harness::with_compaction(&fake, json!({}), Some(acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: false }));
  let s = started(&h, "/tmp").await;
  let err = s.compact(false).await.expect_err("no /compact yet");
  assert!(err.to_string().contains("/compact"), "{err}");
  prompt(&s, "big").await;
  tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  assert_eq!(turn_count(&s), 2);
  s.compact(false).await.unwrap();
  assert_eq!(turn_count(&s), 4);
  expect_match(&view(&s)["turns"][2], json!({ "role": "user", "text": "/compact" }));
}
