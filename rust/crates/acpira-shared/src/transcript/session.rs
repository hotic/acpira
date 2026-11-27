//! Session-level shapes: summaries, models, categories, native sessions, usage, the queue and the session view

use super::*;

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
  /// Local category assignments for sessions whose records live on an SSH host (`session_id -> category_id`).
  /// Stamped onto `SessionSummary.category` when listing host sessions so categories stay per-machine.
  #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
  pub host_filed: BTreeMap<String, String>,
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
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub install: Option<crate::protocol::AgentInstallProgress>,
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
  /// Only in a view sent to a page that takes windows: what it leaves out ahead of its first turn (`session_window.rs`,
  /// which writes it into the encoded view; a session never sets it)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub earlier: Option<crate::session_window::Earlier>,
  pub running: bool,
  #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "lenient_opt_i64")]
  pub rev: Option<i64>,
  pub controls: SessionControls,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model_shapes: Option<ModelShapes>,
  /// What each option of the sidekick select runs with once picked, keyed by option id: its effort select and Fast
  /// switch as they would read (`host_sidekick.rs`)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub sidekick_shapes: Option<ModelShapes>,
  /// What the user last ran each of this agent's models with (engine prefs)
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub model_memory: Option<ModelMemory>,
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
