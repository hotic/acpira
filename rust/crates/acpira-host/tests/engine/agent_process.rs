//! test/AgentProcess.test.ts, test/AgentPool.test.ts and test/compaction-queue.test.ts: process lifecycle, warm pool, background compaction

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_host::acp::agent_pool::AgentPool;
use acpira_host::acp::agent_process::{AgentProcess, client_version};
use acpira_host::acp::agent_registry::{AgentDef, AgentRegistry};
use acpira_host::acp::session::AcpSession;
use acpira_shared::transcript::StrMap;

use crate::acp_session::{claimed, last_turn, prompt, turn_at, view};
use crate::fake_or_skip;
use crate::subagents::Recorder;
use crate::support::{Disposing, FakeAgent, Harness, expect_eq, expect_match, until};

fn fake_def(fake: &FakeAgent) -> (AgentDef, String) {
  let registry = AgentRegistry::new(&fake.setting(json!({})));
  (registry.get("fake").unwrap().clone(), "node".into())
}

async fn node(fake: &FakeAgent) -> String {
  AgentRegistry::new(&fake.setting(json!({}))).resolve_binary("fake").await.expect("node on PATH")
}

fn env(pairs: &[(&str, &str)]) -> StrMap {
  pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

async fn exited(rec: &Recorder) -> (Option<i32>, Option<String>) {
  until(|| rec.1.lock().unwrap().is_some(), 10_000).await;
  rec.1.lock().unwrap().clone().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_initialize_kills_the_child_instead_of_leaving_an_orphan() {
  let fake = fake_or_skip!();
  let (def, _) = fake_def(&fake);
  let rec = Recorder::default();
  let Err(err) = AgentProcess::spawn(&def, &node(&fake).await, "/tmp", Arc::new(rec.clone()), Some(&env(&[("FAKE_INIT_FAIL", "1"), ("FAKE_STUBBORN", "1")])), None).await else {
    panic!("initialize must fail");
  };
  assert!(err.to_string().contains("initialize refused"), "{err}");
  let (code, signal) = exited(&rec).await;
  assert!(matches!(signal.as_deref(), Some("SIGTERM" | "SIGKILL")) || code.is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn an_initialize_the_agent_never_answers_times_out_and_the_child_is_killed() {
  let fake = fake_or_skip!();
  let (def, _) = fake_def(&fake);
  let rec = Recorder::default();
  let Err(err) = AgentProcess::spawn(&def, &node(&fake).await, "/tmp", Arc::new(rec.clone()), Some(&env(&[("FAKE_INIT_HANG", "1")])), Some(Duration::from_millis(300))).await else {
    panic!("initialize must time out");
  };
  assert!(err.to_string().contains("initialize"), "{err}");
  let (code, signal) = exited(&rec).await;
  assert!(matches!(signal.as_deref(), Some("SIGTERM" | "SIGKILL")) || code.is_some());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn kill_escalates_to_sigkill_when_the_cli_ignores_the_polite_signal() {
  let fake = fake_or_skip!();
  let (def, _) = fake_def(&fake);
  let rec = Recorder::default();
  let proc = AgentProcess::spawn(&def, &node(&fake).await, "/tmp", Arc::new(rec.clone()), Some(&env(&[("FAKE_STUBBORN", "1")])), None).await.unwrap();
  assert!(proc.alive());
  let t0 = Instant::now();
  proc.kill().await;
  let (_, signal) = exited(&rec).await;
  assert_eq!(signal.as_deref(), Some("SIGKILL"));
  assert!(t0.elapsed() >= Duration::from_millis(1500));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_air_capabilities_are_advertised_and_native_subagent_sessions_can_be_opted_out() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let log = dir.path().join("init.log");
  let air = || -> Value {
    let text = std::fs::read_to_string(&log).unwrap();
    serde_json::from_str::<Value>(text.trim().lines().last().unwrap()).unwrap()["jetbrains"]["air"].clone()
  };
  let (mut def, _) = fake_def(&fake);
  let bin = node(&fake).await;
  let extra = env(&[("FAKE_INIT_LOG", log.to_str().unwrap())]);
  let proc = AgentProcess::spawn(&def, &bin, "/tmp", Arc::new(Recorder::default()), Some(&extra), None).await.unwrap();
  expect_eq(air(), json!({ "version": 1, "capabilities": ["nativeSubagentSessions", "sessionFailure", "asyncTasks", "recommendedValue"] }));
  proc.kill().await;
  def.subagents = false;
  let proc2 = AgentProcess::spawn(&def, &bin, "/tmp", Arc::new(Recorder::default()), Some(&extra), None).await.unwrap();
  expect_eq(&air()["capabilities"], json!(["sessionFailure", "asyncTasks", "recommendedValue"]));
  proc2.kill().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_client_info_carries_the_extension_version() {
  let fake = fake_or_skip!();
  let (def, _) = fake_def(&fake);
  let proc = AgentProcess::spawn(&def, &node(&fake).await, "/tmp", Arc::new(Recorder::default()), None, None).await.unwrap();
  let package: Value = serde_json::from_str(&std::fs::read_to_string(fake.root.join("package.json")).unwrap()).unwrap();
  assert_eq!(client_version(), package["version"].as_str().unwrap());
  assert_eq!(proc.init["agentInfo"]["name"], "fake");
  proc.kill().await;
}

fn pool(fake: &FakeAgent, logs: Arc<std::sync::Mutex<Vec<String>>>) -> Arc<AgentPool> {
  let registry = Arc::new(AgentRegistry::new(&fake.setting(json!({}))));
  AgentPool::new(Arc::new(move || registry.clone()), Arc::new(move |l: &str| logs.lock().unwrap().push(l.to_owned())), None)
}

#[tokio::test(flavor = "multi_thread")]
async fn ensure_and_take_hand_over_an_initialized_process_and_a_second_take_misses() {
  let fake = fake_or_skip!();
  let logs = Arc::new(std::sync::Mutex::new(vec![]));
  let p = pool(&fake, logs.clone());
  p.ensure("fake", "/tmp", None);
  let first = p.take("fake", "/tmp", None, Arc::new(Recorder::default())).await.expect("warm process");
  assert!(first.alive());
  assert!(logs.lock().unwrap().iter().any(|l| l.contains("warm fake")));
  let session = first.request("session/new", json!({ "cwd": "/tmp", "mcpServers": [] })).await.unwrap();
  assert!(session["sessionId"].as_str().is_some_and(|s| !s.is_empty()));
  assert!(p.take("fake", "/tmp", None, Arc::new(Recorder::default())).await.is_none());
  first.kill().await;
  p.dispose().1.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn invalidate_drops_a_ready_process_so_the_next_take_misses() {
  let fake = fake_or_skip!();
  let logs = Arc::new(std::sync::Mutex::new(vec![]));
  let p = pool(&fake, logs.clone());
  p.ensure("fake", "/tmp", None);
  until(|| logs.lock().unwrap().iter().any(|l| l.contains("initialize ok")), 10_000).await;
  p.invalidate(None);
  assert!(p.take("fake", "/tmp", None, Arc::new(Recorder::default())).await.is_none());
  p.dispose().1.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn disposing_a_warming_slot_leaves_nothing_to_take() {
  let fake = fake_or_skip!();
  let p = pool(&fake, Arc::new(std::sync::Mutex::new(vec![])));
  p.ensure("fake", "/tmp", None);
  let done = p.dispose().1;
  assert!(p.take("fake", "/tmp", None, Arc::new(Recorder::default())).await.is_none());
  done.await;
}

/// The fake agent under a built-in id with that agent's compaction dialect, and a switchable auto-compaction policy
fn compaction_fixture(fake: &FakeAgent, agent: &str, auto: bool, extra_env: Value) -> (Harness, Arc<std::sync::atomic::AtomicBool>) {
  let flag = Arc::new(std::sync::atomic::AtomicBool::new(auto));
  let f = flag.clone();
  let mut env = json!({ "FAKE_COMPACTION": agent });
  if let (Some(e), Value::Object(x)) = (env.as_object_mut(), extra_env) {
    e.extend(x);
  }
  let mut h = Harness::for_agent(fake, agent, json!({ "env": env }));
  h.deps.compaction = Some(Arc::new(move || acpira_host::acp::session::CompactionPolicy { at_tokens: 300_000.0, auto: f.load(std::sync::atomic::Ordering::SeqCst) }));
  (h, flag)
}

async fn session(h: &Harness) -> Disposing {
  let s = Disposing(h.session("/tmp"));
  s.start().await;
  s
}

fn done_count(h: &Harness) -> usize {
  h.logs().iter().filter(|l| l.contains("prompt done:")).count()
}

fn user_texts(s: &AcpSession) -> Vec<String> {
  view(s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user").map(|t| t["text"].as_str().unwrap().to_owned()).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_accepted_prompt_shows_below_pre_send_compaction_without_going_out_early() {
  let fake = fake_or_skip!();
  for agent in ["devin", "kimi"] {
    let (h, auto) = compaction_fixture(&fake, agent, false, json!({}));
    let s = session(&h).await;
    prompt(&s, "big").await;
    auto.store(true, std::sync::atomic::Ordering::SeqCst);
    let sent = tokio::spawn(s.prompt("follow-up".into(), vec![], false, None, None));
    until(|| h.logs().iter().any(|l| l.contains("waiting for compaction completion")), 5000).await;
    s.set_config("effort".into(), "low".into()).await.ok();
    let pending = last_turn(&view(&s));
    expect_match(&pending, json!({ "role": "user", "text": "follow-up" }));
    expect_match(turn_at(&view(&s), -2), json!({ "role": "agent", "blocks": [{ "type": "text" }] }));
    assert_eq!(done_count(&h), 2, "{agent}");
    s.set_config("effort".into(), "high".into()).await.ok();
    sent.await.unwrap();
    until(|| last_turn(&view(&s))["stop"] == "end_turn", 5000).await;
    assert_eq!(turn_at(&view(&s), -2), pending);
    expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "end_turn" }));
    assert_eq!(done_count(&h), 3, "{agent}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn kimi_cancellation_releases_a_pending_usage_refresh_without_late_compaction() {
  let fake = fake_or_skip!();
  let (h, _) = compaction_fixture(&fake, "kimi", true, json!({}));
  let s = session(&h).await;
  let p = tokio::spawn(s.prompt("delayed-usage".into(), vec![], false, None, None));
  until(|| done_count(&h) > 0, 5000).await;
  s.cancel().await;
  p.await.unwrap();
  assert!(!s.is_running());
  until(|| view(&s)["usage"]["used"] == 350_000, 5000).await;
  assert!(!view(&s)["turns"].as_array().unwrap().iter().any(|t| t["role"] == "user" && t["auto"] == true));
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "cancelled" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn kimi_waits_for_the_post_response_usage_before_dispatching_a_queued_follow_up() {
  let fake = fake_or_skip!();
  let (h, _) = compaction_fixture(&fake, "kimi", true, json!({}));
  let s = session(&h).await;
  let p = tokio::spawn(s.prompt("delayed-usage".into(), vec![], false, None, None));
  until(|| done_count(&h) > 0, 5000).await;
  prompt(&s, "follow-up").await;
  assert_eq!(user_texts(&s), ["delayed-usage"]);
  until(|| done_count(&h) == 2, 5000).await;
  s.set_config("effort".into(), "high".into()).await.ok();
  p.await.unwrap();
  until(|| !s.is_running() && user_texts(&s).len() == 3, 5000).await;
  assert_eq!(user_texts(&s), ["delayed-usage", "/compact", "follow-up"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn usage_arriving_after_the_prompt_acknowledgement_is_evaluated() {
  let fake = fake_or_skip!();
  for agent in ["devin", "kimi"] {
    let (h, _) = compaction_fixture(&fake, agent, true, json!({}));
    let s = session(&h).await;
    prompt(&s, "delayed-usage").await;
    if agent == "devin" {
      assert!(!s.is_running());
    }
    until(|| h.logs().iter().any(|l| l.contains("auto /compact")), 5000).await;
    let autos = || view(&s)["turns"].as_array().unwrap().iter().filter(|t| t["role"] == "user" && t["auto"] == true).count();
    // The logged decision and the queued auto turn land a moment apart (the turn is appended as the prompt claims the session)
    until(|| autos() > 0, 2000).await;
    assert_eq!(autos(), 1, "{agent}");
    until(|| done_count(&h) == 2, 5000).await;
    s.set_config("effort".into(), "high".into()).await.ok();
    until(|| !s.is_running(), 5000).await;
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follow_up_is_held_after_the_compact_rpc_returns_until_compaction_completes() {
  let fake = fake_or_skip!();
  for agent in ["devin", "kimi", "structured"] {
    let (h, _) = compaction_fixture(&fake, agent, false, json!({}));
    let s = session(&h).await;
    prompt(&s, "hi").await;
    h.logs.lock().unwrap().clear();
    let compact = claimed({
      let s = s.0.clone();
      async move { s.compact(false).await }
    });
    until(|| done_count(&h) > 0, 5000).await;
    assert!(s.is_running(), "{agent}");
    prompt(&s, "follow-up").await;
    assert_eq!(view(&s)["queued"].as_array().unwrap().iter().map(|q| q["text"].clone()).collect::<Vec<_>>(), [json!("follow-up")]);
    assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 4);
    s.set_config("effort".into(), "low".into()).await.ok();
    assert!(s.is_running(), "{agent}");
    s.set_config("effort".into(), "high".into()).await.ok();
    compact.await.unwrap().ok();
    until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().len() == 6, 5000).await;
    let vw = view(&s);
    assert!(vw["queued"].is_null());
    expect_match(&vw["turns"][4], json!({ "role": "user", "text": "follow-up" }));
    expect_match(&vw["turns"][5], json!({ "role": "agent", "stop": "end_turn" }));
    assert!(vw["turns"][5]["blocks"].as_array().unwrap().iter().any(|b| b["type"] == "text" && b["markdown"] == "hello world"));
    assert!(!h.logs().iter().any(|l| l.ends_with("] cancel")), "{agent}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_compaction_keeps_the_queue_and_records_usage_after_completion() {
  let fake = fake_or_skip!();
  for agent in ["devin", "kimi"] {
    let (h, _) = compaction_fixture(&fake, agent, true, json!({}));
    let s = session(&h).await;
    prompt(&s, "big").await;
    until(|| done_count(&h) == 2, 5000).await;
    assert!(s.is_running(), "{agent}");
    expect_eq(&view(&s)["turns"][2], json!({ "role": "user", "text": "/compact", "auto": true }));
    // 401234 from the "big" prompt; the fake compaction drops it to a fifth
    let before = view(&s)["usage"]["used"].as_f64().unwrap();
    assert_eq!(before, 401234.0);
    prompt(&s, "follow-up").await;
    s.set_config("effort".into(), "high".into()).await.ok();
    until(|| !s.is_running() && view(&s)["turns"].as_array().unwrap().len() == 6, 5000).await;
    // Kimi pushes no usage_update here: the reading is adopted from its completion prose
    assert_eq!(view(&s)["usage"]["used"].as_f64().unwrap(), (before * 0.2).round(), "{agent}");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancel_releases_a_background_wait_even_without_a_prose_confirmation() {
  let fake = fake_or_skip!();
  let (h, _) = compaction_fixture(&fake, "devin", false, json!({ "FAKE_SILENT_CANCEL": "1" }));
  let s = session(&h).await;
  prompt(&s, "hi").await;
  h.logs.lock().unwrap().clear();
  let compact = claimed({
    let s = s.0.clone();
    async move { s.compact(false).await }
  });
  until(|| h.logs().iter().any(|l| l.contains("waiting for compaction completion")), 5000).await;
  assert!(s.is_running());
  s.cancel().await;
  compact.await.unwrap().ok();
  assert!(!s.is_running());
  assert_eq!(view(&s)["status"], "ready");
  prompt(&s, "after").await;
  expect_match(turn_at(&view(&s), -2), json!({ "role": "user", "text": "after" }));
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_releases_a_background_wait_without_dispatching_the_queued_prompt() {
  let fake = fake_or_skip!();
  let (h, _) = compaction_fixture(&fake, "devin", false, json!({}));
  let s = session(&h).await;
  prompt(&s, "hi").await;
  h.logs.lock().unwrap().clear();
  let compact = claimed({
    let s = s.0.clone();
    async move { s.compact(false).await }
  });
  until(|| done_count(&h) > 0, 5000).await;
  prompt(&s, "follow-up").await;
  s.dispose();
  compact.await.unwrap().ok();
  assert_eq!(view(&s)["status"], "closed");
  assert_eq!(view(&s)["turns"].as_array().unwrap().len(), 4);
}
