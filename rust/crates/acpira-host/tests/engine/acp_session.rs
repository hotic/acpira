//! test/AcpSession.test.ts: the session state machine against test/fake-agent.ts

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::acp::session::AcpSession;
use acpira_host::store::record::SessionRecord;
use acpira_host::store::transcript_store::blob_name;
use acpira_host::util::iso_of_ms;
use acpira_shared::attachments::MAX_IMAGE_BYTES;
use acpira_shared::transcript::{Draft, Turn};

use crate::fake_or_skip;
use crate::support::{Disposing, FakeAgent, Harness, expect_absent, expect_eq, expect_match, until, v};

// Grok-style synthesized modes: not provided by the protocol, declared in the registry
fn syn_modes() -> Value {
  json!([{ "id": "default", "name": "Agent" }, { "id": "plan", "name": "Plan" }, { "id": "yolo", "name": "Auto accept" }])
}

pub fn turns(j: Value) -> Vec<Turn> {
  serde_json::from_value(j).expect("turns")
}

pub fn drafts(j: Value) -> Vec<Draft> {
  serde_json::from_value(j).expect("drafts")
}

pub async fn prompt(s: &Arc<AcpSession>, text: &str) {
  s.prompt(text.into(), vec![], false, None, None).await;
}

/// A prompt left running in the background (the TS `const pending = s.prompt(…)`)
pub fn spawn_prompt(s: &Arc<AcpSession>, text: &str) -> tokio::task::JoinHandle<()> {
  tokio::spawn(s.prompt(text.into(), vec![], false, None, None))
}

pub fn view(s: &AcpSession) -> Value {
  v(s.view())
}

pub fn agent_blocks(view: &Value) -> Vec<Value> {
  view["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "agent").flat_map(|t| t["blocks"].as_array().unwrap().clone()).collect()
}

pub fn find_block(view: &Value, ty: &str) -> Option<Value> {
  agent_blocks(view).into_iter().find(|b| b["type"] == ty)
}

pub fn has_block(s: &AcpSession, ty: &str) -> bool {
  find_block(&view(s), ty).is_some()
}

pub async fn wait_block(s: &AcpSession, ty: &str) -> Value {
  until(|| has_block(s, ty), 5000).await;
  find_block(&view(s), ty).unwrap()
}

pub fn last_turn(view: &Value) -> Value {
  view["turns"].as_array().unwrap().last().cloned().unwrap_or(Value::Null)
}

pub fn turn_at(view: &Value, i: isize) -> Value {
  let t = view["turns"].as_array().unwrap();
  let i = if i < 0 { t.len() as isize + i } else { i };
  t.get(i as usize).cloned().unwrap_or(Value::Null)
}

fn option_value(view: &Value, id: &str) -> Value {
  view["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == id).map(|o| o["value"].clone()).unwrap_or(Value::Null)
}

fn blob(h: &Harness, sid: &str, name: &str) -> Option<Vec<u8>> {
  std::fs::read(h.dir.path().join("sessions").join(sid).join(name)).ok()
}

async fn started(h: &Harness, cwd: &str) -> Disposing {
  let s = Disposing(h.session(cwd));
  s.start().await;
  s
}

#[tokio::test(flavor = "multi_thread")]
async fn restores_interrupted_turns_and_old_background_tools_as_stopped() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let original = Disposing(h.session("/tmp"));
  let mut record = original.to_record();
  record.updated_at = iso_of_ms(5000);
  record.turns = turns(json!([
    { "role": "user", "text": "work" },
    { "role": "agent", "startedAt": 1000, "blocks": [
      { "type": "text", "markdown": "partial", "streaming": true },
      { "type": "tool_call", "id": "server", "kind": "execute", "verb": "Run", "status": "in_progress", "startedAt": 2000, "background": true },
      { "type": "tool_call", "id": "wait", "kind": "other", "verb": "Wait", "status": "pending" },
      { "type": "compaction", "id": "compact", "status": "in_progress" },
    ] },
    { "role": "user", "text": "continue" },
    { "role": "agent", "startedAt": 4000, "endedAt": 5000, "blocks": [], "stop": "error", "error": { "message": "failed" } },
  ]));
  let before = serde_json::to_string(&record).unwrap();
  let restored = Disposing(AcpSession::new(record.clone(), h.deps.clone()));
  assert!(!restored.is_running());
  let rv = view(&restored);
  expect_match(&rv["turns"][1], json!({ "stop": "cancelled", "endedAt": 5000, "blocks": [
    { "streaming": false }, { "status": "cancelled", "endedAt": 5000 }, { "status": "cancelled" }, { "status": "cancelled" },
  ] }));
  expect_eq(&rv["turns"][3], v(&record.turns[3]));
  assert_eq!(serde_json::to_string(&record).unwrap(), before);
  let again = Disposing(AcpSession::new(restored.to_record(), h.deps.clone()));
  assert_eq!(view(&again)["turns"], rv["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_the_native_session_on_context_overflow_and_requires_compaction_before_retrying() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "seed-context").await;
  let peer = s.to_record().acp_session_id;
  // Polled in one go like the TS calls: the first claims the turn synchronously, so the second queues behind it
  tokio::join!(prompt(&s, "context-too-long"), prompt(&s, "queued-follow-up"));
  let queued: Vec<Value> = view(&s)["queued"].as_array().cloned().unwrap_or_default();
  assert_eq!(queued.iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("queued-follow-up")]);
  let before = view(&s)["turns"].clone();
  let err = s.retry_turn().await.expect_err("retry must require compaction");
  assert!(err.to_string().to_lowercase().contains("compact") || err.to_string().contains("压缩"), "{err}");
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  s.compact(false).await.unwrap();
  // The drained follow-up has run to its end (between dequeue and dispatch the view is briefly idle with the prompt pending)
  until(|| {
    let vw = view(&s);
    !s.is_running() && vw["queued"].as_array().is_none_or(|q| q.is_empty()) && turn_at(&vw, -2)["text"] == "queued-follow-up" && !last_turn(&vw)["stop"].is_null()
  }, 5000).await;
  expect_match(turn_at(&view(&s), -2), json!({ "role": "user", "text": "queued-follow-up" }));
  prompt(&s, "context-too-long").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  assert_eq!(s.to_record().acp_session_id, peer);
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_empty_slash_receipts_and_observed_settings_across_persistence() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "/silent").await;
  expect_match(last_turn(&view(&s)), json!({ "blocks": [], "stop": "end_turn", "command": { "name": "silent" } }));
  prompt(&s, "/silent-plan").await;
  expect_match(last_turn(&view(&s)), json!({ "blocks": [], "stop": "end_turn", "command": { "name": "silent-plan", "mode": "Plan" } }));
  prompt(&s, "/silent-plan").await;
  expect_match(last_turn(&view(&s)), json!({ "command": { "name": "silent-plan" } }));
  expect_absent(last_turn(&view(&s)), "command.mode");
  prompt(&s, "/slash-error").await;
  let last = last_turn(&view(&s));
  expect_match(&last, json!({ "stop": "error" }));
  assert!(last["error"]["message"].as_str().unwrap().contains("Unknown command"));
  prompt(&s, "ordinary message").await;
  expect_absent(last_turn(&view(&s)), "command");
  let restored = Disposing(AcpSession::new(s.to_record(), h.deps.clone()));
  assert_eq!(view(&restored)["turns"], view(&s)["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_an_older_session_filters_repeated_completed_plan_snapshots() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let original = Disposing(h.session("/tmp"));
  let mut record = original.to_record();
  record.turns = turns(json!([
    { "role": "agent", "blocks": [{ "type": "plan", "entries": [{ "title": "Done", "status": "completed" }] }] },
    { "role": "user", "text": "Follow-up" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "Answer" }, { "type": "plan", "entries": [{ "title": "Done", "status": "completed" }] }] },
  ]));
  let restored = Disposing(AcpSession::new(record.clone(), h.deps.clone()));
  expect_match(&view(&restored)["turns"][2], json!({ "blocks": [{ "type": "text", "markdown": "Answer" }] }));
  assert_eq!(restored.to_record().turns.len(), 3);
  expect_match(&record.turns[2], json!({ "blocks": [{ "type": "text" }, { "type": "plan" }] }));
}

#[tokio::test(flavor = "multi_thread")]
async fn plan_approval_shows_the_full_plan_and_changes_the_execution_model_before_approval() {
  let fake = fake_or_skip!();
  for script in ["plan-grok", "plan-devin", "plan-devin-early"] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    s.set_mode("plan".into()).await.unwrap();
    let pending = spawn_prompt(&s, script);
    let permission = wait_block(&s, "permission").await;
    let plan = find_block(&view(&s), "plan_document").unwrap();
    assert_eq!(permission["planId"], plan["id"], "{script}");
    assert_eq!(plan["markdown"], "# Demo plan\n\nCreate hello.txt.");
    let path = if script == "plan-devin-early" { Value::Null } else { json!("/Users/test/.devin/plans/demo.md") };
    assert_eq!(plan["path"], path, "{script}");
    let allow = permission["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").unwrap()["id"].as_str().unwrap().to_owned();
    s.resolve_permission(permission["id"].as_str().unwrap(), "invented");
    assert!(view(&s)["running"].as_bool().unwrap());
    let plan_id = plan["id"].as_str().unwrap().to_owned();
    s.build_plan(&plan_id, Some(("model".into(), "m2".into())), Some(allow.clone())).await.unwrap();
    pending.await.unwrap();
    let after = view(&s);
    let plans: Vec<Value> = agent_blocks(&after).into_iter().filter(|b| b["type"] == "plan_document").collect();
    assert_eq!(plans.len(), 1);
    expect_match(&plans[0], json!({ "status": "approved", "path": "/Users/test/.devin/plans/demo.md" }));
    assert_eq!(after["controls"]["modeId"], "agent");
    assert!(after["turns"].to_string().contains("APPROVED model=m2"));
    assert!(s.to_record().turns.iter().any(|t| v(t)["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["type"] == "plan_document"))));
    let count = after["turns"].as_array().unwrap().len();
    s.build_plan(&plan_id, None, Some(allow)).await.ok();
    assert_eq!(view(&s)["turns"].as_array().unwrap().len(), count);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn grok_plan_approval_handles_reject_cancel_and_dispose_without_an_orphaned_permission() {
  let fake = fake_or_skip!();
  for action in ["reject", "cancel", "dispose"] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    s.set_mode("plan".into()).await.unwrap();
    let pending = spawn_prompt(&s, "plan-grok");
    let b = wait_block(&s, "permission").await;
    match action {
      "reject" => s.resolve_permission(b["id"].as_str().unwrap(), "rejected"),
      "cancel" => s.cancel().await,
      _ => s.dispose(),
    }
    pending.await.unwrap();
    let after = view(&s);
    assert!(!after["running"].as_bool().unwrap(), "{action}");
    assert!(find_block(&after, "permission").is_none(), "{action}");
    assert_eq!(after["controls"]["modeId"], "plan", "{action}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn building_a_saved_plan_switches_mode_and_model_and_dispatches_its_content_once() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.set_mode("plan".into()).await.unwrap();
  prompt(&s, "plan-file").await;
  let plan = find_block(&view(&s), "plan_document").unwrap();
  let id = plan["id"].as_str().unwrap();
  let (a, b) = tokio::join!(s.build_plan(id, Some(("model".into(), "m2".into())), None), s.build_plan(id, None, None));
  a.ok();
  b.ok();
  let after = view(&s);
  assert_eq!(after["turns"].as_array().unwrap().len(), 4);
  let text = format!("Implement the following approved plan:\n\n{}", plan["markdown"].as_str().unwrap());
  expect_match(&after["turns"][2], json!({ "role": "user", "planId": id, "text": text }));
  expect_match(&s.to_record().turns[2], json!({ "planId": id }));
  assert_eq!(after["controls"]["modeId"], "agent");
  assert_eq!(option_value(&after, "model"), "m2");
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_failed_plan_execution_keeps_its_internal_instruction_out_of_user_messages() {
  let fake = fake_or_skip!();
  let native = tempfile::tempdir().unwrap();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_SESSION_DIR": native.path() } }));
  let first = started(&h, "/tmp").await;
  prompt(&first, "plan-file").await;
  // The plan's content fails on its first dispatch ("fail" script); the TS suite edited the live block, here the record carries it
  let mut record: SessionRecord = first.to_record();
  let mut plan_id = String::new();
  let mut markdown = String::new();
  for t in record.turns.iter_mut() {
    if let Turn::Agent(a) = t {
      for b in a.blocks.iter_mut() {
        let mut j = v(&*b);
        if j["type"] == "plan_document" {
          markdown = format!("{}\n\nfail once", j["markdown"].as_str().unwrap());
          j["markdown"] = json!(markdown);
          plan_id = j["id"].as_str().unwrap().to_owned();
          *b = serde_json::from_value(j).unwrap();
        }
      }
    }
  }
  first.dispose();
  let s = Disposing(AcpSession::new(record, h.deps.clone()));
  s.start().await;
  s.build_plan(&plan_id, None, None).await.ok();
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "error" }));
  s.retry_turn().await.unwrap();
  let after = view(&s);
  assert_eq!(after["turns"].as_array().unwrap().len(), 4);
  expect_match(&after["turns"][2], json!({ "role": "user", "planId": plan_id, "text": format!("Implement the following approved plan:\n\n{markdown}") }));
  expect_match(last_turn(&after), json!({ "role": "agent", "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rejected_model_selection_leaves_the_plan_approval_waiting() {
  let fake = fake_or_skip!();
  // An advertised model may still fail when the peer applies it
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODELS": "unavailable" } }));
  let s = started(&h, "/tmp").await;
  let pending = spawn_prompt(&s, "plan-devin");
  wait_block(&s, "permission").await;
  let plan = find_block(&view(&s), "plan_document").unwrap();
  let err = s.build_plan(plan["id"].as_str().unwrap(), Some(("model".into(), "unavailable".into())), None).await.expect_err("refused");
  assert!(err.to_string().contains("Model unavailable"), "{err}");
  assert!(view(&s)["running"].as_bool().unwrap());
  assert_eq!(find_block(&view(&s), "plan_document").unwrap()["status"], "ready");
  s.cancel().await;
  pending.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_default_to_no_plan_card_still_approves_through_build() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.set_mode("plan".into()).await.unwrap();
  let pending = spawn_prompt(&s, "plan-veto");
  let permission = wait_block(&s, "permission").await;
  let plan = find_block(&view(&s), "plan_document").unwrap();
  assert_eq!(permission["defaultToNo"], true);
  s.build_plan(plan["id"].as_str().unwrap(), None, None).await.unwrap();
  pending.await.unwrap();
  assert_eq!(find_block(&view(&s), "plan_document").unwrap()["status"], "approved");
  assert!(view(&s)["turns"].to_string().contains("APPROVED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_explicit_reject_option_on_a_plan_card_resolves_it_without_touching_the_executor_model() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.set_mode("plan".into()).await.unwrap();
  let pending = spawn_prompt(&s, "plan-devin");
  let permission = wait_block(&s, "permission").await;
  let plan = find_block(&view(&s), "plan_document").unwrap();
  let reject = permission["options"].as_array().unwrap().iter().find(|o| o["kind"] == "reject_once").unwrap()["id"].as_str().unwrap().to_owned();
  s.build_plan(plan["id"].as_str().unwrap(), Some(("model".into(), "m2".into())), Some(reject)).await.unwrap();
  pending.await.unwrap();
  let after = view(&s);
  assert_eq!(find_block(&after, "plan_document").unwrap()["status"], "rejected");
  assert_eq!(option_value(&after, "model"), "m1");
  assert_eq!(after["controls"]["modeId"], "plan");
  assert!(after["turns"].to_string().contains("REJECTED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn start_session_receives_modes_and_config_options_with_model_first() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  let ids = |k: &str| vw["controls"][k].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  assert_eq!(ids("modes"), ["agent", "plan"]);
  assert_eq!(vw["controls"]["modeId"], "agent");
  assert_eq!(ids("options"), ["model", "effort"]);
  expect_match(&vw["controls"]["options"][0], json!({ "category": "model", "value": "m1" }));
  assert_eq!(vw["controls"]["options"][0]["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("m1"), json!("m2")]);
  expect_match(&vw["controls"]["options"][1], json!({ "name": "Reasoning", "category": "thought_level", "value": "high" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn one_prompt_turn_merges_thought_plan_and_text_and_updates_title_and_commands() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "hi").await;
  let vw = view(&s);
  assert!(!vw["running"].as_bool().unwrap());
  assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "hi" }));
  let agent = &vw["turns"][1];
  assert_eq!(agent["role"], "agent");
  assert!(agent["endedAt"].as_i64().unwrap() >= agent["startedAt"].as_i64().unwrap());
  assert_eq!(agent["blocks"].as_array().unwrap().iter().map(|b| b["type"].clone()).collect::<Vec<_>>(), [json!("thought"), json!("plan"), json!("text")]);
  expect_match(&agent["blocks"][0], json!({ "type": "thought", "text": "thinking hard", "streaming": false }));
  expect_match(&agent["blocks"][2], json!({ "type": "text", "markdown": "hello world", "streaming": false }));
  expect_absent(agent, "activity");
  assert_eq!(vw["title"], "Fake title");
  expect_eq(&vw["commands"], json!([{ "name": "compact", "description": "compact it" }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_are_stored_as_blobs_and_sent_as_image_resource_and_link_blocks() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let png = base64::engine::general_purpose::STANDARD.encode("fake-png-bytes");
  s.prompt("echo blocks".into(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": png, "name": "shot.png" },
    { "kind": "text", "name": "notes.md", "text": "# notes" },
    { "kind": "file", "uri": "file:///repo/src/a.ts", "name": "src/a.ts" },
  ])), false, None, None).await;
  let vw = view(&s);
  let user = &vw["turns"][0];
  expect_match(user, json!({ "role": "user", "text": "echo blocks", "attachments": [
    { "kind": "image", "mimeType": "image/png", "name": "shot.png" },
    { "kind": "text", "name": "notes.md" },
    { "kind": "file", "uri": "file:///repo/src/a.ts", "name": "src/a.ts" },
  ] }));
  let img = user["attachments"][0]["blob"].as_str().unwrap();
  let txt = user["attachments"][1]["blob"].as_str().unwrap();
  assert_eq!(blob(&h, &s.id, img).unwrap(), b"fake-png-bytes");
  assert_eq!(blob(&h, &s.id, txt).unwrap(), b"# notes");
  // the fake agent echoes the block types and key fields it received
  let txt_path = h.dir.path().canonicalize().unwrap().join("sessions").join(&s.id).join(txt);
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], format!("text · image:image/png · resource:file://{}:# notes · resource_link:file:///repo/src/a.ts:src/a.ts", txt_path.display()));
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_only_omit_the_text_block_and_title_the_session_from_what_was_attached() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt(String::new(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": "AAAA" },
    { "kind": "file", "uri": "file:///repo/README.md", "name": "README.md" },
  ])), false, None, None).await;
  let vw = view(&s);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "" }));
  assert_eq!(vw["title"], "1 images, README.md");
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], "image:image/png · resource_link:file:///repo/README.md:README.md");
  // the echoed user_message_chunk (Grok sends the image back too) must not create a second user turn
  assert_eq!(vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_image_file_draft_is_sent_as_pixels_and_an_oversized_image_is_dropped_with_a_note() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let png = dir.path().join("shot.png");
  std::fs::write(&png, "real-png-bytes").unwrap();
  let mut h = Harness::new(&fake, json!({}));
  let notes = Arc::new(Mutex::new(Vec::<String>::new()));
  let n = notes.clone();
  h.deps.notify = Some(Arc::new(move |t: &str| n.lock().unwrap().push(t.to_owned())));
  let s = started(&h, "/tmp").await;
  let huge = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_IMAGE_BYTES + 1]);
  s.prompt("echo blocks".into(), drafts(json!([
    { "kind": "file", "uri": format!("file://{}", png.display()), "name": "shot.png" },
    { "kind": "image", "mimeType": "image/png", "data": huge, "name": "huge.png" },
  ])), false, None, None).await;
  let vw = view(&s);
  let shot = &vw["turns"][0]["attachments"][0];
  expect_match(shot, json!({ "kind": "image", "mimeType": "image/png", "name": "shot.png" }));
  assert_eq!(blob(&h, &s.id, shot["blob"].as_str().unwrap()).unwrap(), b"real-png-bytes");
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], "text · image:image/png");
  assert_eq!(*notes.lock().unwrap(), [format!("huge.png exceeds {} MB, skipped", MAX_IMAGE_BYTES >> 20)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_blob_store_does_not_lose_the_prompt() {
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  // The sessions directory cannot be created: every blob write fails
  std::fs::write(h.dir.path().join("sessions"), "not a directory").unwrap();
  let notes = Arc::new(Mutex::new(Vec::<String>::new()));
  let n = notes.clone();
  h.deps.notify = Some(Arc::new(move |t: &str| n.lock().unwrap().push(t.to_owned())));
  let s = started(&h, "/tmp").await;
  s.prompt("echo blocks".into(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": "AAAA", "name": "shot.png" },
    { "kind": "text", "name": "n.md", "text": "x" },
  ])), false, None, None).await;
  let vw = view(&s);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "echo blocks", "attachments": [{ "kind": "image", "mimeType": "image/png", "name": "shot.png" }, { "kind": "text", "name": "n.md" }] }));
  let echoed = vw["turns"][1]["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(echoed["markdown"], "text · image:image/png · resource:attachment:///n.md:x");
  let notes = notes.lock().unwrap();
  assert_eq!(notes.len(), 2);
  assert!(notes[0].contains("shot.png"), "{notes:?}");
}

