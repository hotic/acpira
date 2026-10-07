//! Login flows and terminal auth methods

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn claude_repeated_401_ends_the_running_turn_without_losing_partial_output() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({}));
  let dir = tempfile::tempdir().unwrap();
  let s = started(&h, dir.path().to_str().unwrap()).await;
  tokio::time::timeout(std::time::Duration::from_secs(5), prompt(&s, "claude-auth-retry")).await.expect("auth rejection must end the RPC");
  let v = view(&s);
  assert_eq!(v["status"], "auth_required");
  assert_eq!(v["running"], false);
  let turn = last_turn(&v);
  assert_eq!(turn["stop"], "error");
  assert_eq!(turn["error"]["kind"], "authentication_failed");
  assert!(turn["error"]["message"].as_str().unwrap().contains("401"));
  assert_eq!(agent_text(&turn), "partial answer");
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_first_401_child_retries_and_rate_limits_allow_the_root_to_recover() {
  let fake = fake_or_skip!();
  let h = Harness::for_agent(&fake, "claude", json!({}));
  let dir = tempfile::tempdir().unwrap();
  let s = started(&h, dir.path().to_str().unwrap()).await;
  prompt(&s, "claude-auth-retry-recovered").await;
  assert_eq!(view(&s)["status"], "ready");
  assert_eq!(last_turn(&view(&s))["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_auth_probe_uses_the_bound_accounts_environment_over_the_agent_default() {
  use acpira_host::acp::session::SessionAccountHooks;
  use acpira_host::acp::transport::{process::AgentProcess, rpc::BoxFuture};
  use acpira_shared::transcript::StrMap;

  struct AccountEnv(StrMap);
  impl SessionAccountHooks for AccountEnv {
    fn spawn_env(&self, _: String, _: String) -> BoxFuture<Option<StrMap>> {
      let env = self.0.clone();
      Box::pin(async move { Some(env) })
    }
    fn authenticate(&self, _: String, _: String, _: Arc<AgentProcess>) -> BoxFuture<anyhow::Result<()>> {
      Box::pin(async { Ok(()) })
    }
  }

  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let default_status = dir.path().join("default.json");
  let account_status = dir.path().join("account.json");
  std::fs::write(&default_status, r#"{"loggedIn":true,"authMethod":"oauth_token","apiProvider":"firstParty"}"#).unwrap();
  std::fs::write(&account_status, r#"{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty"}"#).unwrap();
  let mut h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_CLAUDE_AUTH_FILE": default_status } }));
  h.deps.accounts = Some(Arc::new(AccountEnv([("FAKE_CLAUDE_AUTH_FILE".into(), account_status.to_string_lossy().into_owned())].into())));
  let s = Disposing(AcpSession::fresh("claude", dir.path().to_str().unwrap(), h.deps.clone(), Some("signed-out".into())));
  s.start().await;
  assert_eq!(view(&s)["status"], "auth_required");
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_signed_out_is_blocked_before_a_turn_and_login_retry_rechecks_credentials() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let status_file = dir.path().join("auth.json");
  let session_log = dir.path().join("sessions.log");
  std::fs::write(&status_file, r#"{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty"}"#).unwrap();
  let h = Harness::for_agent(&fake, "claude", json!({ "env": {
    "FAKE_CLAUDE_AUTH_FILE": status_file, "FAKE_MCP_LOG": session_log,
  } }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  assert_eq!(view(&s)["status"], "auth_required");
  assert!(!session_log.exists(), "session/new must wait for credentials");
  prompt(&s, "hello").await;
  assert!(view(&s)["turns"].as_array().unwrap().is_empty());
  assert_eq!(view(&s)["running"], false);
  s.retry().await.unwrap();
  assert_eq!(view(&s)["status"], "auth_required");
  // A terminal login changes the native status; retry must read it again, even with the ACP process still alive.
  std::fs::write(&status_file, r#"{"loggedIn":true,"authMethod":"oauth_token","apiProvider":"firstParty"}"#).unwrap();
  s.retry().await.unwrap();
  assert_eq!(view(&s)["status"], "ready");
  prompt(&s, "hello").await;
  assert_eq!(last_turn(&view(&s))["stop"], "end_turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn claude_api_keys_external_providers_and_unknown_status_do_not_require_a_saved_account() {
  let fake = fake_or_skip!();
  let dir = tempfile::tempdir().unwrap();
  let status_file = dir.path().join("auth.json");
  for status in [
    r#"{"loggedIn":true,"authMethod":"api_key","apiProvider":"firstParty"}"#,
    r#"{"loggedIn":false,"authMethod":"none","apiProvider":"bedrock"}"#,
    r#"{"loggedIn":false,"authMethod":"none","apiProvider":"firstParty","apiKeySource":"apiKeyHelper"}"#,
    "unsupported auth status command",
  ] {
    std::fs::write(&status_file, status).unwrap();
    let h = Harness::for_agent(&fake, "claude", json!({ "env": { "FAKE_CLAUDE_AUTH_FILE": status_file } }));
    let s = started(&h, dir.path().to_str().unwrap()).await;
    assert_eq!(view(&s)["status"], "ready", "{status}");
    prompt(&s, "hello").await;
    assert_eq!(last_turn(&view(&s))["stop"], "end_turn");
  }
}

#[tokio::test(flavor = "multi_thread")]
async fn login_goes_through_auth_required_authenticate_and_a_successful_retry() {
  let fake = fake_or_skip!();
  std::fs::create_dir_all("/tmp/acpira-needs-auth").unwrap();
  let h = Harness::new(&fake, json!({}));
  let s = started(&h, "/tmp/acpira-needs-auth").await;
  assert_eq!(view(&s)["status"], "auth_required");
  assert_eq!(view(&s)["authMethods"][0]["id"], "fake.login");
  // The reason the CLI logged to stderr right before -32000 is surfaced instead of a bare "log in" (stderr may be read after the
  // response, so it can land a moment later)
  until(|| view(&s)["error"] == "provider managed:fake has no credential configured", 5000).await;
  s.authenticate(None).await.unwrap();
  s.retry().await.unwrap();
  assert_eq!(view(&s)["status"], "ready");
  expect_absent(view(&s), "error");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_auth_method_lands_on_the_view_but_authenticate_refuses_to_send_it() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-term-").tempdir().unwrap();
  let auth_log = dir.path().join("auth.log");
  let h = Harness::new(&fake, json!({ "env": { "FAKE_TERMINAL_AUTH": auth_log } }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  assert_eq!(view(&s)["status"], "auth_required");
  let methods = view(&s)["authMethods"].as_array().unwrap().clone();
  let term = methods.iter().find(|m| m["id"] == "term-login").expect("terminal method");
  expect_match(term, json!({ "terminal": { "args": ["--login"], "env": { "FAKE_LOGIN": "1", "FAKE_FLAG": "method" } } }));
  let err = s.authenticate(Some("term-login")).await.expect_err("refused");
  assert!(err.to_string().contains("term-login"), "{err}");
  tokio::time::sleep(std::time::Duration::from_millis(100)).await;
  assert!(!auth_log.exists());
  // A plain method still goes over the wire
  s.authenticate(Some("fake.login")).await.unwrap();
  assert_eq!(std::fs::read_to_string(&auth_log).unwrap(), "fake.login\n");
  s.retry().await.unwrap();
  assert_eq!(view(&s)["status"], "ready");
}

// Devin's case: credentials only ever come from the account layer, so the capability is not advertised and the
// agent never offers a terminal login (a `devin acp --login` would write a login the ACP process ignores)
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_opts_out_of_terminal_auth_is_not_offered_terminal_methods() {
  let fake = fake_or_skip!();
  let dir = tempfile::Builder::new().prefix("acpira-term-").tempdir().unwrap();
  let h = Harness::new(&fake, json!({ "env": { "FAKE_TERMINAL_AUTH": dir.path().join("auth.log") }, "terminalAuth": false }));
  let s = started(&h, dir.path().to_str().unwrap()).await;
  assert_eq!(view(&s)["status"], "auth_required");
  assert_eq!(view(&s)["authMethods"].as_array().unwrap().iter().map(|m| m["id"].clone()).collect::<Vec<_>>(), [json!("fake.login")]);
}
