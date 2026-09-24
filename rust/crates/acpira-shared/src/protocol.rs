//! Host ↔ webview message contract (mirror of src/shared/protocol.ts); both sides trust only this

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::appearance::Appearance;
use crate::chatgpt_integration::ChatGptIntegrationStatus;
use crate::i18n::Locale;
use crate::inventory::AgentInventory;
use crate::settings::{HiddenMap, SettingsView};
use crate::transcript::{
  AccountInfo, AgentId, AgentInfo, ConfigControl, Draft, NativeSessionInfo, QuestionAnswers, SessionSummary, Turn, TurnSettings,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebviewHost {
  Sidebar,
  Editor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditIntent {
  Replace,
  Continue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditTurnRequest {
  pub session_id: String,
  #[serde(deserialize_with = "crate::num::lenient_u64")]
  pub turn_index: u64,
  #[serde(deserialize_with = "crate::num::lenient_u64")]
  pub turn_count: u64,
  pub original_text: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub turn_id: Option<String>,
  pub text: String,
  #[serde(default)]
  pub retained_attachments: Vec<i64>,
  #[serde(default)]
  pub attachments: Vec<Draft>,
  #[serde(default)]
  pub settings: TurnSettings,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub intent: Option<EditIntent>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitState {
  pub host: WebviewHost,
  pub appearance: Appearance,
  pub agents: Vec<AgentInfo>,
  pub accounts: Vec<AccountInfo>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub account_actions: Option<Vec<AccountAction>>,
  pub hidden: HiddenMap,
  pub sessions: Vec<SessionSummary>,
  /// The active session's view, serialized once
  #[serde(skip_serializing_if = "Option::is_none")]
  pub active: Option<RawJson>,
  pub settings: SettingsView,
  pub locale: Locale,
  pub home: String,
  pub cwd: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub blob_base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHit {
  pub uri: String,
  pub path: String,
}

/// Links in agent output open on the host side; only these schemes are ever handed to openExternal
pub fn is_safe_external_url(url: &str) -> bool {
  let Some(colon) = url.find(':') else { return false };
  let scheme = url[..colon].trim().to_ascii_lowercase();
  let valid = !scheme.is_empty()
    && scheme.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
    && scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c));
  if !valid {
    return false;
  }
  let rest = &url[colon + 1..];
  match scheme.as_str() {
    // new URL() rejects http(s) without a host
    "http" | "https" => rest.trim_start_matches(['/', '\\']).chars().next().is_some_and(|c| !c.is_whitespace() && c != '?' && c != '#'),
    "mailto" => true,
    _ => false,
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddAccountVia {
  Import,
  Login,
  Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountActionStatus {
  Pending,
  Success,
  Missing,
  Cancelled,
  Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountAction {
  pub agent: AgentId,
  pub via: AddAccountVia,
  pub status: AccountActionStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum HostMsg {
  ChatgptStatus {
    status: ChatGptIntegrationStatus,
  },
  EditTurnResult {
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  Init {
    state: Box<InitState>,
  },
  Appearance {
    appearance: Appearance,
  },
  Agents {
    agents: Vec<AgentInfo>,
  },
  Sessions {
    sessions: Vec<SessionSummary>,
  },
  /// The session view, already serialized once for every viewer that shows it
  Session {
    session: RawJson,
    /// Not on the wire: an idle edge flushes the batch at once
    #[serde(skip)]
    running: bool,
  },
  Subagent {
    session_id: String,
    subagent_id: String,
    rev: i64,
    running: bool,
    turns: RawJson,
  },
  Accounts {
    accounts: Vec<AccountInfo>,
  },
  AccountActions {
    actions: Vec<AccountAction>,
  },
  Hidden {
    hidden: HiddenMap,
  },
  Settings {
    settings: SettingsView,
    locale: Locale,
  },
  Inventory {
    agent: AgentId,
    inventory: AgentInventory,
  },
  Controls {
    agent: AgentId,
    controls: Vec<ConfigControl>,
  },
  Files {
    seq: i64,
    files: Vec<FileHit>,
  },
  NativeSessions {
    agent: AgentId,
    sessions: Vec<NativeSessionInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
}

impl HostMsg {
  /// The batch key: several subagent streams can be live at once, so they key on their ids
  pub fn batch_key(&self) -> String {
    match self {
      HostMsg::Subagent { session_id, subagent_id, .. } => format!("subagent:{session_id}:{subagent_id}"),
      other => other.type_name().to_owned(),
    }
  }

  pub fn type_name(&self) -> &'static str {
    match self {
      HostMsg::ChatgptStatus { .. } => "chatgptStatus",
      HostMsg::EditTurnResult { .. } => "editTurnResult",
      HostMsg::Init { .. } => "init",
      HostMsg::Appearance { .. } => "appearance",
      HostMsg::Agents { .. } => "agents",
      HostMsg::Sessions { .. } => "sessions",
      HostMsg::Session { .. } => "session",
      HostMsg::Subagent { .. } => "subagent",
      HostMsg::Accounts { .. } => "accounts",
      HostMsg::AccountActions { .. } => "accountActions",
      HostMsg::Hidden { .. } => "hidden",
      HostMsg::Settings { .. } => "settings",
      HostMsg::Inventory { .. } => "inventory",
      HostMsg::Controls { .. } => "controls",
      HostMsg::Files { .. } => "files",
      HostMsg::NativeSessions { .. } => "nativeSessions",
    }
  }
}

/// A pre-serialized JSON fragment shared between viewers: a session view is encoded once per change, not once per view
#[derive(Debug, Clone)]
pub struct RawJson(pub std::sync::Arc<serde_json::value::RawValue>);

impl RawJson {
  pub fn new<T: Serialize>(v: &T) -> RawJson {
    RawJson(std::sync::Arc::from(serde_json::value::to_raw_value(v).expect("serializable")))
  }
  pub fn get(&self) -> &str {
    self.0.get()
  }
}

impl PartialEq for RawJson {
  fn eq(&self, other: &Self) -> bool {
    self.get() == other.get()
  }
}

impl Serialize for RawJson {
  fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    self.0.serialize(s)
  }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WebviewMsg {
  ChatgptStatus,
  ConnectChatgpt,
  EditTurn {
    request_id: String,
    edit: EditTurnRequest,
  },
  Ready,
  Send {
    session_id: Option<String>,
    text: String,
    #[serde(default)]
    attachments: Vec<Draft>,
  },
  Stop {
    session_id: Option<String>,
  },
  SearchFiles {
    query: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  Permission {
    session_id: String,
    block_id: String,
    option_id: String,
  },
  Answer {
    session_id: String,
    block_id: String,
    #[serde(default)]
    answers: QuestionAnswers,
    #[serde(default)]
    skip: Option<bool>,
  },
  BuildPlan {
    session_id: String,
    plan_id: String,
    option_id: Option<String>,
    model: Option<ModelPick>,
  },
  OpenPlan {
    session_id: String,
    plan_id: String,
  },
  SetMode {
    session_id: Option<String>,
    id: String,
  },
  SetConfig {
    session_id: Option<String>,
    config_id: String,
    value: String,
  },
  SelectSession {
    id: String,
  },
  NewSession {
    agent: Option<AgentId>,
  },
  RenameSession {
    id: String,
    title: String,
  },
  DeleteSession {
    id: String,
  },
  RestoreSession {
    id: String,
  },
  PinSession {
    id: String,
    pinned: bool,
  },
  MoveSession {
    id: String,
  },
  ForkSession {
    session_id: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    turn_index: i64,
  },
  ExportSession {
    id: String,
    format: ExportFormat,
  },
  SelectAccount {
    session_id: Option<String>,
    id: String,
  },
  AddAccount {
    agent: AgentId,
    via: AddAccountVia,
  },
  RemoveAccount {
    id: String,
  },
  RefreshQuota {
    agent: AgentId,
  },
  Compact {
    session_id: Option<String>,
  },
  Login {
    session_id: Option<String>,
    method_id: Option<String>,
  },
  InstallAgent {
    agent: AgentId,
  },
  Retry {
    session_id: Option<String>,
  },
  RetryTurn {
    session_id: Option<String>,
  },
  Reconnect {
    session_id: Option<String>,
  },
  ObserveSubagent {
    session_id: String,
    subagent_id: String,
  },
  UnobserveSubagent {
    session_id: String,
    subagent_id: String,
  },
  CancelSubagent {
    session_id: String,
    subagent_id: String,
  },
  StopAsyncTask {
    session_id: String,
    task_id: String,
  },
  Dequeue {
    session_id: String,
    id: String,
  },
  SendQueued {
    session_id: String,
    id: String,
  },
  EditQueued {
    session_id: String,
    id: String,
    text: String,
    #[serde(default)]
    retained_attachments: Vec<i64>,
    #[serde(default)]
    attachments: Vec<Draft>,
  },
  OpenInEditor {
    session_id: Option<String>,
  },
  OpenExternal {
    url: String,
  },
  SetSetting {
    key: String,
    #[serde(default)]
    value: Value,
  },
  SetAppearance {
    axis: String,
    value: String,
  },
  OpenPath {
    path: String,
  },
  OpenFile {
    session_id: String,
    path: String,
    #[serde(default, deserialize_with = "crate::num::lenient_opt_i64")]
    line: Option<i64>,
  },
  OpenBlob {
    session_id: String,
    name: String,
  },
  Inventory {
    agent: AgentId,
  },
  Controls {
    agent: AgentId,
    #[serde(default)]
    fresh: Option<bool>,
  },
  ListNativeSessions {
    agent: AgentId,
  },
  ImportNativeSession {
    agent: AgentId,
    session_id: String,
    cwd: String,
    title: Option<String>,
    updated_at: Option<String>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPick {
  pub config_id: String,
  pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
  Markdown,
  Json,
}

impl WebviewMsg {
  /// The `type` of a raw message, for logs about messages that failed to parse
  pub fn raw_type(v: &Value) -> &str {
    v.get("type").and_then(Value::as_str).unwrap_or("?")
  }
}

/// Turns of a subagent transcript, as sent in the `subagent` message
pub type SubagentTurns = Vec<Turn>;

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn safe_urls() {
    assert!(is_safe_external_url("https://example.com/"));
    assert!(is_safe_external_url("mailto:a@b.c"));
    assert!(!is_safe_external_url("javascript:alert(1)"));
    assert!(!is_safe_external_url("file:///etc/passwd"));
    assert!(!is_safe_external_url("https://"));
  }

  #[test]
  fn parses_webview_messages() {
    let m: WebviewMsg = serde_json::from_str(r#"{"type":"send","text":"hi"}"#).unwrap();
    assert_eq!(m, WebviewMsg::Send { session_id: None, text: "hi".into(), attachments: vec![] });
    let m: WebviewMsg = serde_json::from_str(r#"{"type":"openFile","sessionId":"s","path":"a.ts","line":3}"#).unwrap();
    assert!(matches!(m, WebviewMsg::OpenFile { line: Some(3), .. }));
    let r: WebviewMsg = serde_json::from_str(r#"{"type":"ready"}"#).unwrap();
    assert_eq!(r, WebviewMsg::Ready);
  }

  #[test]
  fn host_msg_shape() {
    let v = serde_json::to_value(HostMsg::Files { seq: 2, files: vec![] }).unwrap();
    assert_eq!(v, serde_json::json!({ "type": "files", "seq": 2, "files": [] }));
    let v = serde_json::to_value(HostMsg::EditTurnResult { request_id: "r".into(), error: None }).unwrap();
    assert_eq!(v, serde_json::json!({ "type": "editTurnResult", "requestId": "r" }));
  }
}
