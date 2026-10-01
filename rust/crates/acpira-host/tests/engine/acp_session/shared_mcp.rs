//! Shared MCP servers (`~/.agents/mcp.json`, the project's `.mcp.json`) reach session/new and a reopened session's
//! resume, filtered by the agent's advertised transports

use std::path::Path;

use acpira_host::shared_config::mcp_provider;

use super::*;

/// A harness whose fake logs every mcpServers it receives, with the shared provider over a temp home
fn shared_harness(fake: &FakeAgent, home: &Path, native: &Path, log: &Path) -> Harness {
  let mut h = Harness::new(fake, json!({ "env": { "FAKE_MCP_TRACE": log, "FAKE_SESSION_DIR": native } }));
  let home = home.to_string_lossy().into_owned();
  h.deps.shared_mcp = Some(mcp_provider(Arc::new(move || home.clone())));
  h
}

fn received(log: &Path) -> Vec<(String, Value)> {
  std::fs::read_to_string(log)
    .unwrap_or_default()
    .lines()
    .map(|l| {
      let (method, json) = l.split_once(' ').unwrap();
      (method.to_owned(), serde_json::from_str(json).unwrap())
    })
    .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_servers_reach_new_and_resumed_sessions() {
  let fake = fake_or_skip!();
  let t = tempfile::tempdir().unwrap();
  let (home, native, repo, log) = (t.path().join("home"), t.path().join("native"), t.path().join("repo"), t.path().join("mcp.log"));
  std::fs::create_dir_all(home.join(".agents")).unwrap();
  std::fs::create_dir_all(&native).unwrap();
  std::fs::create_dir_all(repo.join(".git")).unwrap();
  std::fs::create_dir_all(repo.join(".agents")).unwrap();
  std::fs::write(
    home.join(".agents/mcp.json"),
    r#"{ "mcpServers": {
      "files": { "command": "/bin/cat" },
      "web": { "type": "http", "url": "https://example.invalid/mcp" },
      "old": { "type": "sse", "url": "https://example.invalid/sse" },
      "off": { "command": "/bin/cat", "disabled": true }
    } }"#,
  )
  .unwrap();
  // The project's server of the same name wins
  std::fs::write(repo.join(".mcp.json"), r#"{ "mcpServers": { "files": { "command": "/bin/echo", "args": ["p"] } } }"#).unwrap();
  let h = shared_harness(&fake, &home, &native, &log);
  let cwd = repo.join("sub");
  std::fs::create_dir_all(&cwd).unwrap();
  let record = ran_once(&h, &cwd.to_string_lossy()).await;
  let s = reopened(&h, record).await;
  prompt(&s, "again").await;

  let got = received(&log);
  let methods: Vec<&str> = got.iter().map(|(m, _)| m.as_str()).collect();
  assert_eq!(methods, ["new", "resume"]);
  for (_, servers) in &got {
    expect_eq(
      servers,
      json!([
        { "name": "files", "command": "/bin/echo", "args": ["p"], "env": [] },
        { "type": "http", "name": "web", "url": "https://example.invalid/mcp", "headers": [] },
      ]),
    );
  }
  assert!(h.logs().iter().any(|l| l.contains("shared MCP: files, web; skipped old (transport not advertised)")), "{:?}", h.logs());
}

#[tokio::test(flavor = "multi_thread")]
async fn without_shared_files_the_request_stays_empty() {
  let fake = fake_or_skip!();
  let t = tempfile::tempdir().unwrap();
  let (home, native, log) = (t.path().join("home"), t.path().join("native"), t.path().join("mcp.log"));
  std::fs::create_dir_all(&native).unwrap();
  let h = shared_harness(&fake, &home, &native, &log);
  ran_once(&h, "/tmp").await;
  assert_eq!(received(&log), [("new".to_owned(), json!([]))]);
  assert!(!h.logs().iter().any(|l| l.contains("shared MCP")));
}

// antigravity-acp stops every turn while a server it was given cannot start: that server is left out of the session and
// the connection rebuilt, so the restored session carries the rest. One from the agent's own config is only reported
#[tokio::test(flavor = "multi_thread")]
async fn a_server_antigravity_cannot_start_is_left_out_after_a_reconnect() {
  let fake = fake_or_skip!();
  let t = tempfile::tempdir().unwrap();
  let (home, native, log) = (t.path().join("home"), t.path().join("native"), t.path().join("mcp.log"));
  std::fs::create_dir_all(home.join(".agents")).unwrap();
  std::fs::create_dir_all(&native).unwrap();
  std::fs::write(home.join(".agents/mcp.json"), r#"{ "mcpServers": { "files": { "command": "/bin/cat" }, "hilfa": { "command": "/bin/cat" } } }"#).unwrap();
  let mut h = Harness::for_agent(&fake, "antigravity", json!({ "env": { "FAKE_MCP_TRACE": log, "FAKE_SESSION_DIR": native } }));
  let home_s = home.to_string_lossy().into_owned();
  h.deps.shared_mcp = Some(mcp_provider(Arc::new(move || home_s.clone())));
  let s = started(&h, "/tmp").await;
  let native_id = s.to_record().acp_session_id;
  let fix = "Fix the MCP server, or remove it from your configuration and start a new session, to continue.";
  prompt(&s, &format!("say:The MCP server 'hilfa' failed to initialize: dial tcp [::1]:4262: connect: connection refused. {fix}")).await;
  let turn = last_turn(&view(&s));
  expect_match(&turn, json!({ "stop": "error", "error": { "kind": "mcp_failed", "retryable": true } }));
  let message = turn["error"]["message"].as_str().unwrap().to_owned();
  assert!(message.contains("hilfa") && message.contains("connection refused") && message.contains("Retry"), "{message}");
  until(|| received(&log).len() == 2 && view(&s)["status"] == "ready", 10_000).await;
  let got = received(&log);
  assert_eq!(got[0].1.as_array().unwrap().len(), 2);
  assert_ne!(got[1].0, "new");
  expect_eq(&got[1].1, json!([{ "name": "files", "command": "/bin/cat", "args": [], "env": [] }]));
  assert_eq!(s.to_record().acp_session_id, native_id);

  prompt(&s, &format!("say:The MCP server 'own' failed to initialize. {fix}")).await;
  let message = last_turn(&view(&s))["error"]["message"].as_str().unwrap().to_owned();
  assert!(message.contains("mcp_config.json"), "{message}");
  tokio::time::sleep(std::time::Duration::from_millis(300)).await;
  assert_eq!(received(&log).len(), 2);
}
