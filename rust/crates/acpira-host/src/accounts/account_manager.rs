//! Account master: who supports the account layer, the list,
//! import / login / removal, the session hooks, and in-memory quotas

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};

use acpira_shared::transcript::{AccountInfo, AccountQuota, AccountQuotaIssue, StrMap};

use super::account_store::AccountStore;
use super::cli_home::{LOCAL_LOGIN, poll_until};
use super::provider::{AccountProvider, QuotaRead, QuotaTokenExpired};
use super::quota_cache::{QuotaCache, QuotaEntry, QuotaFailure};
use super::switch::{SwitchStrategy, parked_until, pick_fallback};
use crate::acp::transport::process::AgentProcess;
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::rpc::BoxFuture;
use crate::acp::session::SessionAccountHooks;
use crate::i18n::{t, tp};
use crate::store::transcript_store::LogFn;
use crate::util::{ms_of_iso, now_ms};

const LOGIN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const UNLOCK_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// How long an unforced read (the open account view, focus) reuses the last answer
const QUOTA_MAX_AGE_MS: i64 = 30_000;
/// Pause after a 429 without a usable Retry-After, as Claude Code does for the same endpoint (2.1.284: 5 min, capped at
/// 1 h); doubles per consecutive 429 because the endpoint has been seen answering 429 for hours to steady polling
const QUOTA_RATE_LIMIT_BASE_MS: i64 = 5 * 60_000;
const QUOTA_RATE_LIMIT_MAX_MS: i64 = 60 * 60_000;

/// Classify a failed quota read. A 429 pauses reads for the vendor's Retry-After, else 5, 10, 20 … 60 min; every other
/// failure only for the usual cache age. Reading again inside a 429 pause would keep the vendor's bucket drained
pub fn quota_failure(err: &anyhow::Error, prev: Option<&QuotaFailure>, now: i64) -> QuotaFailure {
  let status = err.downcast_ref::<crate::http::HttpStatus>();
  if let Some(s) = status.filter(|s| s.status == 429) {
    let rate_limits = prev.filter(|p| p.issue == AccountQuotaIssue::RateLimited).map_or(0, |p| p.rate_limits) + 1;
    let backoff = QUOTA_RATE_LIMIT_BASE_MS.saturating_mul(1 << (rate_limits - 1).min(8)).min(QUOTA_RATE_LIMIT_MAX_MS);
    // `Retry-After: 0` has been reported on this endpoint alongside persistent 429s; it is no hint at all
    let wait = s
      .retry_after
      .filter(|d| !d.is_zero())
      .map_or(backoff, |d| (d.as_millis() as i64).clamp(QUOTA_MAX_AGE_MS, QUOTA_RATE_LIMIT_MAX_MS));
    return QuotaFailure { issue: AccountQuotaIssue::RateLimited, at_ms: now, until_ms: now + wait, rate_limits };
  }
  let expired = err.downcast_ref::<QuotaTokenExpired>().is_some() || status.is_some_and(|s| s.status == 401 || s.status == 403);
  let issue = if expired { AccountQuotaIssue::Expired } else { AccountQuotaIssue::Unavailable };
  QuotaFailure { issue, at_ms: now, until_ms: now + QUOTA_MAX_AGE_MS, rate_limits: 0 }
}

/// A cancel that fires by itself after `after`; the handle stops the timer
fn cancel_after(after: Duration) -> (Cancel, tokio::task::JoinHandle<()>) {
  let cancel = Cancel::new();
  let timer_cancel = cancel.clone();
  let timer = tokio::spawn(async move {
    tokio::time::sleep(after).await;
    timer_cancel.cancel();
  });
  (cancel, timer)
}

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
  /// Accounts whose last quota read failed; cleared by the next successful read
  quota_failures: parking_lot::Mutex<HashMap<String, QuotaFailure>>,
  /// quota-cache.json: what every host on the data dir last learned, so windows do not each poll the vendor
  quota_cache: QuotaCache,
  fetching: parking_lot::Mutex<HashMap<String, tokio::sync::watch::Receiver<bool>>>,
  /// Accounts that reported exhaustion → until when they stay out of the automatic switch (epoch ms, this host only)
  parked: parking_lot::Mutex<HashMap<String, i64>>,
  switch_policy: parking_lot::Mutex<SwitchPolicy>,
  seq: std::sync::atomic::AtomicU64,
  /// One `sync_local` at a time: startup, focus and the account view can all ask at once
  syncing: tokio::sync::Mutex<()>,
  /// Agents whose credential store the last check found locked (`AccountProvider::credentials_locked`)
  locked: parking_lot::Mutex<HashSet<String>>,
  lock_listeners: parking_lot::Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

