//! test/AcpSession.test.ts: the session state machine against test/fake-agent.ts

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use acpira_host::acp::session::AcpSession;
use acpira_host::store::record::SessionRecord;
use acpira_host::store::transcript_store::blob_name;
use acpira_host::util::iso_of_ms;
use acpira_shared::attachments::MAX_IMAGE_BYTES;
use acpira_shared::transcript::{Draft, Turn};

use crate::fake_or_skip;
use crate::support::{Disposing, FakeAgent, Harness, expect_absent, expect_eq, expect_match, turns_in, until, v};

mod prompts;
mod plans;
mod permissions;
mod controls;
mod retry;
mod queue;
mod steer;
mod compaction;
mod lifecycle;
mod auth;
mod fork;
mod edit;
mod edit_context;
mod failures;
mod shared_mcp;
mod ultracode;
mod autonomous;

// Grok-style synthesized modes: not provided by the protocol, declared in the registry
fn syn_modes() -> Value {
  json!([{ "id": "default", "name": "Agent" }, { "id": "plan", "name": "Plan" }, { "id": "yolo", "name": "Auto accept" }])
}

pub fn turns(j: Value) -> Vec<Turn> {
  serde_json::from_value(j).expect("turns")
}

pub fn drafts(j: Value) -> Vec<Draft> {
  serde_json::from_value(j).expect("drafts")
}

pub async fn prompt(s: &Arc<AcpSession>, text: &str) {
  s.prompt(text.into(), vec![], false, None, None).await;
}

/// A prompt left running in the background (the TS `const pending = s.prompt(…)`)
pub fn spawn_prompt(s: &Arc<AcpSession>, text: &str) -> tokio::task::JoinHandle<()> {
  tokio::spawn(s.prompt(text.into(), vec![], false, None, None))
}

pub fn view(s: &AcpSession) -> Value {
  v(s.view())
}

