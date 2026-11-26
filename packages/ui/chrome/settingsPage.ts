import type { AgentInfo } from '@acpira/shared/transcript';

// Orchestration carries the persona or group opened from its lists, and an agent's page is the Models page with that
// agent's dialog open, so an opened dialog is a step of its own in the
// back / forward trail and the chat can open the settings straight at one. Plugins carries its open filter, so the
// rail's search can land on one
export type SettingsPage = { kind: 'general' } | { kind: 'appearance' } | { kind: 'shortcuts' } | { kind: 'models' } | { kind: 'plugins'; tab?: string } | { kind: 'shared' } | { kind: 'connections' }
  | { kind: 'subagents'; persona?: string; group?: string } | { kind: 'usage' } | { kind: 'storage' } | { kind: 'about' }
  | { kind: 'agent'; id: AgentInfo['id']; tab?: 'models' | 'sources' };
