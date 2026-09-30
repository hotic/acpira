//! Restoring records, reopening native sessions, locks, lost sessions and dispose

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn restores_interrupted_turns_and_old_background_tools_as_stopped() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let original = Disposing(h.session("/tmp"));
  let mut record = original.to_record();
  record.updated_at = iso_of_ms(5000);
  record.turns = turns(json!([
    { "role": "user", "text": "work" },
    { "role": "agent", "startedAt": 1000, "blocks": [
      { "type": "text", "markdown": "partial", "streaming": true },
      { "type": "tool_call", "id": "server", "kind": "execute", "verb": "Run", "status": "in_progress", "startedAt": 2000, "background": true },
      { "type": "tool_call", "id": "wait", "kind": "other", "verb": "Wait", "status": "pending" },
      { "type": "compaction", "id": "compact", "status": "in_progress" },
    ] },
    { "role": "user", "text": "continue" },
    { "role": "agent", "startedAt": 4000, "endedAt": 5000, "blocks": [], "stop": "error", "error": { "message": "failed" } },
  ]));
  let before = serde_json::to_string(&record).unwrap();
  let restored = Disposing(AcpSession::new(record.clone(), h.deps.clone()));
  assert!(!restored.is_running());
  let rv = view(&restored);
  expect_match(&rv["turns"][1], json!({ "stop": "cancelled", "endedAt": 5000, "blocks": [
    { "streaming": false }, { "status": "cancelled", "endedAt": 5000 }, { "status": "cancelled" }, { "status": "cancelled" },
  ] }));
  expect_eq(&rv["turns"][3], v(&record.turns[3]));
  assert_eq!(serde_json::to_string(&record).unwrap(), before);
  let again = Disposing(AcpSession::new(restored.to_record(), h.deps.clone()));
  assert_eq!(view(&again)["turns"], rv["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn keeps_empty_slash_receipts_and_observed_settings_across_persistence() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "/silent").await;
  expect_match(last_turn(&view(&s)), json!({ "blocks": [], "stop": "end_turn", "command": { "name": "silent" } }));
  prompt(&s, "/silent-plan").await;
  expect_match(last_turn(&view(&s)), json!({ "blocks": [], "stop": "end_turn", "command": { "name": "silent-plan", "mode": "Plan" } }));
  prompt(&s, "/silent-plan").await;
  expect_match(last_turn(&view(&s)), json!({ "command": { "name": "silent-plan" } }));
  expect_absent(last_turn(&view(&s)), "command.mode");
  prompt(&s, "/slash-error").await;
  let last = last_turn(&view(&s));
  expect_match(&last, json!({ "stop": "error" }));
  assert!(last["error"]["message"].as_str().unwrap().contains("Unknown command"));
  prompt(&s, "ordinary message").await;
  expect_absent(last_turn(&view(&s)), "command");
  let restored = Disposing(AcpSession::new(s.to_record(), h.deps.clone()));
  assert_eq!(view(&restored)["turns"], view(&s)["turns"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_an_older_session_filters_repeated_completed_plan_snapshots() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let original = Disposing(h.session("/tmp"));
  let mut record = original.to_record();
  record.turns = turns(json!([
    { "role": "agent", "blocks": [{ "type": "plan", "entries": [{ "title": "Done", "status": "completed" }] }] },
    { "role": "user", "text": "Follow-up" },
    { "role": "agent", "blocks": [{ "type": "text", "markdown": "Answer" }, { "type": "plan", "entries": [{ "title": "Done", "status": "completed" }] }] },
  ]));
  let restored = Disposing(AcpSession::new(record.clone(), h.deps.clone()));
  expect_match(&view(&restored)["turns"][2], json!({ "blocks": [{ "type": "text", "markdown": "Answer" }] }));
  assert_eq!(restored.to_record().turns.len(), 3);
  expect_match(&record.turns[2], json!({ "blocks": [{ "type": "text" }, { "type": "plan" }] }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resume_unknown_to_the_new_process_leaves_the_history_read_only() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, "/tmp").await;
  // new process answers invalidParams "unknown session" — the peer doesn't know the id, the same conclusion as session_not_found:
  // the transcript already ran, so it stays read-only instead of silently continuing on a fresh native context
  let s2 = reopened(&h, record.clone()).await;
  let vw = view(&s2);
  assert_eq!(vw["status"], "readonly");
  assert!(vw["error"].as_str().unwrap().contains("no longer has this session"));
  assert_eq!(turns_in(&vw), 2);
  assert_eq!(s2.to_record().acp_session_id, record.acp_session_id);
  // No fresh native session was opened, so the persisted command list stays until a peer replaces it
  expect_eq(&vw["commands"], json!([{ "name": "compact", "description": "compact it" }]));
}

// DeepSeek Harness reports every restore problem as a bare invalidParams; the reason only survives in the message text
#[tokio::test(flavor = "multi_thread")]
async fn dsh_restore_failures_are_told_apart_by_their_message() {
  let fake = fake_or_skip!();
  for (tag, status, error) in [
    ("dsh-active", "error", "held by another"),
    ("dsh-cwd", "error", "Could not restore"),
    ("dsh-unresumable", "readonly", "cannot resume this session"),
    ("dsh-mcp", "error", "Could not restore"),
  ] {
    let dir = tempfile::Builder::new().prefix(&format!("acpira-{tag}-")).tempdir().unwrap();
    let h = Harness::new(&fake, json!({}));
    let record = ran_once(&h, dir.path().to_str().unwrap()).await;
    let s2 = reopened(&h, record).await;
    let vw = view(&s2);
    assert_eq!(vw["status"], status, "{tag}");
    assert!(vw["error"].as_str().unwrap_or("").contains(error), "{tag}: {}", vw["error"]);
    assert_eq!(turns_in(&vw), 2);
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn dispose_sends_session_close_to_an_agent_that_advertises_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let close_log = dir.path().join("close.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_CLOSE_LOG": close_log } }));
  let s = h.session("/tmp");
  s.start().await;
  prompt(&s, "hi").await;
  let native = s.to_record().acp_session_id.unwrap();
  s.dispose();
  until(|| std::fs::read_to_string(&close_log).is_ok_and(|t| t.contains(&native)), 5000).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn importing_a_native_session_the_agent_no_longer_has_lands_on_the_error_state() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_SESSION_DIR": dir.path() } }));
  let now = acpira_host::util::now_iso();
  let record: SessionRecord = serde_json::from_value(json!({
    "id": uuid::Uuid::new_v4().to_string(), "agent": "fake", "acpSessionId": "native-gone", "cwd": "/tmp", "title": "Imported session",
    "createdAt": now, "updatedAt": now, "turns": [], "controls": { "modes": [], "options": [] }, "commands": [],
    "importPending": true, "importedFrom": { "sessionId": "native-gone" },
  })).unwrap();
  let s = reopened(&h, record).await;
  // session/load answered session_not_found; an import has no transcript to keep read-only, so it is the error Notice (Retry) — and no fresh native session was created
  assert_eq!(view(&s)["status"], "error");
  assert_eq!(view(&s)["error"], "The agent no longer has this session");
  assert_eq!(h.logs().iter().filter(|l| l.contains("session/new ok")).count(), 0);
  expect_match(s.to_record(), json!({ "acpSessionId": "native-gone", "importedFrom": { "sessionId": "native-gone" } }));
  assert!(!s.to_record().import_pending);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_restore_attempt_lands_on_the_error_state_and_retry_reconnects() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-flaky-resume-").tempdir().unwrap();
  std::fs::write(dir.path().join("resume.lock"), "").unwrap();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, dir.path().to_str().unwrap()).await;
  // resume answers -32603 while resume.lock exists: an internal error is not "can't resume" — the session goes to
  // the error Notice (Retry = full reconnect + resume), not to read-only
  let s2 = reopened(&h, record.clone()).await;
  assert_eq!(view(&s2)["status"], "error");
  assert!(view(&s2)["error"].as_str().unwrap().contains("transient restore failure"));
  std::fs::remove_file(dir.path().join("resume.lock")).unwrap();
  s2.retry().await.unwrap();
  assert_eq!(view(&s2)["status"], "ready");
  assert_eq!(turn_count(&s2), 2);
  assert_eq!(s2.to_record().acp_session_id, record.acp_session_id);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_typed_session_locked_is_reported_as_held_elsewhere() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-locked-").tempdir().unwrap();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, dir.path().to_str().unwrap()).await;
  let s2 = reopened(&h, record).await;
  assert_eq!(view(&s2)["status"], "error");
  assert!(view(&s2)["error"].as_str().unwrap().contains("held by another"));
  assert_eq!(turn_count(&s2), 2);
  assert!(view(&s2)["canTakeOver"].is_null());
}

/// A dropped Remote-SSH connection leaves the old extension host, its sidecar and that sidecar's agent running; the
/// reconnected window's resume then meets the lock. Only a holder whose parent is another `acpira` binary is offered for
/// take-over; any other process (a terminal CLI, this test binary's child) keeps the plain retryable error
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_lock_held_by_another_sidecars_agent_can_be_taken_over() {
  use std::io::BufRead;
  use std::process::{Command, Stdio};
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-locked-").tempdir().unwrap();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, dir.path().to_str().unwrap()).await;
  let alive = |pid: u32| unsafe { libc::kill(pid as libc::pid_t, 0) } == 0;

  let mut stranger = Command::new("sleep").arg("30").spawn().unwrap();
  std::fs::write(dir.path().join("holder.pid"), stranger.id().to_string()).unwrap();
  let s = reopened(&h, record.clone()).await;
  assert_eq!(view(&s)["status"], "error");
  assert!(view(&s)["error"].as_str().unwrap().contains(&format!("PID {}", stranger.id())));
  assert!(view(&s)["canTakeOver"].is_null());
  // Not offered, so a stray takeOverSession leaves the stranger alone and just retries
  s.take_over().await.unwrap();
  assert!(alive(stranger.id()));
  assert_eq!(view(&s)["status"], "error");
  drop(s);
  stranger.kill().unwrap();
  stranger.wait().unwrap();

  // A stand-in sidecar: a shell run through a symlink named acpira (macOS SIGKILLs a copied system binary), whose child
  // plays the orphaned agent
  let bin = dir.path().join("bin");
  std::fs::create_dir(&bin).unwrap();
  let shell = ["/bin/bash", "/usr/bin/bash"].into_iter().find(|p| std::path::Path::new(p).exists()).unwrap();
  std::os::unix::fs::symlink(shell, bin.join("acpira")).unwrap();
  let mut sidecar =
    Command::new(bin.join("acpira")).args(["-c", "sleep 30 & echo $!; wait"]).stdout(Stdio::piped()).spawn().unwrap();
  let mut line = String::new();
  std::io::BufReader::new(sidecar.stdout.take().unwrap()).read_line(&mut line).unwrap();
  let agent: u32 = line.trim().parse().unwrap();
  std::fs::write(dir.path().join("holder.pid"), agent.to_string()).unwrap();
  let s = reopened(&h, record).await;
  assert_eq!(view(&s)["status"], "error");
  assert_eq!(view(&s)["canTakeOver"], true);
  s.take_over().await.unwrap();
  assert!(!alive(agent));
  assert_eq!(view(&s)["status"], "ready");
  assert!(view(&s)["canTakeOver"].is_null());
  assert_eq!(turn_count(&s), 2);
  assert!(h.logs().iter().any(|l| l.contains(&format!("taking the session over from pid {agent}"))));
  let _ = sidecar.kill();
  sidecar.wait().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn session_not_found_replaces_a_session_that_never_talked_and_keeps_history_read_only() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-gone").unwrap();
  let h = Harness::new(&fake, json!({}));
  let record = ran_once(&h, "/tmp/acpira-gone").await;
  // The transcript already ran: swapping in a fresh native session would keep the old conversation on an empty
  // context (compaction included). Read-only, history kept, the native id retained so a later open can retry
  let s2 = reopened(&h, record.clone()).await;
  assert_eq!(view(&s2)["status"], "readonly");
  assert_eq!(turn_count(&s2), 2);
  assert_eq!(s2.to_record().acp_session_id, record.acp_session_id);
  let new_ok = || h.logs().iter().filter(|l| l.contains("session/new ok")).count();
  assert_eq!(new_ok(), 1);
  s2.dispose();
  // An empty session (Devin sweeps exactly those when its process exits) is replaced transparently — nothing visible lost its context
  let empty = started(&h, "/tmp/acpira-gone").await;
  let empty_record = empty.to_record();
  empty.dispose();
  let s3 = reopened(&h, empty_record).await;
  assert_eq!(view(&s3)["status"], "ready");
  assert_eq!(turn_count(&s3), 0);
  assert_eq!(new_ok(), 3);
  // The replacement native session advertised nothing: the old connection's slash commands do not carry over
  assert_eq!(view(&s3)["commands"], json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_prompt_answered_session_not_found_leaves_ready_and_retry_reconnects() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp").await;
  prompt(&s, "hi").await;
  prompt(&s, "prompt-session-gone").await;
  expect_match(last_turn(&view(&s)), json!({ "role": "agent", "stop": "error" }));
  // the native session is gone — resending over this connection could only fail the same way
  assert_eq!(view(&s)["status"], "error");
  // Retry = reconnect + resume; the fresh process doesn't know the id either → read-only history, still no silent context swap
  s.retry().await.ok();
  assert_eq!(view(&s)["status"], "readonly");
  assert_eq!(turn_count(&s), 4);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_async_task_restores_with_observation_unknown_and_its_last_state() {
  let fake = fake_or_skip!();
  let h = Harness::new(&fake, json!({}));
  let original = Disposing(h.session("/tmp"));
  let mut record = original.to_record();
  record.updated_at = iso_of_ms(5000);
  record.turns = turns(json!([
    { "role": "user", "text": "work" },
    { "role": "agent", "startedAt": 1000, "blocks": [
      { "type": "tool_call", "id": "bg", "kind": "execute", "verb": "Run", "status": "in_progress", "startedAt": 2000, "background": true,
        "asyncTask": { "id": "t1", "state": "running", "canStop": true, "stopRequested": true, "name": "sleep 120" } },
    ] },
  ]));
  let restored = Disposing(AcpSession::new(record, h.deps.clone()));
  let row = view(&restored)["turns"][1]["blocks"][0].clone();
  expect_match(&row, json!({ "status": "cancelled", "observation": "unknown", "background": true }));
  expect_match(&row["asyncTask"], json!({ "id": "t1", "state": "running", "canStop": false, "name": "sleep 120" }));
  expect_absent(&row["asyncTask"], "stopRequested");
}

#[tokio::test(flavor = "multi_thread")]
async fn session_new_hands_the_agent_the_host_mcp_server_and_drops_it_for_an_agent_that_refuses_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let log = dir.path().join("mcp.log");
  let lines = |log: &std::path::Path| std::fs::read_to_string(log).unwrap_or_default().lines().map(str::to_owned).collect::<Vec<_>>();

  let mut h = Harness::new(&fake, json!({ "env": { "FAKE_MCP_LOG": log } }));
  h.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new("/opt/acpira/bin/acpira"));
  let s = started(&h, "/tmp").await;
  expect_match(view(&s), json!({ "status": "ready" }));
  assert_eq!(lines(&log), [r#"["acpira"]"#]);

  // An agent that fails session/new with the server still gets a session, and later sessions leave the server out
  let rejecting = dir.path().join("reject.log");
  let mut h = Harness::new(&fake, json!({ "env": { "FAKE_MCP_LOG": rejecting, "FAKE_MCP_REJECT": "1" } }));
  h.deps.host_mcp = Some(acpira_host::host_mcp::HostMcp::new("/opt/acpira/bin/acpira"));
  let first = started(&h, "/tmp").await;
  expect_match(view(&first), json!({ "status": "ready" }));
  let second = started(&h, "/tmp").await;
  expect_match(view(&second), json!({ "status": "ready" }));
  assert_eq!(lines(&rejecting), [r#"["acpira"]"#, "[]", "[]"]);
}