pub fn agent_blocks(view: &Value) -> Vec<Value> {
  view["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "agent").flat_map(|t| t["blocks"].as_array().unwrap().clone()).collect()
}

pub fn find_block(view: &Value, ty: &str) -> Option<Value> {
  agent_blocks(view).into_iter().find(|b| b["type"] == ty)
}

pub fn has_block(s: &AcpSession, ty: &str) -> bool {
  find_block(&view(s), ty).is_some()
}

pub async fn wait_block(s: &AcpSession, ty: &str) -> Value {
  until(|| has_block(s, ty), 5000).await;
  find_block(&view(s), ty).unwrap()
}

pub fn last_turn(view: &Value) -> Value {
  view["turns"].as_array().unwrap().last().cloned().unwrap_or(Value::Null)
}

pub fn turn_at(view: &Value, i: isize) -> Value {
  let t = view["turns"].as_array().unwrap();
  let i = if i < 0 { t.len() as isize + i } else { i };
  t.get(i as usize).cloned().unwrap_or(Value::Null)
}

fn option_value(view: &Value, id: &str) -> Value {
  view["controls"]["options"].as_array().unwrap().iter().find(|o| o["id"] == id).map(|o| o["value"].clone()).unwrap_or(Value::Null)
}

fn blob(h: &Harness, sid: &str, name: &str) -> Option<Vec<u8>> {
  std::fs::read(h.dir.path().join("sessions").join(sid).join(name)).ok()
}

async fn started(h: &Harness, cwd: &str) -> Disposing {
  let s = Disposing(h.session(cwd));
  s.start().await;
  s
}

/// A file draft whose image read blocks until released: staging waits on a FIFO the way the TS suite gated saveBlob
#[cfg(unix)]
struct StagingGate {
  _dir: tempfile::TempDir,
  path: std::path::PathBuf,
}

#[cfg(unix)]
impl StagingGate {
  fn new() -> StagingGate {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gate.png");
    let c = std::ffi::CString::new(path.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    StagingGate { _dir: dir, path }
  }
  fn draft(&self) -> Vec<Draft> {
    drafts(json!([{ "kind": "file", "uri": format!("file://{}", self.path.display()), "name": "gate.png" }]))
  }
  fn release(&self) {
    let path = self.path.clone();
    std::thread::spawn(move || std::fs::write(path, b"png-bytes"));
  }
}

pub fn history_edit(s: &AcpSession, turn_index: usize, text: &str) -> acpira_shared::protocol::EditTurnRequest {
  let vw = s.view();
  let turn = v(&vw.turns[turn_index]);
  assert_eq!(turn["role"], "user", "expected a user turn");
  let retained: Vec<i64> = (0..turn["attachments"].as_array().map_or(0, |a| a.len()) as i64).collect();
  serde_json::from_value(json!({
    "sessionId": s.id, "turnIndex": turn_index, "turnCount": vw.turns.len(), "originalText": turn["text"], "turnId": turn["id"],
    "text": text, "retainedAttachments": retained, "attachments": [],
    "settings": acpira_shared::turn_settings::capture_turn_settings(&vw.controls),
  }))
  .unwrap()
}

fn agent_text(turn: &Value) -> String {
  turn["blocks"].as_array().into_iter().flatten().filter(|b| b["type"] == "text").filter_map(|b| b["markdown"].as_str()).collect()
}

/// Runs a call's synchronous prefix now (the TS call before its first await) and hands back the rest
pub fn claimed<T: Send + 'static>(fut: impl std::future::Future<Output = T> + Send + 'static) -> tokio::sync::oneshot::Receiver<T> {
  let (tx, rx) = tokio::sync::oneshot::channel();
  acpira_host::util::run_prefix(async move {
    let _ = tx.send(fut.await);
  });
  rx
}

/// How many turns the session's view holds
pub fn turn_count(s: &AcpSession) -> usize {
  turns_in(&view(s))
}

/// Waits (5 s) until the view holds exactly `n` turns
pub async fn wait_turns(s: &AcpSession, n: usize) {
  until(|| turn_count(s) == n, 5000).await;
}

fn wire_prompt(turn: &Value) -> Value {
  serde_json::from_str::<Value>(&agent_text(turn)).expect("the fake echoes the prompt as JSON")
}

/// A harness whose fake agent keeps native sessions on disk, so a record reopened in a new session resumes its peer
fn native_harness(fake: &FakeAgent, env: Value) -> (Harness, tempfile::TempDir) {
  let dir = tempfile::tempdir().unwrap();
  let mut env = env;
  env["FAKE_SESSION_DIR"] = json!(dir.path());
  (Harness::new(fake, json!({ "env": env })), dir)
}

/// 'earlier-context' answered, then the UI-only history grown by one huge text block (the TS suites pushed it onto the
/// live transcript); the session is reopened on that record and resumes the same native session
async fn with_ui_history(h: &Harness, markdown: String) -> (Disposing, Value) {
  let first = started(h, "/tmp").await;
  prompt(&first, "earlier-context").await;
  grow_history(h, &first, markdown).await
}

/// Reopens `first`'s record with its turn 1 grown by a huge UI-only block
async fn grow_history(h: &Harness, first: &Arc<AcpSession>, markdown: String) -> (Disposing, Value) {
  let mut record = first.to_record();
  first.dispose();
  if let Some(Turn::Agent(a)) = record.turns.get_mut(1) {
    a.blocks.push(serde_json::from_value(json!({ "type": "text", "markdown": markdown })).unwrap());
  }
  let earlier = v(&record.turns[1]);
  (reopened(h, record).await, earlier)
}

/// A session that ran one turn, disposed, and the record it left
async fn ran_once(h: &Harness, cwd: &str) -> SessionRecord {
  let s = started(h, cwd).await;
  prompt(&s, "hi").await;
  s.to_record()
}

async fn reopened(h: &Harness, record: SessionRecord) -> Disposing {
  let s = Disposing(AcpSession::new(record, h.deps.clone()));
  s.start().await;
  s
}
