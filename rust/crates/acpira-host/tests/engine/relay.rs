//! Cross-harness subagents (acp/session/relay.rs, relay/): a session summons a persona whose CLI is the fake agent, through
//! `relay_ask` directly and through the loopback hub the MCP server uses

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use acpira_host::relay::hub::{HostBrowser, RelayHub, call};
use acpira_shared::sidecar::{BrowserOpened, BrowserReuse};
use acpira_host::relay::roster::Roster;
use acpira_host::relay::wire::{AskArgs, AskCallerArgs, HubOp, HubReply, HubRequest};

use crate::acp_session::{agent_blocks, last_turn, prompt, view};
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

/// A call that waits for the reply (the root's default is to send the task off)
fn ask(agent: &str, prompt: &str) -> AskArgs {
  AskArgs { agent: agent.into(), prompt: prompt.into(), wait_secs: Some(20), wait: Some(true), ..Default::default() }
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
    "task": "relay: look at the diff", "toolCount": 1, "model": "Model 1",
    // The model and effort the child session really runs, by the CLI's option names
    "harness": { "agent": "fake", "persona": "fake-review", "mode": "consult", "thread": thread, "round": 1, "effort": "High" },
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
async fn a_bypass_round_answers_its_childs_permission_without_a_card_and_a_consult_round_shows_one_again() {
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
  // `work` from the caller keeps the persona's bypass; the edit is still allowed without a card
  let (ok, text) =
    last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), mode: Some("work".into()), ..ask("", "relay: perm again") }).await);
  assert!(ok && text.starts_with("reply to relay: perm again (round 2, mode agent)"), "{text}");
  assert_eq!(nodes(&s)[1]["harness"]["mode"], "bypass");
  // A consult round on the same thread asks again
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  let args = AskArgs { thread: Some(thread), mode: Some("consult".into()), ..ask("", "relay: perm look") };
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), None, 0, args, tx));
  until(|| nodes(&s).get(2).is_some_and(|n| n["permissions"].as_array().is_some_and(|p| p.len() == 1)), 8000).await;
  let block = nodes(&s)[2]["permissions"][0]["id"].as_str().unwrap().to_owned();
  s.resolve_permission(&block, "no");
  while tokio::time::timeout(Duration::from_secs(10), rx.recv()).await.unwrap().is_some() {}
  expect_match(&nodes(&s)[2], json!({ "state": "completed", "harness": { "mode": "consult" } }));
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

/// A shell that shows pages: every URL it is asked for, and the forwarded address it answers with
struct FakeBrowser {
  can: bool,
  opened: parking_lot::Mutex<Vec<String>>,
}

