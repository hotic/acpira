//! Desktop Commander presence: executable / process presence and
//! config metadata only; pairing tokens, cookies and process arguments never reach a response

use acpira_shared::chatgpt_integration::DesktopCommanderStatus;

use crate::acp::agent_registry::resolve_command;
use crate::acp::launch::{Os, ProcessEnv};
use crate::store::data_dir::home_dir;

pub fn commander_facts(binary: bool, configuration: bool, running: Option<bool>) -> DesktopCommanderStatus {
  let evidence = if running == Some(true) {
    "process"
  } else if binary {
    "executable"
  } else if configuration {
    "configuration"
  } else {
    "none"
  };
  let installation = if running == Some(true) || binary {
    "detected"
  } else if configuration || running.is_none() {
    "unknown"
  } else {
    "not_detected"
  };
  let process = match running {
    None => "unknown",
    Some(true) => "detected",
    Some(false) => "not_detected",
  };
  DesktopCommanderStatus {
    installation: installation.into(),
    process: process.into(),
    pairing: "unknown".into(),
    evidence: evidence.into(),
  }
}

pub async fn desktop_commander_status() -> DesktopCommanderStatus {
  let binary = resolve_command("desktop-commander", &[], Os::current(), &ProcessEnv).await.is_some();
  let configuration = tokio::fs::metadata(home_dir().join(".claude-server-commander").join("config.json")).await.is_ok_and(|m| m.is_file());
  commander_facts(binary, configuration, commander_process().await)
}

async fn commander_process() -> Option<bool> {
  if cfg!(windows) {
    return None;
  }
  let out =
    tokio::time::timeout(std::time::Duration::from_millis(1500), tokio::process::Command::new("ps").args(["-ax", "-o", "args="]).output())
      .await
      .ok()?
      .ok()?;
  if !out.status.success() {
    return None;
  }
  let re = regex::Regex::new(r"(?:^|\s)desktop-commander\s+remote(?:\s|$)").unwrap();
  Some(String::from_utf8_lossy(&out.stdout).lines().any(|l| l.contains("/desktop-commander/") || re.is_match(l)))
}
