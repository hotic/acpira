//! Normalized transcript shape (mirror of src/shared/transcript.ts): the host reduces ACP session/update into these
//! blocks and the webview only understands these. Optional TS fields are `Option` + `skip_serializing_if`, so the JSON
//! this side writes is the JSON the TS side wrote (absent, never `null`)

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::model_shapes::ModelShapes;
use crate::num::{Num, lenient_opt_i64, lenient_opt_u64};
use crate::subagents::SubagentSummary;

pub type AgentId = String;
pub type StrMap = BTreeMap<String, String>;

fn is_false(v: &bool) -> bool {
  !*v
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
  pub id: AgentId,
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub external: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub accounts: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub local_account: Option<LocalAccountInfo>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub available: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub disabled: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub missing: Option<Vec<String>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub install: Option<AgentInstall>,
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
  #[serde(default)]
  pub options: Vec<ConfigControl>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandInput {
  pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
  pub name: String,
  #[serde(default)]
  pub description: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub input: Option<CommandInput>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
  Starting,
  Ready,
  AuthRequired,
  Readonly,
  Error,
  Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
  Read,
  Edit,
  Delete,
  Move,
  Search,
  Execute,
  Think,
  Fetch,
  SwitchMode,
  Other,
}

impl ToolKind {
  pub fn parse(s: &str) -> Option<ToolKind> {
    serde_json::from_value(Value::String(s.to_owned())).ok()
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
  Pending,
  InProgress,
  Completed,
  Failed,
  Cancelled,
}

impl ToolStatus {
  pub fn parse(s: &str) -> Option<ToolStatus> {
    serde_json::from_value(Value::String(s.to_owned())).ok()
  }
  pub fn is_open(self) -> bool {
    matches!(self, ToolStatus::Pending | ToolStatus::InProgress)
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRef {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub blob: Option<String>,
  pub mime_type: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSource {
  pub path: String,
  pub old_text: String,
  pub new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
  Hunk,
  Add,
  Del,
  Ctx,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
  pub kind: DiffKind,
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_u64")]
  pub old_line: Option<u64>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_u64")]
  pub new_line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolContent {
  Text {
    text: String,
  },
  Diff {
    lines: Vec<DiffLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<DiffSource>,
  },
  List {
    items: Vec<String>,
  },
  Image(ImageRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureAction {
  Retry,
  Login,
  NewSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCategory {
  Connection,
  Access,
  Limit,
  Request,
  Service,
  Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
  Warning,
  Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoticeBlock {
  pub id: String,
  pub revision: Num,
  pub category: FailureCategory,
  pub severity: Severity,
  pub title: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<String>,
  #[serde(default)]
  pub actions: Vec<FailureAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsyncTaskState {
  Running,
  Paused,
  Completed,
  Failed,
  Stopped,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUsage {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub total_tokens: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tool_uses: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub duration_ms: Option<Num>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsyncTaskInfo {
  pub id: String,
  pub state: AsyncTaskState,
  pub can_stop: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub task_type: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub summary: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub last_tool_name: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub output_file_path: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usage: Option<TaskUsage>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub stop_requested: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Observation {
  Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
  pub path: String,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_u64")]
  pub line: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadRange {
  pub path: String,
  #[serde(deserialize_with = "crate::num::lenient_u64")]
  pub start: u64,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_u64")]
  pub end: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStat {
  #[serde(deserialize_with = "crate::num::lenient_u64")]
  pub add: u64,
  #[serde(deserialize_with = "crate::num::lenient_u64")]
  pub del: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallBlock {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub observation: Option<Observation>,
  pub id: String,
  pub kind: ToolKind,
  pub verb: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub verb_key: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub target: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub target_mono: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub locations: Option<Vec<Location>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub read_range: Option<ReadRange>,
  pub status: ToolStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub background: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub async_task: Option<AsyncTaskInfo>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub started_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub ended_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub meta: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub diff_stat: Option<DiffStat>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub todo_entries: Option<Vec<PlanEntry>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub content: Option<ToolContent>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub contents: Option<Vec<ToolContent>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub subagent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThoughtBlock {
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub started_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub duration_sec: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub streaming: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
  Pending,
  InProgress,
  Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanPriority {
  High,
  Medium,
  Low,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEntry {
  pub title: String,
  pub status: PlanStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub priority: Option<PlanPriority>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanBlock {
  pub entries: Vec<PlanEntry>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextPhase {
  Commentary,
  Final,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextBlock {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub phase: Option<TextPhase>,
  pub markdown: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub streaming: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
  AllowOnce,
  AllowAlways,
  RejectOnce,
  RejectAlways,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionOption {
  pub id: String,
  pub label: String,
  pub kind: PermissionKind,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionBlock {
  pub id: String,
  pub title: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub plan_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub default_to_no: Option<bool>,
  pub options: Vec<PermissionOption>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDocStatus {
  Draft,
  Ready,
  Approved,
  Rejected,
  Executing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDocumentBlock {
  pub id: String,
  pub title: String,
  pub markdown: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<String>,
  pub tool_call_id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub approval_tool_call_id: Option<String>,
  pub status: PlanDocStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStatus {
  InProgress,
  Completed,
  Failed,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionBlock {
  pub id: String,
  pub status: CompactionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
  pub id: String,
  pub label: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
  Single,
  Multiple,
  Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  pub text: String,
  pub kind: QuestionKind,
  #[serde(default)]
  pub options: Vec<QuestionOption>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub other: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub numeric: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub required: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum QuestionAnswer {
  One(String),
  Many(Vec<String>),
}

/// Answers keyed by question id, in the order the card sent them (serde_json's preserve_order map)
pub type QuestionAnswers = Map<String, Value>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionOutcome {
  Answered,
  Skipped,
  Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuestionBlock {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tool_call_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub message: Option<String>,
  pub questions: Vec<Question>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub outcome: Option<QuestionOutcome>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub answers: Option<QuestionAnswers>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageBlock {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub blob: Option<String>,
  pub mime_type: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub uri: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentBlock {
  Thought(ThoughtBlock),
  Plan(PlanBlock),
  ToolCall(ToolCallBlock),
  Text(TextBlock),
  Permission(PermissionBlock),
  Compaction(CompactionBlock),
  PlanDocument(PlanDocumentBlock),
  Question(QuestionBlock),
  Image(ImageBlock),
  Notice(NoticeBlock),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Draft {
  #[serde(rename_all = "camelCase")]
  Image {
    mime_type: String,
    data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
  },
  Text {
    name: String,
    text: String,
  },
  File {
    uri: String,
    name: String,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attachment {
  #[serde(rename_all = "camelCase")]
  Image {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
  },
  Text {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    name: String,
  },
  File {
    uri: String,
    name: String,
  },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnSettings {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode_id: Option<String>,
  #[serde(default)]
  pub config: StrMap,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserTurn {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id: Option<String>,
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attachments: Option<Vec<Attachment>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub settings: Option<TurnSettings>,
  #[serde(default, skip_serializing_if = "is_false")]
  pub edited: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub plan_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auto: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStop {
  EndTurn,
  MaxTokens,
  MaxTurnRequests,
  Refusal,
  Cancelled,
  Error,
}

impl TurnStop {
  pub fn parse(s: &str) -> Option<TurnStop> {
    serde_json::from_value(Value::String(s.to_owned())).ok()
  }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnError {
  pub message: String,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub code: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub kind: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retryable: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub failure_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub actions: Option<Vec<FailureAction>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContextUse {
  pub used: Num,
  pub size: Num,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnUsage {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub input: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub output: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cached_read: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cached_write: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub reasoning: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub total: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model_calls: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub request_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub context: Option<ContextUse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
  pub kind: ToolKind,
  pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandOption {
  pub name: String,
  pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandReceipt {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mode: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub options: Option<Vec<CommandOption>>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTurn {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub observation: Option<Observation>,
  #[serde(default)]
  pub blocks: Vec<AgentBlock>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usage: Option<TurnUsage>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub started_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub ended_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub activity: Option<Activity>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub stop: Option<TurnStop>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<TurnError>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<CommandReceipt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum Turn {
  User(UserTurn),
  Agent(AgentTurn),
}

impl Turn {
  pub fn as_agent(&self) -> Option<&AgentTurn> {
    match self {
      Turn::Agent(a) => Some(a),
      Turn::User(_) => None,
    }
  }
  pub fn as_agent_mut(&mut self) -> Option<&mut AgentTurn> {
    match self {
      Turn::Agent(a) => Some(a),
      Turn::User(_) => None,
    }
  }
  pub fn as_user(&self) -> Option<&UserTurn> {
    match self {
      Turn::User(u) => Some(u),
      Turn::Agent(_) => None,
    }
  }
  pub fn is_agent(&self) -> bool {
    matches!(self, Turn::Agent(_))
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryState {
  Working,
  Waiting,
  Unread,
  Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub external: Option<bool>,
  #[serde(default)]
  pub title: String,
  #[serde(default)]
  pub agent: AgentId,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub account_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub acp_session_id: Option<String>,
  #[serde(default)]
  pub cwd: String,
  #[serde(default)]
  pub updated_at: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pinned: Option<bool>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub state: Option<SummaryState>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeSessionInfo {
  pub session_id: String,
  pub cwd: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub title: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub updated_at: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub local_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Usage {
  pub used: Num,
  pub size: Num,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cost: Option<Num>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedPrompt {
  pub id: String,
  pub text: String,
  pub attachments: Vec<Attachment>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sending: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalState {
  Unbound,
  Receiving,
  Idle,
  Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalSessionInfo {
  pub source: String,
  pub source_key: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub connection_prompt: Option<String>,
  pub state: ExternalState,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub active_turn_id: Option<String>,
  pub last_event_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub external: Option<ExternalSessionInfo>,
  pub agent: AgentId,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub account_id: Option<String>,
  pub title: String,
  pub cwd: String,
  pub status: SessionStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auth_methods: Option<Vec<AuthMethodInfo>>,
  pub turns: Vec<Turn>,
  pub running: bool,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub rev: Option<i64>,
  pub controls: SessionControls,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model_shapes: Option<ModelShapes>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub usage: Option<Usage>,
  pub commands: Vec<SlashCommand>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub queued: Option<Vec<QueuedPrompt>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub subagents: Option<Vec<SubagentSummary>>,
  pub created_at: String,
  pub updated_at: String,
}
