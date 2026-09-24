//! A throwaway spawn reading an agent's configOptions

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use acpira_shared::inventory::{AgentHealthStage, AgentRuntimeInfo};
use acpira_shared::model_sources::apply_model_sources;
use acpira_shared::transcript::{ConfigControl, SessionControls, StrMap};

use super::agent_pool::IdleHandlers;
use super::agent_process::{AgentProcess, AgentSpawnError};
use super::agent_registry::AgentDef;
use super::model_sources::read_model_sources;
use super::normalize::{init_controls, runtime_info_of};
use super::session_errors::is_auth;
use crate::store::transcript_store::LogFn;

pub struct ProbeResult {
  pub options: Vec<ConfigControl>,
  pub runtime: AgentRuntimeInfo,
}

/// A probe failure labelled with the stage it died at
#[derive(Debug)]
pub struct ProbeFailure {
  pub stage: AgentHealthStage,
  pub message: String,
}

impl std::fmt::Display for ProbeFailure {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.write_str(&self.message)
  }
}

impl std::error::Error for ProbeFailure {}

pub async fn probe_agent_controls(
  def: &AgentDef,
  binary: &str,
  cwd: &str,
  extra_env: Option<&StrMap>,
  log: LogFn,
  timeout: Option<Duration>,
) -> Result<ProbeResult, ProbeFailure> {
  let timeout = timeout.unwrap_or(Duration::from_secs(20));
  let (l2, cmd) = (log.clone(), def.command.clone());
  let proc = AgentProcess::spawn(
    def,
    binary,
    cwd,
    IdleHandlers::new(Arc::new(move |line: &str| l2(&format!("probe {cmd} stderr: {line}"))), None),
    extra_env,
    None,
  )
  .await
  .map_err(|e| ProbeFailure {
    stage: if e.downcast_ref::<AgentSpawnError>().is_some() { AgentHealthStage::SpawnFailed } else { AgentHealthStage::HandshakeFailed },
    message: e.to_string(),
  })?;
  let mut session_id: Option<String> = None;
  let result = async {
    let r = tokio::time::timeout(timeout, proc.request("session/new", json!({ "cwd": cwd, "mcpServers": [] })))
      .await
      .map_err(|_| anyhow::anyhow!("probe {}: session/new timed out after {}ms", def.command, timeout.as_millis()))??;
    session_id = r.get("sessionId").and_then(Value::as_str).map(str::to_owned);
    let mut controls = SessionControls::default();
    init_controls(&mut controls, r.get("modes"), r.get("configOptions"));
    apply_model_sources(&def.id, &mut controls.options, &read_model_sources(&def.id, cwd).await);
    let summary: Vec<String> = controls.options.iter().map(|o| format!("{}({})", o.id, o.options.len())).collect();
    log(&format!("probe {}: session/new ok · options {}", def.command, if summary.is_empty() { "-".into() } else { summary.join(" ") }));
    Ok::<_, anyhow::Error>(ProbeResult { options: controls.options, runtime: runtime_info_of(&proc.init) })
  }
  .await;
  if let Some(sid) = &session_id
    && proc.alive()
    && crate::json::truthy(proc.caps().get("sessionCapabilities").and_then(|s| s.get("close")))
    && let Err(e) = tokio::time::timeout(Duration::from_secs(3), proc.request("session/close", json!({ "sessionId": sid })))
      .await
      .map_err(|_| anyhow::anyhow!("close timeout"))
      .and_then(|r| r.map_err(anyhow::Error::new))
  {
    log(&format!("probe {}: session/close failed: {e}", def.command));
  }
  proc.kill().await;
  result.map_err(|e| ProbeFailure {
    stage: if is_auth(&e) { AgentHealthStage::AuthRequired } else { AgentHealthStage::HandshakeFailed },
    message: e.to_string(),
  })
}
