//! Adapter prose turned into rows: compaction, retries and their warnings, prose outcomes

use super::*;

fn compaction_rows(s: &NormalizeState) -> Vec<(String, Option<String>)> {
  s.turns
    .iter()
    .filter_map(Turn::as_agent)
    .flat_map(|t| &t.blocks)
    .filter_map(|b| match b {
      AgentBlock::Compaction(c) => Some((v(c.status).as_str().unwrap_or("").to_owned(), c.error.clone())),
      _ => None,
    })
    .collect()
}

// pi-acp 0.0.33 reports automatic compaction mid-turn as two adapter sentences; they become one compaction row in place
#[test]
fn adapter_compaction_prose_mid_turn_becomes_a_compaction_row() {
  let mut s = NormalizeState { agent: Some("pi".into()), ..state() };
  apply(&mut s, chunk("Reading the file"));
  apply(&mut s, chunk("Context nearing limit, running automatic compaction..."));
  expect_match(block(&s, 0, 1), json!({ "type": "compaction", "status": "in_progress" }));
  apply(&mut s, chunk("Automatic compaction finished; context was summarized to continue the session."));
  apply(&mut s, chunk("Continuing"));
  let blocks = v(&s.turns[0])["blocks"].clone();
  expect_eq(&blocks[0]["markdown"], json!("Reading the file"));
  expect_match(&blocks[1], json!({ "type": "compaction", "status": "completed" }));
  expect_eq(&blocks[2]["markdown"], json!("Continuing"));
  assert_eq!(blocks.as_array().unwrap().len(), 3);
}

fn retry_of(s: &NormalizeState) -> Value {
  v(&s.turns[s.turns.len() - 1])["retry"].clone()
}

fn retry_rows(s: &NormalizeState) -> Vec<Value> {
  let blocks = v(&s.turns[s.turns.len() - 1])["blocks"].clone();
  blocks.as_array().unwrap().iter().filter(|b| b["type"] == "notice" && b.get("retry").is_some()).cloned().collect()
}

// pi-acp 0.0.33 forwards pi's auto_retry_start / auto_retry_end as prose: never reply text. The live turn is labelled
// as retrying until the resume line, and one retry row logs the adapter's lines until content follows
#[test]
fn adapter_retry_prose_labels_the_turn_and_logs_one_retry_row() {
  let mut s = NormalizeState { agent: Some("pi".into()), ..state() };
  apply(&mut s, chunk("Reading"));
  apply(&mut s, chunk("Retrying (attempt 1/3, waiting 2s)..."));
  apply(&mut s, chunk("Retrying (attempt 2/3, waiting 4s)..."));
  expect_match(retry_of(&s), json!({ "attempt": 2, "max": 3 }));
  assert!(retry_of(&s)["detail"].as_str().unwrap().contains("2/3"));
  let rows = retry_rows(&s);
  assert_eq!(rows.len(), 1);
  expect_match(&rows[0], json!({ "severity": "warning", "retry": { "status": "in_progress", "max": 3, "attempts": [
    { "attempt": 1, "text": "Retrying (attempt 1/3, waiting 2s)..." },
    { "attempt": 2, "text": "Retrying (attempt 2/3, waiting 4s)..." },
  ] } }));
  assert!(rows[0]["retry"]["nextAt"].as_f64().is_some());
  // The resume line drops the label but leaves the row open: pi-acp sends it after a last failed attempt too
  apply(&mut s, chunk("Retry finished, resuming."));
  assert!(retry_of(&s).is_null());
  expect_eq(&retry_rows(&s)[0]["retry"]["status"], json!("in_progress"));
  apply(&mut s, chunk("Retrying..."));
  expect_match(retry_of(&s), json!({ "detail": "Model request failed, retrying…" }));
  // Content after it: the retries worked
  apply(&mut s, chunk(" Done"));
  let blocks = v(&s.turns[0])["blocks"].clone();
  assert_eq!(blocks.as_array().unwrap().len(), 3);
  expect_eq(&blocks[0]["markdown"], json!("Reading"));
  expect_match(&blocks[1], json!({ "type": "notice", "retry": { "status": "completed" } }));
  assert_eq!(blocks[1]["retry"]["attempts"].as_array().unwrap().len(), 3);
  assert!(blocks[1]["retry"].get("nextAt").is_none());
  expect_eq(&blocks[2]["markdown"], json!(" Done"));
  assert!(retry_of(&s).is_null());
  // A turn that ends mid-retry drops the label, and its row never got through
  apply(&mut s, chunk("Retrying..."));
  end_turn(&mut s, TurnStop::EndTurn);
  assert!(retry_of(&s).is_null());
  let rows = retry_rows(&s);
  assert_eq!(rows.len(), 2);
  expect_eq(&rows[1]["retry"]["status"], json!("failed"));
}

