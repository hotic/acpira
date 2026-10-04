//! Cross-harness subagents (acp/session/relay.rs, relay/): a session summons a persona whose CLI is the fake agent, through
//! `relay_ask` directly and through the loopback hub the MCP server uses

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use acpira_host::relay::hub::{RelayHub, call};
use acpira_host::relay::roster::Roster;
use acpira_host::relay::wire::{AskArgs, HubOp, HubReply, HubRequest};

use crate::acp_session::prompt;
use crate::fake_or_skip;
use crate::support::{Disposing, Harness, expect_match, until, v};

async fn roster(dir: &std::path::Path) -> Arc<Roster> {
  let r = Arc::new(Roster::new(dir));
  r.save(&json!([
    { "name": "Fake Review", "agent": "fake", "mode": "consult", "when": "second opinions", "brief": "Severity levels please." },
    { "name": "Fake Worker", "agent": "fake", "mode": "work", "when": "edits" },
  ]))
  .await
  .unwrap();
  r
}

fn ask(agent: &str, prompt: &str) -> AskArgs {
  AskArgs { agent: agent.into(), prompt: prompt.into(), wait_secs: Some(20), ..Default::default() }
}

/// Every reply line of one call, the final one last
async fn run(s: &Disposing, roster: &Arc<Roster>, depth: u32, args: AskArgs) -> Vec<HubReply> {
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), None, depth, args, tx));
  let mut out = vec![];
  while let Some(r) = tokio::time::timeout(Duration::from_secs(20), rx.recv()).await.expect("reply in time") {
    out.push(r);
  }
  out
}

fn last_text(replies: &[HubReply]) -> (bool, String) {
  match replies.last() {
    Some(HubReply::Done(t)) => (true, t.clone()),
    Some(HubReply::Error(t)) => (false, t.clone()),
    other => panic!("no final reply: {other:?}"),
  }
}

fn nodes(s: &Disposing) -> Vec<Value> {
  v(s.view().subagents.unwrap_or_default()).as_array().cloned().unwrap_or_default()
}

