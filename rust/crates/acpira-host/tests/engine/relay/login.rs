//! A summoned CLI that needs a login is signed in with the parent's account

use super::*;

/// Saved accounts as the session layer sees them: every authenticate hands the account id over as the api key, and
/// the ids it was asked for are kept
struct SavedAccounts(std::sync::Mutex<Vec<String>>);

impl acpira_host::acp::session::SessionAccountHooks for SavedAccounts {
  fn spawn_env(&self, _agent: String, _account: String) -> acpira_host::acp::transport::rpc::BoxFuture<Option<acpira_shared::transcript::StrMap>> {
    Box::pin(async { None })
  }
  fn authenticate(
    &self,
    _agent: String,
    account: String,
    proc: Arc<acpira_host::acp::transport::process::AgentProcess>,
  ) -> acpira_host::acp::transport::rpc::BoxFuture<anyhow::Result<()>> {
    self.0.lock().unwrap().push(account.clone());
    Box::pin(async move {
      proc.request("authenticate", json!({ "methodId": "fake.login", "_meta": { "api_key": account } })).await?;
      Ok(())
    })
  }
  fn default_account(&self, _agent: String) -> Option<String> {
    Some("good-key-default".into())
  }
}

// Regression (Devin 3000.11.92, 2026-10-25): Devin over ACP refuses its local credentials, so a summoned Devin spawned
// without an account hand-off failed at session/new with "ACP host has not authenticated". The child now signs in like
// a new session would: the parent's own account when the persona runs in the same CLI
#[tokio::test(flavor = "multi_thread")]
async fn a_summoned_cli_that_needs_a_login_is_signed_in_with_the_parents_account() {
  let fake = fake_or_skip!();
  let mut h = Harness::new(&fake, json!({}));
  let hooks = Arc::new(SavedAccounts(std::sync::Mutex::new(vec![])));
  h.deps.accounts = Some(hooks.clone());
  let roster = roster(h.dir.path()).await;
  // The fake requires authenticate before session/new in a folder named so
  let cwd = tempfile::Builder::new().prefix("acpira-needs-auth-").tempdir().unwrap();
  let s = Disposing(acpira_host::acp::session::AcpSession::fresh("fake", cwd.path().to_str().unwrap(), h.deps.clone(), Some("good-key".into())));
  s.start().await;
  let (ok, text) = last_text(&run(&s, &roster, 0, ask("fake-review", "relay: look")).await);
  assert!(ok, "{text}");
  assert!(text.starts_with("reply to relay: look (round 1, mode plan)"), "{text}");
  assert_eq!(*hooks.0.lock().unwrap(), ["good-key", "good-key"]);
}
