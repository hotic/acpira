//! Permission cards and synthesized YOLO auto-approval

use super::*;

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
  // The page's view leaves the edit's file texts out; the full source is fetched on demand by tool call id + diff index
  let acpira_shared::protocol::HostMsg::Session { session, .. } = s.view_msg() else { panic!("not a session view") };
  let page = v(&session);
  let sent = agent_blocks(&page).into_iter().find(|b| b["id"] == "tc2").unwrap();
  expect_eq(&sent["content"]["source"], json!({ "path": "/repo/a.ts", "omitted": true }));
  expect_eq(s.diff_source("tc2", 0).unwrap(), json!({ "path": "/repo/a.ts", "oldText": "a\nb\nc\n", "newText": "a\nB\nc\nd\n" }));
  assert!(s.diff_source("tc2", 1).is_none() && s.diff_source("tc1", 0).is_none());
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

// Several allow_once options and no allow_always are answers, not an approval ladder: yolo leaves the card for a person
#[tokio::test(flavor = "multi_thread")]
async fn synthesized_yolo_leaves_an_ambiguous_permission_to_a_person() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-no-modes").unwrap();
  let h = Harness::new(&fake, json!({ "modes": syn_modes() }));
  let s = started(&h, "/tmp/acpira-no-modes").await;
  s.set_mode("yolo".into()).await.unwrap();
  let p = spawn_prompt(&s, "perm-choices");
  let perm = wait_block(&s, "permission").await;
  assert_eq!(perm["options"].as_array().unwrap().iter().map(|o| o["id"].clone()).collect::<Vec<_>>(), [json!("staging"), json!("production"), json!("no")]);
  s.resolve_permission(perm["id"].as_str().unwrap(), "production");
  p.await.unwrap();
  expect_match(agent_blocks(&view(&s)).last().unwrap(), json!({ "type": "text", "markdown": "picked production" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_into_synthesized_yolo_keeps_an_ambiguous_card_open() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-no-modes").unwrap();
  let h = Harness::new(&fake, json!({ "modes": syn_modes() }));
  let s = started(&h, "/tmp/acpira-no-modes").await;
  let p = spawn_prompt(&s, "perm-choices");
  let perm = wait_block(&s, "permission").await;
  s.set_mode("yolo".into()).await.unwrap();
  assert_eq!(view(&s)["running"], true);
  assert!(agent_blocks(&view(&s)).iter().any(|b| b["type"] == "permission" && b["id"] == perm["id"]));
  s.resolve_permission(perm["id"].as_str().unwrap(), "staging");
  p.await.unwrap();
  expect_match(agent_blocks(&view(&s)).last().unwrap(), json!({ "type": "text", "markdown": "picked staging" }));
}

// Antigravity's questions never ride yolo either: the question card opens and waits
#[tokio::test(flavor = "multi_thread")]
async fn an_antigravity_question_still_asks_under_synthesized_yolo() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-no-modes").unwrap();
  let h = Harness::for_agent(&fake, "antigravity", json!({ "modes": syn_modes() }));
  let s = started(&h, "/tmp/acpira-no-modes").await;
  s.set_mode("yolo".into()).await.unwrap();
  let p = spawn_prompt(&s, "ask-agy");
  let q = wait_block(&s, "question").await;
  s.answer_questions(q["id"].as_str().unwrap(), &serde_json::from_value(json!({ "interaction_1a2b3c4d": "blue" })).unwrap(), false);
  p.await.unwrap();
  let text: String = agent_blocks(&view(&s)).iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].as_str().unwrap().to_owned()).collect();
  assert!(text.contains(r#""optionId":"blue""#), "{text}");
}
