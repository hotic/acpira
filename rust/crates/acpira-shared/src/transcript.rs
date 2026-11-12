//! Normalized transcript shape (mirror of packages/shared/transcript.ts): the host reduces ACP session/update into these
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
  pub accounts: Option<bool>,
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

/// The whole files behind a diff. The record keeps both texts; a page is sent the path only (`omitted`), since an edit
/// carries every byte of the file twice and a long session's view ran to megabytes of them over Remote-SSH. The page
/// renders the diff from its `lines` and asks for the texts when it needs them (`diffSource`)
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffSource {
  pub path: String,
  #[serde(default)]
  pub old_text: String,
  #[serde(default)]
  pub new_text: String,
  /// Read back from a page encoding: the texts were left out and are empty here
  #[serde(default)]
  pub omitted: bool,
  /// Big texts live in the session's blob folder instead of the record (`collect_text_blobs`): these name them, and with
  /// either set a missing one is an empty text. The record also says `omitted`, so an older engine reads such a source
  /// as one without texts rather than as two empty files
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub old_blob: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub new_blob: Option<String>,
  /// The blob names the inline texts were stored under, worked out once per source
  #[serde(skip)]
  pub stored: StoredNames,
}

impl DiffSource {
  /// A source with both texts inline
  pub fn whole(path: String, old_text: String, new_text: String) -> Self {
    DiffSource { path, old_text, new_text, ..Default::default() }
  }

  /// Whether the texts are blobs to be read back (`old_blob` / `new_blob`)
  pub fn in_blobs(&self) -> bool {
    self.old_blob.is_some() || self.new_blob.is_some()
  }
}

/// A cache the source's equality ignores: two sources with the same texts are the same diff
#[derive(Debug, Clone, Default)]
pub struct StoredNames(std::sync::Arc<std::sync::OnceLock<(Option<String>, Option<String>)>>);

impl PartialEq for StoredNames {
  fn eq(&self, _: &Self) -> bool {
    true
  }
}

impl Eq for StoredNames {}

/// Where a record write puts diff texts too big to inline (`collect_text_blobs`)
pub struct TextBlobSink {
  /// Below this many bytes (both texts together) a source stays inline: a blob file costs a disk block of its own
  pub min_len: usize,
  /// The content-derived blob name for a text
  pub name: Box<dyn Fn(&str) -> String>,
  /// Whether the session's folder already holds the named blob
  pub known: Box<dyn Fn(&str) -> bool>,
  /// The blobs to write before the record: name and text
  pub out: Vec<(String, String)>,
}

thread_local! {
  static VIEW_ENCODING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
  static TEXT_SINK: std::cell::RefCell<Option<TextBlobSink>> = const { std::cell::RefCell::new(None) };
}

/// Serialize what `f` serializes the way the store writes a record: diff texts of `sink.min_len` or more become blob
/// names, and the texts not yet stored are handed back with the result. Scoped to this thread like `encode_for_view`
pub fn collect_text_blobs<R>(sink: TextBlobSink, f: impl FnOnce() -> R) -> (R, Vec<(String, String)>) {
  // Holds the sink that was there before until it is put back (also when `f` panics)
  struct Restore(Option<Option<TextBlobSink>>);
  impl Drop for Restore {
    fn drop(&mut self) {
      if let Some(prev) = self.0.take() {
        TEXT_SINK.with(|s| *s.borrow_mut() = prev);
      }
    }
  }
  let mut restore = Restore(Some(TEXT_SINK.with(|s| s.borrow_mut().replace(sink))));
  let out = f();
  let prev = restore.0.take().flatten();
  let taken = TEXT_SINK.with(|s| std::mem::replace(&mut *s.borrow_mut(), prev));
  (out, taken.map(|s| s.out).unwrap_or_default())
}

