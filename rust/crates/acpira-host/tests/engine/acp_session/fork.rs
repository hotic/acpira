//! Forked sessions carrying copied transcripts

use super::*;

/// A fork's record: copied turns, history pending, no native session yet
fn fork_record(h: &Harness, turns_json: Value, extra: impl FnOnce(&mut SessionRecord)) -> SessionRecord {
  let base = Disposing(h.session("/tmp"));
  let mut record = base.to_record();
  record.turns = turns(turns_json);
  record.history_pending = true;
  record.acp_session_id = None;
  extra(&mut record);
  record
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forks_copied_transcript_goes_to_the_native_session_as_retained_context() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let record = fork_record(&h, json!([
    { "role": "user", "text": "earlier" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "before" }], "stop": "end_turn" },
  ]), |_| {});
  let s = reopened(&h, record).await;
  prompt(&s, "now").await;
  let vw = view(&s);
  // The copied transcript stays put: nothing was dropped when the context went out
  assert_eq!(turns_in(&vw), 4);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "earlier" }));
  expect_match(&vw["turns"][1], json!({ "role": "agent" }));
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "now", "edited": true }));
  // The fake agent echoes non-text blocks: the embedded history resource and the 'earlier' turn inside its JSON
  let text = agent_text(&vw["turns"][3]);
  assert!(text.contains("resource:acpira://history/") && text.contains("earlier"), "{text}");
  assert!(!s.to_record().history_pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_fork_history_is_compacted_keeping_its_most_recent_turns() {
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  let notes = Arc::new(Mutex::new(Vec::<String>::new()));
  let n = notes.clone();
  h.deps.notify = Some(Arc::new(move |t: &str| n.lock().unwrap().push(t.to_owned())));
  // Each tool output is clipped in the handed-over history, so twelve 30 KB outputs fit where the raw JSON would not;
  // the 200 KB replies do not, so only the newest pair survives
  let mut all = vec![];
  for i in 0..12 {
    all.push(json!({ "role": "user", "text": format!("ask-{i}") }));
    all.push(json!({ "role": "agent", "stop": "end_turn", "blocks": [
      { "type": "thought", "text": "x".repeat(30_000) },
      { "type": "tool_call", "id": format!("t{i}"), "kind": "execute", "verb": "Run", "status": "completed", "content": { "type": "text", "text": "y".repeat(30_000) } },
      { "type": "text", "markdown": format!("reply-{i}") },
    ] }));
  }
  all.push(json!({ "role": "user", "text": "old-big" }));
  all.push(json!({ "role": "agent", "stop": "end_turn", "blocks": [{ "type": "text", "markdown": "z".repeat(200_000) }] }));
  all.push(json!({ "role": "user", "text": "recent" }));
  all.push(json!({ "role": "agent", "stop": "end_turn", "blocks": [{ "type": "text", "markdown": "w".repeat(200_000) }] }));
  let record = fork_record(&h, Value::Array(all), |_| {});
  let s = reopened(&h, record).await;
  prompt(&s, "now").await;
  let text = agent_text(&last_turn(&view(&s)));
  assert!(text.contains("resource:acpira://history/"));
  assert!(text.contains("26 earlier turns were omitted"), "{}", &text[..text.len().min(400)]);
  assert!(text.contains("recent"));
  assert!(!text.contains("old-big"));
  assert!(notes.lock().unwrap().iter().any(|n| n.contains("26")));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_forks_copied_transcript_survives_its_first_prompt_being_cancelled_while_staging() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let record = fork_record(&h, json!([
    { "role": "user", "text": "earlier" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "before" }], "stop": "end_turn" },
  ]), |_| {});
  let s = reopened(&h, record).await;
  // Staging is held open on a gated attachment, so the cancel deterministically lands mid-staging — the send is dropped
  // before anything reaches the wire
  let gate = StagingGate::new();
  let sending = claimed(s.prompt("now".into(), gate.draft(), false, None, None));
  s.cancel().await;
  gate.release();
  sending.await.unwrap();
  assert_eq!(turn_count(&s), 2);
  assert!(s.to_record().history_pending);
  // The copy survived the cancel: the next attempt still hands it to the native session
  prompt(&s, "now").await;
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 4);
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "now", "edited": true }));
  assert!(agent_text(&vw["turns"][3]).contains("resource:acpira://history/"));
  assert!(!s.to_record().history_pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fork_never_adopts_the_agents_own_title() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let record = fork_record(&h, json!([
    { "role": "user", "text": "earlier" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "before" }], "stop": "end_turn" },
  ]), |r| {
    r.forked_from = serde_json::from_value(json!({ "sessionId": "source-session-id", "turnIndex": 1 })).unwrap();
    r.title = "Fork: earlier".into();
  });
  let s = reopened(&h, record).await;
  // The fake agent answers every prompt with session_info_update 'Fake title'; a fork never adopts it —
  // its 'Fork: …' title is provenance, not something the peer gets to re-derive from the injected blob
  prompt(&s, "now").await;
  assert_eq!(view(&s)["title"], "Fork: earlier");
  prompt(&s, "again").await;
  assert_eq!(view(&s)["title"], "Fork: earlier");
}
