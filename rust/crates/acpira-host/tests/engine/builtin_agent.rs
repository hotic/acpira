//! The built-in agent end to end: the engine drives the real `acpira agent` binary against the scripted model server.
//! Eight scenarios (an edit behind a permission card, a Plan-mode approval, a question card, a cancel before the model's
//! first chunk, a reopen that resumes the session, the engine's own MCP server called over stdio, a message steered into
//! a command round, a goal the engine continues until the model marks it complete) each check that the request prefix
//! stays byte for byte between two model calls
//! unless the session log records a view change. The final transcripts are golden output in
//! test/fixtures/engine-builtin.json, which the webview suites render (ACPIRA_UPDATE_FIXTURES=1 rewrites it)

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_agent::mock::{self, MockModel, Reply};
use acpira_host::acp::agents::registry::AgentRegistry;
use acpira_host::acp::session::{AcpSession, SessionDeps};
use acpira_host::store::transcript_store::{LogFn, TranscriptStore};
use acpira_shared::transcript::GoalAction;

use crate::acp_session::{agent_blocks, drafts, find_block, last_turn, spawn_prompt, view, wait_block};
use crate::support::{Disposing, expect_match, until};

/// A data root holding one OpenAI-compatible source on the scripted server, a project directory, and the engine's
/// session dependencies with the built-in agent registered the way the runtime does it
struct Env {
  root: tempfile::TempDir,
  cwd: tempfile::TempDir,
  server: MockModel,
  deps: SessionDeps,
}

impl Env {
  fn new() -> Env {
    let root = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let server = MockModel::start();
    let providers = json!({ "version": 1, "providers": [{ "id": "mock", "name": "Mock", "format": "openai-chat", "baseUrl": server.base_url(),
      "models": [{ "id": "m1", "name": "Mock One", "context": 64000 }] }] });
    std::fs::write(root.path().join("providers.json"), providers.to_string()).unwrap();
    std::fs::write(root.path().join("secrets.json"), json!({ "acpira.provider.mock": "sk-test" }).to_string()).unwrap();
    let log: LogFn = Arc::new(|line: &str| eprintln!("host: {line}"));
    let registry = AgentRegistry::new(&json!({})).with_self_agent(env!("CARGO_BIN_EXE_acpira"), root.path());
    let deps = SessionDeps {
      registry: Arc::new(registry),
      log: log.clone(),
      on_change: Arc::new(|_, _| {}),
      blobs: TranscriptStore::new(root.path().join("sessions"), log, None),
      notify: None,
      accounts: None,
      compaction: None,
      plan_auto_approve: None,
      pool: None,
      model_shapes: None,
      shared_mcp: None,
      host_mcp: None,
      claim: None,
      remote: None,
    };
    Env { root, cwd, server, deps }
  }

  fn cwd(&self) -> PathBuf {
    self.cwd.path().to_owned()
  }

  async fn session(&self) -> Disposing {
    let s = Disposing(AcpSession::fresh("acpira", &self.cwd().to_string_lossy(), self.deps.clone(), None));
    s.start().await;
    s
  }

  /// The agent's own log of the session (`<root>/agent/sessions/<id>.jsonl`)
  fn log_of(&self, s: &AcpSession) -> PathBuf {
    let id = s.to_record().acp_session_id.expect("the agent named its session");
    self.root.path().join("agent/sessions").join(format!("{id}.jsonl"))
  }

  /// Requests and view changes are checked together: every model call has its record, and a call that does not
  /// start with the previous call's messages (or sends other tools or another model) follows a recorded view change
  fn assert_prefix_stable(&self, log: &Path) {
    let events: Vec<Value> = std::fs::read_to_string(log).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let mut changed = vec![];
    let mut since = false;
    for e in &events {
      match e["type"].as_str() {
        Some("view") => since = true,
        Some("request") => changed.push(std::mem::take(&mut since)),
        _ => {}
      }
    }
    let reqs = self.server.requests();
    assert_eq!(changed.len(), reqs.len(), "one request record per model call\n{}", lines(&events));
    for (i, pair) in reqs.windows(2).enumerate() {
      let (a, b) = (&pair[0].body, &pair[1].body);
      let (am, bm) = (a["messages"].as_array().unwrap(), b["messages"].as_array().unwrap());
      let stable = bm.len() > am.len()
        && am.iter().zip(bm).all(|(x, y)| x == y)
        && a["tools"] == b["tools"]
        && a["model"] == b["model"];
      assert!(stable || changed[i + 1], "call {} rewrote the prefix without a recorded view change\n{}", i + 1, lines(&events));
    }
  }
}