impl DiffSource {
  /// The blob names for this source's inline texts when a record write is collecting them and they are big enough
  fn stash(&self) -> Option<(Option<String>, Option<String>)> {
    TEXT_SINK.with(|s| {
      let mut guard = s.borrow_mut();
      let sink = guard.as_mut()?;
      if self.old_text.len() + self.new_text.len() < sink.min_len {
        return None;
      }
      let names = self.stored.0.get_or_init(|| {
        let name = |t: &str| (!t.is_empty()).then(|| (sink.name)(t));
        (name(&self.old_text), name(&self.new_text))
      });
      for (name, text) in [(&names.0, &self.old_text), (&names.1, &self.new_text)] {
        if let Some(n) = name
          && !(sink.known)(n)
          && !sink.out.iter().any(|(o, _)| o == n)
        {
          sink.out.push((n.clone(), text.clone()));
        }
      }
      Some(names.clone())
    })
  }
}

/// Serialize what `f` serializes the way a page gets it: diff sources without their texts. Scoped to this thread and
/// restored on return (also when `f` panics), so a record written elsewhere is never affected
pub fn encode_for_view<R>(f: impl FnOnce() -> R) -> R {
  struct Restore(bool);
  impl Drop for Restore {
    fn drop(&mut self) {
      VIEW_ENCODING.with(|v| v.set(self.0));
    }
  }
  let _restore = Restore(VIEW_ENCODING.with(|v| v.replace(true)));
  f()
}

impl Serialize for DiffSource {
  fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeStruct;
    // A page gets the path only; a record keeps blob names when the texts live in blobs; otherwise both texts inline
    let blobs = if VIEW_ENCODING.with(|v| v.get()) {
      None
    } else if self.in_blobs() {
      Some((self.old_blob.clone(), self.new_blob.clone()))
    } else if self.omitted {
      None
    } else {
      self.stash()
    };
    let omit = blobs.is_some() || self.omitted || VIEW_ENCODING.with(|v| v.get());
    let mut m = s.serialize_struct("DiffSource", 4)?;
    m.serialize_field("path", &self.path)?;
    if omit {
      m.serialize_field("omitted", &true)?;
      if let Some((old, new)) = blobs {
        if let Some(o) = old {
          m.serialize_field("oldBlob", &o)?;
        }
        if let Some(n) = new {
          m.serialize_field("newBlob", &n)?;
        }
      }
    } else {
      m.serialize_field("oldText", &self.old_text)?;
      m.serialize_field("newText", &self.new_text)?;
    }
    m.end()
  }
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

/// Where a shared text came from. It rides on the `text` kind instead of a kind of its own: engines released before it
/// ignore the unknown field and read a plain text attachment, where an unknown kind would fail the whole record
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContext {
  /// Stable id of the sharing tool (`asgard.questCanvas`, `browser`)
  pub source_id: String,
  /// Its display name, written into the prompt header
  pub source_name: String,
  /// What the text was taken from (a document, a page URL)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub uri: Option<String>,
  /// The user's remark on it
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub comment: Option<String>,
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
    /// Set when the text was shared from another tool (an extension, a picked web page) rather than dropped as a file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<TextContext>,
  },
  File {
    uri: String,
    name: String,
    /// Base64 bytes of a file the webview only holds as a blob (an OS paste / drop): staged into the session's blob dir,
    /// whose path replaces `uri`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    data: Option<String>,
  },
  /// A line range of an editor document; lines are 1-based and inclusive
  #[serde(rename_all = "camelCase")]
  Selection {
    uri: String,
    name: String,
    start_line: u32,
    end_line: u32,
    text: String,
  },
  /// Text quoted from the conversation with an optional remark
  Quote {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    context: Option<TextContext>,
  },
  File {
    uri: String,
    name: String,
  },
  /// The selected text is in the blob store like dropped text
  #[serde(rename_all = "camelCase")]
  Selection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<String>,
    uri: String,
    name: String,
    start_line: u32,
    end_line: u32,
  },
  Quote {
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    comment: Option<String>,
  },
}

