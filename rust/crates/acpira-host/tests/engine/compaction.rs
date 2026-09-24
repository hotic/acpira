//! test/compaction.test.ts and test/grok-usage.test.ts

use serde_json::{Value, json};

use acpira_host::acp::compaction::{CompactionCompletion, is_compact_command};
use acpira_host::acp::session_prompt::grok_context_usage;

use crate::acp_session::{agent_blocks, prompt, spawn_prompt, view};
use crate::fake_or_skip;
use crate::support::{Disposing, Harness, expect_eq, until, v};

fn text(t: &str) -> Value {
  json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": t } })
}

fn feed(c: &mut CompactionCompletion, message: &str) {
  for ch in message.chars() {
    c.update(&text(&ch.to_string()));
  }
}

#[tokio::test]
async fn terminal_text_releases_the_wait_even_when_fragmented() {
  for (agent, message) in [
    ("devin", "Context compacted"),
    ("devin", "Nothing to compact."),
    ("devin", "Compaction canceled."),
    ("devin", "Force compaction failed: unavailable"),
    ("devin", "Compaction failed: unavailable"),
    ("kimi", "Compaction completed.\n- Tokens after: 1234"),
    ("kimi", "Compaction cancelled."),
    ("kimi", "Compaction is blocked by the current turn; retry when the turn is idle."),
    ("kimi", "/compact failed: No messages to compact in current history."),
  ] {
    let mut c = CompactionCompletion::new(Some(agent));
    let waiting = c.wait().expect("armed");
    feed(&mut c, message);
    waiting.await.unwrap();
    assert!(c.wait().is_none(), "{agent}: {message}");
  }
}

#[test]
fn a_completion_before_the_acknowledgement_needs_no_later_wait() {
  let mut c = CompactionCompletion::new(Some("devin"));
  c.update(&text("Nothing to compact."));
  assert!(c.wait().is_none());
}

#[test]
fn ordinary_prose_and_synchronous_peers_arm_no_latch() {
  for agent in [None, Some("grok"), Some("custom")] {
    let mut c = CompactionCompletion::new(agent);
    c.update(&text("Compacting context…"));
    assert!(c.wait().is_none());
  }
}

#[tokio::test]
async fn structured_events_replace_the_text_fallback_and_await_every_id() {
  let mut c = CompactionCompletion::new(Some("devin"));
  for id in ["a", "b"] {
    c.update(&json!({ "sessionUpdate": "compaction_update", "compactionId": id, "status": "in_progress" }));
  }
  let mut waiting = c.wait().unwrap();
  c.update(&text("Context compacted"));
  c.update(&json!({ "sessionUpdate": "compaction_update", "compactionId": "a", "status": "completed" }));
  assert!(waiting.try_recv().is_err());
  c.update(&json!({ "sessionUpdate": "compaction_update", "compactionId": "b", "status": "failed" }));
  waiting.await.unwrap();
}

#[tokio::test]
async fn failure_or_disposal_releases_an_outstanding_wait() {
  let mut c = CompactionCompletion::new(Some("kimi"));
  let waiting = c.wait().unwrap();
  c.close();
  waiting.await.unwrap();
  assert!(c.wait().is_none());
}

#[tokio::test]
async fn the_reported_post_compaction_token_count_is_captured() {
  let mut c = CompactionCompletion::new(Some("kimi"));
  let waiting = c.wait().unwrap();
  feed(&mut c, "Compaction completed.\n- Messages compacted: 3\n- Tokens after: 24,898");
  waiting.await.unwrap();
  assert_eq!(c.tokens_after, Some(24898.0));
}

#[test]
fn no_token_count_comes_from_cancellations_failures_or_prose_without_a_result_line() {
  for message in ["Compaction cancelled.", "/compact failed: No messages to compact in current history.", "Context compacted"] {
    let mut c = CompactionCompletion::new(Some("kimi"));
    feed(&mut c, message);
    assert!(c.tokens_after.is_none(), "{message}");
  }
}

#[test]
fn token_counts_are_never_mined_from_ordinary_prose() {
  let mut c = CompactionCompletion::new(None);
  feed(&mut c, "Compaction completed.\n- Tokens after: 42");
  assert!(c.tokens_after.is_none());
}

#[test]
fn compact_instructions_are_told_apart_from_other_slash_commands() {
  assert!(is_compact_command(" /compact focus on the tests "));
  assert!(is_compact_command("/compact"));
  assert!(!is_compact_command("/compactor"));
  assert!(!is_compact_command("describe /compact"));
}

#[test]
fn exact_zero_and_overfull_windows_are_accepted_without_spend() {
  for used in [0, 16378, 260000] {
    expect_eq(grok_context_usage(&json!({ "result": { "sessionId": "s1", "context": { "used": used, "total": 250000 } }, "_meta": { "usage": { "totalTokens": 999999 } } }), "s1"),
      json!({ "used": used, "size": 250000 }));
  }
}

