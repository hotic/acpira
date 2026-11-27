//! What the page knows about agents and accounts: installs, quota, session options and controls, slash commands, auth methods

use super::*;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
  pub id: AgentId,
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub accounts: Option<bool>,
  /// Its account layer also takes a pasted API key as an account (`addAccountKey`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub account_keys: Option<bool>,
  /// Quota is read only on an explicit user refresh, never automatically (e.g. `agy -p /usage`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub manual_quota: Option<bool>,
  /// Minimum seconds between manual quota refreshes (mirrors the provider's rate-limit floor)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quota_cooldown: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub credentials_locked: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub local_account: Option<LocalAccountInfo>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub available: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub disabled: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub missing: Option<Vec<String>>,
  /// Directories searched for the agent's own command when it was not found
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub searched: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub install: Option<AgentInstall>,
  /// The adapter version this release's install line pins, when the installed adapter is older (the in-app install updates it)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub update: Option<String>,
  /// Who makes the CLI, for an agent the page has no vendor of its own for (an `agents.json` entry or a plugin's)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub vendor: Option<String>,
  /// The agent's mark as SVG path data on a 24 x 24 box, for an agent the page has no mark of its own for
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mark: Option<Vec<String>>,
  /// It can run on this machine in a folder on an SSH host, only its tools there (`sshLocalAgents`); absent from an
  /// engine before this field
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub ssh_local: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentInstall {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub docs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWindow {
  pub id: String,
  pub remaining: Num,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountQuota {
  pub windows: Vec<QuotaWindow>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub on_demand_balance_usd: Option<Num>,
  pub fetched_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalAccountStatus {
  Loading,
  Ready,
  LoginRequired,
  Expired,
  Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalAccountInfo {
  pub label: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
  pub status: LocalAccountStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quota: Option<AccountQuota>,
}

/// Why a saved account shows no quota: the last read failed and the next one waits out a cooldown
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountQuotaIssue {
  /// The vendor answered 429; reads pause until its Retry-After or an exponential backoff passes
  RateLimited,
  /// The stored access token is past its expiry or was rejected (401 / 403)
  Expired,
  /// Any other failure (network, 5xx, malformed reply)
  Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
  pub id: String,
  pub agent: AgentId,
  pub label: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
  pub added_at: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_used_at: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quota: Option<AccountQuota>,
  /// Set only while `quota` is absent and the last read failed
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub quota_issue: Option<AccountQuotaIssue>,
  /// The credential is a pasted API key: no sign-in renews it, so a rejected one is pasted again
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub key_auth: Option<bool>,
  /// The name of the plugin that lists this account (`accounts/set`): it lives as long as that plugin runs, never stored
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub plugin: Option<String>,
  /// The plugin's tag beside the label, shown in place of a plan
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub badge: Option<String>,
  /// False: the account has no remove button (a plugin account whose plugin answers no `account.remove`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub removable: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionGroup {
  pub id: String,
  pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
  Official,
  Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptionSource {
  pub id: String,
  pub name: String,
  pub kind: SourceKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOption {
  pub id: String,
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub group: Option<OptionGroup>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub source: Option<OptionSource>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub kind: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlType {
  Boolean,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigControl {
  pub id: String,
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub category: Option<String>,
  #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
  pub kind: Option<ControlType>,
  #[serde(default)]
  pub options: Vec<SessionOption>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub value: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionControls {
  #[serde(default)]
  pub modes: Vec<SessionOption>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode_config_id: Option<String>,
  /// Model-specific parameters advertised by the agent, including empty lists for models with none.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub model_params: ModelShapes,
  #[serde(default)]
  pub options: Vec<ConfigControl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInput {
  pub hint: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
  pub name: String,
  #[serde(default)]
  pub description: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub input: Option<CommandInput>,
  /// A plugin's command: the plugin, and the composer button it also is (`CommandDef::button` / `draft`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub plugin: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub button: Option<String>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub draft: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub icon: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub look: Option<String>,
  /// The button runs with the draft as its arguments (`CommandDef::takes_draft`); otherwise it leaves the draft alone
  #[serde(default, rename = "takesDraft", skip_serializing_if = "is_false")]
  pub takes_draft: bool,
  /// The button opens this menu instead of running the command, with the session's picks checked
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub menu: Vec<CommandMenuItem>,
  /// The label of the first group's pick, shown on the chip the way the mode chip shows its access level
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub badge: Option<String>,
}

/// One row of a plugin command button's menu (`plugins/manifest.rs` `MenuItemDef`)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandMenuItem {
  #[serde(default)]
  pub id: String,
  /// `radio`, `toggle`, `action` or `separator`
  pub kind: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub group: Option<String>,
  #[serde(default)]
  pub label: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub checked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalAuth {
  pub args: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub env: Option<StrMap>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthMethodInfo {
  pub id: String,
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub terminal: Option<TerminalAuth>,
}
