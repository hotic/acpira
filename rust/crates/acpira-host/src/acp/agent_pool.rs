//! Warm agent processes: one idle initialized CLI per agent + cwd + account;
//! new sessions take it and only run session/new

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::agent_process::{AgentProcess, ClientHandlers};
use super::agent_registry::AgentRegistry;
use super::cancel::Cancel;
use super::rpc::{BoxFuture, RpcError};
use crate::store::transcript_store::LogFn;

const WARM_TTL: Duration = Duration::from_secs(5 * 60);

/// Handlers for a process that only answers initialize / session metadata: updates ignored, anything interactive declined
pub struct IdleHandlers {
  log: LogFn,
  on_exit: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl IdleHandlers {
  pub fn new(log: LogFn, on_exit: Option<Arc<dyn Fn() + Send + Sync>>) -> Arc<Self> {
    Arc::new(IdleHandlers { log, on_exit })
  }
}

impl ClientHandlers for IdleHandlers {
  fn on_update(&self, _: Value) {}
  fn on_permission(&self, _: Value, _: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    Box::pin(async { Ok(json!({ "outcome": { "outcome": "cancelled" } })) })
  }
  fn on_elicitation(&self, _: Value, _: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    Box::pin(async { Ok(json!({ "action": "cancel" })) })
  }
  fn on_grok_question(&self, _: Value, _: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    Box::pin(async { Ok(json!({ "outcome": "skip_interview" })) })
  }
  fn on_stderr(&self, line: &str) {
    (self.log)(line);
  }
  fn on_exit(&self, _: Option<i32>, _: Option<String>) {
    if let Some(f) = &self.on_exit {
      f();
    }
  }
}

pub type SpawnEnv = Arc<dyn Fn(String, String) -> BoxFuture<Option<acpira_shared::transcript::StrMap>> + Send + Sync>;

enum Slot {
  Warming(u64, tokio::sync::watch::Receiver<Option<Option<Arc<AgentProcess>>>>),
  Ready(u64, Arc<AgentProcess>),
}

pub struct AgentPool {
  registry: Arc<dyn Fn() -> Arc<AgentRegistry> + Send + Sync>,
  log: LogFn,
  spawn_env: Option<SpawnEnv>,
  slots: Arc<parking_lot::Mutex<HashMap<String, Slot>>>,
  seq: std::sync::atomic::AtomicU64,
}

pub fn pool_key(agent: &str, cwd: &str, account: Option<&str>) -> String {
  format!("{agent}\0{cwd}\0{}", account.unwrap_or(""))
}

impl AgentPool {
  pub fn new(registry: Arc<dyn Fn() -> Arc<AgentRegistry> + Send + Sync>, log: LogFn, spawn_env: Option<SpawnEnv>) -> Arc<Self> {
    Arc::new(AgentPool { registry, log, spawn_env, slots: Default::default(), seq: Default::default() })
  }

  pub fn ensure(self: &Arc<Self>, agent: &str, cwd: &str, account: Option<&str>) {
    let key = pool_key(agent, cwd, account);
    let mut slots = self.slots.lock();
    if slots.contains_key(&key) {
      return;
    }
    let gen_id = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (tx, rx) = tokio::sync::watch::channel(None);
    slots.insert(key.clone(), Slot::Warming(gen_id, rx));
    drop(slots);
    let me = self.clone();
    let (agent, cwd, account) = (agent.to_owned(), cwd.to_owned(), account.map(str::to_owned));
    tokio::spawn(async move {
      let proc = me.spawn_warm(&agent, &cwd, account.as_deref(), &key, gen_id).await;
      let _ = tx.send(Some(proc));
    });
  }

  async fn spawn_warm(
    self: &Arc<Self>,
    agent: &str,
    cwd: &str,
    account: Option<&str>,
    key: &str,
    gen_id: u64,
  ) -> Option<Arc<AgentProcess>> {
    let registry = (self.registry)();
    let result: anyhow::Result<Arc<AgentProcess>> = async {
      let def = registry.get(agent)?.clone();
      let bin = registry.resolve_binary(agent).await.ok_or_else(|| anyhow::anyhow!("no binary for {agent}"))?;
      let env = match (account, &self.spawn_env) {
        (Some(a), Some(f)) => f(agent.to_owned(), a.to_owned()).await,
        _ => None,
      };
      AgentProcess::spawn(&def, &bin, cwd, self.warm_handlers(key, gen_id), env.as_ref(), None).await
    }
    .await;
    match result {
      Ok(proc) => {
        (self.log)(&format!("warm {agent}: initialize ok"));
        let mut slots = self.slots.lock();
        if matches!(slots.get(key), Some(Slot::Warming(g, _)) if *g == gen_id) {
          slots.insert(key.to_owned(), Slot::Ready(gen_id, proc.clone()));
          drop(slots);
          self.arm(key.to_owned(), gen_id);
        }
        Some(proc)
      }
      Err(e) => {
        let mut slots = self.slots.lock();
        if matches!(slots.get(key), Some(Slot::Warming(g, _)) if *g == gen_id) {
          slots.remove(key);
        }
        drop(slots);
        (self.log)(&format!("warm {agent} failed: {e}"));
        None
      }
    }
  }