/// A file draft whose image read blocks until released: staging waits on a FIFO the way the TS suite gated saveBlob
#[cfg(unix)]
struct StagingGate {
  _dir: tempfile::TempDir,
  path: std::path::PathBuf,
}

#[cfg(unix)]
impl StagingGate {
  fn new() -> StagingGate {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gate.png");
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    StagingGate { _dir: dir, path }
  }
  fn draft(&self) -> Vec<Draft> {
    drafts(json!([{ "kind": "file", "uri": format!("file://{}", self.path.display()), "name": "gate.png" }]))
  }
  fn release(&self) {
    let path = self.path.clone();
    std::thread::spawn(move || std::fs::write(path, b"png-bytes"));
  }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn cancel_while_staging_drops_the_prompt_and_a_send_meanwhile_goes_out_afterwards() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let gate = StagingGate::new();
  let first = tokio::spawn(s.prompt("echo blocks".into(), gate.draft(), false, None, None));
  until(|| view(&s)["running"] == true, 5000).await;
  s.cancel().await;
  prompt(&s, "hi").await;
  let queued: Vec<Value> = view(&s)["queued"].as_array().cloned().unwrap_or_default();
  assert_eq!(queued.iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("hi")]);
  gate.release();
  first.await.unwrap();
  until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().len() == 2, 5000).await;
  let vw = view(&s);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "hi" }));
  expect_absent(&vw, "queued");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn dispose_while_staging_appends_and_sends_nothing_afterwards() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let gate = StagingGate::new();
  let p = tokio::spawn(s.prompt("echo blocks".into(), gate.draft(), false, None, None));
  until(|| view(&s)["running"] == true, 5000).await;
  s.dispose();
  gate.release();
  p.await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["turns"], json!([]));
  assert_eq!(vw["running"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn permission_card_approve_then_the_tool_completes_with_a_normalized_diff_and_usage() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "use tool");
  let perm = wait_block(&s, "permission").await;
  assert_eq!(perm["command"], "pnpm test");
  assert_eq!(perm["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("allow"), json!("reject")]);
  assert_eq!(view(&s)["turns"][1]["activity"]["label"], "Awaiting approval");
  s.resolve_permission(perm["id"].as_str().unwrap(), "allow");
  p.await.unwrap();
  let vw = view(&s);
  let blocks = vw["turns"][1]["blocks"].as_array().unwrap().clone();
  assert!(!blocks.iter().any(|b| b["type"] == "permission"));
  let tc1 = blocks.iter().find(|b| b["id"] == "tc1").unwrap();
  expect_match(tc1, json!({ "kind": "execute", "verb": "Run", "target": "pnpm test", "targetMono": true, "status": "completed" }));
  expect_eq(&tc1["content"], json!({ "type": "text", "text": "12 passed" }));
  let tc2 = blocks.iter().find(|b| b["id"] == "tc2").unwrap();
  expect_match(tc2, json!({ "kind": "edit", "target": "a.ts", "diffStat": { "add": 2, "del": 1 } }));
  expect_eq(&vw["usage"], json!({ "used": 1234, "size": 100000, "cost": 0.01 }));
}

// OpenCode's write: the permission request embeds a low-fidelity copy of the call (kind 'other', the parent dir as
// title, file + dir locations) that races the real in_progress update — neither order may downgrade the block
#[tokio::test(flavor = "multi_thread")]
async fn a_low_fidelity_permission_tool_call_cannot_downgrade_the_edit_block() {
  let fake = fake_or_skip!();
  for script in ["tool-downgrade", "tool-downgrade-late"] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    let p = spawn_prompt(&s, script);
    let perm = wait_block(&s, "permission").await;
    let w1 = || agent_blocks(&view(&s)).into_iter().find(|b| b["id"] == "w1").unwrap();
    expect_match(w1(), json!({ "kind": "edit", "target": "a.txt", "locations": [{ "path": "/tmp/proj/a.txt" }] }));
    let title = perm["title"].as_str().unwrap();
    assert!(title.contains("Edit") && title.contains("a.txt") && !title.contains("Use tool"), "{script}: {title}");
    assert_eq!(perm["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("once"), json!("always"), json!("reject")]);
    s.resolve_permission(perm["id"].as_str().unwrap(), "once");
    p.await.unwrap();
    // The new-file write renders as the all-add diff parked from the in_progress update's rawInput.content
    expect_match(w1(), json!({ "kind": "edit", "status": "completed", "content": { "type": "diff", "source": { "path": "/tmp/proj/a.txt" } } }));
  }
}

// claude-agent-acp / codex-acp decorate session/request_permission with _meta.permission (version 1): the card
// takes the adapter's own title/description/defaultToNo, and each option may carry its own description
#[tokio::test(flavor = "multi_thread")]
async fn permission_meta_drives_the_card_and_option_details_ride_along() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "perm-meta");
  let perm = wait_block(&s, "permission").await;
  assert_eq!(perm["title"], "Run command?");
  assert_eq!(perm["description"], "Reason: cleans the tree");
  assert_eq!(perm["defaultToNo"], true);
  assert_eq!(perm["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("yes-proceed"), json!("yes-session"), json!("no-diff")]);
  assert_eq!(perm["options"][1]["detail"], "Remembered for this session");
  s.resolve_permission(perm["id"].as_str().unwrap(), "yes-proceed");
  p.await.unwrap();
  let blocks = view(&s)["turns"][1]["blocks"].as_array().unwrap().clone();
  expect_match(blocks.iter().find(|b| b["id"] == "pm1").unwrap(), json!({ "status": "completed" }));
  expect_match(blocks.last().unwrap(), json!({ "type": "text", "markdown": "picked yes-proceed" }));
}

// ACP RFD boolean-config-option: the fake only offers its `fast` toggle to clients advertising
// clientCapabilities.session.configOptions.boolean, and the set request must carry a real boolean
#[tokio::test(flavor = "multi_thread")]
async fn a_boolean_config_option_arrives_gated_and_goes_out_as_a_real_boolean() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let log_file = dir.path().join("config.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_BOOL": "1", "FAKE_CONFIG_LOG": log_file } }));
  let s = started(&h, "/tmp").await;
  let fast = view(&s)["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == "fast").cloned().unwrap();
  expect_match(&fast, json!({ "type": "boolean", "name": "Fast mode", "value": "false" }));
  assert_eq!(fast["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("false"), json!("true")]);
  s.set_config("fast".into(), "true".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "fast"), "true");
  s.set_config("fast".into(), "false".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "fast"), "false");
  let wire: Vec<Value> = std::fs::read_to_string(&log_file).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
  expect_eq(wire, json!([{ "configId": "fast", "type": "boolean", "value": true }, { "configId": "fast", "type": "boolean", "value": false }]));
}

