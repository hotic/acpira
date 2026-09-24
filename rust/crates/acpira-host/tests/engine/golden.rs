//! test/fixtures/engine-*.json: what the engine produces for recorded updates and diffs, the input the webview suites render.
//! ACPIRA_UPDATE_FIXTURES=1 rewrites the fixtures from this engine after an intended change

use std::path::PathBuf;

use serde_json::{Value, json};

use acpira_host::acp::attachments::{PromptCaps, prepare_prompt, restore_drafts};
use acpira_host::acp::diff::diff_lines;
use acpira_host::acp::normalize::{NormalizeState, activity_of, apply_update};
use acpira_host::util::mock_now;
use acpira_shared::transcript::{Draft, Turn};

use crate::support::{expect_eq, v};

fn fixture(name: &str) -> PathBuf {
  PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../test/fixtures").join(name)
}

fn read(name: &str) -> Value {
  serde_json::from_str(&std::fs::read_to_string(fixture(name)).unwrap()).unwrap()
}

fn update_mode() -> bool {
  std::env::var("ACPIRA_UPDATE_FIXTURES").is_ok_and(|x| x == "1")
}

/// One-space indentation, as the fixtures were first written
fn write(name: &str, value: &Value) {
  use serde::Serialize;
  let mut out = Vec::new();
  let mut ser = serde_json::Serializer::with_formatter(&mut out, serde_json::ser::PrettyFormatter::with_indent(b" "));
  value.serialize(&mut ser).unwrap();
  out.push(b'\n');
  std::fs::write(fixture(name), out).unwrap();
}

#[test]
fn diff_lines_match_the_recorded_fixtures() {
  let mut cases = read("engine-diff.json");
  for case in cases.as_array_mut().unwrap() {
    let lines = v(diff_lines(case["oldText"].as_str().unwrap(), case["newText"].as_str().unwrap()));
    if update_mode() {
      case["lines"] = lines;
    } else {
      assert_eq!(lines, case["lines"], "{}", case["name"]);
    }
  }
  if update_mode() {
    write("engine-diff.json", &cases);
  }
}

#[test]
fn normalized_turns_match_the_recorded_fixtures() {
  let mut cases = read("engine-normalize.json");
  for case in cases.as_array_mut().unwrap() {
    let mut s = NormalizeState::new(vec![]);
    let name = case["name"].clone();
    for step in case["steps"].as_array_mut().unwrap() {
      mock_now(step["at"].as_i64());
      apply_update(&mut s, &step["update"]);
      let turns = v(&s.turns);
      if update_mode() {
        step["turns"] = turns;
      } else {
        assert_eq!(turns, step["turns"], "{name}");
      }
    }
  }
  mock_now(None);
  if update_mode() {
    write("engine-normalize.json", &cases);
  }
}

#[test]
fn diff_positions_survive_omitted_context_and_inserted_lines() {
  let old: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
  let mut next = old.clone();
  next.splice(10..11, ["replacement".to_owned(), "inserted".to_owned()]);
  let lines = v(diff_lines(&old.join("\n"), &next.join("\n")));
  let find = |t: &str| lines.as_array().unwrap().iter().find(|l| l["text"] == t).cloned().unwrap();
  assert_eq!(find("-line 11")["oldLine"], 11);
  assert_eq!(find("+replacement")["newLine"], 11);
  assert_eq!(find("+inserted")["newLine"], 12);
  assert_eq!((find(" line 12")["oldLine"].clone(), find(" line 12")["newLine"].clone()), (json!(12), json!(13)));
  assert_eq!(lines[0]["kind"], "hunk");
  assert_eq!(v(diff_lines("", "a\nb")).as_array().unwrap().iter().map(|l| l["newLine"].clone()).collect::<Vec<_>>(), [json!(1), json!(2)]);
  let bounded = v(diff_lines(&"old\n".repeat(450), &"new\n".repeat(450)));
  let dels: Vec<i64> = bounded.as_array().unwrap().iter().filter(|l| l["kind"] == "del").map(|l| l["oldLine"].as_i64().unwrap()).collect();
  assert_eq!(dels, (1..=40).collect::<Vec<_>>());
  // No manufactured trailing blank line, and nothing for two empty sides
  assert_eq!(v(diff_lines("", "a\r\nb\r\n")).as_array().unwrap().iter().map(|l| l["text"].clone()).collect::<Vec<_>>(), [json!("+a"), json!("+b")]);
  assert!(diff_lines("", "").is_empty());
}

fn turns(j: Value) -> Vec<Turn> {
  serde_json::from_value(j).unwrap()
}

