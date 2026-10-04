//! Shared config shown on the settings page's Shared tab (mirror of src/shared/sharedConfig.ts).
//! The content lives in open files (`~/.agents`, `<project>/.agents`, `AGENTS.md`); these types only describe it and
//! how far each agent can see it

use serde::{Deserialize, Serialize};

use crate::inventory::McpTransport;
use crate::transcript::AgentId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedScope {
  Project,
  Global,
}

/// How one agent reaches a shared resource
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachState {
  /// The agent reads the shared location itself
  Native,
  /// Through a link (or an import line) Acpira can see in place
  Linked,
  /// Nothing there yet, or an identical copy: "link all" fixes it without a decision
  Missing,
  /// Different content of the agent's own sits where the link would go
  Conflict,
  /// The agent has no way to take it (Pi and MCP)
  Unsupported,
  /// The agent reads it only in a project it trusts (Pi and project `.agents/skills`)
  Untrusted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reach {
  pub agent: AgentId,
  pub state: ReachState,
  /// The agent-side file or link, when there is one
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedSkill {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  /// The skill directory
  pub path: String,
  pub scope: SharedScope,
  pub reach: Vec<Reach>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedMcp {
  pub name: String,
  pub scope: SharedScope,
  pub transport: McpTransport,
  /// Command line or URL, for display
  pub target: String,
  pub enabled: bool,
  /// A project server of the same name replaces this global one
  pub shadowed: bool,
  /// Agents whose advertised transports exclude this one (agents without client MCP at all are in `SharedView.no_mcp`)
  pub unsupported: Vec<AgentId>,
  /// Agents whose own config already declares this name; it is not sent to them twice
  pub native: Vec<AgentId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedPrompt {
  pub scope: SharedScope,
  pub path: String,
  pub exists: bool,
  /// The first lines, for the card
  pub preview: String,
  /// The whole file as it is on disk (empty when missing), for the page's plain-text editor
  pub text: String,
  pub reach: Vec<Reach>,
}

/// An agent's own global instruction file with lines `~/.agents/AGENTS.md` does not have, offered for merging before
/// overwrite replaces the file with a link to the shared prompt
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Takeover {
  pub agent: AgentId,
  /// The agent's file; the key of `SharedAction::Overwrite.merge`
  pub path: String,
  /// The lines only this file has, in file order (runs separated by a blank line)
  pub unique: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivateMatch {
  /// No shared skill has this name
  Unique,
  /// A shared skill of this name has the same files
  Same,
  /// A shared skill of this name differs
  Differs,
}

/// A skill in one agent's own directory rather than in `.agents/skills`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivateSkill {
  pub name: String,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub description: Option<String>,
  pub path: String,
  pub agent: AgentId,
  pub scope: SharedScope,
  pub matches: PrivateMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
  Skill,
  Prompt,
}

/// One user-level link point that is not in place yet, as the link panel lists it
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
  /// Where the link / import goes; the key of a `Pick`
  pub at: String,
  pub agent: AgentId,
  pub kind: PlanKind,
  /// The skill name, or the file name of the agent's instruction file
  pub name: String,
  /// Missing (nothing there, or an identical copy) or Conflict (content of the agent's own)
  pub state: ReachState,
  /// Left out on purpose last time; kept out of automatic linking until picked again
  pub skipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedView {
  /// The project root (nearest git root above the workspace); absent without a workspace
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub root: Option<String>,
  pub home: String,
  /// User level: new shared skills (and newly installed agents) are linked on their own
  pub auto: bool,
  /// Something is linked at user level (the undo button has work to do)
  pub user_linked: bool,
  /// Project level: Claude's links to `.agents/skills` are made on their own (default on)
  pub project_auto: bool,
  /// This project's links are left out of `info/exclude`, so they can be committed for the team
  pub project_shared: bool,
  /// This project has links Acpira made
  pub project_linked: bool,
  /// Pi is installed, the project has `.agents/skills`, and Pi does not trust the project (so it skips those skills)
  pub pi_untrusted: bool,
  /// Whether `~/.agents/AGENTS.md` exists
  pub shared_prompt: bool,
  /// User level is overwritten: every agent's global instruction file and skill link point is kept wired to
  /// `~/.agents`, skipped points and conflicts included (what stood there is backed up and restored when turned off)
  pub overwrite: bool,
  /// Before overwrite is on: the agents' own global prompts with content of their own
  pub takeover: Vec<Takeover>,
  pub skills: Vec<SharedSkill>,
  pub mcp: Vec<SharedMcp>,
  /// Installed agents that take no client MCP servers at all (Pi)
  pub no_mcp: Vec<AgentId>,
  pub prompts: Vec<SharedPrompt>,
  pub private_skills: Vec<PrivateSkill>,
  /// User-level link points still to decide on, for the link panel
  pub plan: Vec<PlanItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
  Shared,
  Private,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedTarget {
  Skills,
  Mcp,
  Prompt,
}

/// What the link panel decided for one plan item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
  /// Missing: make the link
  Link,
  /// Conflict: the shared version wins, the agent's own is backed up
  KeepShared,
  /// Conflict: the agent's own version becomes the shared one (the previous shared one is backed up)
  KeepPrivate,
  /// Leave it as it is and out of automatic linking
  Skip,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pick {
  pub at: String,
  pub choice: Choice,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SharedAction {
  /// The link panel's decisions for user level; `auto` keeps new skills linked from now on
  Link { picks: Vec<Pick>, auto: bool },
  /// Remove every user-level link Acpira made and put back what it moved aside
  Unlink,
  /// User-level overwrite on: the unique lines of the `merge` files are appended to `~/.agents/AGENTS.md` (created
  /// when missing), then every user-level link point is wired, whatever stood there backed up. Off: what overwrite
  /// wired is undone and the backups go back in place
  Overwrite {
    on: bool,
    #[serde(default)]
    merge: Vec<String>,
  },
  /// Write a prompt file (`~/.agents/AGENTS.md` or the project's `AGENTS.md`) verbatim, in place, so links and hard
  /// links to it keep working; refused when the file no longer holds `base`, the text the edit started from
  SavePrompt { scope: SharedScope, text: String, base: String },
  /// Project level: make Claude's links on their own (on), or remove every project link and stop (off)
  ProjectAuto { on: bool },
  /// Leave this project's links out of `info/exclude` so they can be committed (true), or hide them again
  ShareProject { share: bool },
  /// Record in Pi's own trust store that it may load this project's resources (`.agents/skills` included)
  TrustPi,
  /// A private skill against the shared one: keep `private` moves it into `.agents/skills` (the shared copy is backed up),
  /// `shared` backs the private copy up; either way the agent gets a link back when it cannot read `.agents` itself
  ResolveSkill { path: String, keep: Keep },
  /// Put `@AGENTS.md` on top of the project's CLAUDE.md, so Claude reads the shared project prompt too
  ClaudeImport,
  CreateSkill { scope: SharedScope, name: String },
  /// Delete a skill folder: a shared one in `.agents/skills` (agents' links to it are pruned) or one in an agent's own
  /// skills folder. The folder is moved into the backups, never removed outright
  RemoveSkill { path: String },
  /// Create the file / directory when missing, then open it
  Open { scope: SharedScope, target: SharedTarget },
  /// `json`: `{ "mcpServers": {…} }`, a map of servers, or one server (then `name` is required)
  AddMcp {
    scope: SharedScope,
    json: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
  },
  ToggleMcp { scope: SharedScope, name: String, enabled: bool },
  RemoveMcp { scope: SharedScope, name: String },
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn action_shape() {
    let a: SharedAction = serde_json::from_str(r#"{"kind":"resolveSkill","path":"/p","keep":"private"}"#).unwrap();
    assert_eq!(a, SharedAction::ResolveSkill { path: "/p".into(), keep: Keep::Private });
    let a: SharedAction = serde_json::from_str(r#"{"kind":"toggleMcp","scope":"global","name":"x","enabled":false}"#).unwrap();
    assert_eq!(a, SharedAction::ToggleMcp { scope: SharedScope::Global, name: "x".into(), enabled: false });
    let v = serde_json::to_value(SharedAction::Link { picks: vec![Pick { at: "/a".into(), choice: Choice::KeepShared }], auto: true }).unwrap();
    assert_eq!(v, serde_json::json!({ "kind": "link", "picks": [{ "at": "/a", "choice": "keep_shared" }], "auto": true }));
    let a: SharedAction = serde_json::from_str(r#"{"kind":"overwrite","on":false}"#).unwrap();
    assert_eq!(a, SharedAction::Overwrite { on: false, merge: vec![] });
    let a: SharedAction = serde_json::from_str(r#"{"kind":"savePrompt","scope":"global","text":"x","base":""}"#).unwrap();
    assert_eq!(a, SharedAction::SavePrompt { scope: SharedScope::Global, text: "x".into(), base: String::new() });
  }
}
