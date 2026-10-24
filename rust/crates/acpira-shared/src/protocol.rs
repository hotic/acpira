//! Host ↔ webview message contract (mirror of packages/shared/protocol.ts); both sides trust only this

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::appearance::Appearance;
use crate::chatgpt_integration::ChatGptIntegrationStatus;
use crate::i18n::Locale;
use crate::inventory::AgentInventory;
use crate::model_shapes::ModelShapes;
use crate::providers::{ProbeOutcome, ProviderAction, ProviderProbe, ProvidersView};
use crate::remote::{ConnectionOp, ConnectionView, RemoteDirEntry, SshHost};
use crate::shared_config::{SharedAction, SharedView};
use crate::storage::{StorageAction, StorageDetail, StorageOutcome, StorageView};
use crate::settings::{HiddenMap, SettingsView};
use crate::transcript::{
  AccountInfo, AgentId, AgentInfo, CategoryOp, ConfigControl, DiffSource, Draft, NativeSessionInfo, QuestionAnswers,
  SessionCategories, SessionSummary, Turn, TurnSettings,
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
  #[serde(skip_serializing_if = "Option::is_none")]
  pub agent_installs: Option<Vec<AgentInstallProgress>>,
  pub hidden: HiddenMap,
  pub sessions: Vec<SessionSummary>,
  /// User categories of the session list
  pub categories: SessionCategories,
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

/// One entry of a folder in a session's project, for the desktop tool panel's tree (`projectDir`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
  pub name: String,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub dir: bool,
}

/// A changed stretch of a file as it is now: `add` lines from line `at` (1-based) are new, the `del` lines stood just
/// before line `at` (one past the last line for lines removed at the end)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectHunk {
  pub at: u32,
  pub add: u32,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub del: Vec<String>,
}

/// A file a session changed, compared with what it held before the session's first edit: `path` relative to the
/// project folder (`/`-separated), absolute for a file outside it; `gone` when it no longer exists
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectChange {
  pub path: String,
  pub add: u32,
  pub del: u32,
  #[serde(default, skip_serializing_if = "std::ops::Not::not")]
  pub gone: bool,
}

/// What a file's hunks are measured against: the text before the session's first edit of it, or the last commit
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ProjectBase {
  Session,
  Head,
}

