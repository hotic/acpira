//! Account master: who supports the account layer, the list,
//! import / login / removal, the session hooks, and in-memory quotas

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};

use acpira_shared::transcript::{AccountInfo, AccountQuota, StrMap};

use super::account_store::AccountStore;
use super::cli_home::LOCAL_LOGIN;
use super::provider::AccountProvider;
use super::switch::{SwitchStrategy, parked_until, pick_fallback};
use crate::acp::agent_process::AgentProcess;
use crate::acp::cancel::Cancel;
use crate::acp::rpc::BoxFuture;
use crate::acp::session::SessionAccountHooks;
use crate::i18n::{t, tp};
use crate::store::transcript_store::LogFn;
use crate::util::{ms_of_iso, now_ms};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const QUOTA_MAX_AGE_MS: i64 = 30_000;

pub type RunInTerminal = Arc<dyn Fn(String, String, Vec<String>, Option<std::collections::BTreeMap<String, Option<String>>>) + Send + Sync>;
pub type Toast = Arc<dyn Fn(&str, &str) + Send + Sync>;
/// agent → the strategy of its automatic account switch (acpira.accountSwitch)
pub type SwitchPolicy = Arc<dyn Fn(&str) -> SwitchStrategy + Send + Sync>;

pub struct AccountManager {
  store: Arc<AccountStore>,
  providers: HashMap<String, Arc<dyn AccountProvider>>,
  log: LogFn,
  run_in_terminal: RunInTerminal,
  toast: Toast,
  listeners: parking_lot::Mutex<Vec<(u64, Arc<dyn Fn(Vec<AccountInfo>) + Send + Sync>)>>,
  quotas: parking_lot::Mutex<HashMap<String, AccountQuota>>,
  fetching: parking_lot::Mutex<HashMap<String, tokio::sync::watch::Receiver<bool>>>,
  /// Accounts that reported exhaustion → until when they stay out of the automatic switch (epoch ms, this host only)
  parked: parking_lot::Mutex<HashMap<String, i64>>,
  switch_policy: parking_lot::Mutex<SwitchPolicy>,
  seq: std::sync::atomic::AtomicU64,
  /// One `sync_local` at a time: startup, focus and the account view can all ask at once
  syncing: tokio::sync::Mutex<()>,
}

impl AccountManager {
  pub fn new(
    store: Arc<AccountStore>,
    providers: Vec<Arc<dyn AccountProvider>>,
    log: LogFn,
    run_in_terminal: RunInTerminal,
    toast: Toast,
  ) -> Arc<Self> {
    Arc::new(AccountManager {
      store,
      providers: providers.into_iter().map(|p| (p.agent().to_owned(), p)).collect(),
      log,
      run_in_terminal,
      toast,
      listeners: Default::default(),
      quotas: Default::default(),
      fetching: Default::default(),
      parked: Default::default(),
      switch_policy: parking_lot::Mutex::new(Arc::new(|_: &str| SwitchStrategy::EarliestReset)),
      seq: Default::default(),
      syncing: tokio::sync::Mutex::new(()),
    })
  }

  pub fn set_switch_policy(&self, policy: SwitchPolicy) {
    *self.switch_policy.lock() = policy;
  }

  /// The account a session moves to after `current` ran out of quota: `current` is parked until its empty windows refill,
  /// every quota of the agent is brought up to date, then the agent's strategy picks among the rest. None when the strategy
  /// is off or no other account has allowance left
  pub async fn fallback(self: &Arc<Self>, agent: &str, current: Option<&str>) -> Option<AccountInfo> {
    let strategy = (self.switch_policy.lock().clone())(agent);
    if strategy == SwitchStrategy::Off {
      return None;
    }
    if let Some(cur) = current {
      self.refresh_quota(cur, true).await;
      let q = self.quotas.lock().get(cur).cloned();
      self.parked.lock().insert(cur.to_owned(), parked_until(q.as_ref(), now_ms()));
    }
    self.refresh_quotas(Some(agent), false).await;
    let list = self.store.list(Some(agent)).into_iter().map(|a| self.with_quota(a)).collect::<Vec<_>>();
    let id = pick_fallback(&list, current, strategy, &self.parked.lock(), now_ms())?;
    (self.log)(&format!("account fallback ({strategy:?}): {} → {id}", current.unwrap_or("-")));
    list.into_iter().find(|a| a.id == id)
  }