fn thread_of(text: &str) -> String {
  let rest = text.split("thread: ").nth(1).expect("thread named");
  rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_summoned_round_runs_in_its_own_process_and_its_thread_continues_the_same_native_session() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;

  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { title: Some("Look at the diff".into()), ..ask("fake-review", "relay: look at the diff") }).await);
  assert!(ok, "{text}");
  assert!(text.starts_with("reply to relay: look at the diff (round 1, mode plan)"), "{text}");
  let thread = thread_of(&text);
  let list = nodes(&s);
  assert_eq!(list.len(), 1);
  expect_match(&list[0], json!({
    "visibility": "session", "state": "completed", "title": "Look at the diff", "role": "Fake Review",
    "task": "relay: look at the diff", "toolCount": 1,
    "harness": { "agent": "fake", "persona": "fake-review", "mode": "consult", "thread": thread, "round": 1 },
  }));
  assert!(list[0]["result"].as_str().unwrap().contains("(round 1, mode plan)"));
  // The child received the persona's brief and the consult rule along with the prompt; its transcript is its own
  let (turns, _, running) = s.subagent_transcript(list[0]["id"].as_str().unwrap()).unwrap();
  assert!(!running);
  assert!(turns.0.get().contains("reply to relay: look at the diff"));

  // Round 2 on the same thread: a node of its own, the same native session (its prompt counter says 2)
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: and now?") }).await);
  assert!(ok, "{text}");
  assert!(text.starts_with("reply to relay: and now? (round 2, mode plan)"), "{text}");
  let list = nodes(&s);
  assert_eq!(list.len(), 2);
  expect_match(&list[1], json!({ "state": "completed", "harness": { "thread": thread, "round": 2 } }));
  assert_eq!(list[0]["peer"]["sessionId"], list[1]["peer"]["sessionId"]);
  // A wait on a finished thread answers its last reply without running anything
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "") }).await);
  assert!(text.starts_with("reply to relay: and now? (round 2, mode plan)"), "{text}");
  assert_eq!(nodes(&s).len(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_past_its_deadline_answers_still_working_and_a_wait_call_collects_the_reply() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow job") }).await);
  assert!(ok);
  assert!(text.contains("is still working"), "{text}");
  let thread = thread_of(&text);
  assert_eq!(nodes(&s)[0]["state"], "running");
  // A new prompt on a busy thread is refused; an empty one waits
  let (ok, busy) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: more") }).await);
  assert!(!ok && busy.contains("still working"), "{busy}");
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "") }).await);
  assert!(ok);
  assert!(text.starts_with("reply to relay: slow job (round 1, mode agent)"), "{text}");
  assert_eq!(nodes(&s)[0]["state"], "completed");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_childs_permission_request_lands_on_its_node_and_the_answer_reaches_its_process() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), None, 0, ask("fake-worker", "relay: perm edit"), tx));
  until(|| nodes(&s).first().is_some_and(|n| n["permissions"].as_array().is_some_and(|p| p.len() == 1)), 8000).await;
  let block = nodes(&s)[0]["permissions"][0]["id"].as_str().unwrap().to_owned();
  // The root transcript holds no card: it belongs to the child
  assert!(!serde_json::to_string(&s.view().turns).unwrap().contains(&block));
  s.resolve_permission(&block, "yes");
  let mut last = None;
  while let Some(r) = tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap() {
    last = Some(r);
  }
  let Some(HubReply::Done(text)) = last else { panic!("{last:?}") };
  assert!(text.starts_with("reply to relay: perm edit (round 1, mode agent)"), "{text}");
  expect_match(&nodes(&s)[0], json!({ "state": "completed" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn unknown_personas_threads_and_depth_are_refused_with_text_the_model_can_act_on() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("nobody", "relay: x")).await);
  assert!(!ok && text.contains("fake-review, fake-worker"), "{text}");
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some("zzz".into()), ..ask("", "relay: x") }).await);
  assert!(!ok && text.contains("Unknown thread"), "{text}");
  let (ok, text) = last_text(&run(&s, &roster, acpira_host::relay::MAX_DEPTH, ask("fake-review", "relay: x")).await);
  assert!(!ok && text.contains("levels deep"), "{text}");
  assert!(nodes(&s).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hub_lists_enabled_personas_and_forwards_an_ask_to_the_session_its_grant_names() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let hub = RelayHub::start(roster.clone(), Arc::new(|_: &str| {})).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let token_of = |env: Value| {
    let names: Vec<&str> = env.as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    // Only the way back and the grant travel in the env: session, thread and depth stay with the hub
    assert_eq!(names, ["ACPIRA_RELAY", "ACPIRA_RELAY_TOKEN"]);
    env[1]["value"].as_str().unwrap().to_owned()
  };
  let root = token_of(hub.env(&s.0, None, 0));
  let deepest = token_of(hub.env(&s.0, Some("thr"), acpira_host::relay::MAX_DEPTH));
  let addr = hub.addr().to_owned();
  let replies = tokio::task::spawn_blocking(move || {
    let req = |token: &str, op| HubRequest { token: token.into(), op };
    let mut out = vec![];
    call(&addr, &req(&root, HubOp::List), |r| {
      out.push(r);
      false
    })
    .unwrap();
    call(&addr, &req(&root, HubOp::Ask(ask("fake-review", "relay: via hub"))), |r| {
      let done = !matches!(r, HubReply::Progress(_));
      out.push(r);
      !done
    })
    .unwrap();
    // A grant at the depth limit cannot summon, whatever the request says
    call(&addr, &req(&deepest, HubOp::Ask(ask("fake-review", "relay: deeper"))), |r| {
      out.push(r);
      false
    })
    .unwrap();
    // An unknown token gets no answer at all
    let mut refused = vec![];
    call(&addr, &req("nope", HubOp::List), |r| {
      refused.push(r);
      true
    })
    .unwrap();
    assert!(refused.is_empty());
    out
  })
  .await
  .unwrap();
  let HubReply::Personas(p) = &replies[0] else { panic!("{replies:?}") };
  assert_eq!(p.len(), 2);
  let Some(HubReply::Done(text)) = replies.iter().rev().nth(1) else { panic!("{replies:?}") };
  assert!(text.starts_with("reply to relay: via hub (round 1, mode plan)"), "{text}");
  let Some(HubReply::Error(text)) = replies.last() else { panic!("{replies:?}") };
  assert!(text.contains("levels deep"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_round_that_fails_after_its_deadline_is_collected_as_a_failure() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow fail") }).await);
  assert!(text.contains("is still working"), "{text}");
  let thread = thread_of(&text);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "") }).await);
  assert!(!ok && text.contains("failed") && text.contains("model overloaded"), "{text}");
  // Asked again once it is over: still the failure, never a "done" with no text
  let (ok, again) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "") }).await);
  assert!(!ok && again.contains("model overloaded"), "{again}");
  expect_match(&nodes(&s)[0], json!({ "state": "failed" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_switches_its_mode_per_round_and_one_round_at_a_time() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: first")).await);
  let thread = thread_of(&text);
  // A work round on a consult thread leaves the read-only mode, a consult round goes back to it
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), mode: Some("work".into()), ..ask("", "relay: write it") }).await);
  assert!(ok && text.starts_with("reply to relay: write it (round 2, mode agent)"), "{text}");
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: review it") }).await);
  assert!(text.starts_with("reply to relay: review it (round 3, mode plan)"), "{text}");
  // Two prompts on the thread at once: one round starts, the other is told to wait
  let a = run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: slow a") });
  let b = run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: slow b") });
  let (a, b) = tokio::join!(a, b);
  let (ok_a, ta) = last_text(&a);
  let (ok_b, tb) = last_text(&b);
  assert!(ok_a != ok_b, "{ta} / {tb}");
  assert!(ta.contains("still working") || tb.contains("still working"), "{ta} / {tb}");
  assert_eq!(nodes(&s).len(), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn running_rounds_are_capped_per_session() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  for i in 0..acpira_host::relay::MAX_RUNNING {
    let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", &format!("relay: slow {i}")) }).await);
    assert!(ok && text.contains("still working"), "{text}");
  }
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: one too many")).await);
  assert!(!ok && text.contains("already running"), "{text}");
  assert_eq!(nodes(&s).len(), acpira_host::relay::MAX_RUNNING);
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_summoned_node_reaches_its_process_and_closing_the_session_ends_every_child() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow cancel me") }).await);
  let thread = thread_of(&text);
  until(|| nodes(&s).first().is_some_and(|n| n["peer"]["sessionId"].is_string()), 8000).await;
  let id = nodes(&s)[0]["id"].as_str().unwrap().to_owned();
  s.cancel_subagent(&id).await;
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "") }).await);
  assert!(!ok && text.contains("was cancelled"), "{text}");
  expect_match(&nodes(&s)[0], json!({ "state": "cancelled" }));

  // A second child still running: the close hands it over with the agent's own process and ends both
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow left running") }).await);
  assert!(text.contains("still working"), "{text}");
  until(|| nodes(&s).get(1).is_some_and(|n| n["peer"]["sessionId"].is_string()), 8000).await;
  let (procs, closing) = s.0.shutdown();
  assert_eq!(procs.len(), 3, "the agent and both summoned threads' processes");
  if let Some(f) = closing {
    tokio::time::timeout(Duration::from_secs(10), f).await.expect("children end in time");
  }
  assert!(procs.iter().all(|p| !p.alive()));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_root_ask_agent_row_is_tied_to_its_node_and_hides_behind_it() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The row arrives before the hub hears of the call (the agent announces it, then runs it)
  prompt(&s, "relay-call:relay: tied").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: tied")).await);
  assert!(ok);
  let node = nodes(&s)[0].clone();
  assert_eq!(node["peer"]["toolCallId"], "ak1");
  let view = v(s.view());
  let row = view["turns"].as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["id"] == "ak1").unwrap();
  assert_eq!(row["subagentId"], node["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_row_whose_arguments_stream_in_without_its_name_still_ties_to_its_node() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // claude-agent-acp 0.83.0: the name only on the first, empty-input tool_call; the prompt arrives in title-less refinements
  prompt(&s, "relay-call-stream:relay: streamed").await;
  let view = v(s.view());
  let row = view["turns"].as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["id"] == "ak1").unwrap();
  // An empty input is not a wait call
  assert!(row["verbKey"].is_null());
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: streamed")).await);
  assert!(ok);
  let node = nodes(&s)[0].clone();
  assert_eq!(node["peer"]["toolCallId"], "ak1");
  let view = v(s.view());
  let row = view["turns"].as_array().unwrap().iter().flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default()).find(|b| b["id"] == "ak1").unwrap();
  assert_eq!(row["subagentId"], node["id"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_row_waits_for_the_round_of_its_own_persona() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The row names fake-review; a fake-worker round with the very same task must not take it
  prompt(&s, "relay-call:relay: same task").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: same task")).await);
  assert!(ok);
  assert!(nodes(&s)[0]["peer"]["toolCallId"].is_null());
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: same task")).await);
  assert!(ok);
  assert_eq!(nodes(&s)[1]["peer"]["toolCallId"], "ak1");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_call_that_ended_without_a_round_never_takes_its_retrys_node() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // ak1 failed before any round existed; ak2 is the retry with the same arguments
  prompt(&s, "relay-call-failed:relay: retried").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: retried")).await);
  assert!(ok);
  assert_eq!(nodes(&s)[0]["peer"]["toolCallId"], "ak2");
}

#[tokio::test(flavor = "multi_thread")]
async fn closing_while_a_child_is_still_starting_waits_for_it_to_end() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow closed early") }).await);
  assert!(text.contains("still working"), "{text}");
  // Closed before the child's process is even registered: the close still has it to wait for
  let (_, closing) = s.0.shutdown();
  let closing = closing.expect("a start in flight is waited for");
  tokio::time::timeout(Duration::from_secs(10), closing).await.expect("the start ends in time");
  // The close settled the node as disconnected; its round never got to run
  expect_match(&nodes(&s)[0], json!({ "state": "disconnected" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_summoned_node_cancels_the_rounds_its_cli_summoned_in_turn() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow parent") }).await);
  let parent = thread_of(&text);
  // The parent's CLI summons in turn: its grant names the parent's thread, one level deeper
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), Some(parent.clone()), 1, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow child") }, tx));
  while rx.recv().await.is_some() {}
  until(|| nodes(&s).len() == 2 && nodes(&s).iter().all(|n| n["peer"]["sessionId"].is_string()), 8000).await;
  let list = nodes(&s);
  assert_eq!(list[1]["parentId"], list[0]["id"]);
  s.cancel_subagent(list[0]["id"].as_str().unwrap()).await;
  until(|| nodes(&s).iter().all(|n| n["state"] == "cancelled"), 8000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_whose_session_cannot_be_reopened_fails_instead_of_starting_over() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The child's process exits after its reply; the fake keeps no session store, so neither resume nor load finds it
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: exit after this")).await);
  assert!(ok, "{text}");
  let thread = thread_of(&text);
  tokio::time::sleep(Duration::from_millis(400)).await;
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "relay: and then?") }).await);
  assert!(!ok && text.contains("could not be reopened"), "{text}");
  expect_match(&nodes(&s)[1], json!({ "state": "failed" }));
}
