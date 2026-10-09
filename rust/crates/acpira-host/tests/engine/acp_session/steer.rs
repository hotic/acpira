//! Steering queued prompts into the running turn

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn steer_joins_the_running_turn_as_a_steer_block_and_leaves_the_rest_queued() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "1" } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["canSteer"], true);
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  prompt(&s, "first").await;
  s.prompt("steer me".into(), drafts(json!([{ "kind": "text", "name": "note.txt", "text": "context" }])), false, None, None).await;
  let target = view(&s)["queued"][1].clone();
  let (a, b) = tokio::join!(s.steer_queued(target["id"].as_str().unwrap()), s.steer_queued(target["id"].as_str().unwrap()));
  a.unwrap();
  b.unwrap();
  // The steered entry leaves the queue at once; the other one waits for the turn as before
  let vw = view(&s);
  assert_eq!(vw["queued"].as_array().unwrap().len(), 1);
  expect_match(&vw["queued"][0], json!({ "text": "first" }));
  expect_absent(&vw["queued"][0], "sending");
  let steer = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "steer").cloned().expect("steer block");
  expect_match(&steer, json!({ "id": target["id"], "text": "steer me", "attachments": target["attachments"] }));
  until(|| agent_text(&view(&s)["turns"][1]).contains("steered:steer me"), 5000).await;
  running.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  let users: Vec<Value> = vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect();
  assert_eq!(users, [json!("slow"), json!("first")]);
  // The steer sits between the output before it and the reply to it, and the turn still ended normally
  let blocks = vw["turns"][1]["blocks"].as_array().unwrap();
  let at = blocks.iter().position(|b| b["type"] == "steer").unwrap();
  assert!(at > 0 && blocks[..at].iter().all(|b| b["type"] != "text" || b["streaming"] != true));
  assert!(blocks[at + 1..].iter().any(|b| b["type"] == "text" && b["markdown"].as_str().unwrap().starts_with("steered:steer me")));
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
  expect_absent(&vw["turns"][1], "error");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rebuilt_peer_receives_the_attachments_of_a_steered_prompt() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "1" } }));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  s.prompt("steer me".into(), drafts(json!([{ "kind": "text", "name": "note.txt", "text": "steered payload" }])), false, None, None).await;
  let id = view(&s)["queued"][0]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  running.await.unwrap();
  until(|| !s.is_running(), 5000).await;
  prompt(&s, "original").await;
  // Editing the next prompt rebuilds the peer from the transcript, the steered prompt's attachment included
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let wire = wire_prompt(&view(&s)["turns"][3]);
  let blocks = wire["prompt"].as_array().unwrap();
  let history = blocks[0]["resource"]["text"].as_str().or(blocks[0]["text"].as_str()).unwrap();
  assert!(history.contains(r#""user":"steer me","attachments":["note.txt"]"#), "{history}");
  let at = blocks.iter().position(|b| b["text"] == "Attachments from earlier user message: steer me").expect("attachment lead");
  assert!(blocks[at + 1].to_string().contains("steered payload"), "{}", blocks[at + 1]);
}

#[tokio::test(flavor = "multi_thread")]
async fn steer_answered_prompt_required_goes_out_first_when_the_turn_ends() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "idle" } }));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  prompt(&s, "first").await;
  prompt(&s, "second").await;
  let id = view(&s)["queued"][1]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  // Handed back: never cancels the turn, and the entry is first in line
  let vw = view(&s);
  assert_eq!(vw["running"], true);
  expect_match(&vw["queued"], json!([{ "text": "second" }, { "text": "first" }]));
  running.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  let users: Vec<Value> = vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect();
  assert_eq!(users, [json!("slow"), json!("second"), json!("first")]);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
  assert!(vw["turns"][1]["blocks"].as_array().unwrap().iter().all(|b| b["type"] != "steer"));
}