impl HostBrowser for FakeBrowser {
  fn available(&self) -> bool {
    self.can
  }
  fn open<'a>(&'a self, url: &'a str) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<BrowserOpened>> + Send + 'a>> {
    Box::pin(async move {
      // The second request for one URL finds the tab the first one opened
      let reused = self.opened.lock().iter().any(|u| u == url).then_some(BrowserReuse::Revealed);
      self.opened.lock().push(url.to_owned());
      if url.contains("unreachable") { anyhow::bail!("it did not answer from the user's machine") }
      Ok(BrowserOpened { url: url.replace(":1/", ":2/"), reused })
    })
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn open_browser_goes_through_the_hub_to_the_shell_once_the_page_is_served_here() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let hub = RelayHub::start(roster(h.dir.path()).await, Arc::new(|_: &str| {})).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let token = hub.env(&s.0, None, 0)[1]["value"].as_str().unwrap().to_owned();
  let addr = hub.addr().to_owned();
  let one = move |op: HubOp| {
    let (addr, token) = (addr.clone(), token.clone());
    let mut out = None;
    call(&addr, &HubRequest { token, op }, |r| {
      out = Some(r);
      false
    })
    .unwrap();
    out.expect("one reply")
  };
  let ask = |op: HubOp| {
    let one = one.clone();
    tokio::task::spawn_blocking(move || one(op))
  };
  // No shell platform yet: not offered, and a call says why
  let HubReply::Features(f) = ask(HubOp::Features).await.unwrap() else { panic!() };
  assert!(!f.browser);
  let HubReply::Error(text) = ask(HubOp::OpenBrowser { url: "https://example.com/".into() }).await.unwrap() else { panic!() };
  assert!(text.contains("cannot show web pages"), "{text}");

  let browser = Arc::new(FakeBrowser { can: true, opened: Default::default() });
  hub.set_browser(browser.clone());
  let HubReply::Features(f) = ask(HubOp::Features).await.unwrap() else { panic!() };
  assert!(f.browser);
  // A loopback page nobody serves here is refused before the shell is asked
  let closed = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
  let HubReply::Error(text) = ask(HubOp::OpenBrowser { url: format!("http://localhost:{closed}/") }).await.unwrap() else { panic!() };
  assert!(text.contains(&format!("Nothing is listening on localhost:{closed}")), "{text}");
  assert!(browser.opened.lock().is_empty());
  // Served here: the shell opens it and its forwarded address is reported
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let port = listener.local_addr().unwrap().port();
  tokio::spawn(async move { while listener.accept().await.is_ok() {} });
  let url = format!("http://127.0.0.1:{port}/:1/");
  let HubReply::Done(text) = ask(HubOp::OpenBrowser { url: url.clone() }).await.unwrap() else { panic!() };
  assert!(text.contains(&format!("loads it as http://127.0.0.1:{port}/:2/")), "{text}");
  // Asked again for the same page: the tab is brought back, and the model is told it was not reloaded
  let HubReply::Done(text) = ask(HubOp::OpenBrowser { url: url.clone() }).await.unwrap() else { panic!() };
  assert!(text.contains("without reloading"), "{text}");
  // The shell's failure (the page does not answer from the user's side) reaches the model
  let HubReply::Error(text) = ask(HubOp::OpenBrowser { url: format!("http://127.0.0.1:{port}/unreachable") }).await.unwrap() else { panic!() };
  assert!(text.contains("did not answer from the user's machine"), "{text}");
  // Other hosts are the shell's to check
  let HubReply::Done(_) = ask(HubOp::OpenBrowser { url: "https://example.com/".into() }).await.unwrap() else { panic!() };
  assert_eq!(browser.opened.lock().len(), 4);
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
async fn a_root_row_named_before_any_input_still_ties_to_its_node() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  // claude-agent-acp 0.83.0 to an AIR client: the name without rawInput, then the whole input without the name
  prompt(&s, "relay-call-air:relay: air").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: air")).await);
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

#[tokio::test(flavor = "multi_thread")]
async fn an_inherit_persona_follows_the_callers_mode_and_a_call_can_only_narrow_it() {
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
  // The root in plan mode: the next round is a consult, and asking for work does not widen it
  s.set_mode("plan".into()).await.unwrap();
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), mode: Some("work".into()), ..ask("", "relay: write") }).await);
  assert!(ok && text.starts_with("reply to relay: write (round 2, mode plan)"), "{text}");
  assert_eq!(nodes(&s)[1]["harness"]["mode"], "consult");
  // Back in the ordinary mode, a consult still narrows it
  s.set_mode("agent".into()).await.unwrap();
  let (_, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), mode: Some("consult".into()), ..ask("", "relay: look") }).await);
  assert!(text.starts_with("reply to relay: look (round 3, mode plan)"), "{text}");
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
  assert_eq!(s.list_state(), Some(SummaryState::Waiting));
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

#[tokio::test(flavor = "multi_thread")]
async fn a_row_whose_arguments_come_with_its_end_ties_even_while_another_call_waits() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  prompt(&s, "relay-call-late:relay: late").await;
  let (ok, _) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: late")).await);
  assert!(ok);
  assert_eq!(nodes(&s)[0]["peer"]["toolCallId"], "ak1");
}