impl Attachment {
  /// What a chip / export line calls it; quotes have no name of their own
  pub fn label(&self) -> Option<String> {
    match self {
      Attachment::Image { name, .. } => name.clone(),
      Attachment::Text { name, .. } | Attachment::File { name, .. } => Some(name.clone()),
      Attachment::Selection { name, start_line, end_line, .. } => Some(format!("{name} {}", line_range_label(*start_line, *end_line))),
      Attachment::Quote { .. } => None,
    }
  }
}

/// `(12-19)`, or `(12)` for one line
pub fn line_range_label(start_line: u32, end_line: u32) -> String {
  if start_line == end_line { format!("({start_line})") } else { format!("({start_line}-{end_line})") }
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
  /// Why an automatic turn was sent; absent on an automatic turn means the over-threshold /compact
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub auto_reason: Option<AutoReason>,
  /// With `AutoReason::Goal`: the goal's next turn, which the engine sent because the goal was still active when the turn
  /// before ended and the agent leaves continuing to the client (the built-in agent's `acpira/continue`); it reads as
  /// the reply before it carried on. A flag rather than an `AutoReason` value: `~/.acpira` is shared with engines released
  /// before it, which fail the whole record on an unknown value but skip an unknown field (and read a goal control)
  #[serde(default, skip_serializing_if = "is_false")]
  pub goal_continue: bool,
}

impl UserTurn {
  /// An active goal's next turn (`goal_continue`)
  pub fn is_goal_continue(&self) -> bool {
    self.auto == Some(true) && self.auto_reason == Some(AutoReason::Goal) && self.goal_continue
  }
}

/// What made Acpira send an automatic user turn
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AutoReason {
  Compact,
  /// The previous turn ran out of account quota and the session moved to another account
  AccountSwitch,
  /// The previous turn failed after doing part of the work and the user pressed Retry: the agent carries on from its own
  /// context instead of receiving the message again
  Retry,
  /// The workspace gate (`.agents/hooks.json`) blocked the previous turn: its findings go back to the agent
  Gate,
  /// A goal control from the dock went out as its `/goal …` command; the goal row stands for it. Also the goal's next
  /// turn, with `UserTurn::goal_continue`
  Goal,
  /// Summoned agents the root sent off without waiting finished: their replies go to it in one message
  Subagents,
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
  /// Milliseconds the model calls spent streaming, first event to the end of each call, when the agent measures it
  /// itself (the built-in agent's `_meta.usage.decodeMs`); the output speed then needs no estimate
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub decode_ms: Option<Num>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub request_id: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub context: Option<ContextUse>,
}

impl TurnUsage {
  /// `input` always counts the whole prompt, cache reads and writes included (OpenAI semantics, which Devin and the
  /// built-in agent report). claude-agent-acp, codex-acp and OpenCode report Anthropic semantics instead, where `input` is
  /// only the uncached remainder. The tally tells which one it is: a cache larger than `input` cannot be a part of it,
  /// and an exclusive tally adds up as `total = input + cache + output`. Folding makes `total = input + output`, so a
  /// second pass (a record saved after folding) leaves it alone
  pub fn fold_cache_into_input(&mut self) {
    let Some(input) = self.input else { return };
    let cache = self.cached_read.map_or(0.0, Num::get) + self.cached_write.map_or(0.0, Num::get);
    if cache <= 0.0 {
      return;
    }
    let output = self.output.map_or(0.0, Num::get);
    let reasoning = self.reasoning.map_or(0.0, Num::get);
    let sums_apart = self.total.is_some_and(|t| {
      let rest = t.get() - input.get() - cache - output;
      rest == 0.0 || rest == reasoning
    });
    if cache > input.get() || sums_apart {
      self.input = Some(Num(input.get() + cache));
    }
  }
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
  /// When the first streamed block of a live prompt landed; the turn's clock counts from here, so the wait for the
  /// agent's first output is not shown as working time. `started_at` stays the turn's identity
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub responded_at: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub ended_at: Option<i64>,
  /// Milliseconds the live turn sat with only background work open and the model silent past a grace period
  /// (`normalize/idle.rs`): waiting on a background task, not generating. The output speed leaves it out
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub idle_ms: Option<i64>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub activity: Option<Activity>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub stop: Option<TurnStop>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<TurnError>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub command: Option<CommandReceipt>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub retry: Option<TurnRetry>,
  /// Plugins an agent of this turn offered (`plugin_activate`), shown after the reply and clickable for good
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub plugin_cards: Vec<PluginCard>,
}

