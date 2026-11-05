//! The context split (`packages/shared/context.ts`): only an agent that knows what it sent reports one. The built-in
//! agent advertises `_meta["acpira/context"]` at initialize and answers `_acpira/context` from the last request it made
//! (`acpira-agent/src/context.rs`); for every other agent, and while no process runs, there is none and the card keeps
//! the bare figure. Nothing is estimated from the transcript, which still holds what an agent compacted away

use std::sync::Arc;

use serde_json::{Value, json};

use acpira_shared::context::{ContextContent, ContextRaw, ContextReport};

use crate::acp::session::AcpSession;
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
    let (proc, peer) = self.context_peer()?;
    let mut v = proc.request(REPORT, json!({ "sessionId": peer, "detail": detail })).await.ok()?;
    serde_json::from_value(v.get_mut("report")?.take()).ok()
  }

  /// One leaf's content in the report `rev` describes
  pub async fn context_item(&self, rev: &str, id: &str) -> Option<ContextContent> {
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
