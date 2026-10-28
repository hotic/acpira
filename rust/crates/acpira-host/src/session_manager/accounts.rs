//! Accounts and installs: the account list and its actions, switching and adding accounts, unlocking credential
//! stores, logins, and agent installs in the background or in a terminal

use std::sync::Arc;

use anyhow::Result;

use acpira_shared::protocol::{AccountAction, AccountActionStatus, AccountActionVia, AddAccountVia, HostMsg};
use acpira_shared::transcript::*;

use super::{LOGIN_PATH_REFRESH, SessionManager, Viewer};
use crate::acp::agents::login_path;
use crate::acp::session::AcpSession;
use crate::i18n::{t, tp};

impl SessionManager {
  pub fn accounts(&self) -> Vec<AccountInfo> {
    self.deps.accounts.as_ref().map(|a| a.list()).unwrap_or_default()
  }

  pub fn account_actions(&self) -> Vec<AccountAction> {
    self.state.lock().account_actions.clone()
  }

  fn set_account_action(&self, action: AccountAction) {
    let actions = {
      let mut st = self.state.lock();
      match st.account_actions.iter().position(|a| a.agent == action.agent) {
        Some(i) => st.account_actions[i] = action,
        None => st.account_actions.push(action),
      }
      st.account_actions.clone()
    };
    self.emit(HostMsg::AccountActions { actions });
  }

  /// Switching accounts rebinds the current session and becomes the agent's default
  pub async fn select_account(&self, v: &Viewer, account_id: &str, session_id: Option<&str>) -> Result<()> {
    let Some(accounts) = self.deps.accounts.clone() else { return Ok(()) };
    let Some(acc) = accounts.get(account_id) else { return Ok(()) };
    if let Some(cur) = self.target(v, session_id)
      && cur.agent == acc.agent
    {
      cur.rebind_account(account_id).await?;
    }
    accounts.touch(account_id).await
  }

  pub async fn add_account(&self, v: &Viewer, agent: &str, via: AddAccountVia) -> Result<()> {
    let Some(accounts) = self.deps.accounts.clone() else { return Ok(()) };
    if self.state.lock().account_actions.iter().any(|a| a.agent == agent && a.status == AccountActionStatus::Pending) {
      return Ok(());
    }
    let action = |status, error| AccountAction { agent: agent.to_owned(), via: via.into(), status, error };
    self.set_account_action(action(AccountActionStatus::Pending, None));
    let result = match via {
      AddAccountVia::Import => accounts.import(agent).await,
      AddAccountVia::Login => accounts.login(agent).await,
      AddAccountVia::Auto => accounts.add(agent).await,
    };
    match result {
      Ok(None) => {
        self.set_account_action(action(
          if via == AddAccountVia::Import { AccountActionStatus::Missing } else { AccountActionStatus::Cancelled },
          None,
        ));
        Ok(())
      }
      Ok(Some(acc)) => {
        if let Some(cur) = self.current(v)
          && cur.agent == agent
          && (cur.status() == SessionStatus::AuthRequired || cur.account_id().is_none())
          && let Err(e) = cur.rebind_account(&acc.id).await
        {
          self.set_account_action(action(AccountActionStatus::Error, Some(e.to_string())));
          return Err(e);
        }
        self.pool.invalidate(None);
        self.set_account_action(action(AccountActionStatus::Success, None));
        Ok(())
      }
      Err(e) => {
        self.set_account_action(action(AccountActionStatus::Error, Some(e.to_string())));
        Err(e)
      }
    }
  }

  /// Unlock the agent's credential store in a terminal; once it opens, warm processes started without it are dropped
  /// and every session of the agent that stopped on sign-in reconnects on the same native session
  pub async fn unlock_credentials(&self, agent: &str) -> Result<()> {
    let Some(accounts) = self.deps.accounts.clone() else { return Ok(()) };
    if self.state.lock().account_actions.iter().any(|a| a.agent == agent && a.status == AccountActionStatus::Pending) {
      return Ok(());
    }
    let action = |status, error| AccountAction { agent: agent.to_owned(), via: AccountActionVia::Unlock, status, error };
    self.set_account_action(action(AccountActionStatus::Pending, None));
    match accounts.unlock(agent).await {
      Ok(false) => self.set_account_action(action(AccountActionStatus::Cancelled, None)),
      Ok(true) => {
        self.pool.invalidate(Some(agent));
        for s in self.live_sessions().into_iter().filter(|s| s.agent == agent && s.status() == SessionStatus::AuthRequired) {
          tokio::spawn(async move {
            let _ = s.reconnect().await;
          });
        }
        self.set_account_action(action(AccountActionStatus::Success, None));
      }
      Err(e) => {
        self.set_account_action(action(AccountActionStatus::Error, Some(e.to_string())));
        return Err(e);
      }
    }
    Ok(())
  }