impl AccountManager {
  pub fn new(
    store: Arc<AccountStore>,
    providers: Vec<Arc<dyn AccountProvider>>,
    log: LogFn,
    run_in_terminal: RunInTerminal,
    toast: Toast,
  ) -> Arc<Self> {
    let quota_cache = QuotaCache::new(store.quota_cache_file());
    Arc::new(AccountManager {
      store,
      quota_cache,
      providers: providers.into_iter().map(|p| (p.agent().to_owned(), p)).collect(),
      log,
      run_in_terminal,
      toast,
      listeners: Default::default(),
      quotas: Default::default(),
      quota_failures: Default::default(),
      fetching: Default::default(),
      parked: Default::default(),
      switch_policy: parking_lot::Mutex::new(Arc::new(|_: &str| SwitchStrategy::EarliestReset)),
      seq: Default::default(),
      syncing: tokio::sync::Mutex::new(()),
      locked: Default::default(),
      lock_listeners: Default::default(),
    })
  }

  /// The agent's credential store was locked at the last check: its accounts are listed but unusable until unlocked
  pub fn credentials_locked(&self, agent: &str) -> bool {
    self.locked.lock().contains(agent)
  }

  /// Called whenever an agent's credential store turns locked or unlocked
  pub fn on_lock_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
    self.lock_listeners.lock().push(f);
  }

  fn set_locked(&self, agent: &str, locked: bool) {
    let changed = {
      let mut set = self.locked.lock();
      if locked { set.insert(agent.to_owned()) } else { set.remove(agent) }
    };
    if !changed {
      return;
    }
    (self.log)(&format!("account credentials {agent}: {}", if locked { "locked" } else { "unlocked" }));
    let ls: Vec<_> = self.lock_listeners.lock().clone();
    for f in ls {
      f();
    }
  }

  /// Ask each provider with a lockable credential store (all, or one agent's) whether it is locked right now
  pub async fn check_locks(&self, agent: Option<&str>) {
    let providers: Vec<_> = self.providers.values().filter(|p| agent.is_none_or(|a| p.agent() == a)).cloned().collect();
    for p in providers {
      if let Some(check) = p.credentials_locked() {
        self.set_locked(p.agent(), check.await);
      }
    }
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
    // Last known bars win over a failed re-read; the issue only explains why there are none
    if a.quota.is_none() {
      a.quota_issue = self.quota_failures.lock().get(&a.id).map(|f| f.issue);
    }
    a
  }

  /// Drop an account's quota here and in the shared cache (the account was removed or is now someone else)
  async fn forget_quota(&self, id: &str) {
    self.quotas.lock().remove(id);
    self.quota_failures.lock().remove(id);
    if let Err(e) = self.quota_cache.set(id, None).await {
      (self.log)(&format!("quota cache: {e}"));
    }
  }

  /// Take what another host learned if it is newer than what this one knows; true when anything changed
  fn adopt_quota(&self, id: &str, entry: QuotaEntry) -> bool {
    let mut quotas = self.quotas.lock();
    let mut failures = self.quota_failures.lock();
    let mut changed = false;
    if let Some(q) = entry.quota
      && quotas.get(id).is_none_or(|have| ms_of_iso(&q.fetched_at) > ms_of_iso(&have.fetched_at))
    {
      quotas.insert(id.to_owned(), q);
      changed = true;
    }
    if let Some(f) = entry.failure
      && failures.get(id).is_none_or(|have| f.at_ms > have.at_ms)
    {
      failures.insert(id.to_owned(), f);
      changed = true;
    }
    // A successful read after the failure ends it
    if let (Some(q), Some(f)) = (quotas.get(id), failures.get(id))
      && ms_of_iso(&q.fetched_at).unwrap_or(0) > f.at_ms
    {
      failures.remove(id);
      changed = true;
    }
    changed
  }

  async fn share_quota(&self, id: &str) {
    let entry = QuotaEntry { quota: self.quotas.lock().get(id).cloned(), failure: self.quota_failures.lock().get(id).copied() };
    let entry = (entry.quota.is_some() || entry.failure.is_some()).then_some(entry);
    if let Err(e) = self.quota_cache.set(id, entry).await {
      (self.log)(&format!("quota cache: {e}"));
    }
  }

  /// Refresh one account's quota; concurrent callers share the in-flight request. A recent answer — this host's or one
  /// another host left in the shared cache — is reused: for 30 s (at least the provider's floor) on unforced reads, for
  /// the provider's floor on forced ones; a 429 pause holds against both
  pub fn refresh_quota(self: &Arc<Self>, id: &str, force: bool) -> BoxFuture<()> {
    let me = self.clone();
    let id = id.to_owned();
    Box::pin(async move {
      let (tx, rx) = tokio::sync::watch::channel(false);
      let inflight = {
        let mut fetching = me.fetching.lock();
        let inflight = fetching.get(&id).cloned();
        if inflight.is_none() {
          fetching.insert(id.clone(), rx);
        }
        inflight
      };
      if let Some(mut rx) = inflight {
        let _ = rx.wait_for(|done| *done).await;
        return;
      }
      me.read_quota(&id, force).await;
      me.fetching.lock().remove(&id);
      let _ = tx.send(true);
    })
  }

  async fn read_quota(self: &Arc<Self>, id: &str, force: bool) {
    let Some(a) = self.store.get(id) else { return };
    let Some(p) = self.providers.get(&a.agent).cloned() else { return };
    // A locked store cannot hand out the token; the last known quota stays until it is unlocked
    if self.credentials_locked(&a.agent) {
      return;
    }
    // Another window may have read the same account (or been refused) moments ago
    if let Some(entry) = self.quota_cache.get(id).await
      && self.adopt_quota(id, entry)
    {
      self.emit();
    }
    // A 429 pause holds even against forced reads (turn ends, focus); other failures wait only for unforced ones
    if let Some(f) = self.quota_failures.lock().get(id)
      && now_ms() < f.until_ms
      && (!force || f.issue == AccountQuotaIssue::RateLimited)
    {
      return;
    }
    let floor = p.quota_min_interval().as_millis() as i64;
    let fresh_for = if force { floor } else { floor.max(QUOTA_MAX_AGE_MS) };
    if let Some(have) = self.quotas.lock().get(id)
      && now_ms() - ms_of_iso(&have.fetched_at).unwrap_or(0) < fresh_for
    {
      return;
    }
    let result: Result<Option<QuotaRead>> = async {
      let Some(cred) = self.store.credential(id).await? else { return Ok(None) };
      let Some(fut) = p.quota(cred) else { return Ok(None) };
      Ok(Some(fut.await?))
    }
    .await;
    match result {
      Ok(None) => return,
      Ok(Some(read)) => {
        self.quota_failures.lock().remove(id);
        match read.quota {
          Some(q) => self.quotas.lock().insert(id.to_owned(), q),
          None => self.quotas.lock().remove(id),
        };
        // The vendor names the plan the login is on now: an upgrade shows without logging in again
        if let Some(plan) = read.plan
          && self.store.get(id).is_some_and(|cur| cur.detail.as_deref() != Some(plan.as_str()))
        {
          (self.log)(&format!("account plan changed: {} {} → {plan}", a.agent, a.label));
          if let Err(e) = self.store.set_detail(id, Some(plan)).await {
            (self.log)(&format!("account plan {}: {e}", a.label));
          }
        }
        self.emit();
      }
      Err(e) => {
        let now = now_ms();
        let (failure, changed) = {
          let mut failures = self.quota_failures.lock();
          let prev = failures.get(id).copied();
          let failure = quota_failure(&e, prev.as_ref(), now);
          failures.insert(id.to_owned(), failure);
          (failure, prev.map(|p| p.issue) != Some(failure.issue))
        };
        (self.log)(&format!("quota {}: {e}; next read in {}s", a.label, (failure.until_ms - now) / 1000));
        if changed {
          self.emit();
        }
      }
    }
    self.share_quota(id).await;
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
  /// store it points at now holds the new one), the plan of the same identity follows the quota read, and nothing happens when the login is gone, when an account with that
  /// label already exists (e.g. the same identity in a private home) or when the user removed it (`dismissed`, cleared
  /// by importing it again)
  pub async fn sync_local(self: &Arc<Self>, agent: Option<&str>) {
    let _serial = self.syncing.lock().await;
    // Same moments as the local login: startup, window focus, an account view refreshing
    self.check_locks(agent).await;
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
        // Same identity: the plan is the quota read's to update (the local store may keep the login-time plan, Claude
        // Code's credentials do), so a known detail is never overwritten from here, only a missing one filled in
        (Some(a), Some(l)) if a.id == l.id => {
          if a.detail.is_some() || draft.detail.is_none() {
            continue;
          }
          self.store.set_detail(&a.id, draft.detail.clone()).await.map(|_| a.id)
        }
        (Some(_), _) => continue,
        (None, Some(l)) => {
          (self.log)(&format!("account local login changed: {agent} {} → {}", l.label, draft.label));
          self.forget_quota(&l.id).await;
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
    let (cancel, timer) = cancel_after(LOGIN_TIMEOUT);
    let draft = (flow.collect)(cancel).await;
    timer.abort();
    let Some(draft) = draft else {
      (self.toast)("info", &t("host.loginIncomplete"));
      return Ok(None);
    };
    Ok(Some(self.store_draft(agent, draft, "login", "host.accountAdded").await?))
  }

  /// Unlock the agent's credential store: the provider's unlock command runs in a terminal where the user types the
  /// password (it never passes through Acpira), and the store is polled until it opens. Then the local login and the
  /// quotas are read again. Ok(false) when the provider has nothing to unlock or the terminal was left unfinished
  pub async fn unlock(self: &Arc<Self>, agent: &str) -> Result<bool> {
    let p = self.provider(agent)?;
    let Some((command, args)) = p.unlock_command() else { return Ok(false) };
    let locked = || {
      let p = p.clone();
      async move {
        match p.credentials_locked() {
          Some(check) => check.await,
          None => false,
        }
      }
    };
    if locked().await {
      self.set_locked(agent, true);
      (self.run_in_terminal)(tp("host.unlockTerminalTitle", &[("agent", agent)]), command, args, None);
      (self.toast)("info", &t("host.finishUnlockInTerminal"));
      let (cancel, timer) = cancel_after(UNLOCK_TIMEOUT);
      let opened = poll_until(&cancel, || {
        let still = locked();
        async move { (!still.await).then_some(()) }
      })
      .await;
      timer.abort();
      if opened.is_none() {
        (self.toast)("info", &t("host.unlockIncomplete"));
        return Ok(false);
      }
    }
    self.set_locked(agent, false);
    self.sync_local(Some(agent)).await;
    self.refresh_quotas(Some(agent), true).await;
    Ok(true)
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
    self.forget_quota(id).await;
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
    // A CLI that reads its own store (Claude) only fails later, at the first prompt: learn now whether that store is locked
    if p.credentials_locked().is_some() {
      let (me, agent) = (self.clone(), agent.to_owned());
      tokio::spawn(async move { me.check_locks(Some(&agent)).await });
    }
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

#[cfg(test)]
mod quota_failure_tests {
  use super::*;
  use crate::http::HttpStatus;

  fn http(status: u16, retry_after: Option<u64>) -> anyhow::Error {
    anyhow::Error::new(HttpStatus { status, retry_after: retry_after.map(Duration::from_secs) })
  }

  #[test]
  fn rate_limits_back_off_exponentially_and_honour_retry_after() {
    let mut prev: Option<QuotaFailure> = None;
    let mut waits = vec![];
    for _ in 0..6 {
      let f = quota_failure(&http(429, None), prev.as_ref(), 0);
      assert_eq!(f.issue, AccountQuotaIssue::RateLimited);
      waits.push(f.until_ms / 60_000);
      prev = Some(f);
    }
    assert_eq!(waits, vec![5, 10, 20, 40, 60, 60]);
    // the vendor's own hint wins over the backoff; `Retry-After: 0` is no hint
    assert_eq!(quota_failure(&http(429, Some(120)), None, 0).until_ms, 120_000);
    assert_eq!(quota_failure(&http(429, Some(0)), None, 0).until_ms, 5 * 60_000);
    // a different failure in between restarts the streak
    let other = quota_failure(&http(500, None), prev.as_ref(), 0);
    assert_eq!((other.issue, other.rate_limits, other.until_ms), (AccountQuotaIssue::Unavailable, 0, QUOTA_MAX_AGE_MS));
    assert_eq!(quota_failure(&http(429, None), Some(&other), 0).until_ms, 5 * 60_000);
  }

  #[test]
  fn rejected_or_expired_tokens_read_as_expired() {
    for e in [http(401, None), http(403, None), anyhow::Error::new(QuotaTokenExpired)] {
      assert_eq!(quota_failure(&e, None, 0).issue, AccountQuotaIssue::Expired);
    }
    assert_eq!(quota_failure(&anyhow::anyhow!("no Claude login"), None, 0).issue, AccountQuotaIssue::Unavailable);
  }
}
