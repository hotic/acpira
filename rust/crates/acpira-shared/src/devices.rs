//! Paired devices (a phone, another computer) reach the engine through `acpira-gateway` as a `peer` wire. The gateway is
//! the only process facing the network, so it is where a device's messages are filtered: the engine socket is the
//! user's own (mode 600), and any local process could already talk to it as a window.

use serde::{Deserialize, Serialize};

use crate::protocol::WebviewMsg;

/// The settings a paired device may write: how new sessions start and run, which agents are offered and in what order,
/// and which models their menus list. They steer sessions the device already drives; keys, model sources, MCP, the
/// proxy, appearance and the rest stay on the computer
pub const DEVICE_SETTINGS: &[&str] = &[
  "defaultAgent",
  "agentOrder",
  "disabledAgents",
  "steerQueued",
  "autoCompact",
  "compactAtTokens",
  "accountSwitch",
  "planAutoApprove",
  "hiddenOptions",
];

/// Whether a paired device may send this message. A device has one permission, control: it drives sessions the way the
/// user would at the computer (an agent it starts can run any command anyway). What stays on the computer is what
/// changes the machine rather than a session (settings outside `DEVICE_SETTINGS`, accounts, installs, plugins, storage,
/// SSH hosts), what puts something on the computer's screen (every `open…`, exports, terminals), and reads of arbitrary
/// local paths. Exhaustive on purpose: a new message does not compile until it is decided here.
pub fn device_allowed(m: &WebviewMsg) -> bool {
  use WebviewMsg as W;
  match m {
    // The view itself
    W::Ready { .. } | W::Resync => true,
    // Driving a session. The ones that act on a session another engine runs are forwarded there over the push bus
    W::Send { .. }
    | W::Stop { .. }
    | W::Permission { .. }
    | W::Answer { .. }
    | W::BuildPlan { .. }
    | W::SetMode { .. }
    | W::SetConfig { .. }
    | W::SelectAccount { .. }
    | W::Compact { .. }
    | W::Retry { .. }
    | W::RetryTurn { .. }
    | W::Reconnect { .. }
    | W::TakeOverSession { .. }
    | W::EditTurn { .. }
    | W::Dequeue { .. }
    | W::SendQueued { .. }
    | W::SteerQueued { .. }
    | W::EditQueued { .. }
    | W::StopAsyncTask { .. }
    | W::Goal { .. }
    | W::ObserveSubagent { .. }
    | W::UnobserveSubagent { .. }
    | W::CancelSubagent { .. } => true,
    // The session list. Deleting is undoable (`restoreSession`)
    W::SelectSession { .. }
    | W::DeselectSession { .. }
    | W::NewSession { .. }
    | W::RenameSession { .. }
    | W::PinSession { .. }
    | W::DeleteSession { .. }
    | W::RestoreSession { .. }
    | W::SetSessionCategory { .. }
    | W::CategoryOp { .. }
    | W::MoveSession { .. }
    | W::ForkSession { .. }
    | W::MarkAllRead => true,
    // Reads scoped to a session or its project, and what the composer needs
    W::SearchSessions { .. }
    | W::SearchFiles { .. }
    | W::DiffSource { .. }
    | W::SessionMarkdown { .. }
    | W::SessionChanges { .. }
    | W::ProjectDir { .. }
    | W::ProjectStatus { .. }
    | W::ProjectFile { .. }
    | W::Controls { .. }
    | W::Inventory { .. }
    | W::RefreshQuota { .. } => true,
    // The SSH hosts' state, read only: the list draws each host folder's alias and status from it, and a new session in
    // a host folder picks its agent by the host's run mode. Adding, changing and browsing hosts stay on the computer
    W::Connections => true,
    // Puts something on the computer's screen: the device opens links and files itself
    W::ViewFocus
    | W::OpenPlan { .. }
    | W::OpenInEditor { .. }
    | W::OpenExternal { .. }
    | W::OpenAgentConfig
    | W::OpenPath { .. }
    | W::OpenFile { .. }
    | W::OpenBlob { .. }
    | W::ExportSession { .. }
    | W::InstallInTerminal { .. } => false,
    // Stages any local path into the blob store; a device fetches blobs over its link instead
    W::ImageBlob { .. } => false,
    // A shell on the computer: not offered to devices yet
    W::TerminalOpen { .. }
    | W::TerminalInput { .. }
    | W::TerminalResize { .. }
    | W::TerminalAck { .. }
    | W::TerminalClose { .. }
    | W::TerminalKill { .. } => false,
    // The phone's settings page: session and agent preferences only
    W::SetSetting { key, .. } => DEVICE_SETTINGS.contains(&key.as_str()),
    // Changes the machine: credentials need its keychain or browser, the rest is its configuration
    W::AddAccount { .. }
    | W::RemoveAccount { .. }
    | W::UnlockCredentials { .. }
    | W::Login { .. }
    | W::InstallAgent { .. }
    | W::CancelInstall { .. }
    | W::SetAppearance { .. }
    | W::Shared
    | W::SharedAction { .. }
    | W::Storage { .. }
    | W::StorageAction { .. }
    | W::Plugins
    | W::PluginAction { .. }
    | W::PluginCard { .. }
    | W::PluginBackdrop { .. }
    | W::PluginCall { .. }
    | W::Providers
    | W::ProviderAction { .. }
    | W::ProviderProbe { .. }
    | W::ConnectionOp { .. }
    | W::SshHosts
    | W::RemoteDir { .. }
    | W::ChatgptStatus
    | W::ConnectChatgpt
    | W::ListNativeSessions { .. }
    | W::ImportNativeSession { .. }
    | W::RemoteAccess
    | W::RemoteAccessOp { .. } => false,
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
  Phone,
  Tablet,
  Computer,
}

/// The kind of path a connection uses right now (acpira-net `path::classify`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PathKind {
  /// A private or link-local address: the same network
  Lan,
  /// Tailscale's addresses (Tailscale may itself be relaying underneath; it is still "Tailscale")
  Tailscale,
  /// Any other address: a punched hole or an open port
  Direct,
  /// Through a relay server
  Relay,
}