/// A plugin card (`docs/dev/plugins.md`, Activation): the frozen version an agent asked to enable, as the user sees it
/// in the transcript. Its state (offered, in effect, replaced) is read from the plugins' state, not kept here; only what
/// applying it from this card changed is, so the card can put it back
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCard {
  /// Unique within the session
  pub id: String,
  pub plugin: String,
  /// The frozen version it offers
  pub snapshot: String,
  pub name: String,
  #[serde(default)]
  pub description: String,
  /// `skin` (a theme), `ui` (interface pieces), `agent` (a specialised agent), `exec` (runs a program), `other`
  pub kind: String,
  /// What it runs, for `exec`: the first command, as the card names it
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub runs: Option<String>,
  /// The slots its interface pieces take, for the thumbnail's map
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub slots: Vec<String>,
  /// The cron expressions of its schedules: it runs unattended, and may act on sessions while nobody watches
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub schedules: Vec<String>,
  /// What it contributes (`Contributes::kinds`: `tools`, `hooks`, `agent` …): where the card's mark flies once applied
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub adds: Vec<String>,
  /// The `STYLE_HOOKS` its stylesheet names, in the order they first appear: a style lands on those parts of the page
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub restyles: Vec<String>,
  /// The source folder, which the corner button reveals
  #[serde(default)]
  pub folder: String,
  #[serde(default, deserialize_with = "crate::num::lenient_i64")]
  pub at: i64,
  /// Set when it was applied from this card: what was in effect before
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub undo: Option<PluginCardUndo>,
  /// The tool call that offered it (the agent's `plugin_activate`): the page shows the card right after that row, where
  /// the agent asked, instead of under everything the turn streams later. Absent on older records: after the reply
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub after: Option<String>,
}

/// What a plugin card's apply replaced: the plugin's own entry and the skin in effect
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginCardUndo {
  /// The version enabled before, none when it was off
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub snapshot: Option<String>,
  /// The skin before (the plugin id), empty for none; absent when the card is not a skin
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub skin: Option<String>,
}

/// A provider retry in progress. It lives in the turn's working label, never as a transcript row, and is cleared as
/// soon as the stream resumes or the turn ends
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnRetry {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub attempt: Option<u32>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub max: Option<u32>,
  /// The adapter's own wording (or the host's for prose retries), for the label's tooltip
  pub detail: String,
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
  /// The turn is over but a background task the agent started (background Bash, Monitor, a workflow) still runs; the
  /// agent may wake and report on its own, so the session is neither finished nor busy
  Background,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
  pub id: String,
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
  /// When the last turn ended finished (`TurnStop::EndTurn`), epoch ms (`completed_at`); absent while the last turn is
  /// anything else. A rename, a pin or a read leaves it alone, unlike `updated_at`, so a style can age a finished session
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub completed_at: Option<i64>,
  /// When the session was started, for the list's "created" order; absent on index entries written before the field
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub created_at: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub pinned: Option<bool>,
  /// Archived: left out of the list until its archive filter is on; kept like any other session
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub archived: Option<bool>,
  /// The user category (`SessionCategory.id`) the session is filed under; pinned and filed are exclusive. An id no
  /// category carries any more (deleted in some window) reads as unfiled
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub category: Option<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub state: Option<SummaryState>,
  /// Only beside `Background`: a turn ended there that nobody has looked at yet (`Unread` itself would read as finished)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub unread: Option<bool>,
  /// Only beside `Waiting`: only questions wait; a permission card waiting too wins, as it holds up a tool
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub question: Option<bool>,
  /// Only beside `Background`: when Claude's pending `ScheduleWakeup` resumes the session (ms since the epoch)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub wake_at: Option<i64>,
  /// The model the session last ran (`model_shapes::session_model`); absent while no model control is known and on index
  /// entries written before the field existed
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model: Option<SessionModel>,
}

