//! test/sessionErrors.test.ts, test/sessionFailure.test.ts and test/turnUsage.test.ts

use std::sync::Mutex;

use serde_json::{Value, json};

use acpira_host::acp::rpc::RpcError;
use acpira_host::acp::session_errors::{RestoreFailure, classify_restore_error};
use acpira_host::acp::session_failure::{failure_of, failure_turn_error};
use acpira_host::acp::turn_usage::turn_usage_of;

use crate::support::{expect_eq, expect_match, v};

fn rpc(code: i64, message: &str, data: Option<Value>) -> anyhow::Error {
  anyhow::Error::new(RpcError { code, message: message.into(), data })
}

fn invalid(message: &str) -> anyhow::Error {
  rpc(-32602, message, None)
}

#[test]
fn deepseek_harness_invalid_params_classify_by_their_message() {
  // packages/acp/acp/src/index.ts @ 0.1.6-alpha.2 reports all of these as a bare -32602
  assert_eq!(classify_restore_error(&invalid("unknown session: s1")), Some(RestoreFailure::Gone));
  assert_eq!(classify_restore_error(&invalid("session is already active: s1")), Some(RestoreFailure::Locked));
  assert_eq!(classify_restore_error(&invalid("session is not resumable: s1")), Some(RestoreFailure::Unresumable));
  assert_eq!(classify_restore_error(&invalid("session cwd does not match: /repo")), Some(RestoreFailure::Failed));
  assert_eq!(classify_restore_error(&invalid("mcp server \"fs\": command not found")), Some(RestoreFailure::Failed));
}

#[test]
fn a_bare_invalid_params_with_an_unknown_reason_still_means_gone() {
  assert_eq!(classify_restore_error(&rpc(-32602, "unknown session", Some(json!({ "sessionId": "s1" })))), Some(RestoreFailure::Gone));
}

#[test]
fn devins_typed_errors_classify_as_gone_and_locked() {
  assert_eq!(classify_restore_error(&rpc(-32016, "Session not found", Some(json!({ "cognition.ai/errorKind": "session_not_found" })))), Some(RestoreFailure::Gone));
  assert_eq!(classify_restore_error(&rpc(-32015, "Session is locked", Some(json!({ "cognition.ai/errorKind": "session_locked" })))), Some(RestoreFailure::Locked));
}

#[test]
fn method_not_found_means_no_restore_path() {
  assert_eq!(classify_restore_error(&anyhow::Error::new(RpcError::method_not_found("session/resume"))), None);
}

#[test]
fn anything_else_is_a_retryable_failure() {
  assert_eq!(classify_restore_error(&rpc(-32603, "transient restore failure", None)), Some(RestoreFailure::Failed));
  assert_eq!(classify_restore_error(&anyhow::anyhow!("connection lost")), Some(RestoreFailure::Failed));
}

fn meta(failure: Value) -> Value {
  json!({ "jetbrains": { "air": { "version": 1, "sessionFailure": failure } } })
}

fn valid() -> Value {
  json!({ "id": "turn-1:error", "revision": 3, "category": "limit", "severity": "error", "title": "Rate limit exceeded", "actions": ["retry"] })
}

fn with(k: &str, x: Value) -> Value {
  let mut f = valid();
  if x.is_null() {
    f.as_object_mut().unwrap().remove(k);
  } else {
    f[k] = x;
  }
  f
}

fn failure(m: Value) -> Option<Value> {
  failure_of(Some(&m), None).map(|f| json!({ "id": f.id, "revision": f.revision, "category": v(f.category), "severity": v(f.severity), "title": f.title, "details": f.details, "actions": v(f.actions) }))
}

#[test]
fn a_well_formed_payload_decodes_with_trimmed_strings_and_declared_actions() {
  expect_eq(failure(meta(with("details", json!(" try later ")))).unwrap(),
    json!({ "id": "turn-1:error", "revision": 3, "category": "limit", "severity": "error", "title": "Rate limit exceeded", "details": "try later", "actions": ["retry"] }));
  let mut reason = valid();
  reason["reason"] = json!("quota gone");
  assert_eq!(failure(meta(reason)).unwrap()["details"], "quota gone");
  assert!(failure(meta(valid())).unwrap()["details"].is_null());
}

#[test]
fn meta_without_a_session_failure_decodes_to_nothing() {
  assert!(failure_of(None, None).is_none());
  assert!(failure_of(Some(&json!({})), None).is_none());
  assert!(failure(meta(Value::Null)).is_none());
  assert!(failure_of(Some(&json!({ "jetbrains": { "air": { "version": 1 } } })), None).is_none());
}