#[test]
fn a_thought_without_an_end_signal_keeps_the_gap_generic_until_the_reported_tool() {
  mock_now(Some(1000));
  let mut s = NormalizeState::new(vec![]);
  apply_update(&mut s, &json!({ "sessionUpdate": "agent_thought_chunk", "content": { "type": "text", "text": "I will write the file." } }));
  mock_now(Some(38000));
  assert_eq!(activity_of(&s.turns).unwrap().label, "Working");
  // Tool arguments were generated during the silence; their first packet ends it
  apply_update(&mut s, &json!({ "sessionUpdate": "tool_call", "toolCallId": "write", "title": "write", "kind": "edit", "status": "in_progress", "rawInput": { "file_path": "/tmp/sample.ts" } }));
  crate::support::expect_match(&v(&s.turns)[0]["blocks"][0], json!({ "text": "I will write the file.", "streaming": false }));
  assert_eq!(v(activity_of(&s.turns).unwrap())["kind"], "edit");
  mock_now(None);
}

#[test]
fn startup_and_unclassified_gaps_stay_generic_while_reply_activity_is_kept() {
  assert_eq!(activity_of(&[]).unwrap().label, "Working");
  assert_eq!(activity_of(&turns(json!([{ "role": "agent", "blocks": [] }]))).unwrap().label, "Working");
  assert_eq!(activity_of(&turns(json!([{ "role": "agent", "blocks": [{ "type": "text", "markdown": "Done", "streaming": true }] }]))).unwrap().label, "Replying");
}

fn draft(j: Value) -> Draft {
  serde_json::from_value(j).unwrap()
}

fn caps(embedded_context: bool, image: bool, images_regardless: bool) -> Option<PromptCaps> {
  Some(PromptCaps { embedded_context, image, images_regardless })
}

fn store() -> (tempfile::TempDir, std::sync::Arc<acpira_host::store::transcript_store::TranscriptStore>) {
  let dir = tempfile::tempdir().unwrap();
  let s = acpira_host::store::transcript_store::TranscriptStore::new(dir.path().to_path_buf(), std::sync::Arc::new(|_: &str| {}), None);
  (dir, s)
}

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

#[tokio::test]
async fn a_large_paste_stages_as_a_full_resource_and_restores_for_editing() {
  let (dir, blobs) = store();
  let text = "  完整内容\r\n".repeat(500);
  let d = draft(json!({ "kind": "text", "name": "粘贴的文本.txt", "text": text }));
  let result = prepare_prompt("session", "", std::slice::from_ref(&d), &blobs, None).await;
  assert!(result.problems.is_empty());
  let blob = v(&result.attachments)[0]["blob"].as_str().unwrap().to_owned();
  let path = dir.path().join("session").join(&blob).canonicalize().unwrap();
  expect_eq(&result.blocks, json!([{ "type": "resource", "resource": { "uri": format!("file://{}", path.display()), "mimeType": "text/plain", "text": text } }]));
  assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
  assert_eq!(v(restore_drafts("session", &result.attachments, &blobs).await.unwrap()), v([d]));
}

#[tokio::test]
async fn without_embedded_context_dropped_text_goes_as_marked_up_plain_text_and_still_stages() {
  let (_dir, blobs) = store();
  let result = prepare_prompt("session", "hi", &[draft(json!({ "kind": "text", "name": "notes.txt", "text": "contents" }))], &blobs, caps(false, true, false)).await;
  assert!(result.problems.is_empty());
  expect_eq(&result.blocks, json!([{ "type": "text", "text": "hi" }, { "type": "text", "text": "[Attachment: notes.txt]\ncontents\n[End of attachment: notes.txt]" }]));
  let a = v(&result.attachments);
  assert_eq!((a[0]["kind"].clone(), a[0]["name"].clone()), (json!("text"), json!("notes.txt")));
  assert!(a[0]["blob"].is_string());
}

#[tokio::test]
async fn without_the_image_capability_a_pasted_image_becomes_a_problem() {
  let (_dir, blobs) = store();
  let result = prepare_prompt("session", "hi", &[draft(json!({ "kind": "image", "name": "shot.png", "mimeType": "image/png", "data": PNG }))], &blobs, caps(true, false, false)).await;
  expect_eq(&result.blocks, json!([{ "type": "text", "text": "hi" }]));
  assert!(result.attachments.is_empty());
  assert_eq!(result.problems.len(), 1);
  assert!(result.problems[0].contains("shot.png"));
}

#[tokio::test]
async fn images_regardless_overrides_a_false_image_capability() {
  let (_dir, blobs) = store();
  let result = prepare_prompt("session", "hi", &[draft(json!({ "kind": "image", "name": "shot.png", "mimeType": "image/png", "data": PNG }))], &blobs, caps(true, false, true)).await;
  assert!(result.problems.is_empty());
  expect_eq(&result.blocks[1], json!({ "type": "image", "mimeType": "image/png", "data": PNG }));
  crate::support::expect_match(&v(&result.attachments)[0], json!({ "kind": "image", "mimeType": "image/png", "name": "shot.png" }));
}

#[tokio::test]
async fn no_capabilities_at_all_keep_the_historical_behaviour() {
  let (_dir, blobs) = store();
  let result = prepare_prompt("session", "", &[draft(json!({ "kind": "image", "name": "shot.png", "mimeType": "image/png", "data": PNG }))], &blobs, None).await;
  assert_eq!(result.blocks[0]["type"], "image");
}