/// Scenarios run side by side and share the fixture file
static FIXTURE: std::sync::Mutex<()> = std::sync::Mutex::new(());
const FIXTURE_FILE: &str = "engine-builtin.json";

impl Env {
  /// The transcript as the page would get it, with what changes from run to run made fixed: the data root, the project
  /// and the agent's session id become `/acpira`, `/repo` and `SESSION`, user turn ids are numbered, times dropped.
  /// Unix only: Windows paths would not match the recorded ones
  fn golden(&self, s: &AcpSession, name: &str, turns: &Value) {
    if !cfg!(unix) {
      return;
    }
    let mut text = turns.to_string();
    let id = s.to_record().acp_session_id.unwrap_or_default();
    for (dir, to) in [(self.root.path(), "/acpira"), (self.cwd.path(), "/repo")] {
      // The canonical form first: on macOS /private/var/… holds /var/… as a suffix
      for form in [dir.canonicalize().unwrap(), dir.to_owned()] {
        text = text.replace(&*form.to_string_lossy(), to);
      }
    }
    text = text.replace(&id, "SESSION");
    let mut turns: Value = serde_json::from_str(&text).unwrap();
    fn fix(v: &mut Value, users: &mut usize) {
      match v {
        Value::Object(m) => {
          m.remove("startedAt");
          m.remove("endedAt");
          m.remove("respondedAt");
          // A steer block carries its queue entry's random id; it is numbered with the user turns
          let user = m.get("role").and_then(Value::as_str) == Some("user") || m.get("type").and_then(Value::as_str) == Some("steer");
          if user && m.contains_key("id") {
            *users += 1;
            m.insert("id".into(), json!(format!("user-{users}")));
          }
          m.values_mut().for_each(|x| fix(x, users));
        }
        Value::Array(a) => a.iter_mut().for_each(|x| fix(x, users)),
        _ => {}
      }
    }
    fix(&mut turns, &mut 0);
    let _guard = FIXTURE.lock().unwrap_or_else(|e| e.into_inner());
    let path = crate::golden::fixture(FIXTURE_FILE);
    let mut cases = if path.exists() { crate::golden::read(FIXTURE_FILE) } else { json!([]) };
    let list = cases.as_array_mut().unwrap();
    let at = list.iter().position(|c| c["name"] == name);
    if crate::golden::update_mode() {
      let case = json!({ "name": name, "turns": turns });
      match at {
        Some(i) => list[i] = case,
        None => list.push(case),
      }
      list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
      crate::golden::write(FIXTURE_FILE, &cases);
    } else {
      let recorded = at.map(|i| list[i]["turns"].clone()).unwrap_or_else(|| panic!("no {name} in {FIXTURE_FILE}; run with ACPIRA_UPDATE_FIXTURES=1"));
      assert_eq!(turns, recorded, "{name}: the built-in agent's transcript changed (ACPIRA_UPDATE_FIXTURES=1 records an intended change)");
    }
  }
}

fn lines(events: &[Value]) -> String {
  events.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n")
}

