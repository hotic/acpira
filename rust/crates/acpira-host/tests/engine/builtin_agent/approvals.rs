//! Permission cards and approval levels: edits behind cards, live approval changes, Plan mode, plugins insisting on a card

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_waits_for_its_card_then_lands_with_a_diff() {
  let env = Env::new();
  std::fs::write(env.cwd().join("a.txt"), "hello world\n").unwrap();
  let s = env.session().await;
  env.server.push(mock::tools(&[("call_1", "read", json!({ "path": "a.txt" }))]));
  env.server.push(mock::tools(&[("call_2", "edit", json!({ "path": "a.txt", "old_string": "world", "new_string": "there" }))]));
  env.server.push(mock::text("Changed a.txt."));
  let cards = run(&s, "change it").await;
  assert_eq!(cards.len(), 1, "only the edit asks");
  assert_eq!(std::fs::read_to_string(env.cwd().join("a.txt")).unwrap(), "hello there\n");
  let vw = view(&s);
  let turn = last_turn(&vw);
  expect_match(&turn, json!({ "role": "agent", "stop": "end_turn" }));
  let blocks = agent_blocks(&vw);
  expect_match(blocks.iter().find(|b| b["kind"] == "read").unwrap(), json!({ "type": "tool_call", "status": "completed", "target": "a.txt" }));
  expect_match(blocks.iter().find(|b| b["kind"] == "edit").unwrap(), json!({ "type": "tool_call", "status": "completed", "diffStat": { "add": 1, "del": 1 } }));
  assert!(!blocks.iter().any(|b| b["type"] == "permission"));
  assert!(vw["usage"]["used"].as_u64().is_some_and(|n| n > 0), "{}", vw["usage"]);
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-edit", &vw["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn approval_changes_release_waiting_tools_and_apply_to_the_rest_of_the_turn() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::tools(&[
    ("c1", "write", json!({ "path": "a.txt", "content": "a" })),
    ("c2", "write", json!({ "path": "b.txt", "content": "b" })),
    ("c3", "bash", json!({ "command": "echo hot-permissions" })),
  ]));
  env.server.push(mock::text("Done."));
  let task = spawn_prompt(&s, "go");
  wait_block(&s, "permission").await;
  s.set_config("approval".into(), "auto-edit".into()).await.unwrap();
  until(|| env.cwd().join("b.txt").is_file(), 5000).await;
  let card = wait_block(&s, "permission").await;
  assert!(card["command"].as_str().is_some_and(|c| c.contains("hot-permissions")), "{card}");
  s.set_config("approval".into(), "full".into()).await.unwrap();
  until(|| task.is_finished(), 5000).await;
  task.await.unwrap();
  assert_eq!(std::fs::read_to_string(env.cwd().join("a.txt")).unwrap(), "a");
  assert!(!agent_blocks(&view(&s)).iter().any(|b| b["type"] == "permission"));
  env.assert_prefix_stable(&env.log_of(&s));
}

#[tokio::test(flavor = "multi_thread")]
async fn switching_to_plan_denies_a_waiting_edit_even_after_full_access_is_selected() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "write", json!({ "path": "blocked.txt", "content": "x" }))]));
  env.server.push(mock::text("Not changed."));
  let task = spawn_prompt(&s, "go");
  wait_block(&s, "permission").await;
  s.set_mode("plan".into()).await.unwrap();
  s.set_config("approval".into(), "full".into()).await.unwrap();
  until(|| task.is_finished(), 5000).await;
  task.await.unwrap();
  assert!(!env.cwd().join("blocked.txt").exists());
  assert!(!agent_blocks(&view(&s)).iter().any(|b| b["type"] == "permission"));
}

