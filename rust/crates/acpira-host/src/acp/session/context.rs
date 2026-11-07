//! The context split (`packages/shared/context.ts`). The built-in agent advertises `_meta["acpira/context"]` at
//! initialize and answers `_acpira/context` from the last request it made (`acpira-agent/src/context.rs`); every other
//! agent, and any session while no agent process runs, gets the engine's estimate from the transcript
//! (`context_estimate.rs`)

use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::context::{ContextContent, ContextRaw, ContextReport};

use crate::acp::agents::registry::AgentRegistry;
use crate::acp::session::{AcpSession, context_claude, context_claude_sdk, context_estimate};
use crate::acp::transport::process::AgentProcess;

const REPORT: &str = "_acpira/context";
const ITEM: &str = "_acpira/contextItem";
const RAW: &str = "_acpira/contextRaw";

/// Advertised at initialize (`_meta["acpira/context"].version`)
fn reports(init: &Value) -> bool {
  init.pointer("/_meta/acpira~1context/version").and_then(Value::as_u64).is_some_and(|v| v >= 1)
}

impl AcpSession {
  /// The running agent and its session id, when the agent splits its context
  fn context_peer(&self) -> Option<(Arc<AgentProcess>, String)> {
    let c = self.core.lock();
    let proc = c.proc.clone().filter(|p| p.alive() && reports(&p.init))?;
    Some((proc, c.acp_session_id.clone()?))
  }

  /// The split: category sums, and the whole tree with `detail`
  pub async fn context_report(&self, detail: bool) -> Option<ContextReport> {
    if let Some((proc, peer)) = self.context_peer() {
      let agent = match proc.request(REPORT, json!({ "sessionId": peer, "detail": detail })).await {
        Ok(mut v) => v.get_mut("report").map(Value::take).and_then(|r| serde_json::from_value(r).ok()),
        Err(_) => None,
      };
      if agent.is_some() {
        return agent;
      }
    }
    let known = self.reported_split().await;
    let c = self.core.lock();
    context_estimate::report_with(&c.state.turns, c.state.usage.as_ref(), detail, known.as_ref())
  }

  /// The category sums an agent reports of its own, read when the report is asked for: Grok's session info, Claude's
  /// own session log and Claude Code's split of it
  async fn reported_split(&self) -> Option<context_estimate::Known> {
    if self.vendor == crate::acp::vendors::Vendor::Claude {
      let sid = self.core.lock().acp_session_id.clone()?;
      return claude_known(self.deps.registry.clone(), self.agent.clone(), self.cwd.clone(), sid).await;
    }
    if self.vendor.usage_poll() != Some(crate::acp::vendors::UsagePoll::Grok) {
      return None;
    }
    let (proc, sid) = {
      let c = self.core.lock();
      (c.proc.clone().filter(|p| p.alive())?, c.acp_session_id.clone()?)
    };
    let ask = proc.request("_x.ai/session/info", json!({ "sessionId": sid }));
    let v = tokio::time::timeout(std::time::Duration::from_secs(3), ask).await.ok()?.ok()?;
    context_estimate::grok_known(&v)
  }

  /// One leaf's content in the report `rev` describes
  pub async fn context_item(&self, rev: &str, id: &str) -> Option<ContextContent> {
    if rev.starts_with(context_estimate::REV_PREFIX) {
      let known = if id.starts_with("summary") || id.starts_with(context_estimate::DOC_PREFIX) { self.reported_split().await } else { None };
      if let Some(doc) = context_estimate::doc_content(known.as_ref(), id).await {
        return Some(doc);
      }
      let c = self.core.lock();
      return context_estimate::content(&c.state.turns, c.state.usage.as_ref(), rev, id, known.as_ref());
    }
    let (proc, peer) = self.context_peer()?;
    let mut v = proc.request(ITEM, json!({ "sessionId": peer, "rev": rev, "id": id })).await.ok()?;
    serde_json::from_value(v.get_mut("content")?.take()).ok()
  }

  /// The last request as it went out; Err with the agent's reason when it failed to answer
  pub async fn context_raw(&self) -> Result<Option<ContextRaw>, String> {
    let Some((proc, peer)) = self.context_peer() else { return Ok(None) };
    let mut v = proc.request(RAW, json!({ "sessionId": peer })).await.map_err(|e| e.message)?;
    Ok(v.get_mut("raw").map(Value::take).and_then(|r| serde_json::from_value(r).ok()))
  }
}

/// Claude's split: its own session log (`context_claude.rs`, read off the async runtime), the agent's own part split
/// as Claude Code splits it (`context_claude_sdk.rs`) when node and the adapter's SDK are found
pub async fn claude_known(registry: Arc<AgentRegistry>, agent: String, cwd: String, session_id: String) -> Option<context_estimate::Known> {
  let def = registry.try_get(&agent).cloned().unwrap_or_default();
  let config_env = def.env.as_ref().and_then(|e| e.get(crate::acp::vendors::claude_workflow_log::CONFIG_DIR_ENV).cloned());
  let (dir, sid) = (cwd.clone(), session_id.clone());
  let (path, log) = tokio::task::spawn_blocking(move || {
    let path = context_claude::log_path(config_env, &dir, &sid)?;
    let log = context_claude::known(&path)?;
    Some((path, log))
  })
  .await
  .ok()??;
  let launch = move || -> context_claude_sdk::LaunchFuture {
    Box::pin(async move {
      use crate::acp::agents::{adapter_info, launch::ProcessEnv, registry::resolve_command};
      let binary = registry.resolve_binary(&agent).await?;
      let sdk = adapter_info::engine_dir(&binary, &def).await?;
      let node = resolve_command("node", &[], crate::platform::command::Os::current(), &ProcessEnv).await?;
      Some(context_claude_sdk::Launch { node, sdk, env: def.env.clone().unwrap_or_default(), cwd, session_id })
    })
  };
  Some(context_claude_sdk::refine(path, log, launch).await)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn only_an_advertised_version_reports() {
    assert!(reports(&json!({ "_meta": { "acpira/context": { "version": 1 } } })));
    assert!(!reports(&json!({ "_meta": { "acpira/rewind": { "supported": true } } })));
    assert!(!reports(&json!({ "_meta": { "acpira/context": {} } })));
  }
}
