// Extension session updates through SDK 1.4.0: zSessionUpdate is a closed union, so an unknown `sessionUpdate`
// kind makes parseParams throw and the notification never reaches onUpdate. rewriteExtension swaps the variant
// for `session_info_update` (every field optional, `_meta` preserved) with the original update parked under
// EXT_META_KEY — keeping the same `session/update` handler means wire order against adjacent updates is preserved.

import type * as acp from '@agentclientprotocol/sdk';
import type { SubagentState } from '@shared/subagents';

export const EXT_META_KEY = 'acpira/extension';

const EXT_KINDS = new Set([
  'subagent_update',          // RFD #1992 upsert
  'subagent_spawned',         // claude-agent-acp 0.78/0.79
  'subagent_state_update',    // claude-agent-acp 0.78/0.79
  'async_task_spawned',       // not subagents — surfaced as ignored so the host can log them
  'async_task_progress',
  'async_task_state_update',
]);

export interface SubagentLifecycle {
  kind: 'lifecycle';
  peerSessionId: string;
  title?: string;
  task?: string;
  capabilities?: { cancel?: boolean };
  state?: SubagentState;
  meta?: Record<string, unknown>;
}

export type ExtensionUpdate = SubagentLifecycle | { kind: 'ignored'; sessionUpdate: string };

function record(v: unknown): Record<string, unknown> | undefined {
  return v !== null && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : undefined;
}

function str(v: unknown): string | undefined {
  return typeof v === 'string' && v.length > 0 ? v : undefined;
}

function stateOf(v: unknown, log?: (line: string) => void): SubagentState | undefined {
  if (v === undefined || v === null) return undefined;
  if (v === 'running' || v === 'completed' || v === 'failed' || v === 'cancelled' || v === 'disconnected') return v;
  log?.(`unknown subagent state ${JSON.stringify(v)} — treating as running`);
  return 'running';
}

function capabilitiesOf(v: unknown): { cancel?: boolean } | undefined {
  const c = record(v);
  return c === undefined ? undefined : { cancel: c.cancel === true };
}

// Inbound `session/update` notification → rewritten for the closed SDK union; everything else passes through
export function rewriteExtension(msg: unknown): unknown {
  const m = record(msg);
  if (m === undefined || m.method !== 'session/update' || 'id' in m) return msg;
  const params = record(m.params);
  const update = record(params?.update);
  if (update === undefined || typeof update.sessionUpdate !== 'string' || !EXT_KINDS.has(update.sessionUpdate)) return msg;
  return { ...m, params: { sessionId: params?.sessionId, update: { sessionUpdate: 'session_info_update', _meta: { [EXT_META_KEY]: update } } } };
}

// The reverse of rewriteExtension: a rewritten notification arrives as session_info_update with the original update
// under EXT_META_KEY; plain session_info_updates (session title) return undefined and flow through normalize unchanged
export function extensionOf(u: acp.SessionUpdate, log?: (line: string) => void): ExtensionUpdate | undefined {
  if (u.sessionUpdate !== 'session_info_update') return undefined;
  const raw = record(record(u._meta)?.[EXT_META_KEY]);
  if (raw === undefined) return undefined;
  const kind = raw.sessionUpdate;
  if (kind === 'subagent_update' || kind === 'subagent_spawned' || kind === 'subagent_state_update') {
    const peerSessionId = str(raw.subagentSessionId);
    if (peerSessionId === undefined) {
      log?.(`${kind} without subagentSessionId dropped`);
      return { kind: 'ignored', sessionUpdate: String(kind) };
    }
    const l: SubagentLifecycle = { kind: 'lifecycle', peerSessionId, meta: raw };
    const title = str(raw.name);
    const task = str(raw.task);
    const caps = capabilitiesOf(raw.capabilities);
    const state = stateOf(raw.state, log);
    if (title !== undefined) l.title = title;
    if (task !== undefined) l.task = task;
    if (caps !== undefined) l.capabilities = caps;
    if (state !== undefined) l.state = state;
    return l;
  }
  return { kind: 'ignored', sessionUpdate: typeof kind === 'string' ? kind : '?' };
}