/// Where traffic goes when no direct path works
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Relays {
  /// n0's public relays plus their lookup (which publishes the home relay, not IPs). n0 allows development and hobby
  /// use only: a public release needs another default
  #[default]
  N0,
  /// Own (or paid) relays and no n0 service at all; `token` is the relay's `shared_token`
  Custom {
    urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    token: Option<String>,
  },
  /// LAN, Tailscale and punched holes only: a connection that needs a relay fails instead
  Off,
}

/// A device the gateway admits. `id` is its EndpointId: the TLS handshake proves it, nothing else is checked
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
  pub id: String,
  pub name: String,
  pub kind: DeviceKind,
  /// Unix milliseconds
  pub paired_at: u64,
  #[serde(default)]
  pub version: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_seen: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatus {
  #[serde(flatten)]
  pub device: DeviceInfo,
  pub online: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<PathKind>,
}

/// An open pairing window: the `acpira://pair/…` link and its QR code
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingStatus {
  pub link: String,
  /// The 6 digits a device on the same network types instead of scanning
  #[serde(default)]
  pub code: String,
  pub expires_in_ms: u64,
  /// The link's QR code, one string per row, `1` a dark module, no quiet zone: drawn by the page, which has no encoder
  #[serde(default)]
  pub qr: Vec<String>,
}

/// What the gateway reports on its control socket (`status`, `watch`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayStatus {
  /// Its EndpointId
  pub id: String,
  /// The name devices see
  pub name: String,
  pub version: String,
  pub relays: Relays,
  /// Its home relay right now
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub relay: Option<String>,
  pub relay_connected: bool,
  pub devices: Vec<DeviceStatus>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pairing: Option<PairingStatus>,
}

/// A request on the gateway's control socket (`<ACPIRA_HOME>/run/gateway.sock`): one JSON line each, answered by one
/// line (`watch`: a `GatewayStatus` line on every change until the client hangs up). Here rather than in the gateway so
/// the engine can speak it without linking the network stack
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum GatewayRequest {
  Status,
  Watch,
  Pair,
  CancelPair,
  Revoke { id: String },
  Rename { id: String, name: String },
  /// Read `gateway.json` again (keep-awake; relays need a restart)
  Reload,
  Stop,
  /// A session wants the user: the gateway pushes it to the paired phones that registered for notifications. Sent by
  /// the engine that runs the session, whatever its shell; the gateway decides whether the user is at the computer
  Notify(PushNote),
}