#[test]
fn malformed_payloads_are_rejected_each_with_a_logged_reason() {
  let logs = Mutex::new(Vec::<String>::new());
  let log = |l: &str| logs.lock().unwrap().push(l.to_owned());
  for bad in [json!("oops"), with("id", json!("")), with("id", json!(7)), with("title", json!("  ")), with("revision", json!(0)), with("revision", json!(1.5)),
    with("revision", json!("2")), with("severity", json!("fatal")), with("actions", json!("retry"))] {
    assert!(failure_of(Some(&meta(bad.clone())), Some(&log)).is_none(), "{bad}");
  }
  let logs = logs.lock().unwrap();
  assert_eq!(logs.len(), 9);
  assert!(logs.iter().all(|l| l.starts_with("sessionFailure ignored")));
}

#[test]
fn an_unknown_category_degrades_and_unknown_or_duplicate_actions_are_filtered() {
  let mut f = with("category", json!("quota"));
  f["actions"] = json!(["retry", "cry", "retry", 7]);
  expect_match(failure(meta(f)).unwrap(), json!({ "category": "unknown", "actions": ["retry"] }));
  assert_eq!(failure(meta(with("category", Value::Null))).unwrap()["category"], "unknown");
  assert_eq!(failure(meta(with("actions", json!([])))).unwrap()["actions"], json!([]));
}

#[test]
fn the_turn_error_carries_the_id_the_actions_and_retryable_from_the_retry_action() {
  let f = failure_of(Some(&meta(valid())), None).unwrap();
  expect_eq(failure_turn_error(&f), json!({ "message": "Rate limit exceeded", "kind": "limit", "retryable": true, "failureId": "turn-1:error", "actions": ["retry"] }));
  let none = v(failure_turn_error(&failure_of(Some(&meta(with("actions", json!([])))), None).unwrap()));
  assert_eq!(none["retryable"], false);
  // An explicit empty list, so the alert card shows no generic Retry / Reconnect for it
  assert_eq!(none["actions"], json!([]));
  assert_eq!(failure_turn_error(&failure_of(Some(&meta(with("details", json!("wait a minute")))), None).unwrap()).message, "Rate limit exceeded\nwait a minute");
}

#[test]
fn the_standard_unstable_usage_field_is_read() {
  expect_eq(turn_usage_of(&json!({ "stopReason": "end_turn",
    "usage": { "totalTokens": 120, "inputTokens": 100, "outputTokens": 20, "cachedReadTokens": 64 },
    "_meta": { "cognition.ai/userMessageId": "req-devin-1" } })), json!({ "input": 100, "output": 20, "total": 120, "cachedRead": 64, "requestId": "req-devin-1" }));
}

#[test]
fn groks_meta_gives_token_counts_the_usage_object_model_and_request_ids() {
  expect_eq(turn_usage_of(&json!({ "stopReason": "end_turn", "_meta": {
    "sessionId": "s1", "requestId": "req-grok-1", "promptId": "p1", "modelId": "grok-4.6",
    "totalTokens": 38167, "inputTokens": 38140, "outputTokens": 20, "cachedReadTokens": 37888, "reasoningTokens": 19,
    "usage": { "inputTokens": 38140, "outputTokens": 20, "totalTokens": 38167, "cachedReadTokens": 37888, "cacheCreationTokens": 0,
      "reasoningTokens": 19, "modelCalls": 1, "apiDurationMs": 1200, "costUsdTicks": 42, "modelUsage": {}, "numTurns": 2 },
  } })), json!({ "input": 38140, "output": 20, "total": 38167, "cachedRead": 37888, "reasoning": 19, "cachedWrite": 0, "modelCalls": 1, "model": "grok-4.6", "requestId": "req-grok-1" }));
}

#[test]
fn the_standard_fields_win_over_meta() {
  expect_eq(turn_usage_of(&json!({ "stopReason": "end_turn", "usage": { "totalTokens": 10, "inputTokens": 8, "outputTokens": 2 },
    "_meta": { "inputTokens": 999, "modelId": "m", "requestId": "r" } })), json!({ "input": 8, "output": 2, "total": 10, "model": "m", "requestId": "r" }));
}

#[test]
fn a_response_without_usage_has_none() {
  assert!(turn_usage_of(&json!({ "stopReason": "end_turn" })).is_none());
  assert!(turn_usage_of(&json!({ "stopReason": "end_turn", "_meta": { "other": "stuff" } })).is_none());
}

#[test]
fn garbage_is_dropped_and_the_usable_fields_of_partial_metadata_are_kept() {
  assert!(turn_usage_of(&json!({ "stopReason": "end_turn", "usage": { "totalTokens": null, "inputTokens": -1, "outputTokens": "12" },
    "_meta": { "inputTokens": "x", "usage": "nope", "modelId": 5, "requestId": {}, "modelCalls": "2" } })).is_none());
  expect_eq(turn_usage_of(&json!({ "stopReason": "end_turn", "usage": { "totalTokens": 30, "inputTokens": -4, "outputTokens": 10 },
    "_meta": { "usage": { "modelCalls": "many", "cacheCreationTokens": 7 }, "modelId": "" } })), json!({ "output": 10, "total": 30, "cachedWrite": 7 }));
}
