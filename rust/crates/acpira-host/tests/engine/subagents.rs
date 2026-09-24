//! test/subagents.test.ts: the extension wire, the subagent tree through AcpSession, AIR async tasks

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::acp::agent_process::{AgentProcess, ClientHandlers};
use acpira_host::acp::cancel::Cancel;
use acpira_host::acp::rpc::{BoxFuture, RpcError};
use acpira_host::acp::session::AcpSession;
use acpira_host::acp::wire::{EXT_META_KEY, ExtensionUpdate, TaskEventKind, extension_of};

use crate::acp_session::{claimed, history_edit, prompt, spawn_prompt, view};
use crate::fake_or_skip;
use crate::support::{Disposing, Harness, expect_absent, expect_eq, expect_match, until, v};

fn lifecycle(u: Option<ExtensionUpdate>) -> Value {
  match u {
    Some(ExtensionUpdate::Lifecycle(l)) => json!({ "kind": "lifecycle", "peerSessionId": l.peer_session_id, "title": l.title, "task": l.task,
      "cancel": l.cancel, "state": l.state.map(v) }),
    Some(ExtensionUpdate::AsyncTask(e)) => json!({ "kind": "async_task", "event": match e.event {
      TaskEventKind::Spawned => "spawned", TaskEventKind::Progress => "progress", TaskEventKind::State => "state" },
      "asyncTaskId": e.async_task_id, "name": e.name, "taskType": e.task_type, "description": e.description,
      "showInTranscript": e.show_in_transcript, "canStop": e.can_stop, "outputFilePath": e.output_file_path, "toolCallId": e.tool_call_id,
      "summary": e.summary, "lastToolName": e.last_tool_name, "usage": e.usage.map(v), "state": e.state.map(v) }),
    Some(ExtensionUpdate::Ignored(kind)) => json!({ "kind": "ignored", "sessionUpdate": kind }),
    None => Value::Null,
  }
}

fn parked(inner: Value) -> Value {
  json!({ "sessionUpdate": "session_info_update", "_meta": { EXT_META_KEY: inner } })
}

#[test]
fn the_rfd_dialect_decodes_raw_and_from_the_parked_form() {
  let raw = json!({ "sessionUpdate": "subagent_update", "subagentSessionId": "c1", "name": "N", "task": "T", "capabilities": { "cancel": true } });
  let want = json!({ "kind": "lifecycle", "peerSessionId": "c1", "title": "N", "task": "T", "cancel": true });
  expect_match(lifecycle(extension_of(&raw, &|_| {})), want.clone());
  expect_match(lifecycle(extension_of(&parked(raw), &|_| {})), want);
}

#[test]
fn the_claude_legacy_pair_decodes_and_unknown_states_count_as_running() {
  let logs = Mutex::new(Vec::<String>::new());
  let log = |l: &str| logs.lock().unwrap().push(l.to_owned());
  expect_match(lifecycle(extension_of(&parked(json!({ "sessionUpdate": "subagent_spawned", "subagentSessionId": "k1", "name": "n", "task": "t", "capabilities": {} })), &log)),
    json!({ "kind": "lifecycle", "peerSessionId": "k1", "title": "n", "task": "t", "cancel": false }));
  expect_match(lifecycle(extension_of(&parked(json!({ "sessionUpdate": "subagent_state_update", "subagentSessionId": "k1", "state": "completed" })), &log)),
    json!({ "kind": "lifecycle", "peerSessionId": "k1", "state": "completed" }));
  // 'disconnected' is a legitimate agent-reported state (RFD lifecycle enum) — no log, no fallback
  expect_match(lifecycle(extension_of(&parked(json!({ "sessionUpdate": "subagent_state_update", "subagentSessionId": "k1", "state": "disconnected" })), &log)),
    json!({ "kind": "lifecycle", "peerSessionId": "k1", "state": "disconnected" }));
  expect_match(lifecycle(extension_of(&parked(json!({ "sessionUpdate": "subagent_state_update", "subagentSessionId": "k1", "state": "zzz" })), &log)),
    json!({ "kind": "lifecycle", "peerSessionId": "k1", "state": "running" }));
  assert_eq!(logs.lock().unwrap().iter().filter(|l| l.contains("unknown subagent state")).count(), 1);
}

#[test]
fn an_async_task_without_an_id_is_ignored_and_a_missing_subagent_session_id_is_logged() {
  expect_eq(lifecycle(extension_of(&parked(json!({ "sessionUpdate": "async_task_spawned", "id": "a1" })), &|_| {})), json!({ "kind": "ignored", "sessionUpdate": "async_task_spawned" }));
  let logs = Mutex::new(Vec::<String>::new());
  expect_eq(lifecycle(extension_of(&parked(json!({ "sessionUpdate": "subagent_update" })), &|l| logs.lock().unwrap().push(l.to_owned()))),
    json!({ "kind": "ignored", "sessionUpdate": "subagent_update" }));
  assert!(logs.lock().unwrap()[0].contains("subagentSessionId"));
}