  /// A terminal method runs the agent binary itself in a terminal; other methods go through ACP authenticate, falling
  /// back to the registry's login command
  pub(super) async fn login(&self, s: Option<Arc<AcpSession>>, method_id: Option<&str>) -> Result<()> {
    let Some(s) = s else { return Ok(()) };
    let registry = self.registry();
    let def = registry.get(&s.agent)?.clone();
    let title = tp("host.loginTerminalTitle", &[("agent", &def.name)]);
    if let Some(term) = s.auth_method(method_id).and_then(|m| m.terminal) {
      let bin = registry.resolve_binary(&s.agent).await;
      let mut env: std::collections::BTreeMap<String, Option<String>> =
        def.env.clone().unwrap_or_default().into_iter().map(|(k, v)| (k, Some(v))).collect();
      for (k, v) in term.env.unwrap_or_default() {
        env.insert(k, Some(v));
      }
      let mut args = def.args.clone();
      args.extend(term.args);
      (self.deps.run_in_terminal)(title, bin.unwrap_or_else(|| def.command.clone()), args, (!env.is_empty()).then_some(env));
      (self.deps.toast)("info", &tp("host.loginThenRetry", &[("agent", &def.name)]));
      return Ok(());
    }
    let result: Result<()> = async {
      s.authenticate(method_id).await?;
      s.retry().await
    }
    .await;
    if let Err(e) = result {
      self.log(&format!("authenticate failed: {e}"));
      let Some(login) = &def.login else { return Err(e) };
      let bin = if login.command == def.command { registry.resolve_binary(&s.agent).await } else { None };
      (self.deps.run_in_terminal)(title, bin.unwrap_or_else(|| login.command.clone()), login.args.clone(), None);
      (self.deps.toast)("info", &tp("host.loginThenRetry", &[("agent", &def.name)]));
    }
    Ok(())
  }

  /// Run the registry's install line in the background on this engine's machine (`installer`): progress and log go to
  /// every webview, and a finished install is looked for at once instead of on the next poll
  pub(super) fn install(&self, agent: &str) {
    let registry = self.registry();
    let Ok(def) = registry.get(agent) else { return };
    let Some(command) = registry.install(agent).and_then(|i| i.command) else { return };
    let name = def.name.clone();
    let messages = crate::acp::agents::installer::Messages {
      node_missing: t("host.installNeedsNode"),
      failed: Box::new(|code| match code {
        Some(code) => tp("host.installFailedCode", &[("code", &code.to_string())]),
        None => t("host.installFailed"),
      }),
      spawn: Box::new(|why| tp("host.installSpawnFailed", &[("error", why)])),
    };
    let me = self.me.clone();
    let id = agent.to_owned();
    let os = crate::platform::command::Os::current();
    self.installs.start(agent, command, os, messages, move |ok| {
      let Some(m) = me.upgrade() else { return };
      tokio::spawn(async move {
        // The installer may have just written the rc file that puts its directory on PATH
        if ok && login_path::refresh(LOGIN_PATH_REFRESH).await {
          m.log_login_path();
        }
        let found = m.registry().resolve_binary(&id).await.is_some();
        m.emit_agents();
        let key = match (ok, found) {
          (true, true) => Some(("info", "host.installDone")),
          (true, false) => Some(("info", "host.installDoneNotFound")),
          _ => None,
        };
        if let Some((level, key)) = key {
          (m.deps.toast)(level, &tp(key, &[("agent", &name)]));
        }
      });
    });
  }

  /// The install line in an IDE terminal, for an installer that wants a person at the keyboard; the poll notices the
  /// new executable
  pub(super) fn install_in_terminal(&self, agent: &str) {
    let registry = self.registry();
    let Ok(def) = registry.get(agent) else { return };
    let Some(command) = registry.install(agent).and_then(|i| i.command) else { return };
    let (shell, flag) = if cfg!(windows) { ("powershell", "-Command") } else { ("bash", "-c") };
    (self.deps.run_in_terminal)(tp("host.installTerminalTitle", &[("agent", &def.name)]), shell.into(), vec![flag.into(), command], None);
    (self.deps.toast)("info", &tp("host.installThenDetect", &[("agent", &def.name)]));
  }
}