  fn arm(self: &Arc<Self>, key: String, gen_id: u64) {
    let slots = self.slots.clone();
    let log = self.log.clone();
    tokio::spawn(async move {
      tokio::time::sleep(WARM_TTL).await;
      let proc = {
        let mut s = slots.lock();
        match s.get(&key) {
          Some(Slot::Ready(g, _)) if *g == gen_id => match s.remove(&key) {
            Some(Slot::Ready(_, p)) => Some(p),
            _ => None,
          },
          _ => None,
        }
      };
      if let Some(p) = proc {
        p.kill().await;
        log(&format!("warm {} expired", key.split('\0').next().unwrap_or("")));
      }
    });
  }

  fn warm_handlers(self: &Arc<Self>, key: &str, gen_id: u64) -> Arc<IdleHandlers> {
    let log = self.log.clone();
    let slots = self.slots.clone();
    let key = key.to_owned();
    IdleHandlers::new(
      Arc::new(move |line: &str| log(&format!("warm stderr: {line}"))),
      Some(Arc::new(move || {
        let mut s = slots.lock();
        if matches!(s.get(&key), Some(Slot::Ready(g, _) | Slot::Warming(g, _)) if *g == gen_id) {
          s.remove(&key);
        }
      })),
    )
  }

  /// A warm process for these coordinates, rebound to the session's handlers; None when there is none (or it died)
  pub async fn take(&self, agent: &str, cwd: &str, account: Option<&str>, handlers: Arc<dyn ClientHandlers>) -> Option<Arc<AgentProcess>> {
    let key = pool_key(agent, cwd, account);
    // A warming slot stays in the map while its spawn settles, so invalidate() / dispose() still own (and kill) it; the claim
    // below only succeeds if the same generation is still there afterwards
    let (gen_id, rx) = match self.slots.lock().get(&key)? {
      Slot::Ready(g, _) => (*g, None),
      Slot::Warming(g, rx) => (*g, Some(rx.clone())),
    };
    if let Some(mut rx) = rx {
      let _ = rx.wait_for(Option::is_some).await;
    }
    let proc = {
      let mut slots = self.slots.lock();
      match slots.get(&key) {
        Some(Slot::Ready(g, _) | Slot::Warming(g, _)) if *g == gen_id => {}
        _ => return None,
      }
      match slots.remove(&key)? {
        Slot::Ready(_, p) => p,
        // The spawn task went away without settling
        Slot::Warming(..) => return None,
      }
    };
    if !proc.alive() {
      return None;
    }
    proc.bind(handlers);
    Some(proc)
  }

  /// Drop idle / still-warming processes so a registry or credential change cannot hand out a stale spawn
  pub fn invalidate(&self, agent: Option<&str>) {
    for slot in self.remove(agent) {
      tokio::spawn(kill_slot(slot));
    }
  }

  fn remove(&self, agent: Option<&str>) -> Vec<Slot> {
    let prefix = agent.map(|a| format!("{a}\0"));
    let mut slots = self.slots.lock();
    let keys: Vec<String> = slots.keys().filter(|k| prefix.as_ref().is_none_or(|p| k.starts_with(p))).cloned().collect();
    keys.into_iter().filter_map(|k| slots.remove(&k)).collect()
  }

  /// Every pooled process, ended before the host exits: SIGTERM now, and the returned future resolves once they are gone (a warming
  /// one is killed as soon as its spawn settles). The list holds each process as soon as it exists, for a hard kill if waiting runs out
  pub fn dispose(&self) -> (Arc<parking_lot::Mutex<Vec<Arc<AgentProcess>>>>, BoxFuture<()>) {
    let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let mut exits = tokio::task::JoinSet::new();
    for slot in self.remove(None) {
      let seen = seen.clone();
      exits.spawn(async move {
        let proc = match slot {
          Slot::Ready(_, p) => Some(p),
          Slot::Warming(_, mut rx) => rx.wait_for(Option::is_some).await.ok().and_then(|v| v.clone()).flatten(),
        };
        if let Some(p) = proc {
          seen.lock().push(p.clone());
          p.kill().await;
        }
      });
    }
    (seen, Box::pin(async move { while exits.join_next().await.is_some() {} }))
  }
}

async fn kill_slot(slot: Slot) {
  let proc = match slot {
    Slot::Ready(_, p) => Some(p),
    Slot::Warming(_, mut rx) => rx.wait_for(Option::is_some).await.ok().and_then(|v| v.clone()).flatten(),
  };
  if let Some(p) = proc {
    p.kill().await;
  }
}