/// `SessionSummary::completed_at` of a session's turns: the end of the last one when it finished
pub fn completed_at<'a>(mut turns: impl DoubleEndedIterator<Item = &'a Turn>) -> Option<i64> {
  match turns.next_back()? {
    Turn::Agent(a) if a.stop == Some(TurnStop::EndTurn) => a.ended_at,
    _ => None,
  }
}

/// A session's current model for the list: the wire id is its real identity (the vendor is read from it, "kimi-code/k3"),
/// the name only its label ("K3")
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionModel {
  pub id: String,
  pub name: String,
}

/// A user-made group of sessions inside one project (`cwd`), shown between the project and its sessions
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCategory {
  pub id: String,
  pub name: String,
  pub cwd: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub collapsed: Option<bool>,
}

/// A user-made top-level section of a project shell's list ("Work", "Personal") at the level of Pinned / Projects: whole
/// projects, each in at most one section; projects in none stay under Projects
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSection {
  pub id: String,
  pub name: String,
  #[serde(default)]
  pub projects: Vec<String>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub collapsed: Option<bool>,
}

/// Every category in list order, plus the projects folded in the "all projects" list. Shared by every window
/// (`sessions/categories.json`)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCategories {
  #[serde(default)]
  pub categories: Vec<SessionCategory>,
  #[serde(default)]
  pub collapsed_projects: Vec<String>,
  /// Project folders in the order a project shell (the desktop app) lists them, including folders without a session
  /// yet; absent until a project shell first connects. IDE session trees do not read it
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub projects: Option<Vec<String>>,
  /// Folders a project shell took off its list. Their sessions stay and a search still finds them, but the list no longer
  /// shows the folder for them until it is added again (a session started there adds it)
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub removed_projects: Vec<String>,
  /// Project shells' sections in list order
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub sections: Vec<SessionSection>,
  /// What a project holds beyond its folder, keyed by that folder (the primary one, `projects`' entry): a name of its
  /// own and further folders, sent to agents as ACP `additionalDirectories`. A project without either has no entry
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub project_meta: BTreeMap<String, ProjectMeta>,
}

/// A project's own name and its folders besides the primary one (absolute, in the order the user listed them)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectMeta {
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub name: Option<String>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub extra: Vec<String>,
}

impl SessionCategories {
  /// The project a folder belongs to: the project whose primary folder it is, or the one listing it as a further folder;
  /// else the folder stands for itself
  pub fn project_of<'a>(&'a self, folder: &'a str) -> &'a str {
    if self.project_meta.contains_key(folder) {
      return folder;
    }
    self.project_meta.iter().find(|(_, m)| m.extra.iter().any(|f| f == folder)).map_or(folder, |(k, _)| k.as_str())
  }

  /// Every folder of the project `folder` belongs to except `folder` itself: what a session running there gets as
  /// `additionalDirectories`, the primary folder first
  pub fn other_folders(&self, folder: &str) -> Vec<String> {
    let key = self.project_of(folder);
    let Some(meta) = self.project_meta.get(key) else { return vec![] };
    std::iter::once(key).chain(meta.extra.iter().map(String::as_str)).filter(|f| *f != folder).map(str::to_owned).collect()
  }
}

