//! Envelope protocol between a shell and the sidecar (mirror of packages/shared/sidecar.ts): ndjson, one envelope per line,
//! stdout carries only these. Both sides must speak the same protocolVersion; a mismatch is rejected at hello

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::protocol::{HostMsg, WebviewHost};

pub const SIDECAR_PROTOCOL_VERSION: i64 = 1;

/// Requests the shell must answer (platformResponse); the rest are fire-and-forget
pub const PLATFORM_RPC_METHODS: [&str; 6] = ["openResolvedFile", "openPlanDocument", "revealInOS", "searchFiles", "writeSetting", "openBrowser"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PlanTarget {
  Path { path: String },
  Markdown { markdown: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "method", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PlatformRequest {
  OpenResolvedFile {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<i64>,
  },
  OpenPlanDocument {
    target: PlanTarget,
  },
  #[serde(rename = "revealInOS")]
  RevealInOs {
    path: String,
  },
  SearchFiles {
    query: String,
  },
  WriteSetting {
    key: String,
    value: Value,
  },
  OpenExternal {
    url: String,
  },
  /// An http(s) page in a browser tab inside the editor (`open_browser`). The shell forwards the port of a remote
  /// workspace, checks from the user's machine that the page answers and only then opens it; the result is
  /// `{ url }`, the address the user's side loaded
  OpenBrowser {
    url: String,
    /// The session folder when it is on an SSH host (`ssh://alias/path`): a loopback page is that host's, shown through
    /// the engine's preview proxy for it, and the engine has already found it listening there
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
  },
  OpenInEditor {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
  },
  RunInTerminal {
    title: String,
    command: String,
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    env: Option<Map<String, Value>>,
  },
  Toast {
    level: String,
    text: String,
    /// A file the notification offers to open (an "Open" button); the shell opens it only when clicked
    #[serde(skip_serializing_if = "Option::is_none")]
    open: Option<ToastOpen>,
  },
  /// A session needs the user or has settled: a chime and / or a system notification, each only when its setting is on.
  /// The shell stays quiet when its window is focused and a visible view shows the session
  Notify {
    session_id: String,
    title: String,
    body: String,
    /// The label of the notification's button that brings the session up (already localized)
    open: String,
    sound: bool,
    system: bool,
  },
}

/// openBrowser's answer: the address the user's side shows, and how an earlier open_browser tab was reused
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserOpened {
  #[serde(default)]
  pub url: String,
  #[serde(default)]
  pub reused: Option<BrowserReuse>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BrowserReuse {
  /// The tab already showed that URL: brought to the front, not reloaded
  Revealed,
  /// The tab showed another page of the same site and moved to this one
  Navigated,
}

/// The open-file action a toast carries: the button label (already localized) and the absolute path behind it
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToastOpen {
  pub label: String,
  pub path: String,
}

impl PlatformRequest {
  pub fn method(&self) -> &'static str {
    match self {
      PlatformRequest::OpenResolvedFile { .. } => "openResolvedFile",
      PlatformRequest::OpenPlanDocument { .. } => "openPlanDocument",
      PlatformRequest::RevealInOs { .. } => "revealInOS",
      PlatformRequest::SearchFiles { .. } => "searchFiles",
      PlatformRequest::WriteSetting { .. } => "writeSetting",
      PlatformRequest::OpenExternal { .. } => "openExternal",
      PlatformRequest::OpenBrowser { .. } => "openBrowser",
      PlatformRequest::OpenInEditor { .. } => "openInEditor",
      PlatformRequest::RunInTerminal { .. } => "runInTerminal",
      PlatformRequest::Toast { .. } => "toast",
      PlatformRequest::Notify { .. } => "notify",
    }
  }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellEnv {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cwd: Option<String>,
  #[serde(default)]
  pub host_language: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub blob_base: Option<String>,
  /// The shell organizes sessions by project folder and has no single workspace (the desktop app): whatever needs a
  /// folder follows the view's session instead of `cwd`, and no session starts on its own without a project
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub projects: Option<bool>,
}

/// envChanged carries a partial env: only the keys present change
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellEnvPatch {
  pub cwd: Option<String>,
  pub host_language: Option<String>,
  pub blob_base: Option<String>,
  pub projects: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
  pub name: String,
  #[serde(default)]
  pub version: String,
  #[serde(default)]
  pub capabilities: Vec<String>,
  /// The shell process's `SSH_CONNECTION` (a remote workspace): which SSH login, and so which macOS security session,
  /// the window runs in
  #[serde(default)]
  pub ssh_connection: Option<String>,
  /// `peer`: another engine (an SSH host's client, the phone) rather than a window. Its hello never rebinds the platform,
  /// it never serves IDE actions for the windows, its settings and focus events are ignored, and the platform requests
  /// its own views cause go back to it alone (`sidecar/server.rs`)
  #[serde(default)]
  pub role: Option<String>,
}

impl ClientInfo {
  pub fn is_peer(&self) -> bool {
    self.role.as_deref() == Some("peer")
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum InitialView {
  Id(String),
  MostRecent {
    #[serde(rename = "mostRecent")]
    most_recent: bool,
  },
  /// A view that never opens or starts a session by itself (a peer's list view, `session_manager/hosts.rs`)
  Empty { empty: bool },
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
  #[serde(deserialize_with = "crate::num::lenient_i64")]
  pub protocol_version: i64,
  pub request_id: String,
  pub client: ClientInfo,
  #[serde(default)]
  pub env: ShellEnv,
  #[serde(default)]
  pub settings: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PlatformEvent {
  WindowFocus,
  SettingsChanged {
    #[serde(default)]
    keys: Vec<String>,
    #[serde(default)]
    settings: Map<String, Value>,
  },
  EnvChanged {
    #[serde(default)]
    env: ShellEnvPatch,
  },
}

/// Shell → sidecar. `webviewMessage` keeps its payload raw: a malformed WebviewMsg is the view's problem, logged, never fatal
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ShellMsg {
  Hello(Box<Hello>),
  AttachView {
    view_id: String,
    host: WebviewHost,
    #[serde(default)]
    initial: Option<InitialView>,
    /// This view's own blob base; the env's when absent
    #[serde(default)]
    blob_base: Option<String>,
  },
  DetachView {
    view_id: String,
  },
  WebviewMessage {
    view_id: String,
    #[serde(default)]
    message: Value,
  },
  PlatformResponse {
    request_id: String,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
  },
  PlatformEvent {
    event: PlatformEvent,
  },
  /// A browser panel wants `url` (a loopback dev page) through a preview proxy; answered by `previewOpened`. `picker` is the
  /// script the proxy adds to the page (a file the shell ships). Sent again for the same panel when it navigates elsewhere
  PreviewOpen {
    request_id: String,
    panel_id: String,
    url: String,
    picker: String,
    /// The session folder the panel belongs to, when it is on an SSH host: the page is that host's dev server
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
  },
  /// The panel closed; its proxy goes when no other panel holds it
  PreviewClose {
    panel_id: String,
  },
  Shutdown,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SidecarInfo {
  pub version: String,
  pub pid: u32,
}

/// Sidecar → shell
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SidecarMsg {
  HelloOk {
    request_id: String,
    protocol_version: i64,
    sidecar: SidecarInfo,
    sessions_dir: String,
    /// Additions a shell may use without a protocol bump (older shells, the IDEA plugin and the phone ignore the field):
    /// `preview` = previewOpen / previewClose
    features: Vec<String>,
  },
  HelloReject {
    request_id: String,
    protocol_version: i64,
    reason: String,
  },
  HostMessage {
    view_id: String,
    message: HostMsg,
  },
  PlatformRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    request: PlatformRequest,
  },
  /// The answer to previewOpen: the proxied address and the picker's token, or why not
  PreviewOpened {
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  ShutdownOk,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn envelopes() {
    let m: ShellMsg = serde_json::from_str(r#"{"type":"attachView","viewId":"V","host":"sidebar","initial":{"mostRecent":true}}"#).unwrap();
    assert!(matches!(m, ShellMsg::AttachView { initial: Some(InitialView::MostRecent { most_recent: true }), .. }));
    let v =
      serde_json::to_value(SidecarMsg::PlatformRequest { request_id: None, request: PlatformRequest::RevealInOs { path: "/x".into() } })
        .unwrap();
    assert_eq!(v, serde_json::json!({ "type": "platformRequest", "request": { "method": "revealInOS", "path": "/x" } }));
    assert_eq!(serde_json::to_value(SidecarMsg::ShutdownOk).unwrap(), serde_json::json!({ "type": "shutdownOk" }));
  }
}