  pub fn supports(&self, agent: &str) -> bool {
    self.providers.contains_key(agent)
  }

  pub fn list(&self) -> Vec<AccountInfo> {
    self.store.list(None).into_iter().map(|a| self.with_quota(a)).collect()
  }

  pub fn get(&self, id: &str) -> Option<AccountInfo> {
    self.store.get(id).map(|a| self.with_quota(a))
  }

  pub fn default_for(&self, agent: &str) -> Option<AccountInfo> {
    self.store.default_for(agent)
  }

  /// Pick up accounts another host added or removed; viewers are told only when the list changed
  pub async fn reload(&self) {
    if self.store.reload().await {
      self.emit();
    }
  }

  fn with_quota(&self, mut a: AccountInfo) -> AccountInfo {
    if let Some(q) = self.quotas.lock().get(&a.id) {
      a.quota = Some(q.clone());
    }
    a
  }

  /// Refresh one account's quota; concurrent callers share the in-flight request, a recent result is reused unless `force`
  pub fn refresh_quota(self: &Arc<Self>, id: &str, force: bool) -> BoxFuture<()> {
    let me = self.clone();
    let id = id.to_owned();
    Box::pin(async move {
      let inflight = me.fetching.lock().get(&id).cloned();
      if let Some(mut rx) = inflight {
        let _ = rx.wait_for(|done| *done).await;
        return;
      }
      let Some(a) = me.store.get(&id) else { return };
      let Some(p) = me.providers.get(&a.agent).cloned() else { return };
      if !force
        && let Some(have) = me.quotas.lock().get(&id)
        && now_ms() - ms_of_iso(&have.fetched_at).unwrap_or(0) < QUOTA_MAX_AGE_MS
      {
        return;
      }
      let (tx, rx) = tokio::sync::watch::channel(false);
      me.fetching.lock().insert(id.clone(), rx);
      let result: Result<()> = async {
        let Some(cred) = me.store.credential(&id).await? else { return Ok(()) };
        let Some(fut) = p.quota(cred) else { return Ok(()) };
        match fut.await? {
          Some(q) => {
            me.quotas.lock().insert(id.clone(), q);
          }
          None => {
            me.quotas.lock().remove(&id);
          }
        }
        me.emit();
        Ok(())
      }
      .await;
      if let Err(e) = result {
        (me.log)(&format!("quota {}: {e}", a.label));
      }
      me.fetching.lock().remove(&id);
      let _ = tx.send(true);
    })
  }

  pub async fn refresh_quotas(self: &Arc<Self>, agent: Option<&str>, force: bool) {
    let ids: Vec<String> = self.store.list(agent).into_iter().map(|a| a.id).collect();
    let tasks: Vec<_> = ids.iter().map(|id| tokio::spawn(self.refresh_quota(id, force))).collect();
    for t in tasks {
      let _ = t.await;
    }
  }

  pub fn subscribe(&self, f: Arc<dyn Fn(Vec<AccountInfo>) + Send + Sync>) -> u64 {
    let id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    self.listeners.lock().push((id, f));
    id
  }

  fn emit(&self) {
    let list = self.list();
    let ls: Vec<_> = self.listeners.lock().iter().map(|(_, f)| f.clone()).collect();
    for f in ls {
      f(list.clone());
    }
  }

  fn provider(&self, agent: &str) -> Result<Arc<dyn AccountProvider>> {
    self.providers.get(agent).cloned().ok_or_else(|| anyhow!(tp("host.noAccountLayer", &[("agent", agent)])))
  }