/// One edit of the category list, applied under the file lock to what is on disk so concurrent windows compose
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum CategoryOp {
  /// The page picks the id, so it can file a session or start renaming the new row right away
  Create { id: String, name: String, cwd: String },
  Rename { id: String, name: String },
  Delete { id: String },
  Collapse { id: String, collapsed: bool },
  /// Move `id` before `before` (same project), or to the end of its project's categories
  Reorder {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before: Option<String>,
  },
  CollapseProject { cwd: String, collapsed: bool },
  /// Register a project folder at the end of `projects` (a no-op when listed), undoing an earlier removal
  AddProject { cwd: String },
  /// Take a folder off the list: off `projects`, onto `removed_projects`. Its sessions stay
  RemoveProject { cwd: String },
  /// Edit a project (`cwd`, its primary folder): its name (blank: the folder's), its primary folder (`primary`, which
  /// takes over the project's place, section and categories) and its further folders. A folder that was a project of
  /// its own, or another project's further folder, joins this one
  EditProject {
    cwd: String,
    primary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default)]
    extra: Vec<String>,
  },
  /// Move a registered folder before `before`, or to the end
  ReorderProject {
    cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before: Option<String>,
  },
  /// The first order a project shell sees: the folders given, unless `projects` already exists
  SeedProjects { cwds: Vec<String> },
  /// Sections (project shells). The page picks the id, so it can rename the new section at once; `project` files that
  /// folder into it in the same edit (a separate `SetProjectSection` could land first and find no section)
  CreateSection {
    id: String,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project: Option<String>,
  },
  RenameSection { id: String, name: String },
  /// Its projects go back under Projects
  DeleteSection { id: String },
  CollapseSection { id: String, collapsed: bool },
  /// Move a section before `before`, or to the end
  ReorderSection {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before: Option<String>,
  },
  /// File a project folder in `section` before `before` (absent or not in that section: at its end), out of any other;
  /// no section takes it out
  SetProjectSection {
    cwd: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    section: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    before: Option<String>,
  },
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
#[serde(rename_all = "camelCase")]
pub struct Usage {
  pub used: Num,
  pub size: Num,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub cost: Option<Num>,
  /// Set when a compaction finished and the agent has not reported its context since (Devin): `used` is still the
  /// pre-compaction value, kept here; the next snapshot replaces the whole struct and clears it
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub compacted_from: Option<Num>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedPrompt {
  pub id: String,
  pub text: String,
  pub attachments: Vec<Attachment>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sending: Option<bool>,
}

/// Public account metadata of the SSH engine owning a session; credentials stay on that host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteAuth {
  pub agent: AgentInfo,
  /// The host's CLI availability for new sessions in this folder; absent on older peers.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agents: Option<Vec<AgentInfo>>,
  pub accounts: Vec<AccountInfo>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub action: Option<crate::protocol::AccountAction>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
  pub id: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  // Keep the large agent metadata out of every local session view's inline layout.
  pub remote_auth: Option<Box<RemoteAuth>>,
  pub agent: AgentId,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub account_id: Option<String>,
  pub title: String,
  pub cwd: String,
  pub status: SessionStatus,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
  /// The error is a native session lock held by an agent another Acpira sidecar left running (takeOverSession ends it), or
  /// this is a read-only copy of a session another Acpira engine has open (takeOverSession asks it to hand the session over)
  #[serde(default, skip_serializing_if = "is_false")]
  pub can_take_over: bool,
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
  /// The agent takes `_session/steering` and honours its idle fallback: a queued prompt can join the running turn
  #[serde(default, skip_serializing_if = "is_false")]
  pub can_steer: bool,
  /// A running command row can be stopped on its own (`stopCommand`): its process under the agent's on macOS / Linux,
  /// or the remote call in an SSH workspace
  #[serde(default, skip_serializing_if = "is_false")]
  pub can_stop_commands: bool,
  /// The agent's current goal (AIR goal extension); a completed one stays only until its turn ends
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub goal: Option<SessionGoal>,
  /// The goal controls the running agent advertises; absent when it has no goal support (or no process yet)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub goal_actions: Option<Vec<GoalAction>>,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub subagents: Option<Vec<SubagentSummary>>,
  /// A fork whose copied turns are still in its transcript: where they came from (the boundary drawn under them)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub forked_from: Option<ForkOrigin>,
  pub created_at: String,
  pub updated_at: String,
}