/// The automatic user turns delivering summoned agents' replies
fn deliveries(s: &Disposing) -> Vec<Value> {
  v(s.view())["turns"].as_array().unwrap().iter().filter(|t| t["autoReason"] == "subagents").cloned().collect()
}

fn send_off(agent: &str, prompt: &str) -> AskArgs {
  AskArgs { agent: agent.into(), prompt: prompt.into(), ..Default::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_root_call_returns_at_once_and_its_turns_replies_arrive_together_in_one_message() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, send_off("fake-review", "relay: quick look")).await);
  assert!(ok && text.starts_with("Sent to Fake Review (fake) (thread: ") && text.contains("Do not poll"), "{text}");
  let (_, text) = last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: slow change")).await);
  let slow = thread_of(&text);
  // The quick one ended first, but its turn's batch is still running: nothing is delivered yet
  until(|| nodes(&s).first().is_some_and(|n| n["state"] == "completed"), 8000).await;
  assert!(deliveries(&s).is_empty());
  until(|| deliveries(&s).len() == 1 && !s.is_running(), 8000).await;
  let text = deliveries(&s)[0]["text"].as_str().unwrap().to_owned();
  assert!(text.contains("2 summoned agents") && text.contains("reply to relay: quick look") && text.contains("reply to relay: slow change"), "{text}");
  // Reading a delivered round again is allowed and delivers nothing more
  let (ok, _) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(slow), ..ask("", "") }).await);
  assert!(ok);
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(deliveries(&s).len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn retrying_a_delivery_that_failed_before_any_output_sends_the_same_replies_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  // The persona's name puts "fail" in the delivery, which the fake root fails once before any output
  let roster = Arc::new(Roster::new(h.dir.path()));
  roster.save(&json!([{ "name": "Fake failover", "agent": "fake", "mode": "consult", "when": "checks" }])).await.unwrap();
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  last_text(&run(&s, &roster, 0, send_off("fake-failover", "relay: quick look")).await);
  until(|| deliveries(&s).len() == 1 && !s.is_running() && !last_turn(&view(&s))["stop"].is_null(), 8000).await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "error" }));
  s.retry_turn().await.unwrap();
  until(|| !s.is_running() && last_turn(&view(&s))["stop"] == "end_turn", 8000).await;
  // The failed attempt is replaced by the resent message, still an automatic delivery with the same replies
  let sent = deliveries(&s);
  assert_eq!(sent.len(), 1, "{sent:?}");
  assert!(sent[0]["text"].as_str().unwrap().contains("reply to relay: quick look"), "{}", sent[0]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_a_call_already_read_is_not_delivered_again() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: slow job")).await);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread_of(&text)), ..ask("", "") }).await);
  assert!(ok && text.starts_with("reply to relay: slow job"), "{text}");
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert!(deliveries(&s).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_stop_voids_what_was_owed() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: slow job")).await);
  until(|| nodes(&s).first().is_some_and(|n| n["peer"]["sessionId"].is_string()), 8000).await;
  s.cancel().await;
  until(|| !s.background_running(), 4000).await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert!(deliveries(&s).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn automatic_deliveries_stop_after_six_per_prompt_of_the_users() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  for i in 1..=6 {
    last_text(&run(&s, &roster, 0, send_off("fake-review", &format!("relay: look {i}"))).await);
    until(|| deliveries(&s).len() == i && !s.is_running(), 8000).await;
  }
  let (_, text) = last_text(&run(&s, &roster, 0, send_off("fake-review", "relay: look 7")).await);
  until(|| nodes(&s).last().is_some_and(|n| n["state"] == "completed"), 8000).await;
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(deliveries(&s).len(), 6);
  // The user's next prompt opens the count again: the seventh reply follows that turn
  prompt(&s, "say:what now").await;
  until(|| deliveries(&s).len() == 7, 8000).await;
  assert!(deliveries(&s)[6]["text"].as_str().unwrap().contains(&thread_of(&text)));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_call_answers_with_the_childs_question_and_the_caller_answers_it_by_label() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: askme which")).await);
  assert!(ok && text.contains("is waiting for an answer") && text.contains("1. Colour: Which colour?") && text.contains("- Blue: calm"), "{text}");
  let thread = thread_of(&text);
  // The user's card says the caller was asked as well
  let card = nodes(&s)[0]["question"].clone();
  assert!(card["message"].as_str().is_some_and(|m| m.contains("summoned it is asked too")), "{card}");
  // Waiting again answers with the same question at once
  let (_, again) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), ..ask("", "") }).await);
  assert!(again.contains("1. Colour: Which colour?"), "{again}");
  // A label in any case picks the option
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread.clone()), answers: Some(vec!["green".into()]), ..ask("", "") }).await);
  assert!(ok && text.contains(r#""content":{"q0":"Green"}"#), "{text}");
  // Nothing is open any more: another answer is refused
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), answers: Some(vec!["blue".into()]), ..ask("", "") }).await);
  assert!(!ok, "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_single_question_takes_the_prompt_as_its_answer_and_the_users_card_counts_first() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: askme one")).await);
  let thread = thread_of(&text);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), ..ask("", "Blue") }).await);
  assert!(ok && text.contains(r#""q0":"Blue""#), "{text}");
  // The user answers first: the caller is told the question is gone
  let (_, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", "relay: askme two")).await);
  let thread = thread_of(&text);
  let card = nodes(&s)[1]["question"].clone();
  let qid = card["questions"][0]["id"].as_str().unwrap().to_owned();
  let mut answers = serde_json::Map::new();
  answers.insert(qid, json!("Green"));
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread), answers: Some(vec!["Blue".into()]), ..ask("", "") }).await);
  assert!(!ok && text.contains("no open question"), "{text}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_root_hears_a_childs_question_once_and_then_its_reply() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  last_text(&run(&s, &roster, 0, send_off("fake-worker", "relay: askme later")).await);
  until(|| deliveries(&s).len() == 1 && !s.is_running(), 8000).await;
  let text = deliveries(&s)[0]["text"].as_str().unwrap().to_owned();
  assert!(text.contains("## Fake Worker (fake) asks") && text.contains("Which colour?"), "{text}");
  // Answered on the card: the child finishes, and its reply is the next delivery (the question is not repeated)
  let card = nodes(&s)[0]["question"].clone();
  let mut answers = serde_json::Map::new();
  answers.insert(card["questions"][0]["id"].as_str().unwrap().to_owned(), json!("Blue"));
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  until(|| deliveries(&s).len() == 2 && !s.is_running(), 8000).await;
  let text = deliveries(&s)[1]["text"].as_str().unwrap().to_owned();
  assert!(text.contains("reply to relay: askme later") && !text.contains("asks"), "{text}");
}

