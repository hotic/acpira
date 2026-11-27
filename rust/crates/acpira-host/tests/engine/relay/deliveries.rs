//! Replies the root did not wait for, delivered in one message per batch, retried, voided and capped

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_root_call_returns_at_once_and_its_turns_replies_arrive_together_in_one_message() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, send_off("fake-review", "relay: quick look")).await);
  assert!(ok && text.starts_with("Sent to Fake Review (fake) (thread: ") && text.contains("Do not poll"), "{text}");
  let (_, text) = last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: slow change")).await);
  let slow = thread_of(&text);
  // The quick one ended first, but its turn's batch is still running: nothing is delivered yet
  until(|| nodes(&s).first().is_some_and(|n| n["state"] == "completed"), 8000).await;
  assert!(deliveries(&s).is_empty());
  until(|| deliveries(&s).len() == 1 && !s.is_running(), 8000).await;
  let text = deliveries(&s)[0]["text"].as_str().unwrap().to_owned();
  assert!(text.contains("2 summoned agents") && text.contains("reply to relay: quick look") && text.contains("reply to relay: slow change"), "{text}");
  // Reading a delivered round again is allowed and delivers nothing more
  let (ok, _) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(slow), ..ask("", "") }).await);
  assert!(ok);
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(deliveries(&s).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_early_round_is_delivered_when_it_ends_and_names_the_rounds_of_its_batch_still_running() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  last_text(&run(&s, &roster, 0, AskArgs { early: Some(true), ..send_off("fake-review", "relay: quick look") }).await);
  let (_, text) = last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: slow change")).await);
  let slow = thread_of(&text);
  // The racer's reply goes out while the batch still runs, and says who is still at it
  until(|| deliveries(&s).len() == 1, 8000).await;
  let first = deliveries(&s)[0]["text"].as_str().unwrap().to_owned();
  assert!(first.contains("reply to relay: quick look") && !first.contains("reply to relay: slow change"), "{first}");
  assert!(first.contains(&format!("Still running: Fake Worker (fake) (thread: {slow})")), "{first}");
  // The rest of the batch follows on its own once it ends
  until(|| deliveries(&s).len() == 2 && !s.is_running(), 8000).await;
  let second = deliveries(&s)[1]["text"].as_str().unwrap().to_owned();
  assert!(second.contains("reply to relay: slow change") && !second.contains("Still running"), "{second}");
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_a_delivery_that_failed_before_any_output_sends_the_same_replies_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  // The persona's name puts "fail" in the delivery, which the fake root fails once before any output
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster.save(&json!([{ "name": "Fake failover", "agent": "fake", "mode": "consult", "when": "checks" }])).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  last_text(&run(&s, &roster, 0, send_off("fake-failover", "relay: quick look")).await);
  until(|| deliveries(&s).len() == 1 && !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 8000).await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "error" }));
  s.retry_turn().await.unwrap();
  until(|| !s.is_running() && last_turn(&view(&s))["stop"] == "end_turn", 8000).await;
  // The failed attempt is replaced by the resent message, still an automatic delivery with the same replies
  let sent = deliveries(&s);
  assert_eq!(sent.len(), 1, "{sent:?}");
  assert!(sent[0]["text"].as_str().unwrap().contains("reply to relay: quick look"), "{}", sent[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_a_call_already_read_is_not_delivered_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: slow job")).await);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread_of(&text)), ..ask("", "") }).await);
  assert!(ok && text.starts_with("reply to relay: slow job"), "{text}");
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert!(deliveries(&s).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_stop_voids_what_was_owed() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: slow job")).await);
  until(|| nodes(&s).first().is_some_and(|n| n["peer"]["sessionId"].is_string()), 8000).await;
  s.cancel().await;
  until(|| !s.background_running(), 4000).await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert!(deliveries(&s).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn automatic_deliveries_stop_after_six_per_prompt_of_the_users() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  for i in 1..=6 {
    last_text(&run(&s, &roster, 0, send_off("fake-review", &format!("relay: look {i}"))).await);
    until(|| deliveries(&s).len() == i && !s.is_running(), 8000).await;
  }
  let (_, text) = last_text(&run(&s, &roster, 0, send_off("fake-review", "relay: look 7")).await);
  until(|| nodes(&s).last().is_some_and(|n| n["state"] == "completed"), 8000).await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(deliveries(&s).len(), 6);
  // The user's next prompt opens the count again: the seventh reply follows that turn
  prompt(&s, "say:what now").await;
  until(|| deliveries(&s).len() == 7, 8000).await;
  assert!(deliveries(&s)[6]["text"].as_str().unwrap().contains(&thread_of(&text)));
}