/// Runs one prompt to its end, answering every permission card with its allow-once option; the cards, in order
async fn run(s: &Arc<AcpSession>, text: &str) -> Vec<Value> {
  let task = spawn_prompt(s, text);
  let (mut cards, mut answered) = (vec![], HashSet::new());
  let t0 = Instant::now();
  while !task.is_finished() {
    assert!(t0.elapsed() < Duration::from_secs(15), "the turn did not end: {}", view(s));
    if let Some(card) = find_block(&view(s), "permission").filter(|c| !answered.contains(&c["id"].to_string())) {
      let allow = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").expect("an allow-once option")["id"].clone();
      answered.insert(card["id"].to_string());
      s.resolve_permission(card["id"].as_str().unwrap(), allow.as_str().unwrap());
      cards.push(card);
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  task.await.unwrap();
  cards
}

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

#[tokio::test(flavor = "multi_thread")]
async fn the_question_tool_opens_the_question_card_and_the_answers_reach_the_model() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "question", json!({ "questions": [
    { "question": "Which file name?", "header": "Name", "options": [{ "label": "report", "description": "A generic report" }, { "label": "notes" }] },
    { "question": "Where should it go?", "options": ["src", "docs"], "multiple": true },
  ] }))]));
  env.server.push(mock::text("Going with my-own-name in docs."));
  let task = spawn_prompt(&s, "make a file");
  let open = || agent_blocks(&view(&s)).into_iter().find(|b| b["type"] == "question" && b["outcome"].is_null());
  until(|| open().is_some(), 10_000).await;
  let card = open().unwrap();
  // The labels and descriptions are the model's own; both questions take a typed answer beside the choices
  expect_match(&card["questions"][0], json!({ "id": "q0", "title": "Name", "text": "Which file name?", "kind": "single", "other": true,
    "options": [{ "id": "report", "label": "report", "description": "A generic report" }, { "id": "notes", "label": "notes" }] }));
  expect_match(&card["questions"][1], json!({ "id": "q1", "text": "Where should it go?", "kind": "multiple", "other": true }));
  let answers = serde_json::from_value(json!({ "q0": "my-own-name", "q1": ["docs"] })).unwrap();
  s.answer_questions(card["id"].as_str().unwrap(), &answers, false);
  task.await.unwrap();
  let reqs = env.server.requests();
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  assert_eq!(msgs.last().unwrap()["content"], "The user answered:\n\nQ: Which file name?\nA: my-own-name\n\nQ: Where should it go?\nA: docs");
  let vw = view(&s);
  let blocks = agent_blocks(&vw);
  expect_match(blocks.iter().find(|b| b["type"] == "question").unwrap(), json!({ "outcome": "answered", "answers": { "q0": "my-own-name", "q1": ["docs"] } }));
  expect_match(blocks.iter().find(|b| b["type"] == "tool_call").unwrap(), json!({ "verbKey": "verb.ask", "status": "completed" }));
  expect_match(last_turn(&vw), json!({ "stop": "end_turn" }));
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-question", &vw["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancel_before_the_first_chunk_ends_the_turn_at_once_and_nothing_follows() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(Reply::Hang);
  let task = spawn_prompt(&s, "wait for it");
  until(|| env.server.hanging() == 1, 10_000).await;
  let t0 = Instant::now();
  s.cancel().await;
  task.await.unwrap();
  assert!(t0.elapsed() < Duration::from_secs(2), "the cancel took {:?}", t0.elapsed());
  let vw = view(&s);
  expect_match(last_turn(&vw), json!({ "role": "agent", "stop": "cancelled" }));
  // Nothing arrives for the cancelled turn afterwards
  tokio::time::sleep(Duration::from_millis(500)).await;
  assert_eq!(view(&s)["turns"], vw["turns"]);
  // The session takes the next prompt as usual
  env.server.push(mock::text("Ready."));
  run(&s, "go on").await;
  expect_match(last_turn(&view(&s)), json!({ "stop": "end_turn", "blocks": [{ "type": "text", "markdown": "Ready." }] }));
  let log = std::fs::read_to_string(env.log_of(&s)).unwrap();
  assert!(log.lines().any(|l| l.contains(r#""type":"request""#) && l.contains(r#""stop":"Cancelled""#)), "the cancelled call has its record\n{log}");
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-cancel", &view(&s)["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_steered_into_a_command_round_reaches_the_next_call_in_the_same_turn() {
  let env = Env::new();
  let s = env.session().await;
  assert_eq!(view(&s)["canSteer"], true);
  env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "echo hi" }))]));
  env.server.push(mock::text("Switched to printf."));
  let task = spawn_prompt(&s, "print hi");
  // The command waits on its card while the user sends a correction with a file attached (the composer's steer send)
  let card = wait_block(&s, "permission").await;
  s.steer_prompt("use printf instead".into(), drafts(json!([{ "kind": "text", "name": "note.txt", "text": "printf is portable" }]))).await.unwrap();
  let allow = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").unwrap()["id"].clone();
  s.resolve_permission(card["id"].as_str().unwrap(), allow.as_str().unwrap());
  task.await.unwrap();
  // The model got it behind the command's result, attachment included, and answered in the same turn
  let reqs = env.server.requests();
  assert_eq!(reqs.len(), 2);
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  let steered = msgs.last().unwrap()["content"].to_string();
  assert!(steered.contains("use printf instead") && steered.contains("printf is portable"), "{steered}");
  assert_eq!(msgs[msgs.len() - 2]["role"], "tool");
  let vw = view(&s);
  let users: Vec<Value> = vw["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].clone()).collect();
  assert_eq!(users, [json!("print hi")], "the message joined the turn instead of opening one");
  assert!(vw["queued"].is_null());
  let blocks = agent_blocks(&vw);
  let at = blocks.iter().position(|b| b["type"] == "steer").expect("a steer block");
  expect_match(&blocks[at], json!({ "text": "use printf instead" }));
  assert!(blocks[..at].iter().any(|b| b["type"] == "tool_call"));
  assert!(blocks[at + 1..].iter().any(|b| b["type"] == "text" && b["markdown"] == "Switched to printf."));
  expect_match(last_turn(&vw), json!({ "role": "agent", "stop": "end_turn" }));
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-steer", &vw["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reopened_session_resumes_with_the_same_prefix() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::text("Noted: 42."));
  run(&s, "remember 42").await;
  let log = env.log_of(&s);
  let record = s.to_record();
  s.dispose();
  // A new window: the record reopens the agent's session in a fresh process
  let again = Disposing(AcpSession::new(record.clone(), env.deps.clone()));
  again.start().await;
  assert_eq!(again.to_record().acp_session_id, record.acp_session_id);
  env.server.push(mock::text("42."));
  run(&again, "which number?").await;
  let reqs = env.server.requests();
  let last = reqs.last().unwrap().body["messages"].to_string();
  assert!(last.contains("remember 42") && last.contains("Noted: 42."), "{last}");
  // The reopened process rebuilt the history the first one sent, byte for byte
  env.assert_prefix_stable(&log);
  let turns = view(&again)["turns"].clone();
  assert_eq!(turns.as_array().unwrap().len(), 4, "{turns}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_engines_own_mcp_server_reaches_the_model_and_show_image_shows_the_picture() {
  let mut env = Env::new();
  // What the runtime hands every session: `acpira mcp` on stdio (show_image, workspace_hooks)
  env.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")));
  let png = env.cwd().join("shot.png");
  std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
  let s = env.session().await;
  env.server.push(mock::tools(&[("c1", "acpira_show_image", json!({ "path": png.to_string_lossy() }))]));
  env.server.push(mock::text("Here it is."));
  let cards = run(&s, "show me the shot").await;
  assert!(cards.is_empty(), "the engine's own tools ask nothing: {cards:?}");
  let reqs = env.server.requests();
  let names: Vec<String> = reqs[0].body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_owned()).collect();
  assert!(names.iter().any(|n| n == "acpira_show_image") && names.iter().any(|n| n == "acpira_workspace_hooks"), "{names:?}");
  // The server's instructions came with the first message, the receipt with the result
  let first = reqs[0].body["messages"][1]["content"].as_str().unwrap().to_owned();
  assert!(first.starts_with("<mcp>") && first.contains("## acpira\n"), "{first}");
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  let receipt = msgs.iter().find(|m| m["role"] == "tool").unwrap()["content"].as_str().unwrap().to_owned();
  assert!(receipt.contains("shot.png"), "{receipt}");
  // The engine knows the call by its name: a show-image row carrying the picture
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "verbKey": "verb.showImage", "status": "completed" }));
  assert!(row.to_string().contains("image/png"), "the picture is attached: {row}");
  env.assert_prefix_stable(&env.log_of(&s));
}