/// The node's open question card once it has one
async fn question_card(s: &Disposing, i: usize) -> Value {
  until(|| nodes(s).get(i).is_some_and(|n| n["question"].is_object()), 8000).await;
  nodes(s)[i]["question"].clone()
}

/// A summoned child held on a permission card (its round stays running), and the reply lines of one `ask_caller` from it
async fn ask_caller_from_held_child(s: &Disposing, roster: &Arc<Roster>, wait_secs: u64) -> tokio::sync::mpsc::UnboundedReceiver<HubReply> {
  let (tx, _) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask(roster.clone(), None, 0, send_off("fake-worker", "relay: perm hold"), tx));
  until(|| nodes(s).first().is_some_and(|n| n["permissions"].as_array().is_some_and(|p| p.len() == 1)), 8000).await;
  let thread = nodes(s)[0]["harness"]["thread"].as_str().unwrap().to_owned();
  let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
  let args = AskCallerArgs { question: "Tabs or spaces?".into(), options: vec!["Tabs".into(), "Spaces".into(), "Tabs".into()], wait_secs: Some(wait_secs) };
  tokio::spawn(s.0.clone().relay_ask_caller(thread, args, tx));
  rx
}

async fn final_of(rx: &mut tokio::sync::mpsc::UnboundedReceiver<HubReply>) -> HubReply {
  loop {
    match tokio::time::timeout(Duration::from_secs(20), rx.recv()).await.expect("reply in time") {
      Some(HubReply::Progress(_)) => continue,
      Some(r) => return r,
      None => panic!("closed without a reply"),
    }
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_childs_ask_caller_goes_through_acpira_mcp_and_the_hub_to_the_waiting_caller_and_back() {
  let fake = fake_or_skip!();
  let (_h, s, roster) = hub_session(&fake).await;
  let call = r#"mcp-call:ask_caller {"question":"Tabs or spaces?","options":["Tabs","Spaces"]}"#;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", call)).await);
  assert!(ok && text.contains("is waiting for an answer") && text.contains("1. Tabs or spaces?") && text.contains("   - Spaces"), "{text}");
  let card = nodes(&s)[0]["question"].clone();
  assert!(card["message"].as_str().is_some_and(|m| m.contains("summoned it is asked too")), "{card}");
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { thread: Some(thread_of(&text)), answers: Some(vec!["spaces".into()]), ..ask("", "") }).await);
  assert!(ok && text.contains("Answer: Spaces") && text.contains(r#""ask_caller""#), "{text}");
  expect_match(&nodes(&s)[0], json!({ "state": "completed" }));
  // The root's own server has no caller: the tool is not listed, and a call anyway is refused
  // The question went to the waiting call only: no automatic message told it again
  assert!(deliveries(&s).is_empty(), "{:?}", deliveries(&s));
  prompt(&s, r#"mcp-call:ask_caller {"question":"x"}"#).await;
  until(|| !s.is_running(), 8000).await;
  let text = agent_blocks(&view(&s)).into_iter().filter(|b| b["type"] == "text").filter_map(|b| b["markdown"].as_str().map(str::to_owned)).next_back().unwrap_or_else(|| panic!("{}", view(&s)["turns"]));
  let reply: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"));
  assert!(!reply["tools"].as_array().unwrap().iter().any(|t| t == "ask_caller"), "{reply}");
  assert!(reply["result"]["content"][0]["text"].as_str().is_some_and(|t| t.contains("Only a summoned agent")), "{reply}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_ask_caller_the_user_answers_on_the_card_reaches_the_child_and_one_nobody_answers_is_skipped() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  let mut rx = ask_caller_from_held_child(&s, &roster, 20).await;
  let card = question_card(&s, 0).await;
  // Duplicate options are offered once
  let labels: Vec<&str> = card["questions"][0]["options"].as_array().unwrap().iter().map(|o| o["label"].as_str().unwrap()).collect();
  assert_eq!(labels, ["Tabs", "Spaces"]);
  let mut answers = serde_json::Map::new();
  answers.insert(card["questions"][0]["id"].as_str().unwrap().to_owned(), json!("Tabs"));
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  let HubReply::Done(text) = final_of(&mut rx).await else { panic!() };
  assert_eq!(text, "Answer: Tabs");
  // Asked again with a short bound and left alone: the call answers that no answer came, and the card is settled skipped
  let thread = nodes(&s)[0]["harness"]["thread"].as_str().unwrap().to_owned();
  let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
  tokio::spawn(s.0.clone().relay_ask_caller(thread, AskCallerArgs { question: "Which file?".into(), wait_secs: Some(1), ..Default::default() }, tx));
  let HubReply::Done(text) = final_of(&mut rx).await else { panic!() };
  assert!(text.starts_with("No answer came in time"), "{text}");
  until(|| nodes(&s)[0]["question"].is_null(), 4000).await;
  let id = nodes(&s)[0]["id"].as_str().unwrap().to_owned();
  let turns = serde_json::to_string(&s.subagent_transcript(&id).unwrap().0).unwrap();
  assert!(turns.contains(r#""outcome":"skipped""#), "{turns}");
  let block = nodes(&s)[0]["permissions"][0]["id"].as_str().unwrap().to_owned();
  s.resolve_permission(&block, "no");
  until(|| !s.background_running(), 8000).await;
}

/// A session whose agents get Acpira's MCP entry with a live hub, so a fake CLI's `mcp-call:` reaches the relay for real
/// (the harness first, so it outlives the session when the bindings drop)
async fn hub_session(fake: &crate::support::FakeAgent) -> (Harness, Disposing, Arc<Roster>) {
  let mut h = Harness::new(fake, json!({}));
  let roster = roster(h.dir.path()).await;
  let hub = RelayHub::start(roster.clone(), Arc::new(|_: &str| {})).await.unwrap();
  h.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub));
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  (h, s, roster)
}

fn node_turns(s: &Disposing, id: &str) -> Vec<Value> {
  let (turns, ..) = s.subagent_transcript(id).unwrap();
  serde_json::from_str::<Value>(turns.get()).unwrap().as_array().cloned().unwrap_or_default()
}

fn node_of(s: &Disposing, persona: &str) -> Value {
  nodes(s).into_iter().find(|n| n["harness"]["persona"] == persona).unwrap_or_else(|| panic!("no {persona} node"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_childs_sent_off_round_comes_back_to_it_in_a_follow_up_before_its_reply_reaches_the_caller() {
  let fake = fake_or_skip!();
  let (_h, s, roster) = hub_session(&fake).await;
  let call = r#"mcp-call:ask_agent {"agent":"fake-review","prompt":"relay: grand"}"#;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-worker", call)).await);
  // The child's answer after the follow-up is the round's reply (the fake's default script)
  assert!(ok && text.starts_with("hello world"), "{text}");
  let child = node_of(&s, "fake-worker");
  let grand = node_of(&s, "fake-review");
  assert_eq!(grand["parentId"], child["id"]);
  assert_eq!(grand["state"], "completed");
  // The receipt, the sealed answer, the automatic message with the grandchild's reply, then the answer to it
  let turns = node_turns(&s, child["id"].as_str().unwrap());
  assert_eq!(turns.len(), 3, "{turns:?}");
  assert!(serde_json::to_string(&turns[0]).unwrap().contains("Sent to Fake Review"), "{}", turns[0]);
  assert_eq!(turns[0]["stop"], "end_turn");
  expect_match(&turns[1], json!({ "role": "user", "auto": true, "autoReason": "subagents" }));
  assert!(turns[1]["text"].as_str().unwrap().contains("reply to relay: grand"), "{}", turns[1]);
  // The root is never handed the grandchild's result
  until(|| !s.background_running(), 8000).await;
  assert!(deliveries(&s).is_empty(), "{:?}", deliveries(&s));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_gets_at_most_six_follow_ups_per_round() {
  let fake = fake_or_skip!();
  let (_h, s, roster) = hub_session(&fake).await;
  // The child summons again on every follow-up, so only the cap ends its round
  let call = r#"mcp-call-loop:ask_agent {"agent":"fake-review","prompt":"relay: quick"}"#;
  let (ok, text) = last_text(&run(&s, &roster, 0, AskArgs { wait_secs: Some(60), ..ask("fake-worker", call) }).await);
  assert!(ok && text.contains("Sent to Fake Review"), "{text}");
  let child = node_of(&s, "fake-worker");
  let turns = node_turns(&s, child["id"].as_str().unwrap());
  let follow_ups = turns.iter().filter(|t| t["autoReason"] == "subagents").count();
  assert_eq!(follow_ups, 6, "{turns:?}");
  until(|| !s.background_running(), 8000).await;
  assert!(deliveries(&s).is_empty(), "{:?}", deliveries(&s));
}