fn air_failure(id: &str, revision: u32, severity: &str, category: &str, title: &str, actions: Value) -> Value {
  json!({ "sessionUpdate": "session_info_update", "_meta": { "jetbrains": { "air": { "version": 1, "sessionFailure": {
    "id": id, "revision": revision, "severity": severity, "category": category, "title": title, "actions": actions } } } } })
}

// claude-agent-acp 0.81.0 api_retry: a session-scoped id per attempt (`…:session-error:<epoch>:<n>`, seen in stored
// transcripts 2026-09-26). The live turn's label carries the attempt, and every attempt lands on one retry row
#[test]
fn air_retry_warnings_label_the_live_turn_and_share_one_retry_row() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "Planning" } }));
  for n in 1..=3 {
    apply(&mut s, air_failure(&format!("s:session-error:e:{n}"), 1, "warning", "connection", &format!("Reconnecting to Claude, attempt {n} of 10."), json!([])));
  }
  expect_eq(retry_of(&s), json!({ "attempt": 3, "max": 10, "detail": "Reconnecting to Claude, attempt 3 of 10." }));
  let rows = retry_rows(&s);
  assert_eq!(rows.len(), 1);
  expect_match(&rows[0], json!({ "category": "connection", "retry": { "status": "in_progress", "max": 10, "reconnect": true } }));
  assert_eq!(rows[0]["retry"]["attempts"].as_array().unwrap().len(), 3);
  // An advisory is still a row of its own
  apply(&mut s, air_failure("s:notice:e:1", 1, "warning", "unknown", "Switched to the fallback model.", json!([])));
  apply(&mut s, json!({ "sessionUpdate": "tool_call", "toolCallId": "t1", "title": "Read", "kind": "read", "status": "in_progress" }));
  let types: Vec<Value> = v(&s.turns[0])["blocks"].as_array().unwrap().iter().map(|b| b["type"].clone()).collect();
  expect_eq(types, json!(["thought", "notice", "notice", "tool_call"]));
  expect_eq(&retry_rows(&s)[0]["retry"]["status"], json!("completed"));
  assert!(retry_of(&s).is_null());
  // Between turns there is nothing to label or log
  end_turn(&mut s, TurnStop::EndTurn);
  apply(&mut s, air_failure("s:session-error:e:4", 1, "warning", "connection", "Reconnecting to Claude, attempt 1 of 10.", json!([])));
  assert!(retry_of(&s).is_null());
  assert_eq!(s.turns.len(), 1);
  assert_eq!(retry_rows(&s).len(), 1);
}

// Retries that end in a real error: the error row appears, the label goes and the retry row reads as failed
#[test]
fn an_air_error_clears_the_retry_label_and_fails_the_retry_row() {
  let mut s = state();
  apply(&mut s, chunk("Working"));
  apply(&mut s, air_failure("turn-1:error", 1, "warning", "service", "Retrying Claude, attempt 1 of 2.", json!([])));
  apply(&mut s, air_failure("turn-1:error", 2, "error", "service", "Claude is temporarily overloaded.", json!(["retry"])));
  let blocks = v(&s.turns[0])["blocks"].clone();
  assert_eq!(blocks.as_array().unwrap().len(), 3);
  expect_match(&blocks[1], json!({ "type": "notice", "retry": { "status": "failed", "attempts": [{ "attempt": 1, "text": "Retrying Claude, attempt 1 of 2." }] } }));
  expect_match(&blocks[2], json!({ "type": "notice", "severity": "error", "id": "turn-1:error", "revision": 2 }));
  assert!(retry_of(&s).is_null());
}