#[test]
fn the_three_async_task_kinds_decode_and_an_unknown_state_or_missing_id_drops() {
  let logs = Mutex::new(Vec::<String>::new());
  let log = |l: &str| logs.lock().unwrap().push(l.to_owned());
  let dec = |inner: Value| lifecycle(extension_of(&parked(inner), &log));
  expect_match(dec(json!({ "sessionUpdate": "async_task_spawned", "asyncTaskId": "t1", "name": "sleep", "taskType": "shell",
    "showInTranscript": false, "canStop": true, "toolCallId": "tc1", "description": "run it", "outputFilePath": "/tmp/o.log" })),
    json!({ "kind": "async_task", "event": "spawned", "asyncTaskId": "t1", "name": "sleep", "taskType": "shell",
      "canStop": true, "toolCallId": "tc1", "description": "run it", "outputFilePath": "/tmp/o.log" }));
  // showInTranscript is only recorded when true — false is the default
  assert_eq!(dec(json!({ "sessionUpdate": "async_task_spawned", "asyncTaskId": "t1", "showInTranscript": false }))["showInTranscript"], false);
  assert_eq!(dec(json!({ "sessionUpdate": "async_task_spawned", "asyncTaskId": "t1", "showInTranscript": true }))["showInTranscript"], true);
  expect_match(dec(json!({ "sessionUpdate": "async_task_progress", "asyncTaskId": "t1", "summary": "half", "lastToolName": "wc",
    "usage": { "totalTokens": 5, "toolUses": 1, "durationMs": 42, "bogus": "x" } })),
    json!({ "kind": "async_task", "event": "progress", "summary": "half", "lastToolName": "wc", "usage": { "totalTokens": 5, "toolUses": 1, "durationMs": 42 } }));
  expect_absent(dec(json!({ "sessionUpdate": "async_task_progress", "asyncTaskId": "t1", "usage": { "bogus": "x" } }))["usage"].clone(), "bogus");
  expect_match(dec(json!({ "sessionUpdate": "async_task_state_update", "asyncTaskId": "t1", "state": "stopped" })), json!({ "kind": "async_task", "event": "state", "state": "stopped" }));
  expect_eq(dec(json!({ "sessionUpdate": "async_task_state_update", "asyncTaskId": "t1", "state": "zzz" })), json!({ "kind": "ignored", "sessionUpdate": "async_task_state_update" }));
  expect_eq(dec(json!({ "sessionUpdate": "async_task_progress" })), json!({ "kind": "ignored", "sessionUpdate": "async_task_progress" }));
  let logs = logs.lock().unwrap();
  assert_eq!(logs.iter().filter(|l| l.contains("asyncTaskId")).count(), 1);
  assert_eq!(logs.iter().filter(|l| l.contains("unknown state")).count(), 1);
}

#[test]
fn ordinary_updates_and_plain_session_info_updates_are_not_extensions() {
  assert!(extension_of(&json!({ "sessionUpdate": "session_info_update", "title": "t" }), &|_| {}).is_none());
  assert!(extension_of(&json!({ "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": "x" } }), &|_| {}).is_none());
}

fn sub(s: &AcpSession, peer: &str, id: &str) -> Option<Value> {
  view(s)["subagents"].as_array()?.iter().find(|n| n["peer"][peer] == id).cloned()
}

fn sub_id(s: &AcpSession, peer: &str, id: &str) -> String {
  sub(s, peer, id).expect("node")["id"].as_str().unwrap().to_owned()
}

fn root_blocks(s: &AcpSession) -> Vec<Value> {
  crate::acp_session::agent_blocks(&view(s))
}

fn child_turns(s: &AcpSession, node: &str) -> Value {
  s.subagent_transcript(node).map(|(t, _, _)| serde_json::from_str(t.get()).unwrap()).unwrap_or(json!([]))
}

fn child_blocks(s: &AcpSession, node: &str) -> Vec<Value> {
  child_turns(s, node).as_array().unwrap().iter().filter(|t| t["role"] == "agent").flat_map(|t| t["blocks"].as_array().unwrap().clone()).collect()
}

fn has(blocks: &[Value], f: impl Fn(&Value) -> bool) -> bool {
  blocks.iter().any(f)
}

async fn session(h: &Harness) -> Disposing {
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  s
}

fn permissions_of(s: &AcpSession, peer: &str) -> usize {
  sub(s, "sessionId", peer).and_then(|n| n["permissions"].as_array().map(|p| p.len())).unwrap_or(0)
}