// codex-acp / claude-agent-acp send images as message chunks and tool content items; the payload lands in the
// session's blob store under its content-hash name (replay writes the same file, no duplicates)
#[tokio::test(flavor = "multi_thread")]
async fn agent_emitted_images_land_in_the_blob_store() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "image").await;
  let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==").unwrap();
  let name = blob_name(".png", &png);
  // Agent images are written off the update path
  until(|| blob(&h, &s.id, &name).is_some(), 5000).await;
  assert_eq!(blob(&h, &s.id, &name).unwrap(), png);
  let agent = view(&s)["turns"][1].clone();
  let blocks = agent["blocks"].as_array().unwrap();
  expect_match(blocks.iter().find(|b| b["type"] == "image").unwrap(), json!({ "type": "image", "mimeType": "image/png", "blob": name }));
  assert_eq!(blocks.iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].clone()).collect::<Vec<_>>(), [json!("here is "), json!("the red dot")]);
  let tool = blocks.iter().find(|b| b["id"] == "im1").unwrap();
  expect_eq(&tool["contents"], json!([
    { "type": "text", "text": "Revised prompt: red dot" },
    { "type": "image", "mimeType": "image/png", "blob": name, "uri": "/repo/red.png" },
  ]));
  expect_eq(&tool["content"], json!({ "type": "text", "text": "Revised prompt: red dot" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn synthesized_modes_backfill_from_the_registry_and_go_through_set_mode() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-no-modes").unwrap();
  let h = Harness::new(&fake, json!({ "modes": syn_modes() }));
  let s = started(&h, "/tmp/acpira-no-modes").await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  let ids = |k: &str| vw["controls"][k].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
  assert_eq!(ids("modes"), ["default", "plan", "yolo"]);
  assert_eq!(vw["controls"]["modeId"], "default");
  // configOptions unaffected, still land in controls.options
  assert_eq!(ids("options"), ["model", "effort"]);
  s.set_mode("plan".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "plan");
  s.set_mode("default".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "default");
}

#[tokio::test(flavor = "multi_thread")]
async fn synthesized_yolo_auto_approves_permission_requests() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-no-modes").unwrap();
  let h = Harness::new(&fake, json!({ "modes": syn_modes() }));
  let s = started(&h, "/tmp/acpira-no-modes").await;
  // plan → yolo: covers the "pull the CLI back to default first" path
  s.set_mode("plan".into()).await.unwrap();
  s.set_mode("yolo".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "yolo");
  prompt(&s, "use tool").await;
  let blocks = view(&s)["turns"][1]["blocks"].as_array().unwrap().clone();
  assert!(!blocks.iter().any(|b| b["type"] == "permission"));
  assert_eq!(blocks.iter().find(|b| b["id"] == "tc1").unwrap()["status"], "completed");
}

pub fn history_edit(s: &AcpSession, turn_index: usize, text: &str) -> acpira_shared::protocol::EditTurnRequest {
  let vw = s.view();
  let turn = v(&vw.turns[turn_index]);
  assert_eq!(turn["role"], "user", "expected a user turn");
  let retained: Vec<i64> = (0..turn["attachments"].as_array().map_or(0, |a| a.len()) as i64).collect();
  serde_json::from_value(json!({
    "sessionId": s.id, "turnIndex": turn_index, "turnCount": vw.turns.len(), "originalText": turn["text"], "turnId": turn["id"],
    "text": text, "retainedAttachments": retained, "attachments": [],
    "settings": acpira_shared::turn_settings::capture_turn_settings(&vw.controls),
  }))
  .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_into_synthesized_yolo_approves_pending_permissions_too() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-no-modes").unwrap();
  let h = Harness::new(&fake, json!({ "modes": syn_modes() }));
  let s = started(&h, "/tmp/acpira-no-modes").await;
  let p = spawn_prompt(&s, "use tool");
  wait_block(&s, "permission").await;
  s.set_mode("yolo".into()).await.unwrap();
  p.await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["running"], false);
  let blocks = vw["turns"][1]["blocks"].as_array().unwrap().clone();
  assert!(!blocks.iter().any(|b| b["type"] == "permission"));
  assert_eq!(blocks.iter().find(|b| b["id"] == "tc1").unwrap()["status"], "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_views_carry_a_monotonic_rev_and_leave_running_false_after_the_prompt() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = Disposing(h.session("/tmp"));
  let before = s.view().rev.unwrap_or(0);
  s.start().await;
  let ready = s.view().rev.unwrap_or(0);
  assert!(ready > before);
  assert_eq!(s.view().rev.unwrap_or(0), ready);
  prompt(&s, "hi").await;
  let done = view(&s);
  assert_eq!(done["running"], false);
  assert!(done["rev"].as_i64().unwrap_or(0) > ready);
  assert_eq!(last_turn(&done)["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_stops_text_midway_wraps_up_the_turn_and_allows_another_send() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["turns"][1]["blocks"].as_array().is_some_and(|b| b.iter().any(|b| b["type"] == "text")), 5000).await;
  s.cancel().await;
  p.await.unwrap();
  assert_eq!(view(&s)["running"], false);
  prompt(&s, "hi").await;
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_error_ends_the_turn_with_a_typed_error_and_retry_turn_sends_it_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "please fail").await;
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  assert_eq!(vw["running"], false);
  expect_absent(&vw, "error");
  assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  assert_eq!(vw["turns"][1]["stop"], "error");
  expect_eq(&vw["turns"][1]["error"], json!({ "message": "Upstream error: quota exhausted", "code": -32603, "kind": "upstream_error", "retryable": true }));
  s.retry_turn().await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "please fail" }));
  assert_eq!(vw["turns"][1]["stop"], "end_turn");
  assert!(vw["turns"][1]["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "text"));
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_preserves_output_completed_tools_and_the_native_session_after_quota_errors() {
  let fake = fake_or_skip!();
  for edited in [false, true] {
    let native = tempfile::tempdir().unwrap();
    let h = Harness::new(&fake, json!({ "env": { "FAKE_SESSION_DIR": native.path() } }));
    let s = started(&h, "/tmp").await;
    prompt(&s, "earlier-context").await;
    let text = "fail-after-output fail-twice";
    if edited {
      prompt(&s, "original").await;
      s.edit_turn(history_edit(&s, 2, text)).await.unwrap();
      until(|| !s.is_running(), 5000).await;
    } else {
      s.prompt(text.into(), drafts(json!([{ "kind": "text", "name": "plan.txt", "text": "Retain this plan on retry." }])), false, None, None).await;
    }
    let peer = s.to_record().acp_session_id;
    for expected in ["error", "end_turn"] {
      let before = v(&s.to_record().turns);
      let n = before.as_array().unwrap().len();
      expect_match(before.as_array().unwrap().last().unwrap(), json!({ "stop": "error", "blocks": [
        { "type": "thought" }, { "type": "text" }, { "type": "tool_call", "status": "completed" },
      ] }));
      let (a, b) = tokio::join!(s.retry_turn(), s.retry_turn());
      a.ok();
      b.ok();
      until(|| !s.is_running(), 5000).await;
      assert_eq!(s.to_record().acp_session_id, peer, "edited={edited}");
      let vw = view(&s);
      assert_eq!(json!(vw["turns"].as_array().unwrap()[..n]), before, "edited={edited}");
      assert_eq!(vw["turns"].as_array().unwrap().len(), n + 2);
      expect_match(last_turn(&vw), json!({ "stop": expected }));
      if !edited {
        expect_match(turn_at(&vw, -2), json!({ "attachments": [{ "kind": "text", "name": "plan.txt" }] }));
      }
      let restored = Disposing(AcpSession::new(s.to_record(), h.deps.clone()));
      assert_eq!(json!(view(&restored)["turns"].as_array().unwrap()[..n]), before);
    }
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_turn_rebuilds_attachments_from_their_blobs_and_does_nothing_after_a_normal_end() {
  use base64::Engine;
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("fail with picture".into(), drafts(json!([
    { "kind": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode("png!"), "name": "shot.png" },
  ])), false, None, None).await;
  assert_eq!(std::fs::read_dir(h.dir.path().join("sessions").join(&s.id)).unwrap().count(), 1);
  s.retry_turn().await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  let user = &vw["turns"][0];
  expect_match(user, json!({ "role": "user", "text": "fail with picture", "attachments": [{ "kind": "image", "mimeType": "image/png", "name": "shot.png" }] }));
  // The re-sent image is byte-for-byte the original
  assert_eq!(blob(&h, &s.id, user["attachments"][0]["blob"].as_str().unwrap()).unwrap(), b"png!");
  assert_eq!(vw["turns"][1]["stop"], "end_turn");
  expect_match(&vw["turns"][1]["blocks"][0], json!({ "type": "text", "markdown": "text · image:image/png" }));
  s.retry_turn().await.ok();
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_replaces_the_process_and_resumes_the_same_native_session() {
  let fake = fake_or_skip!();
  // A cwd containing "flaky-resume" makes the fake agent resume any known-or-not sessionId while no resume.lock sits in it
  let cwd = tempfile::Builder::new().prefix("acpira-flaky-resume-").tempdir().unwrap();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, cwd.path().to_str().unwrap()).await;
  s.set_config("model".into(), "m2".into()).await.unwrap();
  prompt(&s, "please fail").await;
  let record = s.to_record();
  assert_eq!(view(&s)["status"], "ready");
  expect_match(&view(&s)["turns"][1], json!({ "role": "agent", "stop": "error" }));
  s.reconnect().await.unwrap();
  let vw = view(&s);
  assert_eq!(vw["status"], "ready");
  assert_eq!(s.to_record().acp_session_id, record.acp_session_id);
  let logs = h.logs();
  assert_eq!(logs.iter().filter(|l| l.contains("spawn ")).count(), 2, "{logs:#?}");
  assert!(logs.iter().any(|l| l.contains("session/resume ok")));
  assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  expect_match(&vw["turns"][1], json!({ "role": "agent", "stop": "error" }));
  assert_eq!(option_value(&vw, "model"), "m2");
  // Not retryTurn: the fake's per-process "fail once" map resets on the respawn, so resending 'please fail' would fail again
  prompt(&s, "hi").await;
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 4);
  expect_match(&view(&s)["turns"][3], json!({ "role": "agent", "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn reconnect_is_refused_while_a_turn_runs_and_the_process_is_kept() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let p = spawn_prompt(&s, "slow");
  until(|| view(&s)["running"] == true, 5000).await;
  assert!(s.reconnect().await.is_err());
  assert_eq!(h.logs().iter().filter(|l| l.contains("spawn ")).count(), 1);
  assert_eq!(view(&s)["running"], true);
  s.cancel().await;
  p.await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_completion_reports_missing_output_and_retries_on_the_same_native_session() {
  let fake = fake_or_skip!();
  for text in ["empty-response", "empty-response-whitespace"] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    let native = s.to_record().acp_session_id;
    prompt(&s, text).await;
    assert!(h.logs().iter().any(|l| l.contains("prompt done: end_turn")), "{text}");
    expect_match(view(&s), json!({ "status": "ready", "running": false }));
    let last = last_turn(&view(&s));
    expect_match(&last, json!({ "stop": "error", "error": { "kind": "empty_response", "retryable": true } }));
    assert!(last["error"]["message"].as_str().unwrap().contains("no reply"));
    expect_absent(&last, "error.code");
    expect_match(s.to_record().turns.last().unwrap(), json!({ "stop": "error" }));
    s.retry_turn().await.unwrap();
    assert_eq!(s.to_record().acp_session_id, native);
    assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 2);
    expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn short_stops_record_refusal_and_max_tokens_and_a_normal_turn_records_end_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "refuse this").await;
  prompt(&s, "truncate this").await;
  prompt(&s, "hi").await;
  let vw = view(&s);
  let (refused, truncated, ok) = (&vw["turns"][1], &vw["turns"][3], &vw["turns"][5]);
  assert_eq!(refused["stop"], "refusal");
  assert_eq!(refused["blocks"], json!([]));
  assert_eq!(truncated["stop"], "max_tokens");
  expect_match(truncated["blocks"].as_array().unwrap().last().unwrap(), json!({ "type": "text", "markdown": "once upon a", "streaming": false }));
  assert_eq!(ok["stop"], "end_turn");
  expect_absent(ok, "error");
}

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
  until(|| view(&s)["turns"].as_array().unwrap().len() == 4 && view(&s)["running"] == false, 5000).await;
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
  until(|| view(&s)["turns"].as_array().unwrap().len() == 4 && view(&s)["running"] == false, 5000).await;
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
  until(|| view(&s)["turns"].as_array().unwrap().len() == 2, 5000).await;
  prompt(&s, "first").await;
  s.prompt("priority".into(), drafts(json!([{ "kind": "image", "mimeType": "image/png", "data": "AAAA", "name": "priority.png" }])), false, None, None).await;
  prompt(&s, "last").await;
  let selected = view(&s)["queued"][1].clone();
  let sid = selected["id"].as_str().unwrap();
  let (a, b) = tokio::join!(s.send_queued(sid), s.send_queued(sid));
  a.ok();
  b.ok();
  running.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].is_null(), 5000).await;
  let vw = view(&s);
  let users: Vec<Value> = vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect();
  assert_eq!(users, [json!("slow"), json!("priority"), json!("first"), json!("last")]);
  expect_match(&vw["turns"][1], json!({ "role": "agent", "stop": "cancelled" }));
  expect_match(&vw["turns"][2], json!({ "role": "user", "attachments": selected["attachments"] }));
  // A stale row must not interrupt the next unrelated turn
  let next = spawn_prompt(&s, "slow again");
  until(|| view(&s)["turns"].as_array().unwrap().len() == 10, 5000).await;
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

// The session list sorts by updatedAt: only the user's message may move a session, never the stream that follows it
#[tokio::test(flavor = "multi_thread")]
async fn updated_at_is_bumped_once_by_the_prompt_then_stable_across_the_stream() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  let before = s.view().updated_at.clone();
  tokio::time::sleep(std::time::Duration::from_millis(5)).await;
  let seen = h.sample(&s, |vw| vw.updated_at.clone());
  prompt(&s, "hi").await;
  until(|| seen.lock().unwrap().len() > 2, 5000).await;
  let seen = seen.lock().unwrap().clone();
  assert_ne!(seen[0], before);
  assert_eq!(seen.iter().collect::<std::collections::HashSet<_>>().len(), 1, "{seen:?}");
  assert_eq!(s.view().updated_at, seen[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_mode_model_and_effort_and_rename_and_pin_leave_updated_at_untouched() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.set_mode("plan".into()).await.unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "plan");
  s.set_config("model".into(), "m2".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "model"), "m2");
  s.set_config("effort".into(), "low".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  assert_eq!(option_value(&view(&s), "model"), "m2");
  s.set_config("nope".into(), "x".into()).await.ok();
  let before = s.view().updated_at.clone();
  s.rename("  改个名  ");
  s.set_pinned(true);
  assert_eq!(view(&s)["title"], "改个名");
  assert_eq!(s.to_record().pinned, Some(true));
  assert_eq!(s.view().updated_at, before);
}

/// Runs a call's synchronous prefix now (the TS call before its first await) and hands back the rest
pub fn claimed<T: Send + 'static>(fut: impl std::future::Future<Output = T> + Send + 'static) -> tokio::sync::oneshot::Receiver<T> {
  let (tx, rx) = tokio::sync::oneshot::channel();
  acpira_host::util::run_prefix(async move {
    let _ = tx.send(fut.await);
  });
  rx
}

fn agent_value(s: &AcpSession, id: &str) -> Option<String> {
  s.agent_controls().options.iter().find(|o| o.id == id).and_then(|o| o.value.clone())
}

fn record_value(s: &AcpSession, id: &str) -> Option<String> {
  s.to_record().controls.options.iter().find(|o| o.id == id).and_then(|o| o.value.clone())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_optimistic_config_pick_moves_the_view_at_once_while_the_record_waits_for_the_agent() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150" } }));
  let s = started(&h, "/tmp").await;
  let before = h.changes();
  let p = tokio::spawn({
    let s = s.0.clone();
    async move { s.select_config("effort".into(), "low".into()).await }
  });
  until(|| option_value(&view(&s), "effort") == "low", 1000).await;
  assert!(h.changes() > before);
  assert_eq!(record_value(&s, "effort").as_deref(), Some("high"));
  p.await.unwrap().unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  assert_eq!(record_value(&s, "effort").as_deref(), Some("low"));
  assert_eq!(agent_value(&s, "effort").as_deref(), Some("low"));
}

#[tokio::test(flavor = "multi_thread")]
async fn rapid_config_picks_collapse_to_the_last_value_without_flicker() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150" } }));
  let s = started(&h, "/tmp").await;
  assert_eq!(agent_value(&s, "model").as_deref(), Some("m1"));
  // Both clicks land before anything is observed, like two synchronous TS calls
  let (s1, s2) = (s.0.clone(), s.0.clone());
  let p1 = claimed(async move { s1.select_config("model".into(), "m1".into()).await });
  let p2 = claimed(async move { s2.select_config("model".into(), "m2".into()).await });
  let seen = h.sample(&s, |vw| vw.controls.options.iter().find(|o| o.id == "model").and_then(|o| o.value.clone()));
  p1.await.ok();
  p2.await.ok();
  until(|| !seen.lock().unwrap().is_empty(), 5000).await;
  let seen = seen.lock().unwrap().clone();
  assert!(seen.iter().all(|x| x.as_deref() == Some("m2")), "{seen:?}");
  assert_eq!(agent_value(&s, "model").as_deref(), Some("m2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_optimistic_value_reverts_the_view_to_agent_truth() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150", "FAKE_MODELS": "unavailable" } }));
  let s = started(&h, "/tmp").await;
  let p = tokio::spawn({
    let s = s.0.clone();
    async move { s.select_config("model".into(), "unavailable".into()).await }
  });
  until(|| option_value(&view(&s), "model") == "unavailable", 1000).await;
  assert_eq!(record_value(&s, "model").as_deref(), Some("m1"));
  assert!(p.await.unwrap().is_err());
  assert_eq!(option_value(&view(&s), "model"), "m1");
  assert_eq!(record_value(&s, "model").as_deref(), Some("m1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_optimistic_mode_pick_moves_the_view_at_once_and_agent_truth_follows_the_wire() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "150" } }));
  let s = started(&h, "/tmp").await;
  let p = tokio::spawn({
    let s = s.0.clone();
    async move { s.select_mode("plan".into()).await }
  });
  until(|| view(&s)["controls"]["modeId"] == "plan", 1000).await;
  assert_eq!(s.agent_controls().mode_id.as_deref(), Some("agent"));
  p.await.unwrap().unwrap();
  assert_eq!(view(&s)["controls"]["modeId"], "plan");
  assert_eq!(s.agent_controls().mode_id.as_deref(), Some("plan"));
}

#[tokio::test(flavor = "multi_thread")]
async fn native_effort_survives_a_fusion_sidekick_change_on_the_wire() {
  let fake = fake_or_skip!();
  let first = "Fusion (GPT-6 Astra High Thinking + SWE-2 Medium)";
  let second = "Fusion (GPT-6 Astra High Thinking + SWE-2 High)";
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODELS": format!("{first},{second}"), "FAKE_MODEL_RESETS_EFFORT": "1" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("model".into(), first.into()).await.unwrap();
  s.set_config("effort".into(), "low".into()).await.unwrap();
  s.set_config("model".into(), second.into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  assert_eq!(option_value(&view(&s), "model"), second);
  // Another round trip proves the restored value belongs to the agent, not only the view
  s.set_config("model".into(), first.into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
  s.set_config("model".into(), "m2".into()).await.unwrap();
  assert_eq!(option_value(&view(&s), "effort"), "low");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_model_switch_keeps_the_chosen_effort_and_never_shows_the_interim_reset() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODEL_RESETS_EFFORT": "1", "FAKE_CONFIG_DELAY_MS": "60" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("effort".into(), "low".into()).await.unwrap();
  let seen = h.sample(&s, |vw| vw.controls.options.iter().find(|o| o.id == "effort").and_then(|o| o.value.clone()));
  s.select_config("model".into(), "m2".into()).await.unwrap();
  until(|| !seen.lock().unwrap().is_empty(), 5000).await;
  let seen = seen.lock().unwrap().clone();
  assert!(seen.iter().all(|x| x.as_deref() == Some("low")), "{seen:?}");
  assert_eq!(agent_value(&s, "model").as_deref(), Some("m2"));
  assert_eq!(agent_value(&s, "effort").as_deref(), Some("low"));
}

#[tokio::test(flavor = "multi_thread")]
async fn replaying_remembered_controls_does_not_reset_the_previous_effort_between_model_and_effort() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODEL_RESETS_EFFORT": "1" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("effort".into(), "low".into()).await.unwrap();
  s.adopt_controls(serde_json::from_value(json!({ "config": { "model": "m2" } })).unwrap()).await;
  // Nothing remembered for effort: the agent's own value for the new model stands
  assert_eq!(agent_value(&s, "effort").as_deref(), Some("high"));
}

/// A session that ran one turn, disposed, and the record it left
async fn ran_once(h: &Harness, cwd: &str) -> SessionRecord {
  let s = started(h, cwd).await;
  prompt(&s, "hi").await;
  s.to_record()
}

async fn reopened(h: &Harness, record: SessionRecord) -> Disposing {
  let s = Disposing(AcpSession::new(record, h.deps.clone()));
  s.start().await;
  s
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_unknown_to_the_new_process_leaves_the_history_read_only() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, "/tmp").await;
  // new process answers invalidParams "unknown session" — the peer doesn't know the id, the same conclusion as session_not_found:
  // the transcript already ran, so it stays read-only instead of silently continuing on a fresh native context
  let s2 = reopened(&h, record.clone()).await;
  let vw = view(&s2);
  assert_eq!(vw["status"], "readonly");
  assert!(vw["error"].as_str().unwrap().contains("no longer has this session"));
  assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  assert_eq!(s2.to_record().acp_session_id, record.acp_session_id);
  // No fresh native session was opened, so the persisted command list stays until a peer replaces it
  expect_eq(&vw["commands"], json!([{ "name": "compact", "description": "compact it" }]));
}

// DeepSeek Harness reports every restore problem as a bare invalidParams; the reason only survives in the message text
#[tokio::test(flavor = "multi_thread")]
async fn dsh_restore_failures_are_told_apart_by_their_message() {
  let fake = fake_or_skip!();
  for (tag, status, error) in [
    ("dsh-active", "error", "held by another"),
    ("dsh-cwd", "error", "Could not restore"),
    ("dsh-unresumable", "readonly", "cannot resume this session"),
    ("dsh-mcp", "error", "Could not restore"),
  ] {
    let dir = tempfile::Builder::new().prefix(&format!("acpira-{tag}-")).tempdir().unwrap();
    let h = Harness::new(&fake, json!({}));
    let record = ran_once(&h, dir.path().to_str().unwrap()).await;
    let s2 = reopened(&h, record).await;
    let vw = view(&s2);
    assert_eq!(vw["status"], status, "{tag}");
    assert!(vw["error"].as_str().unwrap_or("").contains(error), "{tag}: {}", vw["error"]);
    assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_sends_session_close_to_an_agent_that_advertises_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let close_log = dir.path().join("close.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CLOSE_LOG": close_log } }));
  let s = h.session("/tmp");
  s.start().await;
  prompt(&s, "hi").await;
  let native = s.to_record().acp_session_id.unwrap();
  s.dispose();
  until(|| std::fs::read_to_string(&close_log).is_ok_and(|t| t.contains(&native)), 5000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ignore_modes_drops_protocol_modes_and_a_pushed_mode_update() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "ignoreModes": true }));
  let s = started(&h, "/tmp").await;
  assert_eq!(view(&s)["controls"]["modes"], json!([]));
  expect_absent(&view(&s)["controls"], "modeId");
  prompt(&s, "mode:plan").await;
  expect_absent(&view(&s)["controls"], "modeId");
  assert_eq!(view(&s)["controls"]["modes"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_strict_prompt_capabilities_agent_receives_dropped_text_as_marked_up_text() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
  let s = started(&h, "/tmp").await;
  s.prompt("echo-blocks".into(), drafts(json!([{ "kind": "text", "name": "notes.txt", "text": "payload" }])), false, None, None).await;
  let block = last_turn(&view(&s))["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "text").cloned().unwrap();
  assert_eq!(block["markdown"], "text,text");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_startup_banner_streamed_before_session_new_returns_is_ignored() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STARTUP_BANNER": "early" } }));
  let s = started(&h, "/tmp").await;
  // The banner carried no session id (none existed yet): it must not open a ghost agent turn
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 0);
  assert!(h.logs().iter().any(|l| l.contains("startup agent_message_chunk ignored")));
  prompt(&s, "hi").await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "end_turn" }));
}