  async fn store_draft(
    self: &Arc<Self>,
    agent: &str,
    draft: super::account_store::AccountDraft,
    verb: &str,
    toast_key: &str,
  ) -> Result<AccountInfo> {
    let local = draft.secret == LOCAL_LOGIN;
    let a = self.store.add(agent, draft).await?;
    if local {
      // Imported on purpose: the automatic import may keep it up to date again
      let _ = self.store.set_dismissed(agent, &a.label, false).await;
    }
    (self.log)(&format!("account {verb}: {agent} {}", a.label));
    (self.toast)("info", &tp(toast_key, &[("label", &a.label)]));
    self.emit();
    // Quota is decoration: do not wait on the vendor before the row appears
    tokio::spawn(self.refresh_quota(&a.id, false));
    Ok(a)
  }

  /// The saved account that stands for the CLI's own login (secret `LOCAL_LOGIN`), if any
  async fn local_account(&self, agent: &str) -> Option<AccountInfo> {
    for a in self.store.list(Some(agent)) {
      if self.store.credential(&a.id).await.ok().flatten().is_some_and(|c| c.secret == LOCAL_LOGIN) {
        return Some(a);
      }
    }
    None
  }

  /// Keep the CLI's own login in the list without a "+" click, for providers whose local login stays in the CLI's store
  /// (`auto_import`): a login appears as an account, a re-login as another identity renames that account (the CLI
  /// store it points at now holds the new one), and nothing happens when the login is gone, when an account with that
  /// label already exists (e.g. the same identity in a private home) or when the user removed it (`dismissed`, cleared
  /// by importing it again)
  pub async fn sync_local(self: &Arc<Self>, agent: Option<&str>) {
    let _serial = self.syncing.lock().await;
    let providers: Vec<_> =
      self.providers.values().filter(|p| p.auto_import() && agent.is_none_or(|a| p.agent() == a)).cloned().collect();
    for p in providers {
      let agent = p.agent().to_owned();
      let Some(draft) = p.import_local().await else { continue };
      if draft.secret != LOCAL_LOGIN || self.store.is_dismissed(&agent, &draft.label).await {
        continue;
      }
      let local = self.local_account(&agent).await;
      let same_label = self.store.list(Some(&agent)).into_iter().find(|a| a.label == draft.label);
      let result = match (same_label, local) {
        (Some(a), Some(l)) if a.id == l.id => {
          if a.detail == draft.detail {
            continue;
          }
          self.store.set_identity(&a.id, &draft.label, draft.detail.clone()).await.map(|_| a.id)
        }
        (Some(_), _) => continue,
        (None, Some(l)) => {
          (self.log)(&format!("account local login changed: {agent} {} → {}", l.label, draft.label));
          self.quotas.lock().remove(&l.id);
          self.store.set_identity(&l.id, &draft.label, draft.detail.clone()).await.map(|_| l.id)
        }
        (None, None) => {
          let label = draft.label.clone();
          let added = self.store.add(&agent, draft).await.map(|a| a.id);
          if added.is_ok() {
            (self.log)(&format!("account local login picked up: {agent} {label}"));
          }
          added
        }
      };
      match result {
        Ok(id) => {
          self.emit();
          tokio::spawn(self.refresh_quota(&id, true));
        }
        Err(e) => (self.log)(&format!("account local login {agent}: {e}")),
      }
    }
  }

  pub async fn import(self: &Arc<Self>, agent: &str) -> Result<Option<AccountInfo>> {
    let Some(draft) = self.provider(agent)?.import_local().await else {
      (self.toast)("info", &tp("host.noLocalLogin", &[("agent", agent)]));
      return Ok(None);
    };
    Ok(Some(self.store_draft(agent, draft, "imported", "host.imported").await?))
  }

  /// The "+" in the menu: import the local login once, otherwise log a new one in a terminal
  pub async fn add(self: &Arc<Self>, agent: &str) -> Result<Option<AccountInfo>> {
    let draft = self.provider(agent)?.import_local().await;
    if let Some(d) = draft
      && !self.list().iter().any(|a| a.agent == agent && a.label == d.label)
    {
      return Ok(Some(self.store_draft(agent, d, "imported", "host.imported").await?));
    }
    self.login(agent).await
  }

