//! One summoned round and its thread: processes, deadlines, permissions, modes, caps, cancellation, edits and teardown

use super::*;

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
    "task": "relay: look at the diff", "toolCount": 1, "model": "Model 1",
    // The model and effort the child session really runs, by the CLI's option names
    "harness": { "agent": "fake", "persona": "fake-review", "mode": "consult", "thread": thread, "round": 1, "effort": "High" },
  }));
  assert!(list[0]["result"].as_str().unwrap().contains("(round 1, mode plan)"));
  // The child received the persona's brief and the consult rule along with the prompt; its transcript is its own
  let (turns, _, running) = s.subagent_transcript(list[0]["id"].as_str().unwrap()).unwrap();
  assert!(!running);
  assert!(turns.0.get().contains("reply to relay: look at the diff"));
  // The turn carries its usage like a root turn, so the inspector's stats card names the model it ran on
  let turns: Value = serde_json::from_str(turns.0.get()).unwrap();
  assert_eq!(turns.as_array().unwrap().last().unwrap()["usage"]["model"], "Model 1");

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
async fn a_bypass_round_answers_its_childs_permission_without_a_card_whatever_the_call_asks() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster.save(&json!([{ "name": "Fake Hand", "agent": "fake", "mode": "bypass", "when": "edits" }])).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The fake agent has no full-access mode: the host's answer alone carries the round
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-hand", "relay: perm edit")).await);
  assert!(ok && text.starts_with("reply to relay: perm edit (round 1, mode agent)"), "{text}");
  expect_match(&nodes(&s)[0], json!({ "state": "completed", "harness": { "mode": "bypass" } }));
  let thread = thread_of(&text);
  // The call's mode does not touch a fixed persona: `work` and `consult` both stay bypass, the edit allowed without a card
  for (round, asked) in [(2, "work"), (3, "consult")] {
    let prompt = format!("relay: perm {asked}");
    let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), mode: Some(asked.into()), ..ask("", &prompt) }).await);
    assert!(ok && text.starts_with(&format!("reply to {prompt} (round {round}, mode agent)")), "{text}");
    expect_match(&nodes(&s)[round - 1], json!({ "state": "completed", "harness": { "mode": "bypass" } }));
    assert!(nodes(&s)[round - 1]["permissions"].as_array().is_none_or(|p| p.is_empty()), "no card on round {round}");
  }
}

// A CLI that sits in full access and refuses to leave it must not get the prompt of a round that asked for approvals
#[tokio::test(flavor = "multi_thread")]
async fn a_round_is_not_sent_when_the_cli_refuses_to_leave_full_access() {
  let fake = fake_or_skip!();
  // A refused switch, and a CLI whose only mode is full access (nothing to switch to)
  for fixture in ["1", "only"] {
    let h = Harness::new(&fake, json!({ "env": { "FAKE_BYPASS_START": fixture } }));
    let roster = Arc::new(Roster::new(h.dir.path()));
    roster.save(&json!([{ "name": "Fake Hand", "agent": "fake", "mode": "work", "when": "edits" }])).await.unwrap();
    let s = Disposing(h.session("/tmp"));
    s.start().await;
    let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-hand", "relay: perm edit")).await);
    assert!(!ok && text.contains("full-access mode"), "{fixture}: {text}");
    assert!(!text.contains("reply to relay"), "{fixture}: the prompt went out: {text}");
  }
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
  roster.save(&json!([{ "name": "Fake Twin", "agent": "fake", "mode": "inherit", "when": "anything" }])).await.unwrap();
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { mode: Some("consult".into()), ..ask("fake-twin", "relay: first") }).await);
  assert!(text.starts_with("reply to relay: first (round 1, mode plan)"), "{text}");
  let thread = thread_of(&text);
  // A work round on a consult thread leaves the read-only mode, a consult round goes back to it
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), mode: Some("work".into()), ..ask("", "relay: write it") }).await);
  assert!(ok && text.starts_with("reply to relay: write it (round 2, mode agent)"), "{text}");
  let (_, text) =
    last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), mode: Some("consult".into()), ..ask("", "relay: review it") }).await);
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
async fn ask_agent_stop_cancels_the_running_round_and_leaves_the_thread_open() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: slow stop me") }).await);
  let thread = thread_of(&text);
  until(|| nodes(&s).first().is_some_and(|n| n["peer"]["sessionId"].is_string()), 8000).await;
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), stop: Some(true), ..ask("", "") }).await);
  assert!(ok && text.contains("was cancelled") && text.contains("stays open"), "{text}");
  expect_match(&nodes(&s)[0], json!({ "state": "cancelled" }));
  // Nothing left to stop, and the thread takes a new prompt
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), stop: Some(true), ..ask("", "") }).await);
  assert!(ok && text.contains("nothing to stop"), "{text}");
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "relay: and now?") }).await);
  assert!(ok, "{text}");
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { stop: Some(true), ..ask("", "") }).await);
  assert!(!ok && text.contains("needs the `thread`"), "{text}");
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

