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
// Only the Unix-only relay case names these
#[cfg(unix)]
use acpira_host::relay::roster::Roster;
#[cfg(unix)]
use acpira_host::relay::wire::{AskArgs, HubReply};
use acpira_host::store::transcript_store::{LogFn, TranscriptStore};
use acpira_shared::transcript::GoalAction;

use crate::acp_session::{agent_blocks, drafts, find_block, last_turn, spawn_prompt, view, wait_block};
use crate::support::{Disposing, expect_match, until};

mod controls;
mod approvals;
mod turns;
mod mcp;
#[cfg(unix)]
mod remote;
mod plugins;
// Every case in it is Unix-only
#[cfg(unix)]
mod commands;

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
    // The fast model is off, so no title call takes one of the scripted replies
    let providers = json!({ "version": 1, "fastModel": "off", "providers": [{ "id": "mock", "name": "Mock", "format": "openai-chat", "baseUrl": server.base_url(),
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
      policy: std::sync::Arc::new(acpira_host::acp::session::InertPolicy),
      pool: None,
      shared_mcp: None,
      plugin_hooks: None,
      plugin_prompts: None,
      data_home: None,
      plugin_runtime: None,
      host_mcp: None,
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
          m.remove("resumedAt");
          // Wall-clock measurements, never the same twice
          m.remove("decodeMs");
          m.remove("idleMs");
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
