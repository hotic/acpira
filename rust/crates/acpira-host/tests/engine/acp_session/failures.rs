//! Agent-published session failures (JetBrains AIR sessionFailure) and async tasks

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn failure_retry_upserts_the_warning_revision_and_the_error_settles_the_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "failure-retry").await;
  let turn = last_turn(&view(&s));
  expect_match(&turn, json!({ "stop": "error", "error": { "kind": "limit", "retryable": true, "failureId": "turn-1:error", "actions": ["retry"] } }));
  assert_eq!(turn["error"]["message"], "Rate limit exceeded\nTry again in a minute");
  // the retry warning (rev 1) is no row; the error (rev 2) is the only notice
  let notices: Vec<&Value> = turn["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "notice").collect();
  assert_eq!(notices.len(), 1);
  expect_match(notices[0], json!({ "type": "notice", "id": "turn-1:error", "revision": 2, "severity": "error", "category": "limit",
    "title": "Rate limit exceeded", "details": "Try again in a minute", "actions": ["retry"] }));
}

#[tokio::test(flavor = "multi_thread")]
async fn failure_dup_ignores_stale_revisions_and_keeps_a_different_id_separate() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "failure-dup").await;
  let notices: Vec<Value> = agent_blocks(&view(&s)).into_iter().filter(|b| b["type"] == "notice").collect();
  expect_match(notices, json!([{ "id": "dup", "revision": 2, "title": "upstream hiccup" }, { "id": "dup-2", "revision": 1, "title": "upstream hiccup" }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn failure_login_flips_the_session_to_auth_required() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "failure-login").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "error", "error": { "kind": "access", "failureId": "auth-1", "actions": ["login"] } }));
  assert_eq!(view(&s)["status"], "auth_required");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_login_failure_published_mid_turn_owns_the_rejected_prompt() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "failure-login-reject").await;
  let turn = last_turn(&view(&s));
  // The card shows the adapter's title / details and exactly its actions; the JSON-RPC code stays for the copy line
  expect_match(&turn, json!({ "stop": "error", "error": { "kind": "access", "code": -32603, "actions": ["login"] } }));
  assert!(turn["error"]["message"].as_str().unwrap().starts_with("Sign in to continue using Claude.\nFailed to authenticate"));
  let notice = turn["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "notice").unwrap();
  assert_eq!(turn["error"]["failureId"], notice["id"]);
  assert_eq!(view(&s)["status"], "auth_required");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failure_landing_after_the_turn_settled_appends_into_the_last_agent_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "failure-idle").await;
  until(|| last_turn(&view(&s))["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["type"] == "notice")), 5000).await;
  let last = last_turn(&view(&s));
  expect_match(&last, json!({ "stop": "end_turn" }));
  expect_match(last["blocks"].as_array().unwrap().last().unwrap(), json!({ "type": "notice", "id": "sess-1", "severity": "error", "category": "connection",
    "title": "Connection lost", "details": "upstream went away", "actions": ["new_session"] }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_failure_synthesizes_its_own_turn_and_a_load_replay_does_not_duplicate_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_SESSION_DIR": dir.path(), "FAKE_LOAD_ONLY": "1", "FAKE_LOAD_FAILURE": "1", "FAKE_IDLE_FAILURE": "1" } }));
  let s = started(&h, "/tmp").await;
  wait_turns(&s, 1).await;
  let idle = view(&s)["turns"][0].clone();
  expect_match(&idle, json!({ "role": "agent", "stop": "end_turn" }));
  expect_absent(&idle, "startedAt");
  expect_match(&idle["blocks"][0], json!({ "type": "notice", "id": "sess-1", "severity": "error", "category": "connection",
    "title": "Connection lost", "details": "upstream went away", "actions": ["new_session"] }));
  // restore: session/load replays the failure notification — the record's notice must not duplicate
  let rec = reopened(&h, s.to_record()).await;
  assert_eq!(view(&rec)["status"], "ready");
  let notices: Vec<Value> = agent_blocks(&view(&rec)).into_iter().filter(|b| b["type"] == "notice").collect();
  expect_match(notices, json!([{ "id": "sess-1", "revision": 1 }]));
}

// antigravity-acp 1.1.1 / 1.2.1 send a failed turn as the reply's last text and end_turn: the tail becomes the turn's
// error, output streamed before it stays
#[tokio::test(flavor = "multi_thread")]
async fn an_antigravity_failure_reply_becomes_the_turn_error() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "antigravity", json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "say:Reading the file.||\n\nAgent execution error: Agent execution terminated due to error. (\"request failed (code 400): User location is not supported for the API use.\")").await;
  let turn = last_turn(&view(&s));
  expect_match(&turn, json!({ "stop": "error", "error": { "kind": "region_unsupported", "retryable": true } }));
  assert!(turn["error"]["message"].as_str().unwrap().contains("User location is not supported"));
  let texts: Vec<Value> = turn["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "text").cloned().collect();
  assert_eq!(texts.len(), 1);
  assert_eq!(texts[0]["markdown"], "Reading the file.");
  expect_match(view(&s), json!({ "status": "ready", "running": false }));

  prompt(&s, "say:Agent execution error: Agent execution terminated due to error. (\"request failed (code 503): overloaded\")").await;
  let turn = last_turn(&view(&s));
  expect_match(&turn, json!({ "stop": "error", "error": { "kind": "agent_error", "message": "request failed (code 503): overloaded" } }));
  assert!(turn["blocks"].as_array().unwrap().iter().all(|b| b["type"] != "text"));

  // The same text from another agent is a reply
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "say:Agent execution error: boom").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
}
