//! A child's own sent-off rounds coming back to it as follow-ups before its reply reaches the caller

use super::*;

/// A first round's own turns: its transcript opens with the task as a user turn, which this checks and drops
fn node_turns(s: &Disposing, id: &str) -> Vec<Value> {
  let (turns, ..) = s.subagent_transcript(id).unwrap();
  let mut turns = serde_json::from_str::<Value>(turns.get()).unwrap().as_array().cloned().unwrap_or_default();
  assert!(turns.first().is_some_and(|t| t["role"] == "user" && t["auto"].is_null()), "{turns:?}");
  turns.remove(0);
  turns
}

fn node_of(s: &Disposing, persona: &str) -> Value {
  nodes(s).into_iter().find(|n| n["harness"]["persona"] == persona).unwrap_or_else(|| panic!("no {persona} node"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_childs_sent_off_round_comes_back_to_it_in_a_follow_up_before_its_reply_reaches_the_caller() {
  let fake = fake_or_skip!();
  let (_h, s, roster) = hub_session(&fake).await;
  let call = r#"mcp-call:ask_agent {"agent":"fake-review","prompt":"relay: grand"}"#;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", call)).await);
  // The child's answer after the follow-up is the round's reply (the fake's default script)
  assert!(ok && text.starts_with("hello world"), "{text}");
  let child = node_of(&s, "fake-worker");
  let grand = node_of(&s, "fake-review");
  assert_eq!(grand["parentId"], child["id"]);
  assert_eq!(grand["state"], "completed");
  // The receipt, the sealed answer, the automatic message with the grandchild's reply, then the answer to it
  let turns = node_turns(&s, child["id"].as_str().unwrap());
  assert_eq!(turns.len(), 3, "{turns:?}");
  assert!(serde_json::to_string(&turns[0]).unwrap().contains("Sent to Fake Review"), "{}", turns[0]);
  assert_eq!(turns[0]["stop"], "end_turn");
  expect_match(&turns[1], json!({ "role": "user", "auto": true, "autoReason": "subagents" }));
  assert!(turns[1]["text"].as_str().unwrap().contains("reply to relay: grand"), "{}", turns[1]);
  // The root is never handed the grandchild's result
  until(|| !s.background_running(), 8000).await;
  assert!(deliveries(&s).is_empty(), "{:?}", deliveries(&s));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_gets_at_most_six_follow_ups_per_round() {
  let fake = fake_or_skip!();
  let (_h, s, roster) = hub_session(&fake).await;
  // The child summons again on every follow-up, so only the cap ends its round
  let call = r#"mcp-call-loop:ask_agent {"agent":"fake-review","prompt":"relay: quick"}"#;
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(60), ..ask("fake-worker", call) }).await);
  assert!(ok && text.contains("Sent to Fake Review"), "{text}");
  let child = node_of(&s, "fake-worker");
  let turns = node_turns(&s, child["id"].as_str().unwrap());
  let follow_ups = turns.iter().filter(|t| t["autoReason"] == "subagents").count();
  assert_eq!(follow_ups, 6, "{turns:?}");
  until(|| !s.background_running(), 8000).await;
  assert!(deliveries(&s).is_empty(), "{:?}", deliveries(&s));
}