/// A fork's origin as the view shows it: the source session, the last copied turn's index, and the source's agent when
/// the fork moved to another one
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkOrigin {
  pub session_id: String,
  #[serde(deserialize_with = "crate::num::lenient_i64")]
  pub turn_index: i64,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub agent: Option<AgentId>,
}

#[cfg(test)]
mod diff_source_tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn a_page_encoding_leaves_the_file_texts_out_and_a_record_keeps_them() {
    let src = DiffSource::whole("/a.ts".into(), "old".into(), "new".into());
    let diff = ToolContent::Diff { lines: vec![], source: Some(src) };
    assert_eq!(serde_json::to_value(&diff).unwrap()["source"], json!({ "path": "/a.ts", "oldText": "old", "newText": "new" }));
    let page = encode_for_view(|| serde_json::to_value(&diff).unwrap());
    assert_eq!(page["source"], json!({ "path": "/a.ts", "omitted": true }));
    // the scope ends with the call: the next record write is whole again
    assert_eq!(serde_json::to_value(&diff).unwrap()["source"]["newText"], "new");
    // read back, an omitted source stays omitted rather than passing for two empty files
    let back: ToolContent = serde_json::from_value(page).unwrap();
    let ToolContent::Diff { source: Some(s), .. } = back else { panic!("diff expected") };
    assert!(s.omitted && s.old_text.is_empty());
    assert_eq!(serde_json::to_value(&s).unwrap(), json!({ "path": "/a.ts", "omitted": true }));
  }

  fn sink(known: &'static [&'static str]) -> TextBlobSink {
    TextBlobSink { min_len: 8, name: Box::new(|t: &str| format!("{}.zst", t.len())), known: Box::new(move |n: &str| known.contains(&n)), out: vec![] }
  }

  #[test]
  fn a_record_write_moves_big_texts_into_blobs_and_older_readers_see_them_omitted() {
    let big = DiffSource::whole("/a.ts".into(), String::new(), "0123456789".into());
    let small = DiffSource::whole("/b.ts".into(), "a".into(), "b".into());
    let diff = |s: &DiffSource| ToolContent::Diff { lines: vec![], source: Some(s.clone()) };
    let (v, out) = collect_text_blobs(sink(&[]), || serde_json::to_value([diff(&big), diff(&small)]).unwrap());
    // An empty text needs no blob: the other name alone says the texts are blobs
    assert_eq!(v[0]["source"], json!({ "path": "/a.ts", "omitted": true, "newBlob": "10.zst" }));
    assert_eq!(v[1]["source"], json!({ "path": "/b.ts", "oldText": "a", "newText": "b" }));
    assert_eq!(out, vec![("10.zst".to_owned(), "0123456789".to_owned())]);
    // A blob the folder already holds is not handed back again, and outside a record write the texts stay inline
    let (_, again) = collect_text_blobs(sink(&["10.zst"]), || serde_json::to_value(diff(&big)).unwrap());
    assert!(again.is_empty());
    assert_eq!(serde_json::to_value(&big).unwrap()["newText"], "0123456789");
    // Read back by this engine the names survive a rewrite; a page still gets the path only
    let back: DiffSource = serde_json::from_value(v[0]["source"].clone()).unwrap();
    assert!(back.in_blobs() && back.omitted && back.new_text.is_empty());
    assert_eq!(serde_json::to_value(&back).unwrap(), v[0]["source"]);
    assert_eq!(encode_for_view(|| serde_json::to_value(&back).unwrap()), json!({ "path": "/a.ts", "omitted": true }));
    // The cached names are no part of equality
    assert_eq!(big, DiffSource::whole("/a.ts".into(), String::new(), "0123456789".into()));
  }
}
