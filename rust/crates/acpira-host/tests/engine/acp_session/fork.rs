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
async fn a_never_prompted_fork_whose_native_session_is_gone_starts_a_new_one_instead_of_going_read_only() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-gone").unwrap();
  let h = Harness::new(&fake, json!({}));
  // The fork opened a native session once, but no prompt ever reached it, so the peer kept nothing (a "gone" cwd
  // makes the fake answer resume with session_not_found)
  let record = fork_record(&h, json!([
    { "role": "user", "text": "earlier" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "before" }], "stop": "end_turn" },
  ]), |r| {
    r.cwd = "/tmp/acpira-gone".into();
    r.acp_session_id = Some("never-prompted".into());
  });
  let s = reopened(&h, record).await;
  assert_eq!(view(&s)["status"], "ready");
  assert_ne!(s.to_record().acp_session_id.as_deref(), Some("never-prompted"));
  assert!(s.to_record().history_pending);
  prompt(&s, "now").await;
  let vw = view(&s);
  assert_eq!(turns_in(&vw), 4);
  assert!(agent_text(&vw["turns"][3]).contains("resource:acpira://history/"));
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
  // Counted in turns as read_session numbers them (a message and its reply are one), behind an outline of the left-out
  // ones that opens with the task
  assert!(text.contains("13 earlier turns were omitted"), "{}", &text[..text.len().min(400)]);
  assert!(text.contains("Turns 1-13 of this conversation were left out") && text.contains("\n1. User: ask-0 | Reply: reply-0"), "{}", &text[..text.len().min(2000)]);
  let (outline, json) = text.split_once("The most recent turns follow as JSON.").expect("outline ahead of the JSON");
  assert!(outline.contains("13. User: old-big") && !outline.contains("recent"));
  assert!(json.contains("recent") && !json.contains("old-big"));
  assert!(notes.lock().unwrap().iter().any(|n| n.contains("13")));
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

