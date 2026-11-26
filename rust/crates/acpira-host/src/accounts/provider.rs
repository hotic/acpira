//! What one agent's account adaptation provides

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;

use acpira_shared::transcript::AccountQuota;

use super::account_store::{AccountCredential, AccountDraft};
use crate::acp::transport::process::AgentProcess;
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::rpc::BoxFuture;

/// Terminal login: run the CLI's login in an isolated directory and collect the credential once written
pub struct LoginFlow {
  pub command: String,
  pub args: Vec<String>,
  /// None removes the variable from the terminal environment
  pub env: BTreeMap<String, Option<String>>,
  pub collect: Box<dyn FnOnce(Cancel) -> BoxFuture<Option<AccountDraft>> + Send>,
}

/// A quota read found the stored access token past its expiry; the CLI refreshes it on its next run, so the manager
/// reports the account as expired instead of retrying at once
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaTokenExpired;

impl std::fmt::Display for QuotaTokenExpired {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str("access token expired")
  }
}

impl std::error::Error for QuotaTokenExpired {}

/// One quota read: the windows, and the plan the vendor's answer says the login is on now. A login only captures the
/// plan once (Claude Code even keeps the login-time `subscriptionType` through token refreshes), so this is what keeps
/// an account's detail current after an upgrade or downgrade; None leaves the detail as it is
#[derive(Debug, Clone, Default)]
pub struct QuotaRead {
  pub quota: Option<AccountQuota>,
  pub plan: Option<String>,
}

impl From<Option<AccountQuota>> for QuotaRead {
  fn from(quota: Option<AccountQuota>) -> Self {
    QuotaRead { quota, plan: None }
  }
}

pub trait AccountProvider: Send + Sync {
  fn agent(&self) -> &str;
  fn import_local(&self) -> BoxFuture<Option<AccountDraft>>;
  /// The local login is cheap to read and stays in the CLI's own store (the draft's secret is `LOCAL_LOGIN`), so the
  /// manager keeps it in the list by itself (`AccountManager::sync_local`) instead of waiting for a "+" click
  fn auto_import(&self) -> bool {
    false
  }
  /// A private home for the account standing for the CLI's own login under `label`, for a CLI whose own store the
  /// vendor's desktop app signs in with too (Antigravity): an agent process on that store writes the token it holds back
  /// on every refresh, signing the app out whenever the two hold different identities. The local login is copied in
  /// when it is still `label`'s, otherwise the home stays signed out until that account signs in again; the draft's
  /// secret is the home. None keeps local logins in the CLI's store (`LOCAL_LOGIN`). Nothing happens until awaited
  fn adopt_local(&self, _label: String) -> Option<BoxFuture<Result<AccountDraft>>> {
    None
  }
  fn login(&self) -> BoxFuture<Result<LoginFlow>>;
  /// Whether a pasted API key can stand for an account (`key_account`), beside the terminal login
  fn accepts_keys(&self) -> bool {
    false
  }
  /// Check a pasted API key with the vendor and describe the account it signs in as; the draft's secret is the key. A
  /// key the vendor refuses, or that cannot be checked, is an error and leaves nothing behind
  fn key_account(&self, _key: String) -> BoxFuture<Result<AccountDraft>> {
    let agent = self.agent().to_owned();
    Box::pin(async move { Err(anyhow::anyhow!(crate::i18n::tp("host.noKeyAccounts", &[("agent", &agent)]))) })
  }
  fn spawn_env(&self, _cred: &AccountCredential) -> Option<BTreeMap<String, String>> {
    None
  }
  /// A plugin account's variables (`accounts/set`) get what `spawn_env` adds around a saved account's: a store they name
  /// is linked to the regular one, and anything else the CLI needs to read a login from it is set
  fn complete_plugin_env(&self, _env: &mut BTreeMap<String, String>) {}
  /// Why a plugin account's variables cannot sign this CLI in apart from the user's own login (a store that is the
  /// regular one, or not an absolute path); checked when the plugin lists the account
  fn plugin_env_error(&self, _env: &BTreeMap<String, String>) -> Option<String> {
    None
  }
  fn authenticate(&self, _proc: Arc<AgentProcess>, _cred: AccountCredential) -> Option<BoxFuture<Result<()>>> {
    None
  }
  fn quota(&self, _cred: AccountCredential) -> Option<BoxFuture<Result<QuotaRead>>> {
    None
  }
  /// Even a forced read (turn end, focus) reuses an answer younger than this: for vendors whose quota endpoint
  /// rate-limits per token after a few calls
  fn quota_min_interval(&self) -> std::time::Duration {
    std::time::Duration::ZERO
  }
  /// Quota is read only on an explicit user refresh (`refresh_quotas_manual`), never automatically on startup, turn
  /// end, fallback or panel open: for CLIs whose quota read is an out-of-band command rather than an ACP / API call
  fn manual_quota(&self) -> bool {
    false
  }
  /// The account was removed: drop whatever the provider keeps outside the vault
  fn forget(&self, _cred: AccountCredential) -> Option<BoxFuture<()>> {
    None
  }
  /// Whether the store the CLI keeps its logins in is locked for this host's session (the macOS keychain over SSH), so
  /// saved logins exist but cannot be read; None when the provider has no such store
  fn credentials_locked(&self) -> Option<BoxFuture<bool>> {
    None
  }
  /// The terminal command (program, args) that unlocks that store; the user types the password in the terminal
  fn unlock_command(&self) -> Option<(String, Vec<String>)> {
    None
  }
}