/// A session whose conversation contains every term of a history search, with the text around the first match
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionHit {
  pub id: String,
  pub snippet: String,
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

/// What an `AccountAction` is doing: adding an account, or unlocking the credential store that holds them
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountActionVia {
  Import,
  Login,
  Auto,
  Unlock,
}

impl From<AddAccountVia> for AccountActionVia {
  fn from(v: AddAccountVia) -> Self {
    match v {
      AddAccountVia::Import => AccountActionVia::Import,
      AddAccountVia::Login => AccountActionVia::Login,
      AddAccountVia::Auto => AccountActionVia::Auto,
    }
  }
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
  pub via: AccountActionVia,
  pub status: AccountActionStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
}

/// Where an in-app agent install stands
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentInstallStatus {
  Running,
  Success,
  Failed,
  Cancelled,
}

/// One agent's in-app install (the registry's install line run by the engine itself, no terminal). Host-owned like
/// AccountAction: it survives webview remounts and every panel shows the same run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInstallProgress {
  pub agent: AgentId,
  pub status: AgentInstallStatus,
  /// The last lines of the installer's output, ANSI sequences removed, carriage-return progress collapsed
  pub log: Vec<String>,
  /// The proxy the installer was routed through; absent when it used the inherited environment
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub proxy: Option<String>,
  /// npm's global prefix was not writable, so packages went to this user directory instead
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub prefix: Option<String>,
  /// Why it failed, already localized
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
  /// The category list changed (here or in another window)
  Categories {
    categories: SessionCategories,
  },
  /// The view shows no session: a project shell (`ShellEnv.projects`) with no project left, so the page shows its
  /// empty state instead of a session started in the home directory
  NoSession,
  /// The session view, already serialized once for every viewer that shows it
  Session {
    session: RawJson,
    /// Not on the wire: an idle edge flushes the batch at once
    #[serde(skip)]
    running: bool,
    /// Not on the wire: the same view turn by turn, from which a view that takes patches gets a `sessionPatch` instead
    #[serde(skip)]
    parts: Option<crate::session_patch::ViewParts>,
  },
  /// The changes since the view this viewer was sent last (`session_patch.rs`); only for pages that asked for patches
  SessionPatch {
    patch: RawJson,
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
  AgentInstalls {
    installs: Vec<AgentInstallProgress>,
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
  /// The Shared tab; `error` when the action that triggered this reply failed
  Shared {
    view: SharedView,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// The Storage tab; `outcome` for the action that triggered this reply, `error` when it failed
  Storage {
    view: StorageView,
    #[serde(skip_serializing_if = "Option::is_none")]
    outcome: Option<StorageOutcome>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// The built-in agent's model sources; `error` when the action that triggered this reply failed
  Providers {
    view: ProvidersView,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// The answer to `ProviderProbe` with the same id
  ProviderProbed {
    id: String,
    outcome: ProbeOutcome,
  },
  Controls {
    agent: AgentId,
    controls: Vec<ConfigControl>,
    /// The agent's remembered per-model parameters, so a model picked on the settings page shows its own effort levels
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shapes: Option<ModelShapes>,
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
  /// Reply to searchSessions; seq echoes the request so stale replies can be dropped
  SessionHits {
    seq: i64,
    hits: Vec<SessionHit>,
  },
  /// Reply to diffSource: the whole files behind one diff, absent when the session or the diff is gone
  DiffSource {
    seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<DiffSource>,
  },
  /// Reply to sessionMarkdown: the session as Markdown, or why it could not be read
  SessionMarkdown {
    seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// Reply to projectDir: a folder of the session's project, folders first; `error` (`noProject`, `outside`, `missing`,
  /// or the system's message) when it could not be listed
  ProjectDir {
    seq: i64,
    entries: Vec<ProjectEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// Reply to projectStatus: the project folder and its git marks by relative path (`M` modified, `A` added, `D` deleted,
  /// `U` untracked, `I` ignored; a folder as a whole ends in `/`), empty outside a work tree
  ProjectStatus {
    seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    root: Option<String>,
    marks: std::collections::BTreeMap<String, String>,
  },
  /// Reply to projectFile: the file's text with its hunks against the asked base, or `error` (`noProject`, `outside`,
  /// `missing`, `binary`, `tooLarge`, or the system's message)
  ProjectFile {
    seq: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    hunks: Vec<ProjectHunk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// Reply to sessionChanges: the files the session's edits left changed, in the order it first edited them
  SessionChanges {
    seq: i64,
    files: Vec<ProjectChange>,
  },
  /// SSH hosts (helloOk.features has `remote`): the list on every change
  Connections {
    connections: Vec<ConnectionView>,
  },
  /// The add dialog's hosts from ~/.ssh/config
  SshHosts {
    hosts: Vec<SshHost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// A folder of a host for the remote folder picker, echoing its request's seq
  RemoteDir {
    seq: i64,
    alias: String,
    path: String,
    entries: Vec<RemoteDirEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  /// A terminal's output (terminal.rs), to every page viewing it. Posted at once, never batched: every chunk counts
  TerminalOutput {
    id: String,
    data: String,
  },
  /// A terminal's state. Answering terminalOpen it carries the size, the title, the output so far (`replay`, which the
  /// page shows instead of what it had) and the exit if the shell ended; later only what changed. `error`: the shell
  /// could not start
  TerminalState {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cols: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    replay: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit: Option<TerminalExit>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
}

/// How a terminal's shell ended: its exit code, or the signal that ended it
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExit {
  pub code: i64,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub signal: Option<String>,
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
      HostMsg::Categories { .. } => "categories",
      HostMsg::NoSession => "noSession",
      HostMsg::Session { .. } => "session",
      HostMsg::SessionPatch { .. } => "sessionPatch",
      HostMsg::Subagent { .. } => "subagent",
      HostMsg::Accounts { .. } => "accounts",
      HostMsg::AccountActions { .. } => "accountActions",
      HostMsg::AgentInstalls { .. } => "agentInstalls",
      HostMsg::Hidden { .. } => "hidden",
      HostMsg::Settings { .. } => "settings",
      HostMsg::Inventory { .. } => "inventory",
      HostMsg::Shared { .. } => "shared",
      HostMsg::Storage { .. } => "storage",
      HostMsg::Providers { .. } => "providers",
      HostMsg::ProviderProbed { .. } => "providerProbed",
      HostMsg::Controls { .. } => "controls",
      HostMsg::Files { .. } => "files",
      HostMsg::NativeSessions { .. } => "nativeSessions",
      HostMsg::SessionHits { .. } => "sessionHits",
      HostMsg::DiffSource { .. } => "diffSource",
      HostMsg::SessionMarkdown { .. } => "sessionMarkdown",
      HostMsg::ProjectDir { .. } => "projectDir",
      HostMsg::ProjectStatus { .. } => "projectStatus",
      HostMsg::ProjectFile { .. } => "projectFile",
      HostMsg::SessionChanges { .. } => "sessionChanges",
      HostMsg::Connections { .. } => "connections",
      HostMsg::SshHosts { .. } => "sshHosts",
      // Each listing answers its own request
      HostMsg::RemoteDir { .. } => "remoteDir",
      HostMsg::TerminalOutput { .. } => "terminalOutput",
      HostMsg::TerminalState { .. } => "terminalState",
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WebviewMsg {
  ChatgptStatus,
  ConnectChatgpt,
  EditTurn {
    request_id: String,
    edit: EditTurnRequest,
  },
  /// `patches`: the page applies `sessionPatch` messages (the real webview always does; scripts and tests take whole views)
  Ready {
    #[serde(default)]
    patches: bool,
  },
  /// A `sessionPatch` did not fit the view the page holds: send the whole view again
  Resync,
  /// The page's window got focus: only the VS Code shell uses it (the "Add to Chat" target)
  ViewFocus,
  Send {
    session_id: Option<String>,
    text: String,
    #[serde(default)]
    attachments: Vec<Draft>,
    /// Sent while a turn runs with steering on: join that turn over `_session/steering` instead of waiting behind it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    steer: Option<bool>,
  },
  Stop {
    session_id: Option<String>,
  },
  SearchFiles {
    query: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  /// The whole files behind the `nth` diff of a tool call (page views carry diff sources without their texts), answered
  /// with a `diffSource` message echoing `seq`. `subagent_id` names the subagent transcript the diff was shown in; absent,
  /// the session's own turns
  DiffSource {
    session_id: String,
    tool_call_id: String,
    nth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subagent_id: Option<String>,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  /// SSH hosts: the current list (pushed on every change after), an edit of one host, the ~/.ssh/config hosts, a
  /// folder of a host (`path` absent: its home)
  Connections,
  ConnectionOp {
    op: ConnectionOp,
  },
  SshHosts,
  RemoteDir {
    alias: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  /// The desktop tool panel: one folder of the session's project (`dir` relative to it, empty for the folder itself),
  /// answered with a `projectDir` message echoing `seq`
  ProjectDir {
    session_id: String,
    #[serde(default)]
    dir: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  /// The project's git marks, answered with `projectStatus`
  ProjectStatus {
    session_id: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  /// A file of the project (`path` relative to it) with its hunks against `base` (none: no hunks), answered with `projectFile`
  ProjectFile {
    session_id: String,
    path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    base: Option<ProjectBase>,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  /// The files the session's edits left changed, answered with `sessionChanges`
  SessionChanges {
    session_id: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
  },
  /// The desktop tool panel's terminal tabs (terminal.rs). Open terminal `id`, a login shell in `cwd` sized `cols` x
  /// `rows`, or view it again if it still runs (a reload, the page's engine coming back); answered with `terminalState`
  /// carrying the output so far. Opening one already viewed starts that view over
  TerminalOpen {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cols: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rows: Option<u16>,
  },
  /// Keys typed or text pasted, as the terminal sends them; `binary`: each character is one byte (xterm's legacy mouse
  /// reports), not UTF-8
  TerminalInput {
    id: String,
    data: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    binary: bool,
  },
  TerminalResize {
    id: String,
    cols: u16,
    rows: u16,
  },
  /// How much of `terminalOutput` (UTF-16 units) the page has rendered: output pauses while too much is unrendered
  TerminalAck {
    id: String,
    chars: u64,
  },
  /// The page stops viewing the terminal; it runs on a while, for a reload to come back to
  TerminalClose {
    id: String,
  },
  /// The tab was closed, or the terminal restarts: end its shell
  TerminalKill {
    id: String,
  },
  /// Full-text search over saved session conversations for the history list
  SearchSessions {
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
    /// File the new session under this category (one of the window's project) from the start
    #[serde(default)]
    category: Option<String>,
    /// The project folder to start in (a project shell); the shell's workspace when absent
    #[serde(default)]
    cwd: Option<String>,
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
  /// File a session under a category of its own project, or take it out (`None`); filing unpins it
  SetSessionCategory {
    id: String,
    #[serde(default)]
    category: Option<String>,
  },
  CategoryOp {
    #[serde(flatten)]
    op: CategoryOp,
    /// With a `create`: file this session under the new category once it exists (one message, since a view's
    /// messages run concurrently and a separate filing could land before the category)
    #[serde(default)]
    file: Option<String>,
  },
  /// The list's "mark all as read": the engine forgets every unread dot it holds
  MarkAllRead,
  /// Re-home a session into `cwd` (absent: the window's workspace folder), filed under `category` when it is one of
  /// that project's
  MoveSession {
    id: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    category: Option<String>,
  },
  /// `agent`: continue the fork on another agent (absent: the source's own)
  ForkSession {
    session_id: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    turn_index: i64,
    #[serde(default)]
    agent: Option<String>,
  },
  ExportSession {
    id: String,
    format: ExportFormat,
  },
  /// "Copy as Markdown": the export's Markdown as text, answered with a `sessionMarkdown` message echoing `seq`
  SessionMarkdown {
    id: String,
    #[serde(deserialize_with = "crate::num::lenient_i64")]
    seq: i64,
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
  UnlockCredentials {
    agent: AgentId,
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
  /// Stop the agent's in-app install (its whole process tree)
  CancelInstall {
    agent: AgentId,
  },
  /// The same install line in an IDE terminal: the fallback when an installer needs a person at the keyboard
  InstallInTerminal {
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
  TakeOverSession {
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
  /// A goal control (SessionView.goalActions): the engine sends turn-starting ones as a `/goal …` prompt, the rest as `_session/goal`
  Goal {
    session_id: String,
    action: crate::transcript::GoalAction,
    #[serde(default)]
    objective: Option<String>,
  },
  Dequeue {
    session_id: String,
    id: String,
  },
  SendQueued {
    session_id: String,
    id: String,
  },
  SteerQueued {
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
  OpenAgentConfig,
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
  Shared,
  SharedAction {
    action: SharedAction,
  },
  /// Scan the data root (plus the details a review needs), or run a cleanup and get the view back
  Storage {
    #[serde(default)]
    detail: Vec<StorageDetail>,
  },
  StorageAction {
    action: StorageAction,
    #[serde(default)]
    detail: Vec<StorageDetail>,
  },
  /// The built-in agent's model sources: read them, or apply an edit and get the view back
  Providers,
  ProviderAction {
    action: ProviderAction,
  },
  /// A network question about a source (list models, check, test, find local servers); answered by `ProviderProbed`
  ProviderProbe {
    id: String,
    probe: ProviderProbe,
  },
  Controls {
    agent: AgentId,
    #[serde(default)]
    fresh: Option<bool>,
  },
  ListNativeSessions {
    agent: AgentId,
    /// The project folder to list for (a project shell); the view's folder when absent
    #[serde(default)]
    cwd: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
  Markdown,
  Json,
  /// Plain text: only what the user and the agent said (`export_text`)
  Text,
}

impl ExportFormat {
  pub fn extension(self) -> &'static str {
    match self {
      Self::Markdown => "md",
      Self::Json => "json",
      Self::Text => "txt",
    }
  }
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
    assert_eq!(m, WebviewMsg::Send { session_id: None, text: "hi".into(), attachments: vec![], steer: None });
    let m: WebviewMsg = serde_json::from_str(r#"{"type":"send","text":"hi","steer":true}"#).unwrap();
    assert!(matches!(m, WebviewMsg::Send { steer: Some(true), .. }));
    let m: WebviewMsg = serde_json::from_str(r#"{"type":"openFile","sessionId":"s","path":"a.ts","line":3}"#).unwrap();
    assert!(matches!(m, WebviewMsg::OpenFile { line: Some(3), .. }));
    let r: WebviewMsg = serde_json::from_str(r#"{"type":"ready"}"#).unwrap();
    assert_eq!(r, WebviewMsg::Ready { patches: false });
    let r: WebviewMsg = serde_json::from_str(r#"{"type":"ready","patches":true}"#).unwrap();
    assert_eq!(r, WebviewMsg::Ready { patches: true });
  }

  #[test]
  fn host_msg_shape() {
    let v = serde_json::to_value(HostMsg::Files { seq: 2, files: vec![] }).unwrap();
    assert_eq!(v, serde_json::json!({ "type": "files", "seq": 2, "files": [] }));
    let v = serde_json::to_value(HostMsg::EditTurnResult { request_id: "r".into(), error: None }).unwrap();
    assert_eq!(v, serde_json::json!({ "type": "editTurnResult", "requestId": "r" }));
  }
}
