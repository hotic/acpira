import type { AgentId } from './transcript';
import type { McpTransport } from './inventory';

// The settings page's Shared tab (mirror of rust/crates/acpira-shared/src/shared_config.rs).
// The content lives in open files (`~/.agents`, `<project>/.agents`, `AGENTS.md`); these types only describe it and
// how far each agent can see it

export type SharedScope = 'project' | 'global';

// native: the agent reads the shared location itself; linked: through a link / import line; missing: a link fixes it
// without a decision; conflict: different content of the agent's own sits there; unsupported: the agent cannot take it;
// untrusted: the agent reads it only in a project it trusts (Pi and project skills)
export type ReachState = 'native' | 'linked' | 'missing' | 'conflict' | 'unsupported' | 'untrusted';

export interface Reach {
  agent: AgentId;
  state: ReachState;
  // The agent-side file or link, when there is one
  path?: string;
}

export interface SharedSkill {
  name: string;
  description?: string;
  // The skill directory
  path: string;
  scope: SharedScope;
  reach: Reach[];
}

export interface SharedMcp {
  name: string;
  scope: SharedScope;
  transport: McpTransport;
  // Command line or URL, for display
  target: string;
  enabled: boolean;
  // A project server of the same name replaces this global one
  shadowed: boolean;
  // Agents whose advertised transports exclude this one (agents without client MCP at all are in SharedView.noMcp)
  unsupported: AgentId[];
  // Agents whose own config already declares this name; it is not sent to them twice
  native: AgentId[];
}

export interface SharedPrompt {
  scope: SharedScope;
  path: string;
  exists: boolean;
  // The first lines, for the card
  preview: string;
  // The whole file as it is on disk (empty when missing), for the page's plain-text editor
  text: string;
  reach: Reach[];
}

// An agent's own global instruction file with lines ~/.agents/AGENTS.md does not have, offered for merging before
// overwrite replaces the file with a link to the shared prompt
export interface Takeover {
  agent: AgentId;
  // The agent's file; the key of the overwrite action's merge list
  path: string;
  // The lines only this file has, in file order (runs separated by a blank line)
  unique: string;
}

// unique: no shared skill of this name; same: identical files; differs: a shared skill of this name has other content
export type PrivateMatch = 'unique' | 'same' | 'differs';

// A skill in one agent's own directory rather than in `.agents/skills`
export interface PrivateSkill {
  name: string;
  description?: string;
  path: string;
  agent: AgentId;
  scope: SharedScope;
  matches: PrivateMatch;
}

export type PlanKind = 'skill' | 'prompt';

// One user-level link point that is not in place yet, as the link panel lists it
export interface PlanItem {
  // Where the link / import goes; the key of a Pick
  at: string;
  agent: AgentId;
  kind: PlanKind;
  // The skill name, or the file name of the agent's instruction file
  name: string;
  // missing (nothing there, or an identical copy) or conflict (content of the agent's own)
  state: ReachState;
  // Left out on purpose last time; kept out of automatic linking until picked again
  skipped: boolean;
}

export interface SharedView {
  // The project root (nearest git root above the workspace); absent without a workspace or when it is home
  root?: string;
  home: string;
  // User level: new shared skills (and newly installed agents) are linked on their own
  auto: boolean;
  // Something is linked at user level (the undo button has work to do)
  userLinked: boolean;
  // Project level: Claude's links to `.agents/skills` are made on their own (default on)
  projectAuto: boolean;
  // This project's links are left out of `info/exclude`, so they can be committed for the team
  projectShared: boolean;
  // This project has links Acpira made
  projectLinked: boolean;
  // Pi is installed, the project has `.agents/skills`, and Pi does not trust the project
  piUntrusted: boolean;
  // Whether `~/.agents/AGENTS.md` exists
  sharedPrompt: boolean;
  // User level is overwritten: every agent's global instruction file and skill link point is kept wired to ~/.agents,
  // skipped points and conflicts included (what stood there is backed up and restored when turned off)
  overwrite: boolean;
  // Before overwrite is on: the agents' own global prompts with content of their own
  takeover: Takeover[];
  skills: SharedSkill[];
  mcp: SharedMcp[];
  // Installed agents that take no client MCP servers at all (Pi)
  noMcp: AgentId[];
  prompts: SharedPrompt[];
  privateSkills: PrivateSkill[];
  // User-level link points still to decide on, for the link panel
  plan: PlanItem[];
}

export type Keep = 'shared' | 'private';
export type SharedTarget = 'skills' | 'mcp' | 'prompt';

// link: make a missing link; keep_shared: the shared version wins, the agent's own is backed up;
// keep_private: the agent's own version becomes the shared one; skip: leave it and out of automatic linking
export type Choice = 'link' | 'keep_shared' | 'keep_private' | 'skip';

export interface Pick {
  at: string;
  choice: Choice;
}

export type SharedAction =
  // The link panel's decisions for user level; auto keeps new skills linked from now on
  | { kind: 'link'; picks: Pick[]; auto: boolean }
  // Remove every user-level link Acpira made and put back what it moved aside
  | { kind: 'unlink' }
  // On: the unique lines of the merge files go to the end of ~/.agents/AGENTS.md, then every user-level link point is
  // wired; off: what overwrite wired is undone and the backups go back in place
  | { kind: 'overwrite'; on: boolean; merge?: string[] }
  // Write a prompt file verbatim, in place; refused when the file no longer holds base, the text the edit started from
  | { kind: 'savePrompt'; scope: SharedScope; text: string; base: string }
  // Project level: make Claude's links on their own (on), or remove every project link and stop (off)
  | { kind: 'projectAuto'; on: boolean }
  // Leave this project's links out of `info/exclude` so they can be committed (true), or hide them again
  | { kind: 'shareProject'; share: boolean }
  // Record in Pi's own trust store that it may load this project's resources
  | { kind: 'trustPi' }
  // keep private: the private copy moves into `.agents/skills` (the shared one is backed up); shared: the private copy is backed up
  | { kind: 'resolveSkill'; path: string; keep: Keep }
  // Put `@AGENTS.md` on top of the project's CLAUDE.md
  | { kind: 'claudeImport' }
  | { kind: 'createSkill'; scope: SharedScope; name: string }
  // Delete a shared skill folder (agents' links to it are pruned) or one in an agent's own folder; moved into the backups
  | { kind: 'removeSkill'; path: string }
  // Create the file / directory when missing, then open it
  | { kind: 'open'; scope: SharedScope; target: SharedTarget }
  // json: `{ "mcpServers": {…} }`, a map of servers, or one server (then name is required)
  | { kind: 'addMcp'; scope: SharedScope; json: string; name?: string }
  | { kind: 'toggleMcp'; scope: SharedScope; name: string; enabled: boolean }
  | { kind: 'removeMcp'; scope: SharedScope; name: string };

// The agents a skill / prompt still needs a decision or a click for
export function pending(reach: Reach[]): { missing: AgentId[]; conflict: AgentId[] } {
  return {
    missing: reach.filter((r) => r.state === 'missing').map((r) => r.agent),
    conflict: reach.filter((r) => r.state === 'conflict').map((r) => r.agent),
  };
}