/// Stands in for the session manager behind the hub: what reaches it, and a fixed answer
struct CannedLibrary;

impl acpira_host::session_library::SessionLibrary for CannedLibrary {
  fn search_sessions<'a>(
    &'a self,
    _caller: &'a acpira_host::session_library::LibraryCaller,
    args: acpira_host::relay::wire::SessionSearchArgs,
  ) -> acpira_host::session_library::LibraryFuture<'a> {
    Box::pin(async move { Ok(format!("searched {:?}", args.query)) })
  }
  fn read_session<'a>(
    &'a self,
    caller: &'a acpira_host::session_library::LibraryCaller,
    args: acpira_host::relay::wire::SessionReadArgs,
  ) -> acpira_host::session_library::LibraryFuture<'a> {
    let me = caller.session_id.clone();
    Box::pin(async move { Ok(format!("# Outline of {} 2-3 asked by {me}", args.session)) })
  }
}

// The whole way an agent reads another session: the model's call, the built-in agent's MCP client, the `acpira mcp` process,
// the hub with the session's grant, the library, and back into a "Read session" row naming the turns
#[tokio::test(flavor = "multi_thread")]
async fn read_session_goes_from_the_model_through_acpira_mcp_and_the_hub_into_a_session_row() {
  let mut env = Env::new();
  let roster = Arc::new(acpira_host::relay::roster::Roster::new(env.root.path()));
  let hub = acpira_host::relay::hub::RelayHub::start(roster, Arc::new(|_: &str| {})).await.unwrap();
  let library: Arc<dyn acpira_host::session_library::SessionLibrary> = Arc::new(CannedLibrary);
  hub.set_library(Arc::downgrade(&library));
  env.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new(env!("CARGO_BIN_EXE_acpira")).with_hub(hub));
  let s = env.session().await;
  let args = json!({ "session": "vscode://hotic.acpira/session/abc", "from": 2, "to": 3 });
  env.server.push(mock::tools(&[("r1", "acpira_read_session", args)]));
  env.server.push(mock::text("Read it."));
  let cards = run(&s, "what happened in that session?").await;
  assert!(cards.is_empty(), "a read-only engine tool asks nothing: {cards:?}");
  let reqs = env.server.requests();
  let names: Vec<String> = reqs[0].body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_owned()).collect();
  assert!(names.iter().any(|n| n == "acpira_search_sessions") && names.iter().any(|n| n == "acpira_read_session"), "{names:?}");
  let first = reqs[0].body["messages"][1]["content"].as_str().unwrap().to_owned();
  assert!(first.contains("acpira://session/<id>"), "the instructions name the links: {first}");
  let msgs = reqs[1].body["messages"].as_array().unwrap();
  let result = msgs.iter().find(|m| m["role"] == "tool").unwrap()["content"].as_str().unwrap().to_owned();
  assert!(result.contains(&format!("# Outline of vscode://hotic.acpira/session/abc 2-3 asked by {}", s.0.id)), "{result}");
  let vw = view(&s);
  let row = agent_blocks(&vw).into_iter().find(|b| b["type"] == "tool_call").unwrap();
  expect_match(&row, json!({ "verbKey": "verb.readSession", "target": "acpira://session/abc 2-3", "status": "completed" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edited_message_rewinds_the_agents_own_history_instead_of_superseding_it() {
  let env = Env::new();
  let s = env.session().await;
  env.server.push(mock::text("one"));
  run(&s, "first message").await;
  env.server.push(mock::text("bad answer"));
  run(&s, "the wrong message").await;
  let peer = s.to_record().acp_session_id;
  env.server.push(mock::text("good answer"));
  s.edit_turn(crate::acp_session::history_edit(&s, 2, "the right message")).await.unwrap();
  until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().len() == 4 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  // The same native session, which no longer holds the original: no rebuilt history, no superseded note
  assert_eq!(s.to_record().acp_session_id, peer);
  let sent = env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("first message") && sent.contains("one") && sent.contains("the right message"), "{sent}");
  assert!(!sent.contains("the wrong message") && !sent.contains("bad answer") && !sent.contains("superseded"), "{sent}");
  let turns = view(&s)["turns"].clone();
  expect_match(&turns[2], json!({ "text": "the right message", "edited": true }));
  // The rewound history is what the model saw before the original: the prefix up to it stays byte for byte
  let reqs = env.server.requests();
  let (before, after) = (reqs[0].body["messages"].as_array().unwrap(), reqs[2].body["messages"].as_array().unwrap());
  assert!(after.len() > before.len() && before.iter().zip(after).all(|(x, y)| x == y), "{before:?}\n{after:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_goal_set_from_the_dock_is_continued_until_the_model_marks_it_complete() {
  let env = Env::new();
  std::fs::write(env.cwd().join("a.txt"), "alpha\n").unwrap();
  let s = env.session().await;
  assert_eq!(view(&s)["goalActions"], json!(["set", "pause", "resume", "clear"]));
  // The set turn reads and stops short; the engine continues the goal, where the edit lands and the model marks it met
  env.server.push(mock::tools(&[("c1", "read", json!({ "path": "a.txt" }))]));
  env.server.push(mock::text("It says alpha; the edit comes next."));
  env.server.push(mock::tools(&[("c2", "edit", json!({ "path": "a.txt", "old_string": "alpha", "new_string": "beta" }))]));
  env.server.push(mock::tools(&[("c3", "update_goal", json!({ "status": "complete" }))]));
  env.server.push(mock::text("Done: a.txt says beta."));
  s.control_goal(GoalAction::Set, Some("make a.txt say beta".into())).await.unwrap();
  // The continuation's edit asks like any other: answered here, since no prompt of the test's own runs it
  let mut answered = HashSet::new();
  let t0 = Instant::now();
  while s.is_running() || env.server.requests().len() < 5 || !view(&s)["goal"].is_null() {
    assert!(t0.elapsed() < Duration::from_secs(20), "the goal did not settle: {}", view(&s));
    if let Some(card) = find_block(&view(&s), "permission").filter(|c| !answered.contains(&c["id"].to_string())) {
      let allow = card["options"].as_array().unwrap().iter().find(|o| o["kind"] == "allow_once").expect("an allow-once option")["id"].clone();
      answered.insert(card["id"].to_string());
      s.resolve_permission(card["id"].as_str().unwrap(), allow.as_str().unwrap());
    }
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
  assert_eq!(std::fs::read_to_string(env.cwd().join("a.txt")).unwrap(), "beta\n");
  let vw = view(&s);
  let turns = vw["turns"].as_array().unwrap();
  assert_eq!(turns.len(), 4, "{}", vw["turns"]);
  expect_match(&turns[0], json!({ "role": "user", "text": "/goal make a.txt say beta", "auto": true, "autoReason": "goal" }));
  expect_match(&turns[2], json!({ "role": "user", "text": "/goal continue", "auto": true, "autoReason": "goal", "goalContinue": true }));
  let blocks = agent_blocks(&vw);
  let goals: Vec<Value> = blocks.iter().filter(|b| b["type"] == "goal").map(|b| b["event"].clone()).collect();
  assert_eq!(goals, [json!("set"), json!("complete")]);
  // The goal's tool shows no row; the goal rows stand for it
  let tools: Vec<Value> = blocks.iter().filter(|b| b["type"] == "tool_call").map(|b| b["kind"].clone()).collect();
  assert_eq!(tools, [json!("read"), json!("edit")]);
  // The continuation opened with the goal's note, and a met goal is not continued
  let reqs = env.server.requests();
  let note = reqs[2].body["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_owned();
  assert!(note.starts_with("<goal>\nContinue working toward the active goal.") && note.contains("<objective>\nmake a.txt say beta\n</objective>"), "{note}");
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert_eq!(env.server.requests().len(), 5);
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-goal", &vw["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edited_first_message_carries_the_workspace_context_again_after_the_rewind() {
  let env = Env::new();
  let git = std::process::Command::new("git").arg("init").arg("-q").current_dir(env.cwd()).status();
  if !git.is_ok_and(|s| s.success()) {
    return;
  }
  std::fs::create_dir_all(env.cwd().join(".agents")).unwrap();
  std::fs::write(env.cwd().join(".agents/hooks.json"), json!({ "context": ["RULES.md"] }).to_string()).unwrap();
  std::fs::write(env.cwd().join("RULES.md"), "house rule: answer briefly\n").unwrap();
  let s = env.session().await;
  env.server.push(mock::text("one"));
  run(&s, "first message").await;
  assert!(env.server.requests()[0].body["messages"].to_string().contains("house rule"));
  env.server.push(mock::text("two"));
  s.edit_turn(crate::acp_session::history_edit(&s, 0, "first message, edited")).await.unwrap();
  until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().len() == 2 && !last_turn(&view(&s))["stop"].is_null(), 5000).await;
  // The rewound session holds nothing, so the edited message is its first and brings the context files along
  let sent = env.server.requests().last().unwrap().body["messages"].to_string();
  assert!(sent.contains("house rule") && sent.contains("first message, edited") && !sent.contains("\"one\""), "{sent}");
}

/// Local run on an SSH host (docs/dev/remote.md, "Built-in agent"): the session's folder is `ssh://gen1/<virtual
/// root>/work`, a path that does not exist here; the fake host's helper maps it to a temp folder. So every project
/// call has to go through the helper, and one that read this disk instead would fail
#[cfg(unix)]
mod remote {
  use super::*;
  use crate::remote_connections::Lab;
  use acpira_shared::remote::{ConnectionOp, RunMode};

  const VIRT: &str = "/acpira-virtual-host";

  struct Remote {
    env: Env,
    lab: Lab,
    real: PathBuf,
    connections: Arc<acpira_host::remote::connections::Connections>,
  }

  impl Remote {
    async fn new() -> Remote {
      let mut env = Env::new();
      let lab = Lab::new();
      let real = lab.path().join("hostfs");
      std::fs::create_dir_all(real.join("work")).unwrap();
      lab.map_root(VIRT, &real);
      let connections = lab.engine("engine");
      connections.op(ConnectionOp::Add { alias: "gen1".into(), mode: RunMode::Local }).await.unwrap();
      connections.ensure("gen1", Duration::from_secs(30)).await.unwrap();
      env.deps.remote = Some(connections.clone());
      Remote { env, lab, real, connections }
    }

    /// `approval: full`: no permission card asks
    fn full_access(&self) {
      let path = self.env.root.path().join("providers.json");
      let mut p: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
      p["approval"] = json!("full");
      std::fs::write(path, p.to_string()).unwrap();
    }

    async fn session(&self) -> Disposing {
      let s = Disposing(AcpSession::fresh("acpira", &format!("ssh://gen1{VIRT}/work"), self.env.deps.clone(), None));
      s.start().await;
      s
    }
  }

  impl Drop for Remote {
    fn drop(&mut self) {
      self.connections.dispose();
    }
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn writes_and_commands_run_on_the_host() {
    let r = Remote::new().await;
    assert!(!Path::new(VIRT).exists());
    let s = r.session().await;
    assert!(view(&s)["error"].is_null(), "{}", view(&s)["error"]);
    r.env.server.push(mock::tools(&[("c1", "write", json!({ "path": "hello.txt", "content": "hi from the host\n" }))]));
    r.env.server.push(mock::tools(&[("c2", "bash", json!({ "command": "cat hello.txt; pwd" }))]));
    r.env.server.push(mock::text("Done."));
    let cards = run(&s, "say hi").await;
    // The write asked, with the diff the host prepared
    assert_eq!(cards.len(), 2, "the write and the command ask: {cards:?}");
    assert_eq!(std::fs::read_to_string(r.real.join("work/hello.txt")).unwrap(), "hi from the host\n");
    let vw = view(&s);
    expect_match(last_turn(&vw), json!({ "role": "agent", "stop": "end_turn" }));
    let blocks = agent_blocks(&vw);
    expect_match(blocks.iter().find(|b| b["kind"] == "edit").unwrap(), json!({ "status": "completed", "diffStat": { "add": 1 } }));
    let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
    assert!(sent.contains("hi from the host"), "the command's output came back: {sent}");
    // The host's folder, as the session names it (the helper never shows its test map's real root)
    assert!(sent.contains(&format!("{VIRT}/work")), "it ran in the host's folder: {sent}");
    assert!(!sent.contains(&r.real.to_string_lossy().into_owned()), "{sent}");
    assert!(r.lab.calls().iter().any(|c| c == "start"));
  }

  /// Full access, so no card asks: the host's own `beforeEdit` still sees every write before it lands, and the
  /// hooks run on the host, in its project folder
  #[tokio::test(flavor = "multi_thread")]
  async fn the_hosts_hooks_gate_every_write_there() {
    let r = Remote::new().await;
    r.full_access();
    let work = r.real.join("work");
    assert!(std::process::Command::new("git").arg("init").arg("-q").arg(&work).status().unwrap().success());
    std::fs::create_dir_all(work.join(".agents")).unwrap();
    std::fs::write(work.join(".agents/block-secret.sh"), "payload=$(cat)\ncase \"$payload\" in *secret.txt*) echo 'secrets stay out' >&2; exit 2;; esac\nexit 0\n").unwrap();
    std::fs::write(work.join(".agents/hooks.json"), r#"{ "beforeEdit": "sh .agents/block-secret.sh", "afterTurn": "touch .after-ran" }"#).unwrap();
    let s = r.session().await;
    r.env.server.push(mock::tools(&[("c1", "write", json!({ "path": "secret.txt", "content": "x" }))]));
    r.env.server.push(mock::tools(&[("c2", "write", json!({ "path": "ok.txt", "content": "fine" }))]));
    r.env.server.push(mock::text("Done."));
    // The gate sends the rejection back once the turn ends
    r.env.server.push(mock::text("Understood."));
    run(&s, "write both").await;
    until(|| r.env.server.requests().len() == 4 && !s.is_running(), 15_000).await;
    assert!(!work.join("secret.txt").exists(), "beforeEdit kept the write off the host");
    assert_eq!(std::fs::read_to_string(work.join("ok.txt")).unwrap(), "fine");
    let reqs = r.env.server.requests();
    assert!(reqs[1].body["messages"].to_string().contains("secrets stay out"), "the model got the hook's reason");
    assert!(work.join(".after-ran").exists(), "afterTurn ran in the host's project folder");
  }

  /// The prompt's AGENTS.md, environment and skills are the host's, read there as the session opens
  #[tokio::test(flavor = "multi_thread")]
  async fn instructions_and_skills_come_from_the_host() {
    let r = Remote::new().await;
    r.full_access();
    let work = r.real.join("work");
    std::fs::create_dir_all(work.join(".git")).unwrap();
    std::fs::write(work.join("AGENTS.md"), "Run make check on the host.").unwrap();
    let skill = work.join(".agents/skills/deploy");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), "---\ndescription: Ship the build\n---\nRun ./ship.sh from the host.\n").unwrap();
    let s = r.session().await;
    r.env.server.push(mock::tools(&[("c1", "skill", json!({ "name": "deploy" }))]));
    r.env.server.push(mock::text("Done."));
    run(&s, "deploy it").await;
    let reqs = r.env.server.requests();
    let first = reqs[0].body.to_string();
    assert!(first.contains("Run make check on the host.") && first.contains(&format!("{VIRT}/work/AGENTS.md")), "{first}");
    assert!(first.contains("deploy: Ship the build"), "the skill tool lists the host's skill: {first}");
    assert!(!first.contains(&r.real.to_string_lossy().into_owned()), "{first}");
    let sent = reqs.last().unwrap().body["messages"].to_string();
    assert!(sent.contains("Run ./ship.sh from the host."), "the skill loaded from the host: {sent}");
  }

  /// `/compact` brings back the files the turns read, as they are on the host now
  #[tokio::test(flavor = "multi_thread")]
  async fn compaction_restores_files_from_the_host() {
    let r = Remote::new().await;
    r.full_access();
    let notes = r.real.join("work/notes.txt");
    std::fs::write(&notes, "first draft\n").unwrap();
    let s = r.session().await;
    r.env.server.push(mock::tools(&[("c1", "read", json!({ "path": "notes.txt" }))]));
    r.env.server.push(mock::text("Read it."));
    run(&s, "read the notes").await;
    std::fs::write(&notes, "second draft from the host\n").unwrap();
    r.env.server.push(mock::text("The user had the notes read."));
    run(&s, "/compact").await;
    r.env.server.push(mock::text("Fine."));
    run(&s, "go on").await;
    let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
    assert!(sent.contains("second draft from the host"), "the restored file is the host's current one: {sent}");
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn a_call_cut_off_by_a_dropped_connection_is_reported_as_unknown() {
    let r = Remote::new().await;
    r.full_access();
    let s = r.session().await;
    r.env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "echo started; sleep 31.4" }))]));
    r.env.server.push(mock::text("I will check first."));
    let task = spawn_prompt(&s, "run it");
    until(|| agent_blocks(&view(&s)).iter().any(|b| b["kind"] == "execute" && b["status"] == "in_progress"), 10_000).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    // The helper dies mid-command: its process group goes with it
    let pid = r.connections.client("gen1").unwrap().hello().pid as i32;
    // SAFETY: kill has no memory preconditions; the pid is the helper this test started
    unsafe { libc::kill(pid, libc::SIGKILL) };
    task.await.unwrap();
    let sent = r.env.server.requests().last().unwrap().body["messages"].to_string();
    assert!(sent.contains("unknown"), "the model is told the outcome is unknown: {sent}");
    expect_match(agent_blocks(&view(&s)).iter().find(|b| b["kind"] == "execute").unwrap(), json!({ "status": "failed" }));
  }

  /// A background job outlives its turn on the host, not its session: the close stops it while the helper stays up
  #[tokio::test(flavor = "multi_thread")]
  async fn closing_the_session_stops_its_jobs_on_the_host() {
    let r = Remote::new().await;
    r.full_access();
    let s = r.session().await;
    r.env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "sleep 33.7", "background": true }))]));
    r.env.server.push(mock::text("Started."));
    run(&s, "start it").await;
    let running = || {
      let out = std::process::Command::new("ps").args(["-eo", "command"]).output().unwrap();
      String::from_utf8_lossy(&out.stdout).lines().any(|l| l.contains("sleep 33.7"))
    };
    until(running, 5000).await;
    s.dispose();
    until(|| !running(), 15_000).await;
    assert!(r.connections.client("gen1").is_some(), "the connection stays up");
  }

  #[tokio::test(flavor = "multi_thread")]
  async fn a_cancelled_turn_stops_the_command_on_the_host() {
    let r = Remote::new().await;
    r.full_access();
    let s = r.session().await;
    r.env.server.push(mock::tools(&[("c1", "bash", json!({ "command": "sleep 32.6" }))]));
    let task = spawn_prompt(&s, "run it");
    until(|| agent_blocks(&view(&s)).iter().any(|b| b["kind"] == "execute" && b["status"] == "in_progress"), 10_000).await;
    let running = || {
      let out = std::process::Command::new("ps").args(["-eo", "command"]).output().unwrap();
      String::from_utf8_lossy(&out.stdout).lines().any(|l| l.contains("sleep 32.6"))
    };
    until(running, 5000).await;
    s.cancel().await;
    task.await.unwrap();
    until(|| !running(), 5000).await;
  }
}