/// Why a session wants the user (the engine's `AttentionKind`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PushKind {
  /// A permission card or a question waits on the user
  Waiting,
  Done,
  Error,
}

/// One notification for the paired phones, already in the engine's language
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushNote {
  pub session_id: String,
  pub kind: PushKind,
  /// The session's title
  pub title: String,
  /// What happened, as the desktop notification says it ("Claude is waiting for you")
  pub subtitle: String,
  /// The card's request, the reply's opening or the error; may be empty
  #[serde(default)]
  pub body: String,
}

/// Where a device's notifications go: an APNs device token and what the gateway signs the request for. The device
/// sends it over its link (acpira-net `Control::Push`); the gateway keeps it beside `devices.json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PushTarget {
  /// The APNs device token, hex
  pub token: String,
  /// The app's bundle id (`apns-topic`)
  pub topic: String,
  /// The developer team the app is signed by: the key the gateway signs with must be this team's
  pub team: String,
  /// A development build: APNs' sandbox
  #[serde(default)]
  pub sandbox: bool,
}

/// The settings page's remote access (HostMsg `remoteAccess`)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAccessView {
  /// This engine can turn remote access on here; when not, `unavailable` says why (`platform`: not macOS yet;
  /// `component`: no gateway binary beside the engine)
  pub available: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub unavailable: Option<String>,
  /// The switch: the gateway starts at login
  pub enabled: bool,
  /// An install, uninstall or restart is under way
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub busy: bool,
  /// The gateway's own report, while it answers
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub gateway: Option<GatewayStatus>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub keep_awake: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
}

/// What the settings page asks (WebviewMsg `remoteAccessOp`); every op answers with a `remoteAccess` push
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RemoteAccessOp {
  /// Install the gateway to start at login and start it
  Enable,
  /// Stop it and remove it from login; the paired devices are kept for the next time
  Disable,
  /// Open a pairing window (a fresh code: the previous one stops working)
  Pair,
  CancelPair,
  /// Forget a device; its connection drops at once
  Revoke { id: String },
  Rename { id: String, name: String },
  SetRelays { relays: Relays },
  SetKeepAwake { on: bool },
  /// The name devices see for this computer; blank goes back to the computer's own name
  SetName { name: String },
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn allowed(v: serde_json::Value) -> bool {
    device_allowed(&serde_json::from_value(v).unwrap())
  }

  #[test]
  fn drives_sessions_but_leaves_the_machine_alone() {
    assert!(allowed(json!({ "type": "send", "sessionId": "s", "text": "hi", "clientId": "c" })));
    assert!(allowed(json!({ "type": "permission", "sessionId": "s", "blockId": "b", "optionId": "o" })));
    assert!(allowed(json!({ "type": "deleteSession", "id": "s" })));
    assert!(!allowed(json!({ "type": "setSetting", "key": "k", "value": 1 })));
    assert!(!allowed(json!({ "type": "setSetting", "key": "proxy", "value": "http://x" })));
    assert!(allowed(json!({ "type": "setSetting", "key": "disabledAgents", "value": ["kimi"] })));
    assert!(allowed(json!({ "type": "setSetting", "key": "agentOrder", "value": ["claude", "codex"] })));
    assert!(!allowed(json!({ "type": "openFile", "sessionId": "s", "path": "a" })));
    assert!(!allowed(json!({ "type": "imageBlob", "sessionId": "s", "path": "/etc/passwd", "seq": 1 })));
    assert!(!allowed(json!({ "type": "terminalOpen", "id": "t" })));
    assert!(!allowed(json!({ "type": "removeAccount", "id": "a" })));
    assert!(allowed(json!({ "type": "connections" })));
    assert!(!allowed(json!({ "type": "connectionOp", "op": { "op": "remove", "alias": "h" } })));
    assert!(!allowed(json!({ "type": "remoteDir", "alias": "h", "path": "/", "seq": 1 })));
  }

  #[test]
  fn device_settings_are_setting_keys() {
    for key in DEVICE_SETTINGS {
      assert!(crate::settings::SETTING_KEYS.contains(key), "{key}");
    }
  }
}
