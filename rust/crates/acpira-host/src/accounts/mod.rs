pub mod account_manager;
pub mod account_store;
pub mod antigravity;
pub mod claude;
pub mod cli_home;
pub mod codex;
pub mod devin;
pub mod droid;
pub mod keychain;
pub mod local;
pub mod plugin_accounts;
pub mod provider;
pub mod quota_cache;
pub mod quota_cli;
pub mod switch;

use std::sync::Arc;

use anyhow::Result;

use acpira_shared::transcript::StrMap;

use crate::acp::transport::process::AgentProcess;
use crate::acp::transport::rpc::BoxFuture;

/// The hooks the account layer gives a session: environment variables before spawn, authenticate after initialize, and
/// the automatic switch when the bound account runs out of quota or was removed
pub trait SessionAccountHooks: Send + Sync {
  fn spawn_env(&self, agent: String, account: String) -> BoxFuture<Option<StrMap>>;
  fn authenticate(&self, agent: String, account: String, proc: Arc<AgentProcess>) -> BoxFuture<Result<()>>;
  /// Why no process may start on `account` now (an account its source no longer offers); checked before any spawn, as
  /// a process without the account's variables would run on the CLI's own login
  fn unavailable(&self, _agent: String, _account: String) -> Option<String> {
    None
  }
  /// (id, label) of the account to move to after `current` ran out of quota; None = stay and show the error
  fn fallback(&self, _agent: String, _current: Option<String>) -> BoxFuture<Option<(String, String)>> {
    Box::pin(async { None })
  }
  /// (id, label) of the saved account to bind when `account` was removed; None = keep it (still saved, a plugin's, or
  /// nothing left to move to)
  fn replacement(&self, _agent: String, _account: String) -> BoxFuture<Option<(String, String)>> {
    Box::pin(async { None })
  }
  fn label(&self, _account: String) -> Option<String> {
    None
  }
  /// The account's quota, read again unless a recent read stands; None = nothing known
  fn quota(&self, _account: String) -> BoxFuture<Option<acpira_shared::transcript::AccountQuota>> {
    Box::pin(async { None })
  }
  /// The saved account a new session of `agent` binds (the last one chosen); None = the CLI's own credentials
  fn default_account(&self, _agent: String) -> Option<String> {
    None
  }
}