fn user_prompts(vw: &Value) -> Vec<Value> {
  vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn codex_steer_injects_into_a_bracketed_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "codex" } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["canSteer"], true);
  let running = spawn_prompt(&s, "slow");
  until(|| agent_text(&view(&s)["turns"][1]).contains("2 "), 5000).await;
  prompt(&s, "steer me").await;
  let id = view(&s)["queued"][0]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  until(|| agent_text(&view(&s)["turns"][1]).contains("steered:steer me"), 5000).await;
  running.await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(user_prompts(&vw), [json!("slow")]);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
  assert!(vw["turns"][1]["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "steer"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steer_that_lands_after_the_turn_runs_the_peers_own_turn_until_its_thread_is_idle() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "codex-late" } }));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  until(|| agent_text(&view(&s)["turns"][1]).contains("2 "), 5000).await;
  prompt(&s, "first").await;
  prompt(&s, "steer me").await;
  let id = view(&s)["queued"][1]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  running.await.unwrap();
  // The peer's own turn answers onto the same agent turn; the session stays running and the queue waits for it
  until(|| agent_text(&view(&s)["turns"][1]).contains("detached:steer me"), 5000).await;
  let vw = view(&s);
  assert_eq!(vw["running"], true);
  expect_match(&vw["queued"], json!([{ "text": "first" }]));
  assert_eq!(user_prompts(&vw), [json!("slow")]);
  expect_absent(&vw["turns"][1], "stop");
  let blocks = vw["turns"][1]["blocks"].as_array().unwrap();
  let at = blocks.iter().position(|b| b["type"] == "steer").expect("steer block");
  assert!(blocks[at + 1..].iter().any(|b| b["markdown"].as_str().is_some_and(|m| m.starts_with("detached:steer me"))));
  // Its idle settles the turn and releases the queue
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  assert_eq!(user_prompts(&vw), [json!("slow"), json!("first")]);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
  expect_match(&vw["turns"][3], json!({ "role": "agent", "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn no_steer_goes_out_once_the_peer_reported_the_turn_idle() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let log = dir.path().join("steer.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "codex-gap", "FAKE_STEER_LOG": log } }));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  prompt(&s, "first").await;
  until(|| agent_text(&view(&s)["turns"][1]).contains("9 "), 5000).await;
  tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  assert_eq!(view(&s)["running"], true);
  let id = view(&s)["queued"][0]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  running.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  assert!(!log.exists(), "a steer reached the peer after its idle");
  let vw = view(&s);
  assert_eq!(user_prompts(&vw), [json!("slow"), json!("first")]);
  assert!(vw["turns"][1]["blocks"].as_array().unwrap().iter().all(|b| b["type"] != "steer"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_steer_leaves_the_entry_queued_in_place() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "fail" } }));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  prompt(&s, "first").await;
  prompt(&s, "second").await;
  let id = view(&s)["queued"][1]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["running"], true);
  expect_match(&vw["queued"], json!([{ "text": "first" }, { "text": "second" }]));
  expect_absent(&vw["queued"][1], "sending");
  s.cancel().await;
  running.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn without_steering_support_steer_is_send_now() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  expect_absent(view(&s), "canSteer");
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  prompt(&s, "first").await;
  let id = view(&s)["queued"][0]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  running.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  expect_match(&view(&s)["turns"][1], json!({ "role": "agent", "stop": "cancelled" }));
  expect_match(&view(&s)["turns"][2], json!({ "role": "user", "text": "first" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steered_composer_send_joins_the_running_turn_without_a_queue_row() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "1" } }));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  s.steer_prompt("use pnpm".into(), vec![]).await.unwrap();
  // Steered on the spot: no queued row left behind, the message sits in the running turn
  let vw = view(&s);
  assert!(vw["queued"].is_null(), "{}", vw["queued"]);
  assert!(vw["turns"][1]["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "steer" && b["text"] == "use pnpm"));
  until(|| agent_text(&view(&s)["turns"][1]).contains("steered:use pnpm"), 5000).await;
  running.await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(user_prompts(&vw), [json!("slow")]);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steered_composer_send_without_steering_support_queues_and_never_cancels_the_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow");
  wait_turns(&s, 2).await;
  s.steer_prompt("later".into(), vec![]).await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["running"], true);
  expect_match(&vw["queued"], json!([{ "text": "later" }]));
  running.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  assert_eq!(user_prompts(&vw), [json!("slow"), json!("later")]);
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
}

fn compaction_of(turn: &Value) -> Option<Value> {
  turn["blocks"].as_array().into_iter().flatten().find(|b| b["type"] == "compaction").cloned()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steer_during_the_agents_own_compaction_waits_for_it_and_never_aborts_it() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "1" } }));
  let s = started(&h, "/tmp").await;
  let running = spawn_prompt(&s, "slow compacting");
  until(|| compaction_of(&view(&s)["turns"][1]).is_some_and(|b| b["status"] == "in_progress"), 5000).await;
  // Both the composer's steer and the queue row's Steer button wait: no steer on the wire, no cancel
  s.steer_prompt("use pnpm".into(), vec![]).await.unwrap();
  prompt(&s, "then lint").await;
  let id = view(&s)["queued"][1]["id"].as_str().unwrap().to_owned();
  s.steer_queued(&id).await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["running"], true);
  expect_match(&vw["queued"], json!([{ "text": "use pnpm" }, { "text": "then lint" }]));
  expect_match(compaction_of(&vw["turns"][1]).unwrap(), json!({ "status": "in_progress" }));
  // Once the compaction completes they steer in, in the order they were sent
  until(|| agent_text(&view(&s)["turns"][1]).contains("steered:then lint"), 5000).await;
  running.await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert!(vw["queued"].is_null(), "{}", vw["queued"]);
  assert_eq!(user_prompts(&vw), [json!("slow compacting")]);
  let blocks = vw["turns"][1]["blocks"].as_array().unwrap();
  let compaction = blocks.iter().position(|b| b["type"] == "compaction").unwrap();
  expect_match(&blocks[compaction], json!({ "status": "completed" }));
  expect_absent(&blocks[compaction], "error");
  let steers: Vec<(usize, Value)> = blocks.iter().enumerate().filter(|(_, b)| b["type"] == "steer").map(|(i, b)| (i, b["text"].clone())).collect();
  assert_eq!(steers.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>(), [json!("use pnpm"), json!("then lint")]);
  assert!(steers.iter().all(|(i, _)| *i > compaction));
  expect_match(&vw["turns"][1], json!({ "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_steered_composer_send_while_idle_is_an_ordinary_prompt() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STEERING": "1" } }));
  let s = started(&h, "/tmp").await;
  s.steer_prompt("hi".into(), vec![]).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(user_prompts(&vw), [json!("hi")]);
  assert!(vw["turns"][1]["blocks"].as_array().unwrap().iter().all(|b| b["type"] != "steer"));
}
