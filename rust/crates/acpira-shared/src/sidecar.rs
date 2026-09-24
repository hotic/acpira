//! Envelope protocol between a shell and the sidecar (mirror of src/shared/sidecar.ts): ndjson, one envelope per line,
//! stdout carries only these. Both sides must speak the same protocolVersion; a mismatch is rejected at hello

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::protocol::{HostMsg, WebviewHost};

pub const SIDECAR_PROTOCOL_VERSION: i64 = 1;

/// Requests the shell must answer (platformResponse); the rest are fire-and-forget
pub const PLATFORM_RPC_METHODS: [&str; 5] = ["openResolvedFile", "openPlanDocument", "revealInOS", "searchFiles", "writeSetting"];

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
  },
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
      PlatformRequest::OpenInEditor { .. } => "openInEditor",
      PlatformRequest::RunInTerminal { .. } => "runInTerminal",
      PlatformRequest::Toast { .. } => "toast",
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
}

/// envChanged carries a partial env: only the keys present change
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShellEnvPatch {
  pub cwd: Option<String>,
  pub host_language: Option<String>,
  pub blob_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ClientInfo {
  pub name: String,
  #[serde(default)]
  pub version: String,
  #[serde(default)]
  pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum InitialView {
  Id(String),
  MostRecent {
    #[serde(rename = "mostRecent")]
    most_recent: bool,
  },
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
