//! The history list's "Import from <agent>": a throwaway spawn runs
//! initialize + session/list for this workspace, then dies; session/new is never called

use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use acpira_shared::transcript::StrMap;

use super::agent_pool::IdleHandlers;
use super::agent_process::AgentProcess;
use super::agent_registry::AgentDef;
use crate::i18n::tp;
use crate::store::transcript_store::LogFn;

const MAX_PAGES: usize = 40;
const MAX_SESSIONS: usize = 200;

pub async fn list_native_sessions(
  def: &AgentDef,
  binary: &str,
  cwd: &str,
  extra_env: Option<&StrMap>,
  log: LogFn,
  timeout: Option<Duration>,
) -> Result<Vec<Value>> {
  let timeout = timeout.unwrap_or(Duration::from_secs(20));
  let l2 = log.clone();
  let cmd = def.command.clone();
  let proc = AgentProcess::spawn(
    def,
    binary,
    cwd,
    IdleHandlers::new(std::sync::Arc::new(move |line: &str| l2(&format!("native list {cmd} stderr: {line}"))), None),
    extra_env,
    None,
  )
  .await?;
  let out: Result<Vec<Value>> = async {
    if !crate::json::truthy(proc.caps().get("sessionCapabilities").and_then(|s| s.get("list"))) {
      return Err(anyhow!(tp("session.import.unsupported", &[("agent", &def.name)])));
    }
    let deadline = tokio::time::Instant::now() + timeout;
    let list_pages = |list_cwd: String| {
      let proc = proc.clone();
      async move {
        let mut sessions: Vec<Value> = vec![];
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_PAGES {
          if sessions.len() >= MAX_SESSIONS {
            break;
          }
          let mut params = json!({ "cwd": list_cwd });
          if let Some(c) = &cursor {
            params["cursor"] = Value::from(c.clone());
          }
          let r = tokio::time::timeout_at(deadline, proc.request("session/list", params))
            .await
            .map_err(|_| anyhow!("session/list timed out after {}ms", timeout.as_millis()))??;
          sessions.extend(r.get("sessions").and_then(Value::as_array).cloned().unwrap_or_default());
          cursor = r.get("nextCursor").and_then(Value::as_str).filter(|c| !c.is_empty()).map(str::to_owned);
          if cursor.is_none() {
            break;
          }
        }
        Ok::<_, anyhow::Error>(sessions)
      }
    };
    let mut sessions = list_pages(cwd.to_owned()).await?;
    // codex-acp stores the canonicalized thread cwd (macOS /var → /private/var): retry once with the resolved path
    let real = tokio::fs::canonicalize(cwd).await.map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| cwd.to_owned());
    if sessions.is_empty() && real != cwd {
      sessions = list_pages(real).await?;
    }
    log(&format!("native list {}: {} session(s) in {cwd}", def.command, sessions.len()));
    Ok(sessions)
  }
  .await;
  proc.kill().await;
  out
}