  /// Open a terminal for the isolated login and wait for it to write to disk
  pub async fn login(self: &Arc<Self>, agent: &str) -> Result<Option<AccountInfo>> {
    let flow = self.provider(agent)?.login().await?;
    (self.run_in_terminal)(tp("host.loginTerminalTitle", &[("agent", agent)]), flow.command, flow.args, Some(flow.env));
    (self.toast)("info", &t("host.finishLoginInTerminal"));
    let cancel = Cancel::new();
    let timer_cancel = cancel.clone();
    let timer = tokio::spawn(async move {
      tokio::time::sleep(LOGIN_TIMEOUT).await;
      timer_cancel.cancel();
    });
    let draft = (flow.collect)(cancel).await;
    timer.abort();
    let Some(draft) = draft else {
      (self.toast)("info", &t("host.loginIncomplete"));
      return Ok(None);
    };
    Ok(Some(self.store_draft(agent, draft, "login", "host.accountAdded").await?))
  }

  pub async fn remove(&self, id: &str) -> Result<()> {
    let owner = self.store.get(id).and_then(|a| self.providers.get(&a.agent).cloned());
    let cred = self.store.credential(id).await.ok().flatten();
    let info = self.store.get(id);
    self.store.remove(id).await?;
    // A removed local login would be picked up again on the next sync: remember the user's choice
    if let (Some(p), Some(a), Some(c)) = (&owner, &info, &cred)
      && p.auto_import()
      && c.secret == LOCAL_LOGIN
      && let Err(e) = self.store.set_dismissed(&a.agent, &a.label, true).await
    {
      (self.log)(&format!("account dismiss {}: {e}", a.label));
    }
    // The provider cleans up what it keeps outside the vault (a CLI home, a keychain entry)
    if let (Some(p), Some(cred)) = (owner, cred)
      && let Some(fut) = p.forget(cred)
    {
      fut.await;
    }
    self.quotas.lock().remove(id);
    self.parked.lock().remove(id);
    self.emit();
    Ok(())
  }

  pub async fn touch(&self, id: &str) -> Result<()> {
    self.store.touch(id).await
  }

  pub async fn spawn_env_for(&self, agent: &str, account: &str) -> Option<StrMap> {
    let p = self.providers.get(agent)?;
    let cred = self.store.credential(account).await.ok()??;
    p.spawn_env(&cred)
  }

  pub async fn authenticate_for(self: &Arc<Self>, agent: &str, account: &str, proc: Arc<AgentProcess>) -> Result<()> {
    let Some(p) = self.providers.get(agent).cloned() else { return Ok(()) };
    let cred = self.store.credential(account).await?;
    let Some(cred) = cred else {
      let label = self.get(account).map(|a| a.label).unwrap_or_else(|| account.to_owned());
      return Err(anyhow!(tp("host.credentialGone", &[("label", &label)])));
    };
    let Some(fut) = p.authenticate(proc, cred) else { return Ok(()) };
    fut.await?;
    self.touch(account).await?;
    tokio::spawn(self.refresh_quota(account, false));
    Ok(())
  }
}

/// The session side of the account layer
pub struct AccountHooks(pub Arc<AccountManager>);

impl SessionAccountHooks for AccountHooks {
  fn spawn_env(&self, agent: String, account: String) -> BoxFuture<Option<StrMap>> {
    let m = self.0.clone();
    Box::pin(async move { m.spawn_env_for(&agent, &account).await })
  }

  fn authenticate(&self, agent: String, account: String, proc: Arc<AgentProcess>) -> BoxFuture<Result<()>> {
    let m = self.0.clone();
    Box::pin(async move { m.authenticate_for(&agent, &account, proc).await })
  }

  fn fallback(&self, agent: String, current: Option<String>) -> BoxFuture<Option<(String, String)>> {
    let m = self.0.clone();
    Box::pin(async move { m.fallback(&agent, current.as_deref()).await.map(|a| (a.id, a.label)) })
  }

  fn label(&self, account: String) -> Option<String> {
    self.0.get(&account).map(|a| a.label)
  }
}