// pi-acp's real timing: the prelude text rides session/new's _meta.piAcp.startupInfo and is re-sent as one
// agent_message_chunk a tick after the response — past the no-session guard, so the exact text does the match
#[tokio::test(flavor = "multi_thread")]
async fn a_startup_banner_sent_right_after_session_new_is_matched_and_dropped() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STARTUP_BANNER": "1" } }));
  let s = started(&h, "/tmp").await;
  until(|| h.logs().iter().any(|l| l.contains("startup banner ignored")), 5000).await;
  // The banner must not open a ghost agent turn
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 0);
  prompt(&s, "hi").await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_prompt_queued_during_start_is_not_prefixed_by_the_late_banner() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STARTUP_BANNER": "1" } }));
  let s = Disposing(h.session("/tmp"));
  let started = claimed({
    let s = s.0.clone();
    async move { s.start().await }
  });
  // enqueue resolves once the prompt is parked — the turn runs after start flushes the queue
  prompt(&s, "hello").await;
  started.await.unwrap();
  until(|| last_turn(&view(&s))["stop"] == "end_turn", 5000).await;
  let text: String = last_turn(&view(&s))["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].as_str().unwrap().to_owned()).collect();
  // The observed wire shape was "pi v0.86.0 --- ## Skills …pong" — the banner prepended to the first reply chunk
  assert!(!text.contains("pi v0.0 banner"), "{text}");
  assert!(text.contains("hello world"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_a_native_session_the_agent_no_longer_has_lands_on_the_error_state() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_SESSION_DIR": dir.path() } }));
  let now = acpira_host::util::now_iso();
  let record: SessionRecord = serde_json::from_value(json!({
    "id": uuid::Uuid::new_v4().to_string(), "agent": "fake", "acpSessionId": "native-gone", "cwd": "/tmp", "title": "Imported session",
    "createdAt": now, "updatedAt": now, "turns": [], "controls": { "modes": [], "options": [] }, "commands": [],
    "importPending": true, "importedFrom": { "sessionId": "native-gone" },
  })).unwrap();
  let s = reopened(&h, record).await;
  // session/load answered session_not_found; an import has no transcript to keep read-only, so it is the error Notice (Retry) — and no fresh native session was created
  assert_eq!(view(&s)["status"], "error");
  assert_eq!(view(&s)["error"], "The agent no longer has this session");
  assert_eq!(h.logs().iter().filter(|l| l.contains("session/new ok")).count(), 0);
  expect_match(s.to_record(), json!({ "acpSessionId": "native-gone", "importedFrom": { "sessionId": "native-gone" } }));
  assert!(!s.to_record().import_pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_restore_attempt_lands_on_the_error_state_and_retry_reconnects() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-flaky-resume-").tempdir().unwrap();
  std::fs::write(dir.path().join("resume.lock"), "").unwrap();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, dir.path().to_str().unwrap()).await;
  // resume answers -32603 while resume.lock exists: an internal error is not "can't resume" — the session goes to
  // the error Notice (Retry = full reconnect + resume), not to read-only
  let s2 = reopened(&h, record.clone()).await;
  assert_eq!(view(&s2)["status"], "error");
  assert!(view(&s2)["error"].as_str().unwrap().contains("transient restore failure"));
  std::fs::remove_file(dir.path().join("resume.lock")).unwrap();
  s2.retry().await.unwrap();
  assert_eq!(view(&s2)["status"], "ready");
  assert_eq!(view(&s2)["turns"].as_array().unwrap().len(), 2);
  assert_eq!(s2.to_record().acp_session_id, record.acp_session_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_typed_session_locked_is_reported_as_held_elsewhere() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-locked-").tempdir().unwrap();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, dir.path().to_str().unwrap()).await;
  let s2 = reopened(&h, record).await;
  assert_eq!(view(&s2)["status"], "error");
  assert!(view(&s2)["error"].as_str().unwrap().contains("held by another"));
  assert_eq!(view(&s2)["turns"].as_array().unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn session_not_found_replaces_a_session_that_never_talked_and_keeps_history_read_only() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-gone").unwrap();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, "/tmp/acpira-gone").await;
  // The transcript already ran: swapping in a fresh native session would keep the old conversation on an empty
  // context (compaction included). Read-only, history kept, the native id retained so a later open can retry
  let s2 = reopened(&h, record.clone()).await;
  assert_eq!(view(&s2)["status"], "readonly");
  assert_eq!(view(&s2)["turns"].as_array().unwrap().len(), 2);
  assert_eq!(s2.to_record().acp_session_id, record.acp_session_id);
  let new_ok = || h.logs().iter().filter(|l| l.contains("session/new ok")).count();
  assert_eq!(new_ok(), 1);
  s2.dispose();
  // An empty session (Devin sweeps exactly those when its process exits) is replaced transparently — nothing visible lost its context
  let empty = started(&h, "/tmp/acpira-gone").await;
  let empty_record = empty.to_record();
  empty.dispose();
  let s3 = reopened(&h, empty_record).await;
  assert_eq!(view(&s3)["status"], "ready");
  assert_eq!(view(&s3)["turns"].as_array().unwrap().len(), 0);
  assert_eq!(new_ok(), 3);
  // The replacement native session advertised nothing: the old connection's slash commands do not carry over
  assert_eq!(view(&s3)["commands"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_answered_session_not_found_leaves_ready_and_retry_reconnects() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "hi").await;
  prompt(&s, "prompt-session-gone").await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "error" }));
  // the native session is gone — resending over this connection could only fail the same way
  assert_eq!(view(&s)["status"], "error");
  // Retry = reconnect + resume; the fresh process doesn't know the id either → read-only history, still no silent context swap
  s.retry().await.ok();
  assert_eq!(view(&s)["status"], "readonly");
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 4);
}

fn auto_compaction() -> Option<acpira_host::acp::session::CompactionPolicy> {
  Some(acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: true })
}

