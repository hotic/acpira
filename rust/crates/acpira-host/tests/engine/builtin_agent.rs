//! The built-in agent end to end: the engine drives the real `acpira agent` binary against the scripted model server.
//! Four scenarios (an edit behind a permission card, a Plan-mode approval, a cancel before the model's first chunk, a
//! reopen that resumes the session) each check that the request prefix stays byte for byte between two model calls
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

use crate::acp_session::{agent_blocks, find_block, last_turn, spawn_prompt, view};
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
          if m.get("role").and_then(Value::as_str) == Some("user") && m.contains_key("id") {
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
  assert!(cards.iter().any(|c| c["planId"].is_string()), "the plan went through the plan card: {cards:?}");
  assert_eq!(std::fs::read_to_string(env.cwd().join("src.txt")).unwrap(), "x\n");
  let vw = view(&s);
  expect_match(find_block(&vw, "plan_document").unwrap(), json!({ "status": "approved", "markdown": "# Add src.txt\n\n1. Write it." }));
  assert_eq!(vw["controls"]["modeId"], "agent");
  expect_match(last_turn(&vw), json!({ "stop": "end_turn" }));
  env.assert_prefix_stable(&env.log_of(&s));
  env.golden(&s, "builtin-plan", &vw["turns"]);
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
