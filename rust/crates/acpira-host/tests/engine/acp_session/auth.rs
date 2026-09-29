//! Login flows and terminal auth methods

use super::*;

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