fn turn_count(s: &AcpSession) -> usize {
  view(s)["turns"].as_array().unwrap().len()
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_sends_an_auto_turn_over_the_threshold_and_does_not_repeat_without_growth() {
  let fake = fake_or_skip!();
  let h = Harness::with_compaction(&fake, json!({}), auto_compaction());
  let s = started(&h, "/tmp").await;
  prompt(&s, "big").await;
  // auto compaction is already queued (async) when prompt() returns; wait for it to finish
  until(|| turn_count(&s) == 4 && !s.is_running(), 5000).await;
  let vw = view(&s);
  expect_eq(&vw["turns"][2], json!({ "role": "user", "text": "/compact", "auto": true }));
  expect_eq(&vw["turns"][3]["blocks"], json!([{ "type": "compaction", "id": "cp1", "status": "completed" }]));
  assert!(vw["usage"]["used"].as_f64().unwrap() < 300_000.0);
  assert_eq!(vw["title"], "big");
  // grows again → compacts once more, but this time the fake agent can't compact (usage unchanged)
  prompt(&s, "big").await;
  until(|| turn_count(&s) == 8 && !s.is_running(), 5000).await;
  let used = view(&s)["usage"]["used"].as_f64().unwrap();
  assert!(used > 300_000.0);
  // usage didn't grow back much: the next turn end doesn't resend /compact
  prompt(&s, "hi").await;
  tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  assert_eq!(turn_count(&s), 10);
  assert_eq!(view(&s)["usage"]["used"].as_f64().unwrap(), used);
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_runs_before_a_follow_up_queued_during_the_over_threshold_turn() {
  let fake = fake_or_skip!();
  let h = Harness::with_compaction(&fake, json!({}), auto_compaction());
  let s = started(&h, "/tmp").await;
  tokio::join!(prompt(&s, "big"), prompt(&s, "follow-up"));
  until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().iter().any(|t| t["role"] == "user" && t["text"] == "follow-up"), 5000).await;
  let users: Vec<Value> = view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").cloned().collect();
  expect_match(users, json!([{ "text": "big" }, { "text": "/compact", "auto": true }, { "text": "follow-up" }]));
  assert!(view(&s)["usage"]["used"].as_f64().unwrap() < 300_000.0);
}

/// A compaction policy whose `auto` flag the test flips mid-way (the TS `let auto` captured by the deps closure)
fn switchable_compaction() -> (Arc<std::sync::atomic::AtomicBool>, Arc<dyn Fn() -> acpira_host::acp::session::CompactionPolicy + Send + Sync>) {
  let auto = Arc::new(std::sync::atomic::AtomicBool::new(false));
  let a = auto.clone();
  (auto, Arc::new(move || acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: a.load(std::sync::atomic::Ordering::SeqCst) }))
}

