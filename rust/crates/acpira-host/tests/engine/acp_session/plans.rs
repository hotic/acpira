//! Plan approval cards and building saved plans

use super::*;

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
    let count = turns_in(&after);
    s.build_plan(&plan_id, None, Some(allow)).await.ok();
    assert_eq!(turn_count(&s), count);
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
  // One of the racing calls may be refused; the state below is what counts
  let _ = (a, b);
  let after = view(&s);
  assert_eq!(turns_in(&after), 4);
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
  assert_eq!(turns_in(&after), 4);
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
