// The plugin page's view and actions (mirror of rust/crates/acpira-shared/src/plugins.rs; the contract is
// docs/dev/plugins.md): the user's plugins with their state, the built-in tool groups, recent changes and safe mode

export type PluginLayer = 'agents' | 'tools' | 'interface' | 'automation';
export const PLUGIN_LAYERS: readonly PluginLayer[] = ['agents', 'tools', 'interface', 'automation'];

export type PluginContribution = 'tools' | 'mcp' | 'hooks' | 'theme' | 'prompts' | 'agent';

// Who made a change: user (a page), session (a card in a session, with its agent and model) or cli
export interface PluginProvenance {
  by: 'user' | 'session' | 'cli' | (string & {});
  session?: string;
  agent?: string;
  model?: string;
}

// A frozen version waiting for the user to confirm it
export interface PluginProposal {
  snapshot: string;
  provenance: PluginProvenance;
  at: number;
}

// What the static check of the source folder found
export interface PluginFindings {
  errors?: string[];
  notes?: string[];
}

// One user plugin: a source folder, an entry of the state, or both
export interface PluginInfo {
  id: string;
  name: string;
  description: string;
  version: string;
  layer: PluginLayer;
  contributes: PluginContribution[];
  // The names its script tools are served under (`<id>_<tool>`)
  tools?: string[];
  enabled: boolean;
  // Enabled and in effect: not under safe mode, its frozen copy intact
  loaded: boolean;
  snapshot?: string;
  canRollback: boolean;
  provenance?: PluginProvenance;
  committedAt?: number;
  proposal?: PluginProposal;
  // The source folder differs from the version in use ("changes not enabled")
  changed: boolean;
  // Why an enabled plugin is not in effect, or why its source cannot be read
  error?: string;
  findings?: PluginFindings;
  // The source folder
  path: string;
}

export type ToolGroupId = 'summon' | 'sessions' | 'display' | 'browser';
export const TOOL_GROUPS: readonly ToolGroupId[] = ['summon', 'sessions', 'display', 'browser'];

// A built-in tool group of Acpira's MCP server
export interface ToolGroupInfo {
  id: ToolGroupId;
  on: boolean;
  tools: string[];
}

export type PluginChangeAction = 'enable' | 'disable' | 'rollback' | 'remove' | 'propose' | 'dismiss' | 'safeMode' | 'toolGroup';

// One committed change (changes.jsonl)
export interface PluginChange {
  generation: number;
  at: number;
  id: string;
  action: PluginChangeAction;
  snapshot?: string;
  on?: boolean;
  provenance: PluginProvenance;
}

export interface PluginsView {
  // The plugins folder (~/.acpira/plugins)
  root: string;
  generation: number;
  // User plugins are unloaded
  safeMode: boolean;
  // Safe mode comes from the engine's launch (--safe-mode, ACPIRA_SAFE_MODE), not the switch
  safeModeForced: boolean;
  plugins: PluginInfo[];
  toolGroups: ToolGroupInfo[];
  // The newest first
  changes: PluginChange[];
}

// The plugin page's actions. Its switch is the user's confirmation: enable freezes the source folder, checks it and
// commits it, like a card in a session
export type PluginAction =
  | { op: 'enable'; id: string }
  // Enable the version a proposal froze
  | { op: 'confirm'; id: string; snapshot: string }
  | { op: 'dismiss'; id: string }
  | { op: 'disable'; id: string }
  | { op: 'rollback'; id: string }
  | { op: 'remove'; id: string }
  // Show the source folder in the OS file manager
  | { op: 'reveal'; id: string }
  | { op: 'safeMode'; on: boolean }
  | { op: 'toolGroup'; id: ToolGroupId; on: boolean };
