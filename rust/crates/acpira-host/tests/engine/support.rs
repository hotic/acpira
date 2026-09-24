//! Assertion helpers with the vitest semantics the suites were written against

#![allow(dead_code)]

use serde::Serialize;
use serde_json::Value;

pub fn v(x: impl Serialize) -> Value {
  serde_json::to_value(x).expect("serializable")
}

fn num_eq(a: &Value, b: &Value) -> bool {
  match (a.as_f64(), b.as_f64()) {
    (Some(x), Some(y)) => x == y,
    _ => false,
  }
}

fn mismatch(actual: &Value, pattern: &Value, path: &str) -> Option<String> {
  match pattern {
    // toMatchObject({ k: undefined }): the key is absent (or null)
    Value::Null => (!actual.is_null()).then(|| format!("{path}: expected absent/null, got {actual}")),
    Value::Object(p) => {
      let Some(a) = actual.as_object() else { return Some(format!("{path}: expected an object, got {actual}")) };
      p.iter().find_map(|(k, pv)| mismatch(a.get(k).unwrap_or(&Value::Null), pv, &format!("{path}.{k}")))
    }
    Value::Array(p) => {
      let Some(a) = actual.as_array() else { return Some(format!("{path}: expected an array, got {actual}")) };
      if a.len() != p.len() {
        return Some(format!("{path}: expected {} items, got {} ({actual})", p.len(), a.len()));
      }
      a.iter().zip(p).enumerate().find_map(|(i, (av, pv))| mismatch(av, pv, &format!("{path}[{i}]")))
    }
    Value::Number(_) => (!num_eq(actual, pattern)).then(|| format!("{path}: expected {pattern}, got {actual}")),
    _ => (actual != pattern).then(|| format!("{path}: expected {pattern}, got {actual}")),
  }
}

/// toMatchObject: every key of the pattern matches recursively (a null pattern means absent), arrays match item by item
#[track_caller]
pub fn expect_match(actual: impl Serialize, pattern: Value) {
  let actual = v(actual);
  if let Some(m) = mismatch(&actual, &pattern, "$") {
    panic!("{m}\nactual: {}", serde_json::to_string_pretty(&actual).unwrap_or_default());
  }
}

/// toEqual on the serialized form (absent and undefined fields are the same thing there)
#[track_caller]
pub fn expect_eq(actual: impl Serialize, expected: Value) {
  let actual = v(actual);
  let expected = strip_nulls(expected);
  assert_eq!(strip_nulls(actual), expected);
}

fn strip_nulls(v: Value) -> Value {
  match v {
    Value::Object(m) => Value::Object(m.into_iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k, strip_nulls(x))).collect()),
    Value::Array(a) => Value::Array(a.into_iter().map(strip_nulls).collect()),
    Value::Number(n) => n.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9.0e15).map(|f| Value::from(f as i64)).unwrap_or(Value::Number(n)),
    other => other,
  }
}

/// not.toHaveProperty(key)
#[track_caller]
pub fn expect_absent(actual: impl Serialize, key: &str) {
  let actual = v(actual);
  let mut cur = &actual;
  for part in key.split('.') {
    match cur.get(part) {
      Some(next) if !next.is_null() => cur = next,
      _ => return,
    }
  }
  panic!("expected no `{key}`, got {cur} in {actual}");
}

/// Pins the thread's clock for the guard's lifetime
pub struct Clock;

impl Clock {
  pub fn at(ms: i64) -> Clock {
    acpira_host::util::mock_now(Some(ms));
    Clock
  }
  pub fn set(&self, ms: i64) {
    acpira_host::util::mock_now(Some(ms));
  }
}

impl Drop for Clock {
  fn drop(&mut self) {
    acpira_host::util::mock_now(None);
  }
}

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use acpira_host::acp::agent_registry::AgentRegistry;
use acpira_host::acp::session::{AcpSession, CompactionPolicy, SessionDeps};
use acpira_host::store::transcript_store::TranscriptStore;

/// test/fake-agent.ts, run by node through the tsx loader directly (not the tsx wrapper, so a SIGKILL reaches the agent itself)
pub struct FakeAgent {
  pub root: PathBuf,
  pub script: PathBuf,
  pub loader: PathBuf,
}

impl FakeAgent {
  /// None when the repository's node_modules are not installed (a Rust-only checkout): the calling test is skipped
  pub fn locate() -> Option<FakeAgent> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().ok()?;
    let fake = FakeAgent { script: root.join("test/fake-agent.ts"), loader: root.join("node_modules/tsx/dist/loader.mjs"), root };
    if fake.loader.is_file() && fake.script.is_file() {
      Some(fake)
    } else {
      eprintln!("skipped: {} is missing (run `pnpm install` in the repository root)", fake.loader.display());
      None
    }
  }

  /// The acpira.agents entry for the fake agent under id `fake`, merged with `extra` (env, modes, ignoreModes, …)
  pub fn setting(&self, extra: Value) -> Value {
    self.setting_as("fake", extra)
  }

  /// The same entry under another id (a custom entry with a built-in id replaces that agent: its quirks, the fake's wire)
  pub fn setting_as(&self, id: &str, extra: Value) -> Value {
    let mut entry = serde_json::json!({
      "name": "Fake",
      "command": "node",
      "args": ["--import", self.loader.to_string_lossy(), self.script.to_string_lossy()],
      "login": "echo login",
    });
    if let (Some(e), Value::Object(x)) = (entry.as_object_mut(), extra) {
      e.extend(x);
    }
    let mut out = serde_json::Map::new();
    out.insert(id.to_owned(), entry);
    Value::Object(out)
  }
}