#[test]
fn unknown_mismatched_or_invalid_usage_is_rejected() {
  let mut bad = vec![Value::Null, json!({}), json!({ "_meta": { "usage": { "inputTokens": 100, "totalTokens": 120 } } }), json!({ "result": { "sessionId": "other", "context": { "used": 1, "total": 2 } } })];
  for used in [json!(-1), Value::Null, json!("100"), json!(1.5)] {
    bad.push(json!({ "result": { "sessionId": "s1", "context": { "used": used, "total": 250000 } } }));
  }
  for total in [json!(0), json!(-1), json!("250000")] {
    bad.push(json!({ "result": { "sessionId": "s1", "context": { "used": 10, "total": total } } }));
  }
  for b in bad {
    assert!(grok_context_usage(&b, "s1").is_none(), "{b}");
  }
}

fn grok(fake: &crate::support::FakeAgent, style: &str, auto: bool) -> Harness {
  let mut h = Harness::for_agent(fake, "grok", json!({ "env": { "FAKE_GROK_USAGE": style } }));
  h.deps.compaction = Some(std::sync::Arc::new(move || acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto }));
  h
}

async fn started(h: &Harness) -> Disposing {
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  s
}

fn usage(s: &acpira_host::acp::session::AcpSession) -> Value {
  view(s)["usage"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn live_context_is_published_and_persisted_model_size_refreshes_and_spend_is_ignored() {
  let fake = fake_or_skip!();
  let h = grok(&fake, "context", false);
  let s = started(&h).await;
  until(|| usage(&s) == json!({ "used": 1234, "size": 1_000_000 }), 3000).await;
  prompt(&s, "big").await;
  until(|| usage(&s) == json!({ "used": 401234, "size": 1_000_000 }), 3000).await;
  assert_eq!(v(s.to_record().usage), usage(&s));
  s.set_config("model".into(), "m2".into()).await.unwrap();
  until(|| usage(&s) == json!({ "used": 401234, "size": 250_000 }), 3000).await;
  s.compact(false).await.unwrap();
  until(|| usage(&s) == json!({ "used": 80247, "size": 250_000 }), 3000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn automatic_compaction_runs_from_the_live_snapshot_and_refreshes_after() {
  let fake = fake_or_skip!();
  let h = grok(&fake, "context", true);
  let s = started(&h).await;
  prompt(&s, "big").await;
  until(|| usage(&s)["used"] == 80247, 5000).await;
  until(|| !s.is_running(), 5000).await;
  let autos = |s: &acpira_host::acp::session::AcpSession| view(s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user" && t["auto"] == true).count();
  assert_eq!(autos(&s), 1);
  prompt(&s, "hi").await;
  assert_eq!(autos(&s), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn unsupported_or_malformed_context_stays_unknown_without_breaking_prompts() {
  let fake = fake_or_skip!();
  for style in ["unsupported", "malformed"] {
    let h = grok(&fake, style, true);
    let s = started(&h).await;
    prompt(&s, "big").await;
    assert_eq!(view(&s)["status"], "ready");
    assert!(usage(&s).is_null(), "{style}");
    assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 2);
    if style == "unsupported" {
      assert_eq!(h.logs().iter().filter(|l| l.contains("context unavailable")).count(), 1);
    }
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn context_refreshes_while_a_turn_is_still_on_the_wire() {
  let fake = fake_or_skip!();
  let h = grok(&fake, "context", false);
  let s = started(&h).await;
  until(|| usage(&s) == json!({ "used": 1234, "size": 1_000_000 }), 3000).await;
  let p = spawn_prompt(&s, "slow");
  until(|| usage(&s)["used"].as_f64().unwrap_or(0.0) > 1234.0, 4000).await;
  assert!(s.is_running());
  s.cancel().await;
  p.await.unwrap();
  assert!(usage(&s)["used"].as_f64().unwrap() > 1234.0);
  assert_eq!(usage(&s)["size"], 1_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn standard_usage_notifications_win_over_the_grok_fallback() {
  let fake = fake_or_skip!();
  let h = grok(&fake, "context", false);
  let s = started(&h).await;
  let p = spawn_prompt(&s, "tool");
  until(|| agent_blocks(&view(&s)).iter().any(|b| b["type"] == "permission"), 5000).await;
  let perm = agent_blocks(&view(&s)).into_iter().find(|b| b["type"] == "permission").unwrap();
  s.resolve_permission(perm["id"].as_str().unwrap(), "reject");
  p.await.unwrap();
  expect_eq(usage(&s), json!({ "used": 1234, "size": 100000, "cost": 0.01 }));
}

#[tokio::test(flavor = "multi_thread")]
async fn polling_continues_during_quiet_model_work() {
  let fake = fake_or_skip!();
  let h = grok(&fake, "context", false);
  let s = started(&h).await;
  let p = spawn_prompt(&s, "quiet-context");
  until(|| usage(&s)["used"] == 42_000, 2650).await;
  assert!(s.is_running());
  p.await.unwrap();
}
