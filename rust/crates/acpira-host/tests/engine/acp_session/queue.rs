//! The follow-up queue: prompts sent while starting or running, edits, removal, Send now

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_while_starting_waits_for_ready_then_goes_out() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = Disposing(h.session("/tmp"));
  prompt(&s, "hi").await;
  assert_eq!(view(&s)["queued"].as_array().unwrap().iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("hi")]);
  s.start().await;
  until(|| {
    let vw = view(&s);
    (vw["turns"].as_array().unwrap().iter().any(|t| t["role"] == "agent") && vw["queued"].is_null()) || vw["status"] != "ready"
  }, 5000).await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  expect_absent(&vw, "queued");
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "hi" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_sent_while_running_goes_out_after_the_turn_ends() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["running"] == true, 5000).await;
  prompt(&s, "hi").await;
  assert_eq!(view(&s)["queued"].as_array().unwrap().iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("hi")]);
  s.cancel().await;
  p.await.unwrap();
  until(|| turn_count(&s) == 4 && view(&s)["running"] == false, 5000).await;
  expect_absent(view(&s), "queued");
}

fn queued_texts(s: &AcpSession) -> Vec<String> {
  view(s)["queued"].as_array().map(|q| q.iter().map(|x| x["text"].as_str().unwrap().to_owned()).collect()).unwrap_or_default()
}

// Several sends during one turn line up in order and go out one after another; attachments are staged at queue time so the
// queue row shows them, and the flushed turn carries the same blobs. Removing / editing addresses an entry by id
#[tokio::test(flavor = "multi_thread")]
async fn queued_prompts_keep_their_order_stage_images_at_once_and_can_be_edited_or_removed() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["running"] == true, 5000).await;
  s.prompt("first".into(), drafts(json!([{ "kind": "image", "mimeType": "image/png", "data": "AAAA", "name": "a.png" }])), false, None, None).await;
  prompt(&s, "second").await;
  prompt(&s, "third").await;
  let queued = view(&s)["queued"].clone();
  assert_eq!(queued_texts(&s), ["first", "second", "third"]);
  let staged = &queued[0]["attachments"][0];
  expect_match(staged, json!({ "kind": "image", "mimeType": "image/png", "name": "a.png" }));
  assert!(blob(&h, &s.id, staged["blob"].as_str().unwrap()).is_some());
  let id = |i: usize| queued[i]["id"].as_str().unwrap().to_owned();
  // Edit the first: new text, the image kept, a text draft added; the entry stays first
  s.edit_queued(&id(0), "first edited".into(), vec![0], drafts(json!([{ "kind": "text", "name": "n.md", "text": "x" }]))).await.unwrap();
  assert_eq!(queued_texts(&s), ["first edited", "second", "third"]);
  assert_eq!(view(&s)["queued"][0]["attachments"].as_array().unwrap().iter().map(|a| a["kind"].clone()).collect::<Vec<_>>(), [json!("image"), json!("text")]);
  // Remove the middle one; removing something already gone is a no-op, editing it is an error
  s.dequeue(&id(1));
  assert_eq!(queued_texts(&s), ["first edited", "third"]);
  s.dequeue(&id(1));
  assert!(s.edit_queued(&id(1), "x".into(), vec![], vec![]).await.is_err());
  // Emptying an entry removes it
  s.edit_queued(&id(2), "   ".into(), vec![], vec![]).await.unwrap();
  assert_eq!(queued_texts(&s), ["first edited"]);
  s.cancel().await;
  p.await.unwrap();
  until(|| turn_count(&s) == 4 && view(&s)["running"] == false, 5000).await;
  let vw = view(&s);
  expect_absent(&vw, "queued");
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "first edited", "attachments": [{ "kind": "image", "mimeType": "image/png" }, { "kind": "text", "name": "n.md" }] }));
  expect_absent(&vw["turns"][2], "edited");
}

#[tokio::test(flavor = "multi_thread")]
async fn send_now_cancels_the_active_turn_sends_the_selected_payload_once_and_keeps_the_rest_in_order() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  prompt(&s, "first").await;
  s.prompt("priority".into(), drafts(json!([{ "kind": "image", "mimeType": "image/png", "data": "AAAA", "name": "priority.png" }])), false, None, None).await;
  prompt(&s, "last").await;
  let selected = view(&s)["queued"][1].clone();
  let sid = selected["id"].as_str().unwrap();
  let (a, b) = tokio::join!(s.send_queued(sid), s.send_queued(sid));
  // One of the racing calls may be refused; the state below is what counts
  let _ = (a, b);
  running.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  let users: Vec<Value> = vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect();
  assert_eq!(users, [json!("slow"), json!("priority"), json!("first"), json!("last")]);
  expect_match(&vw["turns"][1], json!({ "role": "agent", "stop": "cancelled" }));
  expect_match(&vw["turns"][2], json!({ "role": "user", "attachments": selected["attachments"] }));
  // A stale row must not interrupt the next unrelated turn
  let next = spawn_prompt(&s, "slow again");
  wait_turns(&s, 10).await;
  s.send_queued(sid).await.ok();
  assert!(s.is_running());
  s.cancel().await;
  next.await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn send_now_during_attachment_staging_waits_for_the_cancelled_staging() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let gate = StagingGate::new();
  let running = tokio::spawn(s.prompt("original".into(), gate.draft(), false, None, None));
  until(|| view(&s)["running"] == true, 5000).await;
  prompt(&s, "priority").await;
  let qid = view(&s)["queued"][0]["id"].as_str().unwrap().to_owned();
  let send = tokio::spawn({
    let s = s.0.clone();
    async move { s.send_queued(&qid).await }
  });
  until(|| view(&s)["queued"][0]["sending"] == true, 5000).await;
  assert_eq!(view(&s)["turns"], json!([]));
  expect_match(&view(&s)["queued"][0], json!({ "text": "priority", "sending": true }));
  gate.release();
  running.await.unwrap();
  send.await.unwrap().ok();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let users: Vec<Value> = view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect();
  assert_eq!(users, [json!("priority")]);
}