// The same attempt reported by several sources is one entry with the best text: the adapter's title, the raw SDK
// frame's status and kind, then the body from the CLI's log, which also reaches a row that already settled
#[test]
fn a_better_report_of_an_attempt_replaces_its_text_and_a_vaguer_one_never_does() {
  let mut s = state();
  apply(&mut s, chunk("Working"));
  let frame = |n: u32| RetryReport {
    attempt: Some(n), max: Some(10), text: format!("529 overloaded #{n}"), quality: RetryText::Summary, reconnect: false, wait_ms: Some(12_000),
  };
  apply(&mut s, air_failure("e1", 1, "warning", "service", "Retrying Claude, attempt 1 of 10.", json!([])));
  assert!(report_retry(&mut s, frame(1)));
  apply(&mut s, air_failure("e2", 1, "warning", "service", "Retrying Claude, attempt 2 of 10.", json!([])));
  assert!(report_retry(&mut s, frame(2)));
  // The adapter's title for attempt 2 arriving late does not undo the frame's text
  apply(&mut s, air_failure("e2b", 1, "warning", "service", "Retrying Claude, attempt 2 of 10.", json!([])));
  let row = &retry_rows(&s)[0];
  expect_match(&row["retry"]["attempts"], json!([{ "attempt": 1, "text": "529 overloaded #1" }, { "attempt": 2, "text": "529 overloaded #2" }]));
  assert!(row["retry"]["nextAt"].as_f64().is_some());
  apply(&mut s, chunk(" again"));
  let body = r#"529 {"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
  assert!(refine_retry(&mut s, 2, body, RetryText::Raw));
  assert!(!refine_retry(&mut s, 2, "529 overloaded", RetryText::Summary));
  assert!(!refine_retry(&mut s, 7, body, RetryText::Raw));
  let row = &retry_rows(&s)[0];
  expect_match(row, json!({ "details": body, "retry": { "status": "completed", "attempts": [{ "attempt": 1 }, { "attempt": 2, "text": body }] } }));
}

// The built-in agent puts the provider's error in `details` and the wait in `retryAfterMs`
#[test]
fn a_retry_warning_with_details_logs_them_and_schedules_the_next_attempt() {
  let mut s = state();
  apply(&mut s, chunk("Working"));
  let mut u = air_failure("acpira-retry-1", 1, "warning", "service", "Retrying Gateway, attempt 2 of 4", json!([]));
  u["_meta"]["jetbrains"]["air"]["sessionFailure"]["details"] = json!("HTTP 503: upstream connect error");
  u["_meta"]["jetbrains"]["air"]["sessionFailure"]["retryAfterMs"] = json!(2000);
  apply(&mut s, u);
  let row = &retry_rows(&s)[0];
  expect_match(row, json!({ "retry": { "max": 4, "attempts": [{ "attempt": 2, "text": "HTTP 503: upstream connect error" }] } }));
  assert!(row["retry"]["nextAt"].as_f64().is_some());
  // A stopped turn leaves it stopped, not failed
  end_turn(&mut s, TurnStop::Cancelled);
  expect_eq(&retry_rows(&s)[0]["retry"]["status"], json!("cancelled"));
}

// Devin: start and outcome arrive as separate whole chunks; a failure keeps its reason; Kimi's statistics are absorbed
#[test]
fn prose_outcomes_settle_the_open_row_and_keep_failure_reasons() {
  let mut s = NormalizeState { agent: Some("devin".into()), ..state() };
  apply(&mut s, chunk("Compacting context…"));
  apply(&mut s, chunk("Context compacted"));
  apply(&mut s, chunk("Compacting context…"));
  apply(&mut s, chunk("Compaction failed: window too small"));
  assert_eq!(compaction_rows(&s), vec![
    ("completed".into(), None),
    ("failed".into(), Some("Compaction failed: window too small".into())),
  ]);
  assert_eq!(s.turns[0].as_agent().unwrap().blocks.len(), 2);

  let mut k = NormalizeState { agent: Some("kimi".into()), ..state() };
  apply(&mut k, chunk("Compaction completed."));
  apply(&mut k, chunk("\n- Messages compacted: 3\n- Tokens after: 1200"));
  assert_eq!(compaction_rows(&k), vec![("completed".into(), None)]);
  assert_eq!(k.turns[0].as_agent().unwrap().blocks.len(), 1);
}

// Model prose that merely mentions compaction, and agents without a prose table, stay text
#[test]
fn prose_that_is_not_an_adapter_marker_stays_text() {
  let mut s = NormalizeState { agent: Some("devin".into()), ..state() };
  apply(&mut s, chunk("Context compacted, so I will continue"));
  let mut c = NormalizeState { agent: Some("claude".into()), ..state() };
  apply(&mut c, chunk("Context compacted"));
  let mut k = NormalizeState { agent: Some("kimi".into()), ..state() };
  apply(&mut k, chunk("- Tokens after: 1200"));
  for st in [&s, &c, &k] {
    assert!(compaction_rows(st).is_empty());
    expect_match(block(st, 0, 0), json!({ "type": "text" }));
  }
}

#[test]
fn a_failed_compaction_update_carries_its_error() {
  let mut s = state();
  apply(&mut s, json!({ "sessionUpdate": "compaction_update", "compactionId": "c1", "status": "in_progress" }));
  apply(&mut s, json!({ "sessionUpdate": "compaction_update", "compactionId": "c1", "status": "failed", "error": "too large" }));
  assert_eq!(compaction_rows(&s), vec![("failed".into(), Some("too large".into()))]);
}
