//! The plugin page's view and actions (mirror of packages/shared/plugins.ts; the contract is docs/dev/plugins.md): the
//! user's plugins with their state, the built-in tool groups, recent changes and safe mode

use serde::{Deserialize, Serialize};

/// Who made a change: `user` (a page), `session` (a card in a session, with its agent and model) or `cli`
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProvenance {
  pub by: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub session: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
}

/// A frozen version waiting for the user to confirm it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginProposal {
  pub snapshot: String,
  pub provenance: PluginProvenance,
  pub at: i64,
}

/// What the static check of the source folder found
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginFindings {
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub errors: Vec<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub notes: Vec<String>,
}

/// One user plugin: a source folder, an entry of the state, or both
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
  pub id: String,
  pub name: String,
  pub description: String,
  pub version: String,
  /// `agents`, `tools`, `interface` or `automation`
  pub layer: String,
  /// The contribution kinds: `tools`, `mcp`, `hooks`, `theme`, `prompts`, `agent`
  pub contributes: Vec<String>,
  /// The names its script tools are served under
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub tools: Vec<String>,
  pub enabled: bool,
  /// Enabled and in effect: not under safe mode, its frozen copy intact
  pub loaded: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub snapshot: Option<String>,
  pub can_rollback: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub provenance: Option<PluginProvenance>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub committed_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub proposal: Option<PluginProposal>,
  /// The source folder differs from the version in use ("changes not enabled")
  #[serde(default)]
  pub changed: bool,
  /// Why an enabled plugin is not in effect, or why its source cannot be read
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub findings: Option<PluginFindings>,
  /// The source folder
  pub path: String,
}

/// A built-in tool group of Acpira's MCP server
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolGroupInfo {
  /// `summon`, `sessions`, `display`, `browser`
  pub id: String,
  pub on: bool,
  pub tools: Vec<String>,
}

/// One committed change (`changes.jsonl`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginChange {
  pub generation: u64,
  pub at: i64,
  pub id: String,
  /// `enable`, `disable`, `rollback`, `remove`, `propose`, `dismiss`, `safeMode`, `toolGroup`
  pub action: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub snapshot: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on: Option<bool>,
  pub provenance: PluginProvenance,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsView {
  /// The plugins folder (`~/.acpira/plugins`)
  pub root: String,
  pub generation: u64,
  /// User plugins are unloaded
  pub safe_mode: bool,
  /// Safe mode comes from the engine's launch (`--safe-mode`, `ACPIRA_SAFE_MODE`), not the switch
  #[serde(default)]
  pub safe_mode_forced: bool,
  pub plugins: Vec<PluginInfo>,
  pub tool_groups: Vec<ToolGroupInfo>,
  /// The newest first
  pub changes: Vec<PluginChange>,
}

/// The plugin page's actions. Its switch is the user's confirmation: `enable` freezes the source folder, checks it and
/// commits it, like a card in a session
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum PluginAction {
  Enable { id: String },
  /// Enable the version a proposal froze
  Confirm { id: String, snapshot: String },
  Dismiss { id: String },
  Disable { id: String },
  Rollback { id: String },
  Remove { id: String },
  /// Show the source folder in the OS file manager
  Reveal { id: String },
  SafeMode { on: bool },
  ToolGroup { id: String, on: bool },
}

/// The places of the page a plugin's interface may take (`contributes.ui`, docs/dev/plugins.md, Interface): above
/// the composer, in its toolbar, the session's side panel, the session list's side, the status bar, the start page,
/// the actions of a message, a tool panel tab and a page of its own
pub const UI_SLOTS: &[&str] = &["composer.above", "composer.toolbar", "session.panel", "sidebar", "statusbar", "start", "message.actions", "panel.tab", "page"];

/// One interface piece of a plugin in effect, as the page renders it: the HTML runs in a sandboxed frame of its own,
/// never in the page, and talks to its plugin through the page (`pluginCall`, `pluginPost`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSlot {
  pub plugin: String,
  /// Unique within the plugin
  pub id: String,
  /// One of `UI_SLOTS`
  pub slot: String,
  pub title: String,
  /// The document, read from the frozen copy
  pub html: String,
}

