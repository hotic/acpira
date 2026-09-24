//! Agent inventory shown on the settings page (mirror of src/shared/inventory.ts)

use serde::{Deserialize, Serialize};

use crate::transcript::AgentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
  Stdio,
  Http,
  Sse,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryScope {
  User,
  Project,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryFile {
  pub path: String,
  pub scope: InventoryScope,
  pub exists: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryMcp {
  pub name: String,
  pub transport: McpTransport,
  pub target: String,
  pub source: String,
  pub scope: InventoryScope,
  pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventorySkill {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  pub path: String,
  pub scope: InventoryScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpCaps {
  pub http: bool,
  pub sse: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeInfo {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mcp: Option<McpCaps>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterPart {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub root: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnginePart {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub version: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub r#override: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub override_env: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterInfo {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub adapter: Option<AdapterPart>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub engine: Option<EnginePart>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHealthStage {
  SpawnFailed,
  HandshakeFailed,
  AuthRequired,
  Ready,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthSource {
  Probe,
  Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHealth {
  pub stage: AgentHealthStage,
  pub at: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  pub source: HealthSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInventory {
  pub agent: AgentId,
  pub binary: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub runtime: Option<AgentRuntimeInfo>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub adapter: Option<AdapterInfo>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub health: Option<AgentHealth>,
  pub steer: bool,
  pub config: Vec<InventoryFile>,
  pub mcp: Vec<InventoryMcp>,
  pub skills: Vec<InventorySkill>,
  pub rules: Vec<InventoryFile>,
  pub scanned_at: String,
}
