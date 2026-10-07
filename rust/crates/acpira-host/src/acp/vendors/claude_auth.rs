//! Claude can create an ACP session with no credentials. Ask the same bundled CLI for its read-only auth status
//! before declaring the session ready; the saved-account list cannot decide this (API keys and cloud providers work too).

use std::process::Stdio;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;
use tokio::io::AsyncReadExt;

use acpira_shared::transcript::StrMap;

use crate::acp::agents::{launch, registry::AgentDef};

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: u64 = 64 * 1024;

/// A first 401 can be the CLI refreshing OAuth. A second rejected attempt is a failed credential, not useful progress.
/// claude-agent-acp suppresses its normal retry notice for these frames, leaving the host's Working row otherwise empty.
pub fn repeated_auth_retry(params: &Value) -> bool {
  let Some(msg) = params.get("message") else { return false };
  msg.get("type").and_then(Value::as_str) == Some("system")
    && msg.get("subtype").and_then(Value::as_str) == Some("api_retry")
    && msg.get("error_status").and_then(Value::as_u64) == Some(401)
    && msg.get("error").and_then(Value::as_str) == Some("authentication_failed")
    && msg.get("attempt").and_then(Value::as_u64).is_some_and(|n| n >= 2)
}

/// None means the CLI did not establish a status, not that the user is signed out. Only a definite false blocks ACP.
pub async fn authenticated(def: &AgentDef, binary: &str, cwd: &str, account_env: Option<&StrMap>) -> Result<Option<bool>> {
  let mut args = def.args.clone();
  args.extend(["--cli", "auth", "status", "--json"].map(str::to_owned));
  let mut cmd = launch::command(binary, &args);
  cmd.current_dir(cwd).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
  for (k, v) in def.env.iter().flatten().chain(account_env.into_iter().flatten()) {
    cmd.env(k, v);
  }
  #[cfg(unix)]
  cmd.process_group(0);
  #[cfg(windows)]
  let (mut child, _job) = crate::platform::windows_process::spawn(&mut cmd).await?;
  #[cfg(not(windows))]
  let mut child = cmd.spawn()?;
  #[cfg(unix)]
  let mut group = ProbeGroup(child.id());
  let mut stdout = child.stdout.take().expect("piped").take(OUTPUT_LIMIT + 1);
  let mut output = Vec::new();
  tokio::time::timeout(PROBE_TIMEOUT, async {
    // Drain before reaping: a timed-out wrapper's PID still identifies its group while its native child owns stdout.
    stdout.read_to_end(&mut output).await?;
    anyhow::ensure!(output.len() <= OUTPUT_LIMIT as usize, "Claude auth status output exceeded its limit");
    child.wait().await?;
    #[cfg(unix)]
    {
      group.0 = None;
    }
    Ok::<_, anyhow::Error>(())
  })
  .await??;
  // A signed-out CLI exits 1 with valid JSON, so the exit code is deliberately not used as the login state.
  Ok(serde_json::from_slice::<Value>(&output).ok().and_then(|v| status_of(&v)))
}

#[cfg(unix)]
struct ProbeGroup(Option<u32>);

#[cfg(unix)]
impl Drop for ProbeGroup {
  fn drop(&mut self) {
    if let Some(pid) = self.0.and_then(|p| i32::try_from(p).ok()) {
      // The unreaped leader owns this process group; cancellation/timeout must also terminate the wrapped CLI.
      unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
  }
}

fn status_of(v: &Value) -> Option<bool> {
  let logged_in = v.get("loggedIn")?.as_bool()?;
  let provider = v.get("apiProvider").and_then(Value::as_str);
  let key = v.get("apiKeySource").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
  if logged_in || key || provider.is_some_and(|p| !p.is_empty() && p != "firstParty") {
    Some(true)
  } else if provider == Some("firstParty") && v.get("authMethod").and_then(Value::as_str) == Some("none") {
    Some(false)
  } else {
    None
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_refresh_attempt_or_a_non_auth_retry_is_not_terminal() {
    let frame = json!({ "message": { "type": "system", "subtype": "api_retry", "attempt": 2,
      "error_status": 401, "error": "authentication_failed" } });
    assert!(repeated_auth_retry(&frame));
    for (field, value) in [
      ("attempt", json!(1)),
      ("error_status", json!(429)),
      ("error_status", Value::Null),
      ("error", json!("rate_limit")),
      ("subtype", json!("task_progress")),
    ] {
      let mut other = frame.clone();
      other["message"][field] = value;
      assert!(!repeated_auth_retry(&other), "{other}");
    }
  }

  #[test]
  fn only_a_definite_missing_first_party_credential_requires_login() {
    assert_eq!(status_of(&json!({ "loggedIn": false, "authMethod": "none", "apiProvider": "firstParty" })), Some(false));
    for status in [
      json!({ "loggedIn": true, "authMethod": "oauth_token", "apiProvider": "firstParty" }),
      json!({ "loggedIn": true, "authMethod": "api_key", "apiProvider": "firstParty" }),
      json!({ "loggedIn": false, "authMethod": "none", "apiProvider": "bedrock" }),
      json!({ "loggedIn": false, "authMethod": "none", "apiProvider": "firstParty", "apiKeySource": "apiKeyHelper" }),
    ] {
      assert_eq!(status_of(&status), Some(true), "{status}");
    }
    for status in [Value::Null, json!({}), json!({ "loggedIn": false }), json!({ "loggedIn": "false" })] {
      assert_eq!(status_of(&status), None, "{status}");
    }
  }
}