#[tokio::test(flavor = "multi_thread")]
async fn full_access_keeps_a_waiting_protected_config_edit_behind_its_card() {
  let env = Env::new();
  let s = env.session().await;
  let path = env.root.path().join("providers.json");
  let before = std::fs::read_to_string(&path).unwrap();
  env.server.push(mock::tools(&[("c1", "write", json!({ "path": path, "content": "{}" }))]));
  env.server.push(mock::text("Not changed."));
  let task = spawn_prompt(&s, "go");
  let card = wait_block(&s, "permission").await;
  s.set_config("approval".into(), "full".into()).await.unwrap();
  // Wait for a later wire operation too: the approval change has been processed, not merely painted optimistically.
  s.set_mode("agent".into()).await.unwrap();
  assert!(!task.is_finished());
  assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
  assert_eq!(find_block(&view(&s), "permission").unwrap()["id"], card["id"]);
  s.resolve_permission(card["id"].as_str().unwrap(), "reject");
  task.await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plugin_can_still_insist_on_a_card_after_a_live_approval_change() {
  if !acpira_host::plugins::host::on_path("node") {
    return;
  }
  let mut env = Env::new();
  let plugins = tempfile::tempdir().unwrap();
  let script = r#"
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\n');
createInterface({ input: process.stdin }).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, result: m.method === 'hook/permission' ? { decision: 'ask' } : null });
}).on('close', () => process.exit(0));
"#;
  let rt = crate::acp_session::plugin_runtime::plugin(plugins.path(), "guard", script, &[], &["permission"]).await;
  env.deps.plugin_runtime = Some(rt.clone());
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "write", json!({ "path": "gated.txt", "content": "x" }))]));
  env.server.push(mock::text("Done."));
  let task = spawn_prompt(&s, "go");
  let card = wait_block(&s, "permission").await;
  s.set_config("approval".into(), "full".into()).await.unwrap();
  until(|| find_block(&view(&s), "permission").is_some_and(|c| c["id"] != card["id"]), 5000).await;
  assert!(!env.cwd().join("gated.txt").exists());
  let card = wait_block(&s, "permission").await;
  s.resolve_permission(card["id"].as_str().unwrap(), "allow");
  task.await.unwrap();
  assert!(env.cwd().join("gated.txt").exists());
  rt.stop_all().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lowering_approval_during_a_model_call_gates_the_next_tool_in_the_same_turn() {
  let env = Env::new();
  let s = env.session().await;
  s.set_config("approval".into(), "full".into()).await.unwrap();
  let gate = mock::Gate::default();
  env.server.push(Reply::Held(gate.clone(), Box::new(mock::tools(&[("c1", "write", json!({ "path": "held.txt", "content": "x" }))]))));
  env.server.push(mock::text("Done."));
  let task = spawn_prompt(&s, "go");
  until(|| !env.server.requests().is_empty(), 5000).await;
  s.set_config("approval".into(), "ask".into()).await.unwrap();
  gate.open();
  let card = wait_block(&s, "permission").await;
  assert!(!env.cwd().join("held.txt").exists());
  s.resolve_permission(card["id"].as_str().unwrap(), "allow");
  task.await.unwrap();
  assert!(env.cwd().join("held.txt").exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_plan_card_approves_and_the_same_turn_builds_it() {
  let env = Env::new();
  let s = env.session().await;
  s.set_mode("plan".into()).await.unwrap();
  let id = s.to_record().acp_session_id.unwrap();
  let plan = env.root.path().join("agent/sessions").join(&id).join("plan.md");
  env.server.push(mock::tools(&[("c1", "write", json!({ "path": plan.to_string_lossy(), "content": "# Add src.txt\n\n1. Write it.\n" }))]));
  env.server.push(mock::tools(&[("c2", "exit_plan", json!({}))]));
  env.server.push(mock::tools(&[("c3", "write", json!({ "path": "src.txt", "content": "x\n" }))]));
  env.server.push(mock::text("Built."));
  let cards = run(&s, "add src.txt").await;
  let plan_card = cards.iter().find(|c| c["planId"].is_string()).unwrap_or_else(|| panic!("the plan went through the plan card: {cards:?}"));
  assert_eq!(plan_card["title"], "Approval needed: Exit plan mode");
  assert_eq!(std::fs::read_to_string(env.cwd().join("src.txt")).unwrap(), "x\n");
  let vw = view(&s);
  expect_match(find_block(&vw, "plan_document").unwrap(), json!({ "status": "approved", "markdown": "# Add src.txt\n\n1. Write it." }));
  assert_eq!(vw["controls"]["modeId"], "agent");
  expect_match(last_turn(&vw), json!({ "stop": "end_turn" }));
  env.assert_prefix_stable(&env.log_of(&s));
  // Every mode offers the same tools, so entering and leaving Plan never shows up as a tools change
  let log = std::fs::read_to_string(env.log_of(&s)).unwrap();
  assert!(!log.lines().any(|l| l.contains(r#""type":"view""#) && l.contains(r#""tools""#)), "a mode switch changed the tools\n{log}");
  env.golden(&s, "builtin-plan", &vw["turns"]);
}
