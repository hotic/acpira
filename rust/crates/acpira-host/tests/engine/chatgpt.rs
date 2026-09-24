//! test/chatgptBridge.test.ts and test/chatgptContinuation.test.ts: ChatGPT conversation mirrors, their events and honest status

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use serde_json::{Value, json};

use acpira_host::external::chatgpt_binding::chatgpt_binding;
use acpira_host::external::chatgpt_events::{ChatGptRecord, OUTPUT_LIMIT, STALE_AFTER_MS, apply_chatgpt_event, chatgpt_session_id, chatgpt_view, parse_chatgpt_event};
use acpira_host::external::chatgpt_store::ChatGptBridgeStore;
use acpira_host::external::desktop_commander::commander_facts;
use acpira_shared::transcript::SessionView;

use crate::support::{expect_absent, expect_match, v};

struct Setup {
  root: tempfile::TempDir,
  cwd: String,
  dir: std::path::PathBuf,
  clock: Arc<AtomicI64>,
  store: Arc<ChatGptBridgeStore>,
  view: SessionView,
  seq: std::sync::atomic::AtomicUsize,
}

impl Setup {
  async fn send(&self, body: Value) -> anyhow::Result<()> {
    let mut e = json!({ "id": format!("event-{}", self.seq.fetch_add(1, Ordering::SeqCst) + 1), "turnId": "turn-a" });
    e.as_object_mut().unwrap().extend(body.as_object().unwrap().clone());
    self.store.accept(&self.view.id, &e).await
  }
  fn tick(&self, ms: i64) {
    self.clock.fetch_add(ms, Ordering::SeqCst);
  }
  fn now(&self) -> Arc<dyn Fn() -> i64 + Send + Sync> {
    let c = self.clock.clone();
    Arc::new(move || c.load(Ordering::SeqCst))
  }
  fn current(&self) -> Value {
    v(self.store.view(&self.view.id))
  }
}

async fn setup() -> Setup {
  let root = tempfile::tempdir().unwrap();
  let cwd = root.path().canonicalize().unwrap().to_string_lossy().into_owned();
  let clock = Arc::new(AtomicI64::new(1_800_000_000_000));
  let dir = root.path().join("bridges").join("chatgpt");
  let c = clock.clone();
  let store = ChatGptBridgeStore::with_clock(dir.clone(), Arc::new(|_: &str| {}), Some("/extension/bin/acpira".into()), Arc::new(move || c.load(Ordering::SeqCst)));
  store.init(false).await.unwrap();
  let view = store.open("test-conversation-a", &cwd, "ChatGPT test").await.unwrap();
  Setup { root, cwd, dir, clock, store, view, seq: Default::default() }
}