fn user_texts(s: &AcpSession) -> Vec<String> {
  view(s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_runs_before_the_next_typed_prompt_when_usage_was_left_over_the_threshold() {
  let fake = fake_or_skip!();
  let (auto, policy) = switchable_compaction();
  let h = Harness::with_compaction_fn(&fake, json!({}), Some(policy));
  let s = started(&h, "/tmp").await;
  prompt(&s, "big").await;
  assert_eq!(turn_count(&s), 2);
  assert!(view(&s)["usage"]["used"].as_f64().unwrap() > 300_000.0);
  auto.store(true, std::sync::atomic::Ordering::SeqCst);
  prompt(&s, "hi").await;
  until(|| !s.is_running() && user_texts(&s).contains(&"hi".to_owned()), 5000).await;
  let users: Vec<Value> = view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").cloned().collect();
  expect_match(users, json!([{ "text": "big" }, { "text": "/compact", "auto": true }, { "text": "hi" }]));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_submitted_message_shows_below_automatic_compaction_while_the_peer_is_compacting() {
  let fake = fake_or_skip!();
  let (auto, policy) = switchable_compaction();
  let h = Harness::with_compaction_fn(&fake, json!({ "env": { "FAKE_COMPACTION": "structured" } }), Some(policy));
  let s = started(&h, "/tmp").await;
  prompt(&s, "big").await;
  auto.store(true, std::sync::atomic::Ordering::SeqCst);
  let sent = tokio::spawn(s.prompt("visible follow-up".into(), drafts(json!([{ "kind": "text", "name": "note.txt", "text": "attached note" }])), false, None, None));
  until(|| h.logs().iter().any(|l| l.contains("waiting for compaction completion")), 5000).await;
  // The peer has not received the follow-up yet, but its bubble and staged attachment must already be visible
  expect_match(last_turn(&view(&s)), json!({ "role": "user", "text": "visible follow-up", "attachments": [{ "name": "note.txt" }] }));
  expect_match(s.to_record().turns.last().unwrap(), json!({ "role": "user", "text": "visible follow-up" }));
  expect_match(turn_at(&view(&s), -2), json!({ "role": "agent", "blocks": [{ "type": "compaction", "status": "in_progress" }] }));
  prompt(&s, "later queued message").await;
  s.set_config("effort".into(), "high".into()).await.ok();
  sent.await.unwrap();
  until(|| !s.is_running() && view(&s)["queued"].as_array().is_none_or(|q| q.is_empty()) && user_texts(&s).len() == 4, 5000).await;
  assert_eq!(user_texts(&s), ["big", "/compact", "visible follow-up", "later queued message"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_compaction_sends_compact_when_available_and_errors_when_not() {
  let fake = fake_or_skip!();
  let h = Harness::with_compaction(&fake, json!({}), Some(acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: false }));
  let s = started(&h, "/tmp").await;
  let err = s.compact(false).await.expect_err("no /compact yet");
  assert!(err.to_string().contains("/compact"), "{err}");
  prompt(&s, "big").await;
  tokio::time::sleep(std::time::Duration::from_millis(200)).await;
  assert_eq!(turn_count(&s), 2);
  s.compact(false).await.unwrap();
  assert_eq!(turn_count(&s), 4);
  expect_match(&view(&s)["turns"][2], json!({ "role": "user", "text": "/compact" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn login_goes_through_auth_required_authenticate_and_a_successful_retry() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-needs-auth").unwrap();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp/acpira-needs-auth").await;
  assert_eq!(view(&s)["status"], "auth_required");
  assert_eq!(view(&s)["authMethods"][0]["id"], "fake.login");
  // The reason the CLI logged to stderr right before -32000 is surfaced instead of a bare "log in" (stderr may be read after the
  // response, so it can land a moment later)
  until(|| view(&s)["error"] == "provider managed:fake has no credential configured", 5000).await;
  s.authenticate(None).await.unwrap();
  s.retry().await.unwrap();
  assert_eq!(view(&s)["status"], "ready");
  expect_absent(view(&s), "error");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_auth_method_lands_on_the_view_but_authenticate_refuses_to_send_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-term-").tempdir().unwrap();
  let auth_log = dir.path().join("auth.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_TERMINAL_AUTH": auth_log } }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  assert_eq!(view(&s)["status"], "auth_required");
  let methods = view(&s)["authMethods"].as_array().unwrap().clone();
  let term = methods.iter().find(|m| m["id"] == "term-login").expect("terminal method");
  expect_match(term, json!({ "terminal": { "args": ["--login"], "env": { "FAKE_LOGIN": "1", "FAKE_FLAG": "method" } } }));
  let err = s.authenticate(Some("term-login")).await.expect_err("refused");
  assert!(err.to_string().contains("term-login"), "{err}");
  tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  assert!(!auth_log.exists());
  // A plain method still goes over the wire
  s.authenticate(Some("fake.login")).await.unwrap();
  assert_eq!(std::fs::read_to_string(&auth_log).unwrap(), "fake.login\n");
  s.retry().await.unwrap();
  assert_eq!(view(&s)["status"], "ready");
}

// Devin's case: credentials only ever come from the account layer, so the capability is not advertised and the
// agent never offers a terminal login (a `devin acp --login` would write a login the ACP process ignores)
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_opts_out_of_terminal_auth_is_not_offered_terminal_methods() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-term-").tempdir().unwrap();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_TERMINAL_AUTH": dir.path().join("auth.log") }, "terminalAuth": false }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  assert_eq!(view(&s)["status"], "auth_required");
  assert_eq!(view(&s)["authMethods"].as_array().unwrap().iter().map(|m| m["id"].clone()).collect::<Vec<_>>(), [json!("fake.login")]);
}

#[tokio::test(flavor = "multi_thread")]
async fn per_prompt_token_usage_lands_on_the_agent_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "usage-devin").await;
  expect_match(&last_turn(&view(&s))["usage"], json!({ "input": 100, "output": 20, "cachedRead": 64, "requestId": "req-devin-1", "context": { "used": 5000, "size": 100_000 } }));
  prompt(&s, "usage-grok").await;
  expect_match(&last_turn(&view(&s))["usage"], json!({ "input": 38_140, "output": 20, "modelCalls": 2, "model": "grok-4.6", "requestId": "req-grok-1" }));
}

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

fn reply_text(turn: &Value) -> String {
  turn["blocks"].as_array().unwrap().iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].as_str().unwrap().to_owned()).collect()
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
  assert_eq!(vw["turns"].as_array().unwrap().len(), 4);
  expect_match(&vw["turns"][0], json!({ "role": "user", "text": "earlier" }));
  expect_match(&vw["turns"][1], json!({ "role": "agent" }));
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "now", "edited": true }));
  // The fake agent echoes non-text blocks: the embedded history resource and the 'earlier' turn inside its JSON
  let text = reply_text(&vw["turns"][3]);
  assert!(text.contains("resource:acpira://history/") && text.contains("earlier"), "{text}");
  assert!(!s.to_record().history_pending);
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
  let text = reply_text(&last_turn(&view(&s)));
  assert!(text.contains("resource:acpira://history/"));
  assert!(text.contains("26 earlier turns were omitted"), "{}", &text[..text.len().min(400)]);
  assert!(text.contains("recent"));
  assert!(!text.contains("old-big"));
  assert!(notes.lock().unwrap().iter().any(|n| n.contains("26")));
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
  assert_eq!(vw["turns"].as_array().unwrap().len(), 4);
  expect_match(&vw["turns"][2], json!({ "role": "user", "text": "now", "edited": true }));
  assert!(reply_text(&vw["turns"][3]).contains("resource:acpira://history/"));
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

/// A harness whose fake agent keeps native sessions on disk, so a record reopened in a new session resumes its peer
fn native_harness(fake: &FakeAgent, env: Value) -> (Harness, tempfile::TempDir) {
  let dir = tempfile::tempdir().unwrap();
  let mut env = env;
  env["FAKE_SESSION_DIR"] = json!(dir.path());
  (Harness::new(fake, json!({ "env": env })), dir)
}

/// 'earlier-context' answered, then the UI-only history grown by one huge text block (the TS suites pushed it onto the
/// live transcript); the session is reopened on that record and resumes the same native session
async fn with_ui_history(h: &Harness, markdown: String) -> (Disposing, Value) {
  let first = started(h, "/tmp").await;
  prompt(&first, "earlier-context").await;
  grow_history(h, &first, markdown).await
}

/// Reopens `first`'s record with its turn 1 grown by a huge UI-only block
async fn grow_history(h: &Harness, first: &Arc<AcpSession>, markdown: String) -> (Disposing, Value) {
  let mut record = first.to_record();
  first.dispose();
  if let Some(Turn::Agent(a)) = record.turns.get_mut(1) {
    a.blocks.push(serde_json::from_value(json!({ "type": "text", "markdown": markdown })).unwrap());
  }
  let earlier = v(&record.turns[1]);
  (reopened(h, record).await, earlier)
}

fn wire_prompt(turn: &Value) -> Value {
  serde_json::from_str::<Value>(&reply_text(turn)).expect("the fake echoes the prompt as JSON")
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_empty_cancelled_turn_resends_natively_even_with_multi_megabyte_history() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let (s, _) = with_ui_history(&h, "old output ".repeat(400_000)).await;
  prompt(&s, "cancel-empty-once").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "cancelled", "blocks": [] }));
  let peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 2, "cancel-empty-once")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  expect_absent(&view(&s)["turns"][2], "edited");
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_historical_edit_continues_without_replacing_the_native_session_or_history() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let (s, _) = with_ui_history(&h, "archived output ".repeat(300_000)).await;
  prompt(&s, "original").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(json!(turns[..turns.len() - 2]), before);
  assert_eq!(turns.len(), 6);
  expect_absent(&turns[4], "edited");
  expect_eq(&wire_prompt(&turns[5])["prompt"], json!([{ "type": "text", "text": "inspect-history" }]));
  prompt(&s, "ordinary follow-up").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  assert_eq!(s.to_record().acp_session_id, peer);
}

#[tokio::test(flavor = "multi_thread")]
async fn resending_an_unchanged_message_after_empty_failures_keeps_native_compacted_context() {
  let fake = fake_or_skip!();
  for attempts in [1, 2] {
    let (h, _native) = native_harness(&fake, json!({}));
    // The UI retains old tool output even after native compaction. It must never be injected into an unchanged failed-message retry
    let (s, earlier) = with_ui_history(&h, "archived output ".repeat(250_000)).await;
    let text = if attempts == 2 { "fail-twice" } else { "please fail" };
    for _ in 0..attempts {
      prompt(&s, text).await;
    }
    expect_match(last_turn(&view(&s)), json!({ "stop": "error", "blocks": [] }));
    let native = s.to_record().acp_session_id;
    s.edit_turn(history_edit(&s, 2, text)).await.unwrap();
    until(|| !s.is_running(), 5000).await;
    assert_eq!(s.to_record().acp_session_id, native, "attempts={attempts}");
    let vw = view(&s);
    assert_eq!(vw["turns"].as_array().unwrap().len(), 4);
    assert_eq!(vw["turns"][1], earlier);
    expect_absent(&vw["turns"][2], "edited");
    expect_match(&vw["turns"][3], json!({ "stop": "end_turn" }));
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_starts_a_fresh_peer_with_only_earlier_context_and_applies_mode_and_effort_first() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "replaced-original").await;
  prompt(&s, "discarded-future").await;
  let old_peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 2, "inspect-history");
  edit.settings.mode_id = Some("plan".into());
  edit.settings.config.insert("effort".into(), "low".into());
  edit.settings.config.insert("model".into(), "m2".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  assert_ne!(s.to_record().acp_session_id, old_peer);
  let vw = view(&s);
  assert_eq!(vw["turns"].as_array().unwrap().len(), 4);
  expect_match(&vw["turns"][0], json!({ "text": "earlier-context", "settings": { "config": { "effort": "high" } } }));
  expect_match(&vw["turns"][2], json!({ "text": "inspect-history", "settings": { "modeId": "plan", "config": { "effort": "low", "model": "m2" } } }));
  let reply = vw["turns"][3].to_string();
  assert!(reply.contains("earlier-context") && !reply.contains("replaced-original") && !reply.contains("discarded-future"), "{reply}");
  assert!(reply.contains("low") && reply.contains("m2") && reply.contains("plan"));
}

