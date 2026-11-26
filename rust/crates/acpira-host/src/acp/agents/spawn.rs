//! Builds a pure transport `SpawnSpec` from an `AgentDef`, vendor rules, local-run profiles, and account env.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use acpira_shared::transcript::StrMap;

use crate::acp::agents::registry::AgentDef;
use crate::acp::transport::process::{AgentProcess, ClientHandlers, SpawnSpec, initialize_request_caps};
use crate::acp::vendors::{Vendor, antigravity, claude_auth, claude_workflow, droid_usage};

/// The initialize request advertised for an agent definition.
pub fn initialize_request(def: &AgentDef) -> Value {
  initialize_request_caps(def.terminal_auth, def.subagents)
}

/// `AgentProcess::own_dirs`: the account's value wins over the definition's, as in the child's environment.
fn own_dirs(def: &AgentDef, extra_env: Option<&StrMap>) -> Vec<PathBuf> {
  let Some(profile) = crate::acp::local_run::builtin(&def.id).and_then(|l| l.stand().cloned()) else { return Vec::new() };
  let named = |k: &String| extra_env.and_then(|e| e.get(k)).or_else(|| def.env.as_ref()?.get(k)).filter(|d| !d.is_empty());
  profile.own.env_dirs.iter().filter_map(named).map(PathBuf::from).collect()
}

/// Assemble the complete process specification for launching `def`.
pub fn spawn_spec(def: &AgentDef, binary: &str, cwd: &str, extra_env: Option<&StrMap>, init_timeout: Option<Duration>) -> SpawnSpec {
  // A saved Antigravity account names its store in the account env; `agy` takes it only as a flag.
  let mut args = def.args.clone();
  args.extend(antigravity::spawn_args(&def.id, extra_env));

  let mut env_remove = Vec::new();
  // A saved Claude account carries its OAuth store through the account env. Do not let a
  // stale host-level API credential or gateway override that account at request time.
  if def.id == "claude" && extra_env.is_some() {
    env_remove.extend(claude_auth::ACCOUNT_SCRUB_ENV.iter().map(|s| (*s).to_owned()));
  }
  // A saved or plugin Antigravity account names its own store: a host-level Google credential or endpoint would sign
  // the request in as someone else.
  if def.id == "antigravity" && extra_env.is_some_and(|e| e.contains_key(antigravity::STORE_ENV)) {
    env_remove.extend(antigravity::ACCOUNT_SCRUB_ENV.iter().map(|s| (*s).to_owned()));
  }

  let mut env_set = BTreeMap::new();
  // Claude Code keeps dynamic workflows off for SDK hosts by default; leave explicit host settings alone.
  if Vendor::of(&def.id).workflows() && std::env::var_os(claude_workflow::WORKFLOWS_ENV).is_none() {
    env_set.insert(claude_workflow::WORKFLOWS_ENV.to_owned(), "1".to_owned());
  }
  // On Windows `agy`'s background auto-updater detaches without CREATE_NO_WINDOW, flashing a console window.
  if Vendor::of(&def.id) == Vendor::Antigravity
    && antigravity::suppress_acp_auto_update(
      crate::platform::command::Os::current(),
      std::env::var_os(antigravity::DISABLE_AUTO_UPDATE_ENV).as_deref(),
    )
  {
    env_set.insert(antigravity::DISABLE_AUTO_UPDATE_ENV.to_owned(), "true".to_owned());
  }
  // The definition and account may override the refreshed environment for this child only.
  for (k, v) in def.env.iter().flatten() {
    env_set.insert(k.clone(), v.clone());
  }
  for (k, v) in extra_env.into_iter().flatten() {
    env_set.insert(k.clone(), v.clone());
  }

  let droid_sessions = (Vendor::of(&def.id) == Vendor::Droid).then(|| droid_usage::sessions_dir_from_env(&env_remove, &env_set, cwd));
  let own_dirs = own_dirs(def, extra_env);

  SpawnSpec {
    command: def.command.clone(),
    binary: binary.to_owned(),
    args,
    cwd: cwd.to_owned(),
    env_remove,
    env_set,
    initialize_params: initialize_request(def),
    init_timeout,
    droid_sessions,
    own_dirs,
  }
}

/// Spawn and initialize an agent subprocess from its registry definition and optional account environment.
pub async fn spawn_agent(
  def: &AgentDef,
  binary: &str,
  cwd: &str,
  h: Arc<dyn ClientHandlers>,
  extra_env: Option<&StrMap>,
  init_timeout: Option<Duration>,
) -> Result<Arc<AgentProcess>> {
  AgentProcess::spawn_spec(spawn_spec(def, binary, cwd, extra_env, init_timeout), h).await
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn initialize_advertises_structured_compaction() {
    let req = initialize_request(&AgentDef::default());
    assert!(req["clientCapabilities"]["session"]["compaction"].is_object());
  }

  #[test]
  fn initialize_advertises_session_notices() {
    let req = initialize_request(&AgentDef::default());
    assert!(req["clientCapabilities"]["session"]["notices"].is_object());
  }
}
