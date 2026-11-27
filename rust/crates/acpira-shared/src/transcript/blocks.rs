//! The blocks of an agent turn: tool calls and their content, thoughts, plans, text, permissions, questions, notices, goals

use super::*;

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
  /// An ACP `info` notice: stored as a warning plus this flag, because engines up to 1.10.6 know only warning / error and
  /// would refuse the whole record over an unknown severity (`~/.acpira` is shared with older IDE sidecars)
  #[serde(default, skip_serializing_if = "is_false")]
  pub info: bool,
  pub title: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub details: Option<String>,
  #[serde(default)]
  pub actions: Vec<FailureAction>,
  /// Set on the row that logs one run of provider retries. An optional field on an existing block rather than a block
  /// of its own: older engines skip the field and read a plain warning notice, where an unknown block type would fail
  /// the whole record
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retry: Option<RetryLog>,
}

/// One run of provider retries inside a turn: every failed attempt with the provider's own error text, open until the
/// stream resumes or the turn ends
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryLog {
  /// `in_progress` while retrying; `completed` once the stream resumed, `failed` when the turn errored, `cancelled`
  /// when it was stopped
  pub status: ToolStatus,
  /// The attempt budget the agent announced
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max: Option<u32>,
  /// When the next attempt starts (epoch ms), while one is scheduled
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub next_at: Option<Num>,
  /// The last attempt lost the connection rather than getting an error answer
  #[serde(default, skip_serializing_if = "is_false")]
  pub reconnect: bool,
  pub attempts: Vec<RetryAttempt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryAttempt {
  /// When the failure was reported (epoch ms)
  pub at: Num,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  /// The error as the provider or the CLI worded it, verbatim
  pub text: String,
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
  /// The MCP server a tool call went to, as a label (`Blockbench`): the row reads "Call Blockbench" with the tool's
  /// title as its target, whichever adapter naming the call came in (`normalize/tool_kind.rs`, `mcp_identity`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub mcp_server: Option<String>,
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
#[serde(rename_all = "camelCase")]
pub struct TextBlock {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub phase: Option<TextPhase>,
  pub markdown: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub streaming: Option<bool>,
  /// Local image files the markdown embeds (`![](/abs/shot.png)`), read into the blob store; `uri` is the source as written
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub images: Option<Vec<ImageRef>>,
  /// When the stretch this final reply closed ended (epoch ms), stamped before a detached continuation clears
  /// `AgentTurn.ended_at` (`mark_reply_final`)
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub ended_at: Option<i64>,
  /// When the stretch that followed this final reply first produced output (epoch ms)
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub resumed_at: Option<i64>,
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
  /// When the stretch ahead of this plan card finished (epoch ms)
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub ended_at: Option<i64>,
  /// When the continuation stretch after this plan card first produced output (epoch ms)
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub resumed_at: Option<i64>,
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
  /// The agent's reason for a failed compaction
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
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
  /// The agent takes images with the answers (the built-in agent's `_meta["acpira/answerImages"]`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub images: Option<bool>,
  /// Images sent with the answers, by question id; the bytes are in the session's blob store
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub answer_images: Option<BTreeMap<String, Vec<ImageRef>>>,
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

/// A queued prompt steered into the running turn over `_session/steering`: it joined the agent loop at this point
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SteerBlock {
  pub id: String,
  pub text: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attachments: Option<Vec<Attachment>>,
}

/// A goal's state as the agent reports it (the AIR goal snapshot). Codex sends every status; Claude only `active`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
  Active,
  Paused,
  Blocked,
  /// Codex's `usageLimited` / `budgetLimited`
  Limited,
  Complete,
}

/// The session's current goal: the agent keeps working toward `objective` across turns until it judges it met.
/// Counters are whatever the agent reports: Claude its stop-hook rounds, Codex time and tokens against a budget
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionGoal {
  pub objective: String,
  pub status: GoalStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub iterations: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub token_budget: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tokens_used: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub time_used_seconds: Option<Num>,
  /// Epoch ms of the agent's last goal change: `timeUsedSeconds` counts up to here
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub updated_at: Option<Num>,
}

/// A goal control the agent advertises at initialize. `set` is also what `/goal <objective>` does
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalAction {
  Set,
  Pause,
  Resume,
  Clear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalEvent {
  Set,
  Paused,
  Resumed,
  Blocked,
  Limited,
  Complete,
  Cleared,
}

/// A goal milestone in the transcript: the dock strip shows the present, these rows keep the history.
/// The counters are the snapshot's at that moment
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalBlock {
  pub event: GoalEvent,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub objective: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub iterations: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub tokens_used: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub time_used_seconds: Option<Num>,
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
  Steer(SteerBlock),
  Goal(GoalBlock),
}
