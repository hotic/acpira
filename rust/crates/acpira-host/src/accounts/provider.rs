//! What one agent's account adaptation provides

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;

use acpira_shared::transcript::AccountQuota;

use super::account_store::{AccountCredential, AccountDraft};
use crate::acp::agent_process::AgentProcess;
use crate::acp::cancel::Cancel;
use crate::acp::rpc::BoxFuture;

/// Terminal login: run the CLI's login in an isolated directory and collect the credential once written
pub struct LoginFlow {
  pub command: String,
  pub args: Vec<String>,
  /// None removes the variable from the terminal environment
  pub env: BTreeMap<String, Option<String>>,
  pub collect: Box<dyn FnOnce(Cancel) -> BoxFuture<Option<AccountDraft>> + Send>,
}

pub trait AccountProvider: Send + Sync {
  fn agent(&self) -> &str;
  fn import_local(&self) -> BoxFuture<Option<AccountDraft>>;
  /// The local login is cheap to read and stays in the CLI's own store (the draft's secret is `LOCAL_LOGIN`), so the
  /// manager keeps it in the list by itself (`AccountManager::sync_local`) instead of waiting for a "+" click
  fn auto_import(&self) -> bool {
    false
  }
  fn login(&self) -> BoxFuture<Result<LoginFlow>>;
  fn spawn_env(&self, _cred: &AccountCredential) -> Option<BTreeMap<String, String>> {
    None
  }
  fn authenticate(&self, _proc: Arc<AgentProcess>, _cred: AccountCredential) -> Option<BoxFuture<Result<()>>> {
    None
  }
  fn quota(&self, _cred: AccountCredential) -> Option<BoxFuture<Result<Option<AccountQuota>>>> {
    None
  }
  /// The account was removed: drop whatever the provider keeps outside the vault
  fn forget(&self, _cred: AccountCredential) -> Option<BoxFuture<()>> {
    None
  }
}