#[tokio::test(flavor = "multi_thread")]
async fn native_children_keep_separate_transcripts_and_the_child_card_lives_in_the_child() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  let p = spawn_prompt(&s, "subagents-native");
  until(|| permissions_of(&s, "c1") > 0, 8000).await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  let c2 = sub(&s, "sessionId", "c2").unwrap();
  let (c1id, c2id) = (c1["id"].as_str().unwrap().to_owned(), c2["id"].as_str().unwrap().to_owned());
  expect_match(&c1, json!({ "title": "Map ownership", "task": "Inspect src/shared", "visibility": "session", "state": "running", "controls": { "cancel": true } }));
  expect_eq(&c2["controls"], json!({ "cancel": false }));
  expect_absent(&c1, "parentId");
  // the permission card is in c1's transcript + summary, never the root turn
  let perm = child_blocks(&s, &c1id).into_iter().find(|b| b["type"] == "permission").expect("child card");
  assert!(!has(&root_blocks(&s), |b| b["type"] == "permission"));
  s.resolve_permission(perm["id"].as_str().unwrap(), "allow");
  p.await.unwrap();
  // answering let the child finish; terminal nodes have endedAt and no activity
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  expect_match(&c1, json!({ "state": "completed", "stateSource": "agent", "result": "c1 done" }));
  assert!(c1["endedAt"].is_number());
  expect_absent(&c1, "activity");
  expect_match(sub(&s, "sessionId", "c2").unwrap(), json!({ "state": "completed" }));
  assert!(sub(&s, "sessionId", "c2").unwrap()["endedAt"].is_number());
  // the child turn and its tool rows carry real timestamps (the eager turn opened at announce)
  let c1turn = child_turns(&s, &c1id)[0].clone();
  expect_match(&c1turn, json!({ "role": "agent", "stop": "end_turn" }));
  assert!(c1turn["startedAt"].is_number() && c1turn["endedAt"].is_number());
  let tool = child_blocks(&s, &c1id).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  assert!(tool["startedAt"].is_number() && tool["endedAt"].is_number());
  // transcripts never mix: each child holds only its own blocks, the root holds none of them
  assert!(has(&child_blocks(&s, &c1id), |b| b["type"] == "tool_call" && b["id"] == "c1-t1"));
  assert!(!has(&child_blocks(&s, &c1id), |b| b["type"] == "text" && b["markdown"].as_str().unwrap().contains("c2 working")));
  assert!(has(&child_blocks(&s, &c2id), |b| b["type"] == "tool_call" && b["id"] == "c2-t1"));
  assert!(!has(&root_blocks(&s), |b| b["type"] == "tool_call" && (b["id"] == "c1-t1" || b["id"] == "c2-t1")));
  assert!(!has(&root_blocks(&s), |b| b["type"] == "text" && b["markdown"].as_str().unwrap().contains("c1 done")));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_subagent_sends_session_cancel_for_the_child_and_a_child_without_the_capability_refuses() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  let p = spawn_prompt(&s, "subagents-native");
  until(|| permissions_of(&s, "c1") > 0, 8000).await;
  s.cancel_subagent(&sub_id(&s, "sessionId", "c2")).await;
  assert_eq!(sub(&s, "sessionId", "c2").unwrap()["state"], "running");
  s.cancel_subagent(&sub_id(&s, "sessionId", "c1")).await;
  assert_eq!(sub(&s, "sessionId", "c1").unwrap()["cancelRequested"], true);
  // the fake only reports cancelled after its session/cancel for c1 lands
  p.await.unwrap();
  expect_match(sub(&s, "sessionId", "c1").unwrap(), json!({ "state": "cancelled", "stateSource": "agent" }));
  assert_eq!(sub(&s, "sessionId", "c2").unwrap()["state"], "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_grandchild_announced_on_the_child_stream_resolves_its_parent() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-nested").await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  let c1a = sub(&s, "sessionId", "c1a").unwrap();
  expect_absent(&c1, "parentId");
  assert_eq!(c1a["parentId"], c1["id"]);
  assert_eq!(c1a["state"], "completed");
  assert!(has(&child_blocks(&s, c1a["id"].as_str().unwrap()), |b| b["type"] == "tool_call" && b["id"] == "c1a-t1"));
  assert!(!has(&child_blocks(&s, c1["id"].as_str().unwrap()), |b| b["type"] == "tool_call"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_still_running_child_disconnects_locally_when_the_prompt_returns_and_restore_never_revives_it() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-orphan").await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  expect_match(&c1, json!({ "state": "disconnected", "stateSource": "local" }));
  assert!(c1["endedAt"].is_number());
  let tool = child_blocks(&s, c1["id"].as_str().unwrap()).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  assert_eq!(tool["status"], "cancelled");
  let rec = Disposing(AcpSession::new(s.to_record(), h.deps.clone()));
  let subs = view(&rec)["subagents"].clone();
  assert_eq!(subs.as_array().unwrap().len(), 1);
  expect_match(&subs[0], json!({ "state": "disconnected", "stateSource": "local" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_late_terminal_word_supersedes_a_local_disconnect_and_running_after_it_is_ignored() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-late-terminal").await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  let id = c1["id"].as_str().unwrap().to_owned();
  expect_match(&c1, json!({ "state": "disconnected", "stateSource": "local" }));
  assert_eq!(child_blocks(&s, &id).into_iter().find(|b| b["type"] == "tool_call").unwrap()["status"], "cancelled");
  // the next prompt carries the child's late terminal update on the same connection
  prompt(&s, "hi").await;
  expect_match(sub(&s, "sessionId", "c1").unwrap(), json!({ "state": "completed", "stateSource": "agent", "endedAt": c1["endedAt"] }));
  // the already-sealed transcript keeps its stop reason and tool statuses
  expect_match(&child_turns(&s, &id)[0], json!({ "stop": "cancelled" }));
  assert_eq!(child_blocks(&s, &id).into_iter().find(|b| b["type"] == "tool_call").unwrap()["status"], "cancelled");
  // a 'running' report after the terminal one must be ignored
  prompt(&s, "hi").await;
  assert_eq!(sub(&s, "sessionId", "c1").unwrap()["state"], "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_the_agent_itself_disconnected_ends_as_disconnected_by_the_agent() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-lost").await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  expect_match(&c1, json!({ "state": "disconnected", "stateSource": "agent" }));
  assert!(c1["endedAt"].is_number());
}

#[tokio::test(flavor = "multi_thread")]
async fn updates_buffered_before_the_announce_apply_in_order_and_over_cap_drops_log_once() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-early").await;
  let c9 = sub(&s, "sessionId", "c9").unwrap();
  expect_match(&c9, json!({ "title": "Late announcer", "state": "completed" }));
  let blocks = child_blocks(&s, c9["id"].as_str().unwrap());
  expect_match(&blocks[0], json!({ "type": "thought", "text": "early thought" }));
  assert_eq!(blocks.iter().find(|b| b["type"] == "tool_call").unwrap()["status"], "completed");
  s.dispose();

  let s2 = session(&h).await;
  prompt(&s2, "subagents-early-flood").await;
  assert_eq!(sub(&s2, "sessionId", "c8").unwrap()["state"], "completed");
  assert_eq!(h.logs().iter().filter(|l| l.contains("too many buffered")).count(), 1);
  // 70 sent, 64 fit: the first buffered chunk applied in order after the announce
  let text: String = child_blocks(&s2, &sub_id(&s2, "sessionId", "c8")).iter().filter(|b| b["type"] == "text").map(|b| b["markdown"].as_str().unwrap().to_owned()).collect();
  assert!(text.starts_with("m0 m1 "), "{text}");
  assert!(!text.contains("m69"));
}

#[tokio::test(flavor = "multi_thread")]
async fn devin_nested_shape_stamps_the_delegation_routes_tools_and_isolates_usage() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-devin").await;
  let d1 = sub(&s, "agentId", "d1").unwrap();
  let id = d1["id"].as_str().unwrap().to_owned();
  expect_match(&d1, json!({ "visibility": "nested", "title": "Count files in src/shared", "task": "Count the files under src/shared",
    "role": "Explore", "model": "SWE-2 High", "background": true, "state": "completed", "stateSource": "agent", "result": "2 files in src/shared" }));
  assert!(d1["endedAt"].is_number());
  // the delegation call is stamped, the child's own calls live only in the child transcript
  let root = root_blocks(&s);
  assert_eq!(root.iter().find(|b| b["id"] == "run_subagent:0#a1").unwrap()["subagentId"], id.as_str());
  assert!(has(&child_blocks(&s, &id), |b| b["type"] == "tool_call" && b["id"] == "find:0#c1"));
  assert!(!has(&root, |b| b["type"] == "tool_call" && (b["id"] == "find:0#c1" || b["id"] == "d1")));
  // the child's usage_update fed its summary, not the root's context ring
  expect_eq(&d1["usage"], json!({ "used": 4200, "size": 100_000 }));
  expect_match(&view(&s)["usage"], json!({ "used": 5000 }));
  // read_subagent stays a root row: it is what the parent was doing
  expect_match(root.iter().find(|b| b["id"] == "read_subagent:0#b1").unwrap(), json!({ "verbKey": "verb.awaitSubagent", "target": "Count files in src/shared", "status": "completed" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_legacy_updates_and_an_async_launched_receipt_link_the_root_call() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  for script in ["subagents-claude", "subagents-claude-nolink", "subagents-claude-rootfirst"] {
    let s = session(&h).await;
    prompt(&s, script).await;
    let k1 = sub(&s, "sessionId", "k1").unwrap();
    let id = k1["id"].as_str().unwrap().to_owned();
    if script != "subagents-claude-nolink" {
      expect_match(&k1, json!({ "visibility": "session", "title": "Explore shared", "model": "x", "state": "completed" }));
    }
    if script == "subagents-claude" {
      assert_eq!(k1["task"], "map src/shared");
      assert!(has(&child_blocks(&s, &id), |b| b["type"] == "tool_call" && b["id"] == "k1-t1"));
    }
    assert_eq!(k1["peer"]["toolCallId"], "call_k1", "{script}");
    let receipt = root_blocks(&s).into_iter().find(|b| b["id"] == "call_k1").unwrap();
    assert_eq!(receipt["subagentId"], id.as_str(), "{script}");
    // The launch receipt is the delegation call returning; it never gets a terminal status from the wire
    assert_eq!(receipt["status"], "completed", "{script}");
    assert_eq!(k1["state"], "completed");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_claude_async_child_without_a_terminal_update_disconnects_when_the_root_turn_ends() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-claude-async").await;
  expect_match(sub(&s, "sessionId", "k1").unwrap(), json!({ "state": "disconnected", "stateSource": "local" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kimi_agent_receipt_creates_the_node_and_its_completed_text_becomes_the_result() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-receipt").await;
  let node = sub(&s, "toolCallId", "0:tool_01").unwrap();
  expect_match(&node, json!({ "visibility": "receipt", "title": "List src files", "role": "explore",
    "task": "Read-only task: list every file under src/shared.", "state": "completed", "stateSource": "agent" }));
  assert!(node["result"].as_str().unwrap().contains("found 20 files"));
  expect_match(root_blocks(&s).into_iter().find(|b| b["id"] == "0:tool_01").unwrap(), json!({ "subagentId": node["id"], "verbKey": "verb.delegate" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_node_restores_disconnected_and_unchanged_summaries_stay_stable() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  let p = spawn_prompt(&s, "subagents-native");
  until(|| permissions_of(&s, "c1") > 0, 8000).await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  assert_eq!(c1["state"], "running");
  let mut record = s.to_record();
  assert_eq!(v(&record.subagents).as_array().unwrap().iter().find(|n| n["peer"]["sessionId"] == "c1").unwrap()["state"], "running");
  record.updated_at = acpira_host::util::iso_of_ms(7000);
  let restored = Disposing(AcpSession::new(record, h.deps.clone()));
  let r = view(&restored)["subagents"].as_array().unwrap().iter().find(|n| n["peer"]["sessionId"] == "c1").cloned().unwrap();
  expect_match(&r, json!({ "state": "disconnected", "stateSource": "local", "endedAt": 7000 }));
  // let the live script finish: answer c1's pending permission
  let perm = child_blocks(&s, c1["id"].as_str().unwrap()).into_iter().find(|b| b["type"] == "permission").unwrap();
  s.resolve_permission(perm["id"].as_str().unwrap(), "allow");
  p.await.unwrap();
  // an unchanged node serializes the same across views (the TS suite checked object reuse)
  assert_eq!(view(&s)["subagents"][0], view(&s)["subagents"][0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn edit_truncation_drops_nodes_anchored_at_the_removed_turns() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-orphan").await;
  assert_eq!(view(&s)["subagents"].as_array().unwrap().len(), 1);
  s.edit_turn(history_edit(&s, 0, "hi")).await.unwrap();
  assert_eq!(view(&s)["subagents"].as_array().map_or(0, |a| a.len()), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_call_reusing_a_native_childs_tool_id_stays_on_the_root() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-native-collision").await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  // the child's own 'shared-id' lived in its transcript; the root's same-named call stayed put
  let root = root_blocks(&s);
  let root_tool = root.iter().find(|b| b["type"] == "tool_call" && b["id"] == "shared-id").unwrap();
  expect_match(root_tool, json!({ "kind": "edit", "status": "completed" }));
  expect_absent(root_tool, "subagentId");
  let child_tool = child_blocks(&s, c1["id"].as_str().unwrap()).into_iter().find(|b| b["type"] == "tool_call" && b["id"] == "shared-id").unwrap();
  expect_match(&child_tool, json!({ "kind": "read", "status": "completed" }));
  assert_eq!(c1["toolCount"], 1);
  assert_eq!(root.iter().filter(|b| b["type"] == "tool_call" && b["id"] == "shared-id").count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_devin_nested_spawn_reads_its_parent_link_and_early_child_content_replays() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-devin-deep").await;
  let d1 = sub(&s, "agentId", "d1").unwrap();
  let d2 = sub(&s, "agentId", "d2").unwrap();
  // the spawn carried parentAgentId=d1 — lifecycle, not d1 content
  assert_eq!(d2["parentId"], d1["id"]);
  assert!(!has(&child_blocks(&s, d1["id"].as_str().unwrap()), |b| b["type"] == "tool_call"));
  // d2's tool call was buffered before the spawn and replayed into d2, never the root
  assert!(has(&child_blocks(&s, d2["id"].as_str().unwrap()), |b| b["type"] == "tool_call" && b["id"] == "deep:1" && b["status"] == "completed"));
  assert!(!has(&root_blocks(&s), |b| b["type"] == "tool_call" && (b["id"] == "deep:1" || b["id"] == "d2")));
  expect_match(&d2, json!({ "state": "completed", "result": "inner done" }));
  expect_match(&d1, json!({ "state": "completed", "result": "outer done" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn late_content_after_a_terminal_state_drops_with_one_log_while_usage_still_applies() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-late-drop").await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  let id = c1["id"].as_str().unwrap();
  assert_eq!(c1["state"], "completed");
  assert!(!has(&child_blocks(&s, id), |b| b["type"] == "text" && b["markdown"].as_str().unwrap().contains("late text")));
  assert!(!has(&child_blocks(&s, id), |b| b["type"] == "tool_call" && b["id"] == "c1-late"));
  assert_eq!(c1["toolCount"], 1);
  expect_eq(&c1["usage"], json!({ "used": 42, "size": 100 }));
  assert_eq!(h.logs().iter().filter(|l| l.contains("is completed;") && l.contains("dropped")).count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_nested_node_on_the_root_call_upgrades_into_the_session_node() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-claude-upgrade").await;
  let subs = view(&s)["subagents"].clone();
  assert_eq!(subs.as_array().unwrap().len(), 1);
  let n = &subs[0];
  expect_match(n, json!({ "visibility": "session", "title": "Explore shared", "model": "x", "state": "completed", "peer": { "sessionId": "k1", "toolCallId": "call_x" } }));
  assert_eq!(root_blocks(&s).into_iter().find(|b| b["id"] == "call_x").unwrap()["subagentId"], n["id"]);
  // the child's streamed tool call survived the in-place upgrade
  assert!(has(&child_blocks(&s, n["id"].as_str().unwrap()), |b| b["type"] == "tool_call" && b["id"] == "k1-t1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_announcement_naming_the_root_session_itself_is_rejected() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-self").await;
  assert_eq!(view(&s)["subagents"].as_array().map_or(0, |a| a.len()), 0);
  assert!(h.logs().iter().any(|l| l.contains("own session")));
  assert!(has(&root_blocks(&s), |b| b["type"] == "text" && b["markdown"].as_str().unwrap().contains("root done")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_child_releases_its_card_and_cancelling_a_parent_cascades() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  let s2 = session(&h).await;
  // the child ends while its permission card is up — the card leaves the transcript, the agent got 'cancelled'
  prompt(&s, "subagents-native-termperm").await;
  let c1 = sub(&s, "sessionId", "c1").unwrap();
  assert_eq!(c1["state"], "completed");
  assert!(!has(&child_blocks(&s, c1["id"].as_str().unwrap()), |b| b["type"] == "permission"));
  assert!(has(&root_blocks(&s), |b| b["type"] == "text" && b["markdown"].as_str().unwrap().contains("perm cancelled")));
  // the grandchild's pending card answers cancelled when the parent is cancelled
  let p = spawn_prompt(&s2, "subagents-native-cascade");
  until(|| permissions_of(&s2, "c1a") > 0, 8000).await;
  let inner = sub_id(&s2, "sessionId", "c1a");
  s2.cancel_subagent(&sub_id(&s2, "sessionId", "c1")).await;
  p.await.unwrap();
  expect_match(sub(&s2, "sessionId", "c1").unwrap(), json!({ "state": "cancelled" }));
  expect_match(sub(&s2, "sessionId", "c1a").unwrap(), json!({ "state": "cancelled" }));
  assert!(!has(&child_blocks(&s2, &inner), |b| b["type"] == "permission"));
  assert!(has(&root_blocks(&s2), |b| b["type"] == "text" && b["markdown"].as_str().unwrap().contains("inner perm cancelled")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reopened_childs_fresh_q1_cannot_settle_the_old_q1_in_the_root_history() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  // a root question resolved before the record was written — the gate's next card reuses q-1
  let p = spawn_prompt(&s, "ask-devin");
  until(|| has(&root_blocks(&s), |b| b["type"] == "question"), 8000).await;
  let old = root_blocks(&s).into_iter().find(|b| b["type"] == "question").unwrap();
  s.answer_questions(old["id"].as_str().unwrap(), &serde_json::from_value(json!({ "q0": "report", "q1": ["src"] })).unwrap(), false);
  p.await.unwrap();
  let mut record = s.to_record();
  // reopen the record on a fresh native session: new gate, same history
  record.acp_session_id = None;
  let s2 = Disposing(AcpSession::new(record, h.deps.clone()));
  s2.start().await;
  let p2 = spawn_prompt(&s2, "subagents-native-question");
  until(|| sub(&s2, "sessionId", "c1").is_some_and(|n| !n["question"].is_null()), 8000).await;
  let card = sub(&s2, "sessionId", "c1").unwrap()["question"].clone();
  assert_eq!(card["id"], "q-1");
  s2.answer_questions("q-1", &serde_json::from_value(json!({ "q0": "a" })).unwrap(), false);
  p2.await.unwrap();
  let c1 = sub_id(&s2, "sessionId", "c1");
  expect_match(child_blocks(&s2, &c1).into_iter().find(|b| b["type"] == "question").unwrap(), json!({ "outcome": "answered", "answers": { "q0": "a" } }));
  // the root's historical card keeps its own answers — same id, different owner
  expect_match(root_blocks(&s2).into_iter().find(|b| b["type"] == "question").unwrap(), json!({ "id": "q-1", "outcome": "answered", "answers": { "q0": "report", "q1": ["src"] } }));
}

#[tokio::test(flavor = "multi_thread")]
async fn records_carry_the_node_rev_so_a_restore_keeps_transcript_dedupe_working() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-orphan").await;
  let c1 = sub_id(&s, "sessionId", "c1");
  let record = s.to_record();
  let stored = v(&record.subagents).as_array().unwrap().iter().find(|n| n["peer"]["sessionId"] == "c1").cloned().unwrap();
  let rev = stored["rev"].as_i64().unwrap();
  assert!(rev > 1);
  let restored = Disposing(AcpSession::new(record, h.deps.clone()));
  assert_eq!(restored.subagent_transcript(&c1).map(|(_, r, _)| r), Some(rev));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_codex_reopen_announces_a_new_generation_and_the_old_one_stays_terminal() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "subagents-generation").await;
  let g1 = sub(&s, "sessionId", "thr:generation:1").unwrap();
  let g2 = sub(&s, "sessionId", "thr:generation:2").unwrap();
  assert_eq!(view(&s)["subagents"].as_array().unwrap().len(), 2);
  expect_match(&g1, json!({ "state": "failed", "stateSource": "agent", "controls": { "cancel": false } }));
  expect_match(&g2, json!({ "state": "completed", "stateSource": "agent", "controls": { "cancel": false } }));
  assert!(has(&child_blocks(&s, g1["id"].as_str().unwrap()), |b| b["type"] == "tool_call" && b["id"] == "g1-t1"));
  assert!(has(&child_blocks(&s, g2["id"].as_str().unwrap()), |b| b["type"] == "tool_call" && b["id"] == "g2-t1"));
  assert!(!has(&root_blocks(&s), |b| b["type"] == "tool_call" && (b["id"] == "g1-t1" || b["id"] == "g2-t1")));
}

fn tool_row(s: &AcpSession, id: &str) -> Option<Value> {
  root_blocks(s).into_iter().find(|b| b["type"] == "tool_call" && b["id"] == id)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backgrounded_tool_row_survives_end_turn_and_settles_from_the_task_state() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  let p = spawn_prompt(&s, "async-shell");
  until(|| tool_row(&s, "bg-1").is_some_and(|r| r["asyncTask"]["id"] == "task-1"), 8000).await;
  let row = tool_row(&s, "bg-1").unwrap();
  expect_match(&row, json!({ "status": "in_progress", "background": true, "asyncTask": { "id": "task-1", "state": "running", "canStop": true, "taskType": "shell", "name": "sleep 45" } }));
  p.await.unwrap();
  // the turn ended end_turn while the task still ran — the row is not swept failed/cancelled
  expect_match(crate::acp_session::last_turn(&view(&s)), json!({ "stop": "end_turn" }));
  expect_match(tool_row(&s, "bg-1").unwrap(), json!({ "status": "in_progress", "background": true }));
  until(|| tool_row(&s, "bg-1").is_some_and(|r| r["asyncTask"]["state"] == "completed"), 8000).await;
  expect_match(tool_row(&s, "bg-1").unwrap(), json!({ "status": "completed", "asyncTask": { "summary": "done", "outputFilePath": "/tmp/fake-task.log" } }));
  // exactly one row hosts the task the whole time
  assert_eq!(root_blocks(&s).iter().filter(|b| b["type"] == "tool_call").count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn stop_async_task_sends_the_stop_request_and_the_stopped_update_settles_the_row() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let stop_log = dir.path().join("stop.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STOP_LOG": stop_log } }));
  let s = session(&h).await;
  prompt(&s, "async-stop").await;
  expect_match(&tool_row(&s, "bg-1").unwrap()["asyncTask"], json!({ "id": "task-1", "state": "running", "canStop": true }));
  let peer = s.to_record().acp_session_id.unwrap();
  let stop = claimed({
    let s = s.0.clone();
    async move { s.stop_async_task("task-1").await }
  });
  // the optimistic marker is visible before the request resolves; the adapter's state update settles it
  assert_eq!(tool_row(&s, "bg-1").unwrap()["asyncTask"]["stopRequested"], true);
  stop.await.unwrap().unwrap();
  until(|| tool_row(&s, "bg-1").is_some_and(|r| r["asyncTask"]["state"] == "stopped"), 8000).await;
  assert_eq!(std::fs::read_to_string(&stop_log).unwrap().trim(), format!("{peer} task-1"));
  expect_match(tool_row(&s, "bg-1").unwrap(), json!({ "status": "cancelled" }));
  expect_absent(&tool_row(&s, "bg-1").unwrap()["asyncTask"], "stopRequested");
  // a stop on a terminal task is a local no-op — no second request goes out
  s.stop_async_task("task-1").await.ok();
  assert_eq!(std::fs::read_to_string(&stop_log).unwrap().trim().lines().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_that_cannot_stop_never_sends_the_request() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let stop_log = dir.path().join("stop.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_STOP_LOG": stop_log } }));
  let s = session(&h).await;
  prompt(&s, "async-stop-nostop").await;
  expect_match(&tool_row(&s, "bg-1").unwrap()["asyncTask"], json!({ "id": "task-1", "state": "running", "canStop": false }));
  s.stop_async_task("task-1").await.ok();
  tokio::time::sleep(std::time::Duration::from_millis(150)).await;
  assert!(!stop_log.exists());
  let task = tool_row(&s, "bg-1").unwrap()["asyncTask"].clone();
  expect_match(&task, json!({ "state": "running" }));
  expect_absent(&task, "stopRequested");
  assert!(h.logs().iter().any(|l| l.contains("cannot be stopped")));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_show_in_transcript_spawn_synthesizes_a_row_that_a_later_tool_call_id_migrates() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "async-orphan").await;
  let tools: Vec<Value> = root_blocks(&s).into_iter().filter(|b| b["type"] == "tool_call").collect();
  assert_eq!(tools.len(), 1);
  expect_match(&tools[0], json!({ "id": "bg-1", "status": "in_progress", "background": true, "asyncTask": { "id": "task-9", "name": "orphan build", "state": "running" } }));
  assert!(tool_row(&s, "async:task-9").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_task_on_a_child_session_lands_in_the_child_and_the_node_outlives_the_parent_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  let p = spawn_prompt(&s, "async-child");
  until(|| sub(&s, "sessionId", "c1").is_some(), 8000).await;
  let c1 = sub_id(&s, "sessionId", "c1");
  let tool = |s: &AcpSession| child_blocks(s, &c1).into_iter().find(|b| b["type"] == "tool_call" && b["id"] == "c1-t1");
  until(|| tool(&s).is_some_and(|t| t["asyncTask"]["id"] == "task-c1"), 8000).await;
  p.await.unwrap();
  // the parent turn ended while the child's task still ran — the node stays running, not disconnected
  expect_match(sub(&s, "sessionId", "c1").unwrap(), json!({ "state": "running" }));
  expect_match(tool(&s).unwrap(), json!({ "status": "in_progress", "background": true, "asyncTask": { "id": "task-c1", "state": "running", "canStop": true } }));
  assert!(!has(&root_blocks(&s), |b| b["type"] == "tool_call" && (b["id"] == "c1-t1" || !b["asyncTask"].is_null())));
  until(|| sub(&s, "sessionId", "c1").is_some_and(|n| n["state"] == "completed") && tool(&s).is_some_and(|t| t["asyncTask"]["state"] == "completed"), 8000).await;
  expect_match(tool(&s).unwrap(), json!({ "status": "completed" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restored_live_task_keeps_its_state_but_loses_observation_and_stop_control() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = session(&h).await;
  prompt(&s, "async-stop").await;
  let restored = Disposing(AcpSession::new(s.to_record(), h.deps.clone()));
  let row = || crate::acp_session::last_turn(&view(&restored))["blocks"].as_array().unwrap().iter().find(|b| b["type"] == "tool_call").cloned().unwrap();
  expect_match(row(), json!({ "status": "cancelled", "observation": "unknown", "asyncTask": { "id": "task-1", "state": "running", "canStop": false } }));
  restored.stop_async_task("task-1").await.ok();
  expect_absent(&row()["asyncTask"], "stopRequested");
}

/// (exit code, signal) once the process is gone
pub type Exit = Arc<Mutex<Option<(Option<i32>, Option<String>)>>>;

/// Records what reaches the process handlers (updates, the exit), answering nothing
#[derive(Clone, Default)]
pub struct Recorder(pub Arc<Mutex<Vec<Value>>>, pub Exit);

impl ClientHandlers for Recorder {
  fn on_update(&self, params: Value) {
    self.0.lock().unwrap().push(params);
  }
  fn on_exit(&self, code: Option<i32>, signal: Option<String>) {
    *self.1.lock().unwrap() = Some((code, signal));
  }
  fn on_permission(&self, _req: Value, _cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    Box::pin(async { Ok(json!({ "outcome": { "outcome": "cancelled" } })) })
  }
  fn on_elicitation(&self, _req: Value, _cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    Box::pin(async { Err(RpcError::method_not_found("elicitation/create")) })
  }
  fn on_grok_question(&self, _req: Value, _cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    Box::pin(async { Err(RpcError::method_not_found("_x.ai/ask")) })
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn extension_updates_reach_the_process_handler_raw_and_child_streams_pass_through() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let def = h.deps.registry.get("fake").unwrap().clone();
  let bin = h.deps.registry.resolve_binary("fake").await.expect("node on PATH");
  let rec = Recorder::default();
  let updates = rec.0.clone();
  let proc = AgentProcess::spawn(&def, &bin, "/tmp", Arc::new(rec), None, None).await.unwrap();
  let created = proc.request("session/new", json!({ "cwd": "/tmp", "mcpServers": [] })).await.unwrap();
  let sid = created["sessionId"].as_str().unwrap().to_owned();
  proc.request("session/prompt", json!({ "sessionId": sid, "prompt": [{ "type": "text", "text": "subagents-early" }] })).await.unwrap();
  let ups = updates.lock().unwrap().clone();
  // No SDK sits in between, so the extension kind arrives as itself rather than parked in session_info_update
  assert!(ups.iter().any(|n| n["sessionId"] == sid && extension_of(&n["update"], &|_| {}).is_some_and(|e| matches!(e, ExtensionUpdate::Lifecycle(l) if l.peer_session_id == "c9"))));
  // the child's own stream arrived under its own session id
  assert!(ups.iter().any(|n| n["sessionId"] == "c9" && n["update"]["sessionUpdate"] == "tool_call"));
  assert!(ups.iter().any(|n| n["sessionId"] == sid && n["update"]["sessionUpdate"] == "agent_message_chunk"));
  proc.kill().await;
}