fn err(r: anyhow::Result<impl std::fmt::Debug>) -> String {
  r.expect_err("must be refused").to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_mirror_keeps_its_own_channel_project_and_stable_source_identity() {
  let s = setup().await;
  expect_match(v(&s.view), json!({ "agent": "chatgpt", "status": "readonly", "running": false, "cwd": s.cwd }));
  assert!(v(&s.view)["external"]["connectionPrompt"].as_str().unwrap().contains(&s.view.id));
  assert_eq!(s.store.open("test-conversation-a", &s.cwd, "ChatGPT").await.unwrap().id, s.view.id);
  let other = s.store.open("test-conversation-b", &s.cwd, "ChatGPT").await.unwrap();
  assert_ne!(other.id, s.view.id);
  assert_eq!(s.store.summaries().len(), 2);
  assert!(err(s.store.open("test-conversation-a", std::env::temp_dir().to_str().unwrap(), "ChatGPT").await).contains("another project"));
  let id = chatgpt_session_id("../../escape").unwrap();
  assert!(id.starts_with("chatgpt-") && id.len() == "chatgpt-".len() + 32 && id["chatgpt-".len()..].chars().all(|c| c.is_ascii_hexdigit()));
}

#[tokio::test(flavor = "multi_thread")]
async fn turns_messages_streaming_output_and_completion_receipts_render() {
  let s = setup().await;
  s.send(json!({ "type": "turn_start", "text": "Inspect this test project" })).await.unwrap();
  s.send(json!({ "type": "message", "messageId": "progress", "phase": "commentary", "text": "Checking" })).await.unwrap();
  s.send(json!({ "type": "message", "messageId": "progress", "phase": "commentary", "text": "Checking the project" })).await.unwrap();
  s.send(json!({ "type": "tool_start", "callId": "call-a", "name": "Shell", "kind": "execute", "target": "echo ok", "input": { "command": "echo ok" } })).await.unwrap();
  assert_eq!(s.current()["running"], true);
  s.send(json!({ "type": "tool_output", "callId": "call-a", "text": "ok\n" })).await.unwrap();
  assert!(err(s.send(json!({ "type": "turn_end", "stop": "end_turn" })).await).contains("no completion receipt"));
  s.tick(2000);
  s.send(json!({ "type": "tool_end", "callId": "call-a", "status": "completed", "detail": "exit 0" })).await.unwrap();
  s.send(json!({ "type": "message", "messageId": "answer", "phase": "final", "text": "The check passed." })).await.unwrap();
  s.send(json!({ "type": "turn_end", "stop": "end_turn" })).await.unwrap();
  let r = s.current();
  assert_eq!(r["running"], false);
  expect_match(&r["turns"][0], json!({ "role": "user", "text": "Inspect this test project" }));
  let blocks = r["turns"][1]["blocks"].as_array().unwrap();
  assert_eq!(blocks.len(), 3);
  expect_match(&blocks[0], json!({ "type": "text", "markdown": "Checking the project" }));
  expect_match(&blocks[1], json!({ "type": "tool_call", "status": "completed", "meta": "exit 0" }));
  assert!(blocks[1]["content"]["text"].as_str().unwrap().contains("ok\n"));
  assert_eq!(r["turns"][1]["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disconnect_invents_no_completion_and_a_heartbeat_resumes() {
  let s = setup().await;
  s.send(json!({ "type": "turn_start", "text": "A long operation" })).await.unwrap();
  let before = s.current()["rev"].as_i64().unwrap();
  s.tick(STALE_AFTER_MS + 1);
  s.store.refresh().await;
  let stale = s.current();
  expect_match(&stale, json!({ "running": false, "external": { "state": "stale" } }));
  assert!(stale["rev"].as_i64().unwrap() > before);
  expect_absent(&stale["turns"][1], "stop");
  expect_absent(&stale["turns"][1], "endedAt");
  s.send(json!({ "type": "heartbeat" })).await.unwrap();
  expect_match(s.current(), json!({ "running": true, "external": { "state": "receiving" } }));
  assert!(s.current()["rev"].as_i64().unwrap() > stale["rev"].as_i64().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_receipts_deduplicate_while_conflicting_ids_and_hidden_phases_are_refused() {
  let s = setup().await;
  let e = json!({ "id": "one", "turnId": "turn-a", "type": "turn_start", "text": "hello" });
  s.store.accept(&s.view.id, &e).await.unwrap();
  let rev = s.current()["rev"].clone();
  s.store.accept(&s.view.id, &e).await.unwrap();
  assert_eq!(s.current()["rev"], rev);
  let mut changed = e.clone();
  changed["text"] = json!("different");
  assert!(err(s.store.accept(&s.view.id, &changed).await).contains("different content"));
  assert!(parse_chatgpt_event(&json!({ "id": "__proto__", "turnId": "x", "type": "heartbeat" })).is_err());
  assert!(err(s.store.accept(&s.view.id, &json!({ "id": "two", "turnId": "turn-a", "type": "message", "messageId": "m", "text": "not allowed", "phase": "analysis" })).await).contains("visible"));
  assert_eq!(s.current()["turns"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writers_merge_and_another_host_replays_without_losing_events() {
  let s = setup().await;
  let second = ChatGptBridgeStore::with_clock(s.dir.clone(), Arc::new(|_: &str| {}), None, s.now());
  second.init(false).await.unwrap();
  s.send(json!({ "type": "turn_start", "text": "concurrency test" })).await.unwrap();
  let mut writes = vec![];
  for i in 0..16 {
    let store = if i % 2 == 1 { s.store.clone() } else { second.clone() };
    let id = s.view.id.clone();
    writes.push(tokio::spawn(async move {
      store.accept(&id, &json!({ "id": format!("parallel-{i}"), "turnId": "turn-a", "type": "message", "messageId": format!("message-{i}"), "phase": "commentary", "text": format!("result {i}") })).await
    }));
  }
  for w in writes {
    w.await.unwrap().unwrap();
  }
  s.store.refresh().await;
  second.refresh().await;
  assert_eq!(s.current()["turns"], v(second.view(&s.view.id))["turns"]);
  assert_eq!(s.current()["turns"][1]["blocks"].as_array().unwrap().len(), 16);
  #[cfg(unix)]
  assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(s.dir.join(format!("{}.json", s.view.id))).unwrap().permissions()) & 0o777, 0o600);
  second.dispose().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn deletion_survives_writer_retries_can_be_undone_and_later_drops_the_transcript() {
  let s = setup().await;
  s.send(json!({ "type": "turn_start", "text": "private test fixture" })).await.unwrap();
  s.store.delete(&s.view.id).await.unwrap();
  assert!(s.store.view(&s.view.id).is_none());
  assert!(err(s.send(json!({ "type": "heartbeat" })).await).contains("deleted"));
  s.store.restore(&s.view.id).await.unwrap();
  assert!(s.store.view(&s.view.id).is_some());
  s.store.delete(&s.view.id).await.unwrap();
  s.tick(31_000);
  s.store.refresh().await;
  assert!(err(s.store.restore(&s.view.id).await).contains("expired"));
  let disk: Value = serde_json::from_str(&std::fs::read_to_string(s.dir.join(format!("{}.json", s.view.id))).unwrap()).unwrap();
  assert_eq!(disk["turns"], json!([]));
  assert_eq!(disk["receipts"], json!({}));
  assert!(err(s.store.open("test-conversation-a", disk["cwd"].as_str().unwrap(), "ChatGPT").await).contains("deleted"));
}

#[tokio::test(flavor = "multi_thread")]
async fn capped_output_is_marked_and_only_successful_diff_receipts_count() {
  let s = setup().await;
  s.send(json!({ "type": "turn_start", "text": "test output limits" })).await.unwrap();
  s.send(json!({ "type": "tool_start", "callId": "x", "name": "write", "kind": "edit" })).await.unwrap();
  s.send(json!({ "type": "tool_output", "callId": "x", "text": "x".repeat(OUTPUT_LIMIT + 20) })).await.unwrap();
  assert!(s.current()["turns"][1]["blocks"][0]["content"]["text"].as_str().unwrap().contains("[Output truncated"));
  assert!(err(s.send(json!({ "type": "tool_end", "callId": "x", "status": "failed", "diff": { "path": "/a", "oldText": "a", "newText": "b" } })).await).contains("cannot claim"));
  s.send(json!({ "type": "tool_end", "callId": "x", "status": "completed", "diff": { "path": "/a", "oldText": "a", "newText": "b" } })).await.unwrap();
  expect_match(&s.current()["turns"][1]["blocks"][0], json!({ "content": { "type": "diff", "source": { "oldText": "a", "newText": "b" } } }));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn symlink_records_and_out_of_order_events_are_refused_without_touching_other_files() {
  let s = setup().await;
  assert!(err(s.send(json!({ "type": "tool_output", "callId": "x", "text": "late" })).await).contains("not active"));
  let outside = s.root.path().join("untouched.json");
  std::fs::write(&outside, "{}").unwrap();
  let id = chatgpt_session_id("symlink").unwrap();
  std::os::unix::fs::symlink(&outside, s.dir.join(format!("{id}.json"))).unwrap();
  assert!(err(s.store.open("symlink", &s.cwd, "ChatGPT").await).contains("Invalid"));
  assert_eq!(std::fs::read_to_string(&outside).unwrap(), "{}");
  assert_eq!(s.current()["turns"], json!([]));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn binding_instructions_quote_paths_and_never_substitute_a_codex_invocation() {
  let s = setup().await;
  let text = chatgpt_binding(&s.view, "/a path/it's here/acpira", "/test home");
  assert!(text.contains("'\\''"));
  assert!(text.contains("prompt") && text.contains("exec") && text.contains("explicitly bridged"));
  assert!(!text.contains("codex exec"));
}

struct Fixture {
  r: ChatGptRecord,
  seq: usize,
}

impl Fixture {
  fn new() -> Fixture {
    let at = acpira_host::util::iso_of_ms(0);
    let r = serde_json::from_value(json!({ "version": 1, "id": chatgpt_session_id("fixture").unwrap(), "sourceKey": "fixture", "title": "Fixture", "cwd": "/fixture",
      "createdAt": at, "updatedAt": at, "lastEventAt": at, "revision": 1, "turns": [], "receipts": {} })).unwrap();
    Fixture { r, seq: 0 }
  }
  fn send_at(&mut self, body: Value, now: i64) -> anyhow::Result<()> {
    self.seq += 1;
    let mut e = json!({ "id": format!("e{}", self.seq), "turnId": "a" });
    e.as_object_mut().unwrap().extend(body.as_object().unwrap().clone());
    if let Some(next) = apply_chatgpt_event(&self.r, &e, now)? {
      self.r = next;
    }
    Ok(())
  }
  fn send(&mut self, body: Value) -> anyhow::Result<()> {
    self.send_at(body, 1)
  }
  fn view(&self, now: i64) -> Value {
    v(chatgpt_view(&self.r, now))
  }
}

#[test]
fn an_empty_receiver_is_not_called_connected() {
  assert_eq!(Fixture::new().view(0)["external"]["state"], "unbound");
}

#[test]
fn retrying_the_same_prompt_neither_duplicates_nor_restarts_a_completed_turn() {
  let mut f = Fixture::new();
  f.send(json!({ "type": "turn_start", "text": "hello" })).unwrap();
  f.send(json!({ "type": "turn_end", "stop": "end_turn" })).unwrap();
  f.send(json!({ "type": "turn_start", "text": "hello" })).unwrap();
  assert_eq!(f.r.turns.len(), 2);
  assert!(f.r.active_turn_id.is_none());
  assert!(err(f.send(json!({ "type": "turn_start", "text": "changed" }))).contains("different text"));
}

#[test]
fn the_next_turn_names_the_exact_previous_one_whose_outcome_stays_unconfirmed() {
  let mut f = Fixture::new();
  f.send(json!({ "type": "turn_start", "text": "first" })).unwrap();
  assert!(err(f.send(json!({ "type": "turn_start", "turnId": "b", "text": "next" }))).contains("previous-turn"));
  assert!(f.send(json!({ "type": "turn_start", "turnId": "b", "text": "next", "previousTurnId": "wrong" })).is_err());
  f.send(json!({ "type": "turn_start", "turnId": "b", "text": "next", "previousTurnId": "a" })).unwrap();
  let view = f.view(2);
  assert_eq!(view["turns"].as_array().unwrap().len(), 4);
  expect_match(&view["turns"][1], json!({ "observation": "unknown" }));
  expect_absent(&view["turns"][1], "stop");
  expect_absent(&view["turns"][1], "endedAt");
  assert_eq!(view["running"], true);
}

#[test]
fn late_command_output_goes_to_its_original_turn_without_renewing_the_new_turns_lease() {
  let mut f = Fixture::new();
  f.send(json!({ "type": "turn_start", "text": "first" })).unwrap();
  f.send(json!({ "type": "tool_start", "callId": "shell", "name": "Shell", "kind": "execute" })).unwrap();
  f.send_at(json!({ "type": "turn_start", "turnId": "b", "text": "next", "previousTurnId": "a" }), 2).unwrap();
  f.send_at(json!({ "type": "heartbeat" }), STALE_AFTER_MS + 9).unwrap();
  f.send_at(json!({ "type": "tool_output", "callId": "shell", "text": "late output" }), STALE_AFTER_MS + 10).unwrap();
  let view = f.view(STALE_AFTER_MS + 10);
  assert_eq!(view["external"]["state"], "stale");
  expect_match(&view["turns"][1], json!({ "blocks": [{ "content": { "text": "late output" }, "observation": "unknown" }] }));
  assert_eq!(view["turns"][3]["blocks"], json!([]));
  f.send(json!({ "type": "tool_end", "callId": "shell", "status": "completed" })).unwrap();
  f.send(json!({ "type": "turn_end", "stop": "end_turn" })).unwrap();
  assert_eq!(f.r.active_turn_id.as_deref(), Some("b"));
  expect_match(v(&f.r.turns[1]), json!({ "stop": "end_turn" }));
}

#[test]
fn observation_resumes_without_replaying_prompts_or_tools() {
  let mut f = Fixture::new();
  f.send(json!({ "type": "turn_start", "text": "first" })).unwrap();
  assert_eq!(f.view(STALE_AFTER_MS + 10)["external"]["state"], "stale");
  f.send_at(json!({ "type": "turn_resume" }), STALE_AFTER_MS + 20).unwrap();
  assert_eq!(f.view(STALE_AFTER_MS + 21)["running"], true);
  assert_eq!(f.r.turns.len(), 2);
  f.send(json!({ "type": "turn_end", "stop": "end_turn" })).unwrap();
  assert!(err(f.send(json!({ "type": "turn_resume" }))).contains("unfinished"));
}

#[test]
fn stopping_generation_is_not_read_as_terminating_a_local_process() {
  let mut f = Fixture::new();
  f.send(json!({ "type": "turn_start", "text": "first" })).unwrap();
  f.send(json!({ "type": "tool_start", "callId": "shell", "name": "Shell", "kind": "execute" })).unwrap();
  f.send(json!({ "type": "turn_end", "stop": "cancelled" })).unwrap();
  expect_match(&f.view(2)["turns"][1], json!({ "stop": "cancelled", "blocks": [{ "status": "in_progress", "observation": "unknown" }] }));
  f.send(json!({ "type": "tool_end", "callId": "shell", "status": "completed" })).unwrap();
  expect_match(&f.view(3)["turns"][1], json!({ "stop": "cancelled", "blocks": [{ "status": "completed" }] }));
}

#[test]
fn pairing_is_never_claimed_from_a_process_executable_or_stale_configuration() {
  expect_match(v(commander_facts(false, true, Some(false))), json!({ "installation": "unknown", "pairing": "unknown", "evidence": "configuration" }));
  expect_match(v(commander_facts(true, true, Some(true))), json!({ "installation": "detected", "pairing": "unknown" }));
  expect_match(v(commander_facts(false, false, Some(false))), json!({ "installation": "not_detected", "pairing": "unknown" }));
  expect_match(v(commander_facts(false, false, None)), json!({ "installation": "unknown", "process": "unknown" }));
}