#[tokio::test(flavor = "multi_thread")]
async fn an_inherit_persona_follows_the_callers_mode_and_a_call_overrides_it() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster.save(&json!([{ "name": "Fake Twin", "agent": "fake", "mode": "inherit", "when": "anything" }])).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The root runs its ordinary mode: the child works
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-twin", "relay: first")).await);
  assert!(ok && text.starts_with("reply to relay: first (round 1, mode agent)"), "{text}");
  assert_eq!(nodes(&s)[0]["harness"]["mode"], "work");
  let thread = thread_of(&text);
  // The root in plan mode: a round without a mode is a consult
  s.set_mode("plan".into()).await.unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "relay: look") }).await);
  assert!(ok && text.starts_with("reply to relay: look (round 2, mode plan)"), "{text}");
  assert_eq!(nodes(&s)[1]["harness"]["mode"], "consult");
  // The call's mode overrides the caller's access: work under a plan-mode root
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), mode: Some("work".into()), ..ask("", "relay: write") }).await);
  assert!(ok && text.starts_with("reply to relay: write (round 3, mode agent)"), "{text}");
  assert_eq!(nodes(&s)[2]["harness"]["mode"], "work");
  // and a consult under an ordinary one
  s.set_mode("agent".into()).await.unwrap();
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), mode: Some("consult".into()), ..ask("", "relay: look again") }).await);
  assert!(text.starts_with("reply to relay: look again (round 4, mode plan)"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_yolo_root_leaves_a_summoned_childs_approval_to_its_own_mode() {
  let fake = fake_or_skip!();
  let modes = json!([{ "id": "agent", "name": "Agent" }, { "id": "plan", "name": "Plan" }, { "id": "yolo", "name": "YOLO" }]);
  let h = Harness::new(&fake, json!({ "modes": modes }));
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster
    .save(&json!([
      { "name": "Fake Review", "agent": "fake", "mode": "consult", "when": "opinions" },
      { "name": "Fake Twin", "agent": "fake", "mode": "inherit", "when": "anything" },
    ]))
    .await
    .unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  s.set_mode("yolo".into()).await.unwrap();
  // A consult child under the yolo root still asks
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), None, 0, ask("fake-review", "relay: perm edit"), tx));
  until(|| nodes(&s).first().is_some_and(|n| n["permissions"].as_array().is_some_and(|p| p.len() == 1)), 8000).await;
  let block = nodes(&s)[0]["permissions"][0]["id"].as_str().unwrap().to_owned();
  s.resolve_permission(&block, "no");
  while tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().is_some() {}
  // An inherit child takes the yolo as bypass: answered without a card
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-twin", "relay: perm edit")).await);
  assert!(ok, "{text}");
  expect_match(&nodes(&s)[1], json!({ "state": "completed", "harness": { "mode": "bypass" } }));
  // A consult child asking to leave its mode (Claude Code's ExitPlanMode) is declined without a card
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: leaveplan")).await);
  assert!(ok && text.contains("leave=no"), "{text}");
  assert!(nodes(&s)[2]["permissions"].as_array().is_none_or(Vec::is_empty), "{:?}", nodes(&s)[2]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_childs_card_and_round_outlive_the_root_turn_that_left_it_running() {
  use acpira_shared::transcript::SummaryState;
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // The call answers before the round does (a client with a short tool timeout), and the child asks afterwards
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-worker", "relay: perm slow edit") }).await);
  let thread = thread_of(&text);
  until(|| nodes(&s).first().is_some_and(|n| n["permissions"].as_array().is_some_and(|p| p.len() == 1)), 8000).await;
  // The root turn ends meanwhile: the child's card stays, and the list says the session waits on the user
  prompt(&s, "say:handed it off").await;
  assert_eq!(nodes(&s)[0]["permissions"].as_array().map(Vec::len), Some(1));
  assert_eq!(s.list_marks(), (Some(SummaryState::Waiting), false));
  let block = nodes(&s)[0]["permissions"][0]["id"].as_str().unwrap().to_owned();
  s.resolve_permission(&block, "yes");
  // Still working with the root idle: background, not idle
  until(|| s.list_state() == Some(SummaryState::Background), 4000).await;
  assert!(s.background_running() && !s.is_running());
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "") }).await);
  assert!(ok && text.starts_with("reply to relay: perm slow edit"), "{text}");
  assert_eq!(s.list_state(), None);
  assert!(!s.background_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn editing_history_cancels_the_summoned_rounds_of_the_removed_turns() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  prompt(&s, "say:first").await;
  // A round of the second turn, left running when that turn ended
  prompt(&s, "relay-call:relay: slow orphan").await;
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-review", "relay: slow orphan") }).await);
  let thread = thread_of(&text);
  until(|| nodes(&s).first().is_some_and(|n| n["peer"]["sessionId"].is_string()), 8000).await;
  // Editing the second prompt takes its turn, and the round with it
  s.edit_turn(crate::acp_session::history_edit(&s, 2, "say:instead")).await.unwrap();
  assert!(nodes(&s).is_empty());
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "") }).await);
  assert!(!ok && text.contains("Unknown thread"), "{text}");
  // The cancelled round winds down by itself
  until(|| !s.background_running(), 4000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_round_still_connecting_when_its_turn_is_edited_away_never_sends_its_task() {
  let fake = fake_or_skip!();
  // The persona's model switch keeps the child connecting long enough for the edit to land first
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CONFIG_DELAY_MS": "1500" } }));
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster.save(&json!([{ "name": "Fake Slow", "agent": "fake", "mode": "work", "when": "edits", "model": "m2" }])).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  prompt(&s, "say:first").await;
  prompt(&s, "relay-call:relay: orphan").await;
  let target = h.dir.path().join("written-by-orphan.txt");
  let task = format!("relay: write={}", target.display());
  last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(0), ..ask("fake-slow", &task) }).await);
  assert_eq!(nodes(&s).len(), 1);
  s.edit_turn(crate::acp_session::history_edit(&s, 2, "say:instead")).await.unwrap();
  assert!(nodes(&s).is_empty());
  // The child finishes connecting after its node is gone: the round ends there, the task never reaches it
  until(|| !s.background_running(), 8000).await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert!(!target.exists(), "the edited-away round still ran its task");
}
