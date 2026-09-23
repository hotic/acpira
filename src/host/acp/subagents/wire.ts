// Extension session updates through SDK 1.4.0: zSessionUpdate is a closed union, so an unknown `sessionUpdate`
// kind makes parseParams throw and the notification never reaches onUpdate. rewriteExtension swaps the variant
// for `session_info_update` (every field optional, `_meta` preserved) with the original update parked under
// EXT_META_KEY — keeping the same `session/update` handler means wire order against adjacent updates is preserved.

import type * as acp from '@agentclientprotocol/sdk';
import type { SubagentState } from '@shared/subagents';
import type { AsyncTaskState } from '@shared/transcript';

export const EXT_META_KEY = 'acpira/extension';

const EXT_KINDS = new Set([
  'subagent_update',          // RFD #1992 upsert
  'subagent_spawned',         // claude-agent-acp 0.78/0.79
  'subagent_state_update',    // claude-agent-acp 0.78/0.79
  'async_task_spawned',       // JetBrains AIR asyncTasks (claude-agent-acp acp-subagents, codex-acp background terminal)
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

// One asyncTasks update decoded: the three kinds share asyncTaskId; spawned carries the card fields,
// progress the live ones, state the transition. toolCallId links the task to the tool row that spawned
// it and can show up on any of the three, including after the task was first announced
export interface AsyncTaskEvent {
  kind: 'async_task';
  event: 'spawned' | 'progress' | 'state';
  asyncTaskId: string;
  name?: string;
  taskType?: string;
  description?: string;
  showInTranscript?: boolean;
  canStop?: boolean;
  outputFilePath?: string;
  toolCallId?: string;
  summary?: string;
  lastToolName?: string;
  usage?: { totalTokens?: number; toolUses?: number; durationMs?: number };
  state?: AsyncTaskState;
  meta: Record<string, unknown>;
}

export type ExtensionUpdate = SubagentLifecycle | AsyncTaskEvent | { kind: 'ignored'; sessionUpdate: string };

const ASYNC_TASK_STATES: readonly AsyncTaskState[] = ['running', 'paused', 'completed', 'failed', 'stopped'];
const ASYNC_TASK_KINDS: Record<string, AsyncTaskEvent['event']> = {
  async_task_spawned: 'spawned',
  async_task_progress: 'progress',
  async_task_state_update: 'state',
};

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
  if (typeof kind === 'string' && kind in ASYNC_TASK_KINDS) return asyncTaskEvent(kind, raw, log);
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

function asyncTaskEvent(kind: string, raw: Record<string, unknown>, log?: (line: string) => void): ExtensionUpdate {
  const asyncTaskId = str(raw.asyncTaskId);
  if (asyncTaskId === undefined) {
    log?.(`${kind} without asyncTaskId dropped`);
    return { kind: 'ignored', sessionUpdate: kind };
  }
  const e: AsyncTaskEvent = {
    kind: 'async_task', event: ASYNC_TASK_KINDS[kind]!, asyncTaskId, meta: raw,
  };
  const name = str(raw.name); if (name !== undefined) e.name = name;
  const taskType = str(raw.taskType); if (taskType !== undefined) e.taskType = taskType;
  const description = str(raw.description); if (description !== undefined) e.description = description;
  if (raw.showInTranscript === true) e.showInTranscript = true;
  if (typeof raw.canStop === 'boolean') e.canStop = raw.canStop;
  const outputFilePath = str(raw.outputFilePath); if (outputFilePath !== undefined) e.outputFilePath = outputFilePath;
  const toolCallId = str(raw.toolCallId); if (toolCallId !== undefined) e.toolCallId = toolCallId;
  const summary = str(raw.summary); if (summary !== undefined) e.summary = summary;
  const lastToolName = str(raw.lastToolName); if (lastToolName !== undefined) e.lastToolName = lastToolName;
  const usage = record(raw.usage);
  if (usage) {
    const u: NonNullable<AsyncTaskEvent['usage']> = {};
    if (typeof usage.totalTokens === 'number') u.totalTokens = usage.totalTokens;
    if (typeof usage.toolUses === 'number') u.toolUses = usage.toolUses;
    if (typeof usage.durationMs === 'number') u.durationMs = usage.durationMs;
    e.usage = u;
  }
  if (e.event === 'state') {
    const state = str(raw.state);
    if (state === undefined || !(ASYNC_TASK_STATES as readonly string[]).includes(state)) {
      log?.(`async_task_state_update with unknown state dropped: ${JSON.stringify(raw.state)}`);
      return { kind: 'ignored', sessionUpdate: kind };
    }
    e.state = state as AsyncTaskState;
  }
  return e;
}