fn two_pairs() -> Value {
  json!([
    { "role": "user", "text": "alpha" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "first reply" }], "stop": "end_turn" },
    { "role": "user", "text": "beta" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "second reply" }], "stop": "end_turn" },
  ])
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_a_copied_message_before_the_history_went_out_hands_the_shortened_prefix_over_once() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = reopened(&h, fork_record(&h, two_pairs(), |_| {})).await;
  let peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 2, "gamma")).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 4 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  let vw = view(&s);
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "gamma", "edited": true }));
  let text = agent_text(&vw["turns"][3]);
  // One history block, the fork's own lead, the edit point moved: the dropped pair is not in it
  assert_eq!(text.matches("resource:acpira://history/").count(), 1, "{text}");
  assert!(text.contains("forked from an earlier session") && text.contains("alpha") && !text.contains("beta"), "{text}");
  assert!(!text.contains("acpira://edit/"), "{text}");
  // The never-prompted native session is used as it is, not replaced by a rebuild
  assert_eq!(s.to_record().acp_session_id, peer);
  assert!(!s.to_record().history_pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forks_first_prompt_that_fails_empty_owes_its_history_again_and_retry_hands_it_over() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = reopened(&h, fork_record(&h, two_pairs(), |_| {})).await;
  prompt(&s, "please fail").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "error" }));
  assert!(s.to_record().history_pending);
  let tried = s.to_record().acp_session_id;
  s.retry_turn().await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 6 && last_turn(&view(&s))["stop"] == "end_turn", 5000).await;
  let text = agent_text(&last_turn(&view(&s)));
  assert_eq!(text.matches("resource:acpira://history/").count(), 1, "{text}");
  assert!(text.contains("alpha") && text.contains("beta"), "{text}");
  assert!(!s.to_record().history_pending);
  // The failed attempt went out and may sit in that native session: the handoff goes to a fresh one
  assert_ne!(s.to_record().acp_session_id, tried);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forks_history_keeps_the_summoned_agents_replies_an_automatic_message_delivered() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let record = fork_record(&h, json!([
    { "role": "user", "text": "look into it" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "sent it off" }], "stop": "end_turn" },
    { "role": "user", "text": "(Acpira) A summoned agent you sent off has finished. Its reply: the-finding", "auto": true, "autoReason": "subagents" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "noted" }], "stop": "end_turn" },
  ]), |_| {});
  let s = reopened(&h, record).await;
  prompt(&s, "now").await;
  let text = agent_text(&last_turn(&view(&s)));
  assert!(text.contains("the-finding") && text.contains("summoned agents' replies"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forks_first_prompt_stopped_before_any_output_owes_its_history_to_the_next_message() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = reopened(&h, fork_record(&h, two_pairs(), |_| {})).await;
  prompt(&s, "cancel-empty-once").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "cancelled" }));
  assert!(s.to_record().history_pending);
  prompt(&s, "next").await;
  let text = agent_text(&last_turn(&view(&s)));
  assert!(text.contains("resource:acpira://history/") && text.contains("alpha"), "{text}");
  assert!(!s.to_record().history_pending);
  // Once delivered, a later prompt carries no history
  prompt(&s, "after").await;
  assert!(!agent_text(&last_turn(&view(&s))).contains("resource:acpira://history/"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cross_agent_fork_names_the_source_agent_and_an_unsendable_history_image_becomes_a_label() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
  let record = fork_record(&h, json!([
    { "role": "user", "text": "alpha", "attachments": [{ "kind": "image", "mimeType": "image/png", "blob": "gone.png", "name": "shot.png" }] },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "first reply" }], "stop": "end_turn" },
  ]), |r| {
    r.forked_from = serde_json::from_value(json!({ "sessionId": "source-session-id", "turnIndex": 1, "agent": "claude", "model": "Opus" })).unwrap();
  });
  let s = reopened(&h, record).await;
  prompt(&s, "inspect-history").await;
  let wire = wire_prompt(&last_turn(&view(&s)))["prompt"].to_string();
  assert!(wire.contains("forked from an earlier session") && wire.contains("All of it ran on another agent (claude, model Opus), not on you"), "{wire}");
  assert!(wire.contains("re-read a file before editing it"), "{wire}");
  assert!(wire.contains("Attachments from earlier user message: alpha (not included: shot.png)"), "{wire}");
  assert!(!s.to_record().history_pending);
  // An edit's rebuilt history says the same of the copied turns, and only of them
  prompt(&s, "later").await;
  s.edit_turn(history_edit(&s, 4, "inspect-history")).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 6 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  let wire = wire_prompt(&last_turn(&view(&s)))["prompt"].to_string();
  assert!(wire.contains("Conversation before the edited message") && wire.contains("Turns 1-1 of it ran on another agent (claude, model Opus)"), "{wire}");
  assert!(wire.contains("(not included: shot.png)"), "{wire}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_same_agent_forks_lead_is_the_plain_fork_lead() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
  let s = reopened(&h, fork_record(&h, two_pairs(), |r| {
    r.forked_from = serde_json::from_value(json!({ "sessionId": "source-session-id", "turnIndex": 3 })).unwrap();
  })).await;
  prompt(&s, "inspect-history").await;
  let wire = wire_prompt(&last_turn(&view(&s)));
  let history = wire["prompt"][0]["text"].as_str().unwrap();
  let lead = acpira_host::acp::session::edit::FORK_HISTORY_LEAD;
  assert!(history.starts_with(&format!("{lead}\n[")), "{history}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_fork_whose_agent_can_read_its_own_session_is_told_how_and_pi_is_not() {
  let fake = fake_or_skip!();
  // Pi takes the handed-over servers and starts none of them, so the pointer would send it to a tool it does not have
  for (id, readable) in [("fake", true), ("pi", false)] {
    let mut h = Harness::for_agent(&fake, id, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
    h.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new("/opt/acpira/bin/acpira"));
    let record = fork_record(&h, two_pairs(), |r| {
      r.forked_from = serde_json::from_value(json!({ "sessionId": "source-session-id", "turnIndex": 3 })).unwrap();
    });
    let s = reopened(&h, record).await;
    prompt(&s, "inspect-history").await;
    let wire = wire_prompt(&last_turn(&view(&s)))["prompt"].to_string();
    let pointer = format!("This conversation is the Acpira session acpira://session/{}", s.id);
    assert_eq!(wire.contains(&pointer), readable, "{id}: {wire}");
    assert!(wire.contains("alpha"), "{id}: {wire}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_views_fork_boundary_shrinks_with_an_edit_into_the_copied_turns_and_goes_when_none_are_left() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = reopened(&h, fork_record(&h, two_pairs(), |r| {
    r.forked_from = serde_json::from_value(json!({ "sessionId": "source-session-id", "turnIndex": 3, "agent": "claude" })).unwrap();
    r.title = "Fork: alpha".into();
  })).await;
  expect_match(view(&s), json!({ "forkedFrom": { "sessionId": "source-session-id", "turnIndex": 3, "agent": "claude" } }));
  s.edit_turn(history_edit(&s, 2, "gamma")).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 4 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  expect_match(view(&s), json!({ "forkedFrom": { "turnIndex": 1 } }));
  s.edit_turn(history_edit(&s, 0, "delta")).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 2 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  expect_absent(view(&s), "forkedFrom");
  // The origin itself stays on the record: it still keeps the agent's own titles away
  let record = s.to_record();
  assert_eq!(record.forked_from.map(|f| f.turn_index), Some(-1));
  assert_eq!(view(&s)["title"], "Fork: alpha");
}
