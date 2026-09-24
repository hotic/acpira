//! ChatGPT bridge transport facts (mirror of src/shared/chatgptIntegration.ts)

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopCommanderStatus {
  pub installation: String,
  pub process: String,
  pub pairing: String,
  pub evidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMirrors {
  pub mirrors: u64,
  pub observed_mirrors: u64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub latest_session_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_event_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatGptIntegrationStatus {
  pub checked_at: String,
  pub bridge_available: bool,
  pub desktop_commander: DesktopCommanderStatus,
  pub project: ProjectMirrors,
}