// The editor's picker switches models locally, so its settings still carry the previous model's Fast and effort
// (Devin: GPT-6 Luna Fast → SWE-2, which has no `speed` control and no low effort); the edit must still send
#[tokio::test(flavor = "multi_thread")]
async fn an_edit_lets_the_agent_settle_dependent_controls_the_new_model_no_longer_offers() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_SPEED": "1" } }));
  let s = started(&h, "/tmp").await;
  s.set_config("speed".into(), "fast".into()).await.unwrap();
  s.set_config("effort".into(), "low".into()).await.unwrap();
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  let mut edit = history_edit(&s, 2, "inspect-history");
  expect_match(&edit.settings.config, json!({ "model": "m1", "speed": "fast", "effort": "low" }));
  edit.settings.config.insert("model".into(), "m2".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  let vw = view(&s);
  assert_eq!(option_value(&vw, "model"), "m2");
  assert!(vw["controls"]["options"].as_array().unwrap().iter().all(|o| o["id"] != "speed"));
  assert_eq!(option_value(&vw, "effort"), "high");
  expect_match(&vw["turns"][2], json!({ "text": "inspect-history", "edited": true, "settings": { "config": { "model": "m2", "effort": "high" } } }));
  assert!(vw["turns"][3].to_string().contains("m2"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_still_refuses_a_model_the_agent_no_longer_offers() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  let mut edit = history_edit(&s, 2, "inspect-history");
  edit.settings.config.insert("model".into(), "gone".into());
  let err = s.edit_turn(edit).await.expect_err("refused");
  assert!(err.to_string().contains("model"), "{err}");
  assert_eq!(turn_count(&s), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_ignores_the_rebuilt_peers_title_so_a_renamed_session_keeps_its_title() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  s.rename("Kept title");
  let old_peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running(), 5000).await;
  // The edit rebuilt the context through session/new (fresh peer), and its 'Fake title' update was ignored
  assert_ne!(s.to_record().acp_session_id, old_peer);
  assert_eq!(view(&s)["title"], "Kept title");
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_a_failed_edited_prompt_rebuilds_context_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "earlier-context").await;
  prompt(&s, "original").await;
  s.edit_turn(history_edit(&s, 2, "please fail")).await.unwrap();
  until(|| !s.is_running() && last_turn(&view(&s))["role"] == "agent" && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  expect_match(&view(&s)["turns"][3], json!({ "stop": "error" }));
  let failed_peer = s.to_record().acp_session_id;
  s.retry_turn().await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_ne!(s.to_record().acp_session_id, failed_peer);
  let vw = view(&s);
  assert_eq!(vw["turns"].as_array().unwrap().len(), 4);
  expect_match(&vw["turns"][0], json!({ "text": "earlier-context" }));
  expect_match(&vw["turns"][2], json!({ "text": "please fail", "edited": true }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_keeps_retained_image_bytes_removes_selected_attachments_and_adds_new_ones() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([
    { "kind": "image", "name": "old.png", "mimeType": "image/png", "data": "aGVsbG8=" },
    { "kind": "text", "name": "remove.txt", "text": "removed attachment content" },
  ])), false, None, None).await;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.retained_attachments = vec![0];
  edit.attachments = drafts(json!([{ "kind": "text", "name": "new.txt", "text": "new attachment content" }]));
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  let vw = view(&s);
  assert_eq!(vw["turns"].as_array().unwrap().len(), 2);
  expect_match(&vw["turns"][0], json!({ "attachments": [{ "kind": "image", "name": "old.png" }, { "kind": "text", "name": "new.txt" }] }));
  let reply = vw["turns"][1].to_string();
  assert!(reply.contains("aGVsbG8=") && reply.contains("new attachment content") && !reply.contains("removed attachment content"), "{reply}");
}

#[tokio::test(flavor = "multi_thread")]
async fn under_strict_prompt_capabilities_a_kept_text_attachment_resends_as_text_in_the_rebuilt_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_PROMPT_CAPS": "strict" } }));
  let s = started(&h, "/tmp").await;
  s.prompt("earlier-context".into(), drafts(json!([{ "kind": "text", "name": "keep.txt", "text": "kept payload" }])), false, None, None).await;
  prompt(&s, "original").await;
  // Editing turn 2 rebuilds the context through historyContext: turn 0's kept attachment must be re-encoded with the strict caps
  s.edit_turn(history_edit(&s, 2, "inspect-history")).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  let wire = wire_prompt(&last_turn(&view(&s)));
  assert!(wire["prompt"].as_array().unwrap().iter().all(|b| b["type"] == "text"));
  assert!(wire["prompt"].to_string().contains("[Attachment: keep.txt]"));
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_edits_or_unavailable_settings_preserve_the_transcript_and_peer() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODELS": "unavailable" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "original").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut stale = history_edit(&s, 0, "inspect-history");
  stale.turn_count += 2;
  assert!(s.edit_turn(stale).await.is_err());
  let mut invalid = history_edit(&s, 0, "inspect-history");
  invalid.settings.config.insert("model".into(), "unavailable".into());
  assert!(s.edit_turn(invalid).await.is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert_eq!(option_value(&view(&s), "model"), "m1");
  assert!(!s.is_running());
}

fn remove_blobs(h: &Harness, sid: &str) {
  for e in std::fs::read_dir(h.dir.path().join("sessions").join(sid)).unwrap() {
    std::fs::remove_file(e.unwrap().path()).unwrap();
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_retained_blob_does_not_replace_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([{ "kind": "text", "name": "lost.txt", "text": "payload" }])), false, None, None).await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  remove_blobs(&h, &s.id);
  assert!(s.edit_turn(history_edit(&s, 0, "inspect-history")).await.is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert!(!s.is_running());
}

/// A retained attachment whose blob is a FIFO: reading it blocks until released (the TS readBlob gate). It has its own name, so
/// the edit re-staging the same bytes under their content hash never opens the FIFO for writing
#[cfg(unix)]
struct BlobGate {
  path: std::path::PathBuf,
}

#[cfg(unix)]
impl BlobGate {
  const NAME: &'static str = "gate-blob.txt";

  fn create(h: &Harness, sid: &str) -> BlobGate {
    let path = h.dir.path().join("sessions").join(sid).join(Self::NAME);
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    BlobGate { path }
  }
  fn release(&self) {
    let path = self.path.clone();
    std::thread::spawn(move || std::fs::write(path, b"payload"));
  }
}

#[cfg(unix)]
async fn rejects_double_submission_then_cancels(intent: Option<&str>) {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let first = started(&h, "/tmp").await;
  first.prompt("original".into(), drafts(json!([{ "kind": "text", "name": "wait.txt", "text": "payload" }])), false, None, None).await;
  let mut record = first.to_record();
  first.dispose();
  if let Some(Turn::User(u)) = record.turns.get_mut(0) {
    let mut a = v(&u.attachments.as_ref().unwrap()[0]);
    a["blob"] = json!(BlobGate::NAME);
    u.attachments = Some(vec![serde_json::from_value(a).unwrap()]);
  }
  let gate = BlobGate::create(&h, &record.id);
  let s = reopened(&h, record).await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.intent = intent.map(|i| serde_json::from_value(json!(i)).unwrap());
  let pending = claimed({
    let (s, edit) = (s.0.clone(), edit.clone());
    async move { s.edit_turn(edit).await }
  });
  assert!(s.edit_turn(edit).await.is_err());
  // The cancel is requested while the blob read is still blocked; it completes once staging lets go
  let cancel = claimed({
    let s = s.0.clone();
    async move { s.cancel().await }
  });
  gate.release();
  cancel.await.unwrap();
  assert!(pending.await.unwrap().is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert!(!s.is_running());
  if intent.is_some() {
    prompt(&s, "ordinary follow-up").await;
    expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_double_submission_is_rejected_and_a_cancel_lands_before_history_is_replaced() {
  rejects_double_submission_then_cancels(None).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_continue_cancelled_while_attachments_stage_still_accepts_a_normal_prompt() {
  rejects_double_submission_then_cancels(Some("continue")).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_edit_right_after_compaction_goes_out_as_one_request() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let first = started(&h, "/tmp").await;
  prompt(&first, "big").await;
  let (s, _) = grow_history(&h, &first, "archived output ".repeat(300_000)).await;
  s.compact(false).await.unwrap();
  prompt(&s, "cancel-empty-once").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  s.edit_turn(history_edit(&s, 4, "inspect-history")).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null() && turn_count(&s) > before.as_array().unwrap().len(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(json!(turns[..turns.len() - 2]), before);
  expect_eq(&wire_prompt(&turns[turns.len() - 1])["prompt"], json!([{ "type": "text", "text": "inspect-history" }]));
  expect_absent(&turns[turns.len() - 2], "edited");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expanded_payload_over_the_cap_falls_back_to_one_native_prompt() {
  let fake = fake_or_skip!();
  for historical in [false, true] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    let image = json!({ "kind": "image", "name": "large.png", "mimeType": "image/png", "data": "a".repeat(400_000) });
    s.prompt("earlier".into(), if historical { drafts(json!([image])) } else { vec![] }, false, None, None).await;
    prompt(&s, "original").await;
    let before = view(&s)["turns"].clone();
    let peer = s.to_record().acp_session_id;
    let mut edit = history_edit(&s, 2, "inspect-history");
    if !historical {
      edit.attachments = drafts(json!([image]));
    }
    s.edit_turn(edit).await.unwrap();
    until(|| !s.is_running() && turn_count(&s) == 6 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
    assert_eq!(s.to_record().acp_session_id, peer, "historical={historical}");
    let vw = view(&s);
    let turns = vw["turns"].as_array().unwrap();
    assert_eq!(json!(turns[..4]), before);
    expect_absent(&turns[4], "edited");
    let expected = if historical {
      json!([{ "type": "text", "text": "inspect-history" }])
    } else {
      json!([{ "type": "text", "text": "inspect-history" }, { "type": "image", "mimeType": "image/png", "data": "a".repeat(400_000) }])
    };
    expect_eq(&wire_prompt(&turns[5])["prompt"], expected);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unchanged_failed_message_retries_natively_even_when_the_edit_changed_settings() {
  let fake = fake_or_skip!();
  let (h, _native) = native_harness(&fake, json!({}));
  let first = started(&h, "/tmp").await;
  prompt(&first, "big").await;
  let (s, _) = grow_history(&h, &first, "archived output ".repeat(300_000)).await;
  s.compact(false).await.unwrap();
  prompt(&s, "cancel-empty-once").await;
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 4, "cancel-empty-once");
  edit.settings.mode_id = Some("plan".into());
  edit.settings.config.insert("model".into(), "m2".into());
  edit.settings.config.insert("effort".into(), "low".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 6 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  expect_match(&vw["turns"][4], json!({ "text": "cancel-empty-once" }));
  expect_absent(&vw["turns"][4], "edited");
  assert_eq!(vw["controls"]["modeId"], "plan");
  assert_eq!(option_value(&vw, "model"), "m2");
  assert_eq!(option_value(&vw, "effort"), "low");
  prompt(&s, "inspect-history").await;
  let wire = wire_prompt(&last_turn(&view(&s)));
  assert_eq!(wire["mode"], "plan");
  expect_match(&wire["config"], json!({ "model": "m2", "effort": "low" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continue_from_an_earlier_turn_keeps_and_adds_attachments_and_leaves_later_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([
    { "kind": "image", "name": "old.png", "mimeType": "image/png", "data": "aGVsbG8=" },
    { "kind": "text", "name": "remove.txt", "text": "removed attachment content" },
  ])), false, None, None).await;
  s.prompt("later".into(), drafts(json!([{ "kind": "text", "name": "unrelated.txt", "text": "unrelated payload" }])), false, None, None).await;
  let lost = view(&s)["turns"][2]["attachments"][0]["blob"].as_str().unwrap().to_owned();
  std::fs::remove_file(h.dir.path().join("sessions").join(&s.id).join(&lost)).unwrap();
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.retained_attachments = vec![0];
  edit.attachments = drafts(json!([{ "kind": "text", "name": "new.txt", "text": "new attachment content" }]));
  edit.settings.mode_id = Some("plan".into());
  edit.settings.config.insert("model".into(), "m2".into());
  edit.settings.config.insert("effort".into(), "low".into());
  edit.intent = Some(serde_json::from_value(json!("continue")).unwrap());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && turn_count(&s) == 6 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_eq!(s.to_record().acp_session_id, peer);
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(json!(turns[..4]), before);
  expect_absent(&turns[4], "edited");
  expect_match(&turns[4]["attachments"], json!([{ "kind": "image", "name": "old.png" }, { "kind": "text", "name": "new.txt" }]));
  let wire = wire_prompt(&turns[5]);
  let p = wire["prompt"].as_array().unwrap();
  assert_eq!(p.len(), 3);
  expect_eq(&p[0], json!({ "type": "text", "text": "inspect-history" }));
  expect_match(&p[1], json!({ "type": "image", "mimeType": "image/png", "data": "aGVsbG8=" }));
  expect_match(&p[2], json!({ "type": "resource", "resource": { "text": "new attachment content" } }));
  let text = wire["prompt"].to_string();
  assert!(!text.contains("removed attachment content") && !text.contains("Conversation before") && !text.contains("unrelated payload"));
  assert_eq!(wire["mode"], "plan");
  expect_match(&wire["config"], json!({ "model": "m2", "effort": "low" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_native_retry_over_a_context_length_failure_is_still_refused() {
  let fake = fake_or_skip!();
  for changed in [false, true] {
    let h = Harness::new(&fake, json!({}));
    let s = started(&h, "/tmp").await;
    prompt(&s, "context-too-long").await;
    expect_match(last_turn(&view(&s)), json!({ "stop": "error" }));
    let before = view(&s)["turns"].clone();
    let peer = s.to_record().acp_session_id;
    let mut edit = history_edit(&s, 0, "context-too-long");
    if changed {
      edit.settings.mode_id = Some("plan".into());
      edit.settings.config.insert("model".into(), "m2".into());
    }
    let err = s.edit_turn(edit).await.expect_err("refused");
    assert!(err.to_string().to_lowercase().contains("compact") || err.to_string().contains("压缩"), "{err}");
    assert_eq!(view(&s)["turns"], before);
    assert_eq!(s.to_record().acp_session_id, peer);
    assert!(!s.is_running());
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continue_with_an_unavailable_selection_touches_neither_the_session_nor_the_controls() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_MODELS": "unavailable" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "original").await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.settings.config.insert("model".into(), "unavailable".into());
  edit.intent = Some(serde_json::from_value(json!("continue")).unwrap());
  assert!(s.edit_turn(edit).await.is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert_eq!(option_value(&view(&s), "model"), "m1");
  assert!(!s.is_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continue_with_a_missing_blob_or_a_stale_or_malformed_request_is_rejected() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  s.prompt("original".into(), drafts(json!([{ "kind": "text", "name": "lost.txt", "text": "payload" }])), false, None, None).await;
  let before = view(&s)["turns"].clone();
  let peer = s.to_record().acp_session_id;
  let lost = view(&s)["turns"][0]["attachments"][0]["blob"].as_str().unwrap().to_owned();
  let lost_path = h.dir.path().join("sessions").join(&s.id).join(&lost);
  std::fs::remove_file(&lost_path).unwrap();
  let cont = || {
    let mut e = history_edit(&s, 0, "inspect-history");
    e.intent = Some(serde_json::from_value(json!("continue")).unwrap());
    e
  };
  assert!(s.edit_turn(cont()).await.is_err());
  std::fs::write(&lost_path, "payload").unwrap();
  let mut stale = cont();
  stale.turn_count += 2;
  assert!(s.edit_turn(stale).await.is_err());
  let mut wrong_session = cont();
  wrong_session.session_id = "other".into();
  assert!(s.edit_turn(wrong_session).await.is_err());
  let mut wrong_turn = cont();
  wrong_turn.turn_id = Some("other".into());
  assert!(s.edit_turn(wrong_turn).await.is_err());
  let mut dup = cont();
  dup.retained_attachments = vec![0, 0];
  assert!(s.edit_turn(dup).await.is_err());
  // An unknown intent never gets past decoding
  let mut bogus = serde_json::to_value(cont()).unwrap();
  bogus["intent"] = json!("bogus");
  assert!(serde_json::from_value::<acpira_shared::protocol::EditTurnRequest>(bogus).is_err());
  assert_eq!(view(&s)["turns"], before);
  assert_eq!(s.to_record().acp_session_id, peer);
  assert!(!s.is_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn native_usage_reported_while_applying_editor_settings_is_retained() {
  let fake = fake_or_skip!();
  for rejected in [false, true] {
    // The rejected effort is offered and refused on the wire; an effort the model does not offer would just yield
    let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_USAGE": "1", "FAKE_EFFORTS": "unavailable" } }));
    let s = started(&h, "/tmp").await;
    prompt(&s, "big").await;
    let peer = s.to_record().acp_session_id;
    // usage.context on the last agent turn tracks every usage_update, including ones the settings apply triggers — the
    // equality check is about the rejected edit not rewriting the transcript, so usage snapshots are left out of it
    let sans_usage = |s: &AcpSession| {
      let mut t = view(s)["turns"].clone();
      for turn in t.as_array_mut().unwrap() {
        turn.as_object_mut().unwrap().remove("usage");
      }
      t
    };
    let before = sans_usage(&s);
    let mut edit = history_edit(&s, 0, "inspect-history");
    edit.intent = Some(serde_json::from_value(json!("continue")).unwrap());
    edit.settings.config.insert("model".into(), "m2".into());
    if rejected {
      edit.settings.config.insert("effort".into(), "unavailable".into());
      assert!(s.edit_turn(edit).await.is_err());
    } else {
      s.edit_turn(edit).await.unwrap();
      until(|| !s.is_running() && turn_count(&s) == 4 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
    }
    expect_match(&view(&s)["usage"], json!({ "used": 24_000, "size": 200_000 }));
    assert_eq!(option_value(&view(&s), "model"), "m2", "rejected={rejected}");
    assert_eq!(s.to_record().acp_session_id, peer);
    if rejected {
      assert_eq!(sans_usage(&s), before);
    }
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_reported_on_the_fresh_peer_during_a_rebuild_is_ignored() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_USAGE": "1" } }));
  let s = started(&h, "/tmp").await;
  prompt(&s, "original").await;
  let peer = s.to_record().acp_session_id;
  let mut edit = history_edit(&s, 0, "inspect-history");
  edit.settings.config.insert("model".into(), "m2".into());
  s.edit_turn(edit).await.unwrap();
  until(|| !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  assert_ne!(s.to_record().acp_session_id, peer);
  expect_absent(view(&s), "usage");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_oversized_edit_after_native_style_compaction_keeps_context_and_applies_the_mode() {
  let fake = fake_or_skip!();
  for (agent, mode_id) in [("grok", "plan"), ("grok", "yolo"), ("kimi", "plan")] {
    let cwd = tempfile::Builder::new().prefix(if agent == "grok" { "acpira-grok-no-modes-" } else { "acpira-kimi-edit-" }).tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let extra = if agent == "grok" {
      json!({ "modes": syn_modes(), "env": { "FAKE_GROK_USAGE": "context", "FAKE_SESSION_DIR": native.path() } })
    } else {
      json!({ "env": { "FAKE_COMPACTION": "kimi", "FAKE_SESSION_DIR": native.path() } })
    };
    let h = Harness::for_agent(&fake, agent, extra);
    let first = started(&h, cwd.path().to_str().unwrap()).await;
    prompt(&first, "big").await;
    until(|| view(&first)["commands"].as_array().is_some_and(|c| c.iter().any(|c| c["name"] == "compact")), 5000).await;
    let compact = claimed({
      let s = first.0.clone();
      async move { s.compact(false).await }
    });
    if agent == "kimi" {
      until(|| h.logs().iter().any(|l| l.contains("waiting for compaction completion")), 5000).await;
      assert!(first.is_running());
      first.set_config("effort".into(), "high".into()).await.ok();
    }
    compact.await.unwrap().unwrap();
    assert!(!first.is_running());
    let (s, _) = grow_history(&h, &first, "archived output ".repeat(100_000)).await;
    s.prompt("original".into(), drafts(json!([{ "kind": "image", "name": "kept.png", "mimeType": "image/png", "data": "aGVsbG8=" }])), false, None, None).await;
    let before = view(&s)["turns"].clone();
    let peer = s.to_record().acp_session_id;
    let mut edit = history_edit(&s, 4, "inspect-history");
    edit.settings.mode_id = Some(mode_id.into());
    edit.settings.config.insert("model".into(), "m2".into());
    edit.settings.config.insert("effort".into(), "low".into());
    edit.attachments = drafts(json!([{ "kind": "text", "name": "new.txt", "text": "new attachment content" }]));
    s.edit_turn(edit).await.unwrap();
    until(|| !s.is_running() && turn_count(&s) == 8 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
    assert_eq!(s.to_record().acp_session_id, peer, "{agent} {mode_id}");
    let vw = view(&s);
    let turns = vw["turns"].as_array().unwrap();
    assert_eq!(json!(turns[..6]), before);
    assert_eq!(vw["controls"]["modeId"], mode_id);
    let wire = wire_prompt(&turns[7]);
    assert_eq!(wire["mode"], if mode_id == "yolo" { "default" } else { mode_id }, "{agent} {mode_id}");
    expect_match(&wire["config"], json!({ "model": "m2", "effort": "low" }));
    let p = wire["prompt"].as_array().unwrap();
    assert_eq!(p.len(), 3);
    expect_eq(&p[0], json!({ "type": "text", "text": "inspect-history" }));
    expect_match(&p[1], json!({ "type": "image", "data": "aGVsbG8=" }));
    expect_match(&p[2], json!({ "type": "resource", "resource": { "text": "new attachment content" } }));
    expect_absent(&turns[6], "edited");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn failure_retry_upserts_the_warning_revision_and_the_error_settles_the_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "failure-retry").await;
  let turn = last_turn(&view(&s));
  expect_match(&turn, json!({ "stop": "error", "error": { "kind": "limit", "retryable": true, "failureId": "turn-1:error", "actions": ["retry"] } }));
  assert_eq!(turn["error"]["message"], "Rate limit exceeded\nTry again in a minute");
  // warning rev 1 and the error rev 2 share one notice row — the latest revision won
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
  until(|| turn_count(&s) == 1, 5000).await;
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

#[tokio::test(flavor = "multi_thread")]
async fn a_live_async_task_restores_with_observation_unknown_and_its_last_state() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let original = Disposing(h.session("/tmp"));
  let mut record = original.to_record();
  record.updated_at = iso_of_ms(5000);
  record.turns = turns(json!([
    { "role": "user", "text": "work" },
    { "role": "agent", "startedAt": 1000, "blocks": [
      { "type": "tool_call", "id": "bg", "kind": "execute", "verb": "Run", "status": "in_progress", "startedAt": 2000, "background": true,
        "asyncTask": { "id": "t1", "state": "running", "canStop": true, "stopRequested": true, "name": "sleep 120" } },
    ] },
  ]));
  let restored = Disposing(AcpSession::new(record, h.deps.clone()));
  let row = view(&restored)["turns"][1]["blocks"][0].clone();
  expect_match(&row, json!({ "status": "cancelled", "observation": "unknown", "background": true }));
  expect_match(&row["asyncTask"], json!({ "id": "t1", "state": "running", "canStop": false, "name": "sleep 120" }));
  expect_absent(&row["asyncTask"], "stopRequested");
}