/// Skips the test (returns) when the fake agent cannot run here
#[macro_export]
macro_rules! fake_or_skip {
  () => {
    match $crate::support::FakeAgent::locate() {
      Some(f) => f,
      None => return,
    }
  };
}

/// What AcpSession tests build their sessions from: a registry holding the fake agent, a blob store in a temp dir, a log
pub struct Harness {
  pub deps: SessionDeps,
  pub logs: Arc<Mutex<Vec<String>>>,
  pub changes: Arc<AtomicUsize>,
  pub dir: tempfile::TempDir,
  pub agent: String,
  listeners: Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<()>>>>,
}

impl Harness {
  pub fn new(fake: &FakeAgent, extra: Value) -> Harness {
    Harness::with_compaction(fake, extra, None)
  }

  pub fn with_compaction(fake: &FakeAgent, extra: Value, compaction: Option<CompactionPolicy>) -> Harness {
    Harness::with_compaction_fn(fake, extra, compaction.map(|p| Arc::new(move || p) as Arc<dyn Fn() -> CompactionPolicy + Send + Sync>))
  }

  pub fn with_compaction_fn(fake: &FakeAgent, extra: Value, compaction: Option<Arc<dyn Fn() -> CompactionPolicy + Send + Sync>>) -> Harness {
    Harness::build(fake, "fake", extra, compaction)
  }

  /// The fake agent registered under a built-in id, so that agent's host-side quirks apply
  pub fn for_agent(fake: &FakeAgent, id: &str, extra: Value) -> Harness {
    Harness::build(fake, id, extra, None)
  }

  fn build(fake: &FakeAgent, id: &str, extra: Value, compaction: Option<Arc<dyn Fn() -> CompactionPolicy + Send + Sync>>) -> Harness {
    let dir = tempfile::tempdir().expect("temp dir");
    let logs = Arc::new(Mutex::new(vec![]));
    let changes = Arc::new(AtomicUsize::new(0));
    let listeners: Arc<Mutex<Vec<tokio::sync::mpsc::UnboundedSender<()>>>> = Arc::new(Mutex::new(vec![]));
    let (l, c, ls) = (logs.clone(), changes.clone(), listeners.clone());
    let log: acpira_host::store::transcript_store::LogFn = Arc::new(move |line: &str| l.lock().unwrap().push(line.to_owned()));
    let deps = SessionDeps {
      registry: Arc::new(AgentRegistry::new(&fake.setting_as(id, extra))),
      log: log.clone(),
      on_change: Arc::new(move |_, _| {
        c.fetch_add(1, Ordering::SeqCst);
        ls.lock().unwrap().retain(|tx| tx.send(()).is_ok());
      }),
      blobs: TranscriptStore::new(dir.path().join("sessions"), log, None),
      notify: None,
      accounts: None,
      compaction,
      pool: None,
      model_shapes: None,
    };
    Harness { deps, logs, changes, dir, agent: id.to_owned(), listeners }
  }

  /// What the session looked like after each change notification (the TS suites read the view inside onChange; the Rust
  /// callback must not lock the session back, so a task reads it right after instead)
  pub fn sample<T: Send + 'static>(&self, s: &Arc<AcpSession>, f: impl Fn(&acpira_shared::transcript::SessionView) -> T + Send + 'static) -> Arc<Mutex<Vec<T>>> {
    let seen = Arc::new(Mutex::new(vec![]));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    self.listeners.lock().unwrap().push(tx);
    let (out, s) = (seen.clone(), Arc::downgrade(s));
    tokio::spawn(async move {
      while rx.recv().await.is_some() {
        let Some(s) = s.upgrade() else { break };
        out.lock().unwrap().push(f(&s.view()));
      }
    });
    seen
  }

  pub fn session(&self, cwd: &str) -> Arc<AcpSession> {
    AcpSession::fresh(&self.agent, cwd, self.deps.clone(), None)
  }

  pub fn logs(&self) -> Vec<String> {
    self.logs.lock().unwrap().clone()
  }

  pub fn changes(&self) -> usize {
    self.changes.load(Ordering::SeqCst)
  }
}

/// Polls until the condition holds (5 s by default in the TS suites)
pub async fn until(mut pred: impl FnMut() -> bool, ms: u64) {
  let t0 = Instant::now();
  while !pred() {
    assert!(t0.elapsed() < Duration::from_millis(ms), "timeout after {ms} ms");
    tokio::time::sleep(Duration::from_millis(20)).await;
  }
}

/// Disposes the session when the test ends, however it ends
pub struct Disposing(pub Arc<AcpSession>);

impl Drop for Disposing {
  fn drop(&mut self) {
    self.0.dispose();
  }
}

impl std::ops::Deref for Disposing {
  type Target = Arc<AcpSession>;
  fn deref(&self) -> &Arc<AcpSession> {
    &self.0
  }
}
