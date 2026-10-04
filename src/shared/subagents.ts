// First-class subagent nodes shared by host and webview: one summary per delegated child, whatever the wire dialect

import type { AgentId, PermissionBlock, QuestionBlock, Turn } from './transcript';

export type SubagentState = 'running' | 'completed' | 'failed' | 'cancelled' | 'disconnected';

// What Acpira can observe of the child: 'session' = the agent streams the child's own updates under its own session id
// (claude-agent-acp native sessions / RFD #1992); 'nested' = the child's tool calls arrive on the parent session with a
// structured parent link (Devin `cognition.ai/subagent_context`, Claude `claudeCode.parentToolUseId`); 'receipt' = only the
// delegation call and its result text (Kimi's Agent tool)
export type SubagentVisibility = 'session' | 'nested' | 'receipt';

export interface SubagentSummary {
  id: string;                 // Acpira's own stable id (randomUUID) — never a peer id
  parentId?: string;          // another summary's id; absent = child of the root session
  turnIndex: number;          // index in SessionView.turns of the root agent turn during which it was announced
  visibility: SubagentVisibility;
  title?: string;             // short label (RFD/claude `name`, Devin `title`, Claude legacy/Kimi `description`)
  task?: string;              // the delegated task text (RFD `task`, Devin `task`, Claude/Kimi `prompt`)
  role?: string;              // Devin `profile`, Claude/Kimi `subagent_type`; absent when not structured
  state: SubagentState;
  stateSource: 'agent' | 'local';   // 'local' = Acpira synthesized it (disconnected on connection end / parent prompt returned first)
  controls: { cancel: boolean };    // default false; only true when the agent said so for this child
  cancelRequested?: boolean;
  background?: boolean;       // the agent said the child runs detached (Devin isBackground, Claude toolResponse.isAsync)
  announcedAt: number;
  endedAt?: number;
  model?: string;             // only when reported (Devin subagent_started.model, Claude toolResponse.resolvedModel / rawInput.model)
  usage?: { used: number; size: number };  // only when a usage_update was attributed to this child
  peer: { sessionId?: string; agentId?: string; toolCallId?: string };  // the agent's own identifiers (diagnostics + replay re-association)
  activity?: string;          // latest activity label maintained host-side
  toolCount: number;
  result?: string;            // final result text the parent received (receipt content / Devin summary); for 'session' visibility the child's own last text
  permissions?: PermissionBlock[];  // this child's pending permission cards, mirrored so the root view can show them with provenance
  question?: QuestionBlock;         // this child's open question card, same reason
  harness?: SubagentHarness;  // set when Acpira itself runs the child in another CLI (a summoned persona)
}

// A summoned child: the CLI it runs in, the persona it was summoned as (its name is `role`), and its thread — every
// round of one conversation with the same native session is a node of its own
export interface SubagentHarness {
  agent: AgentId;
  persona?: string;
  mode: RelayMode;
  thread: string;
  round: number;
  sessionId?: string;  // the child CLI's own session id (peer.sessionId is Acpira's routing key)
}

export type RelayMode = 'consult' | 'work';

// A cross-harness subagent defined once in the settings (~/.acpira/subagents.json): any session can summon it through
// Acpira's MCP tool `ask_agent`, or the user names it with @name. Mirror of acpira_shared::subagents::SubagentPersona
export interface SubagentPersona {
  id: string;           // stable slug: the tool's `agent` value
  name: string;
  agent: AgentId;       // the CLI it runs in
  model?: string;       // a value of that agent's model select; absent = the CLI's default
  effort?: string;      // a value of its reasoning-effort select; absent = the default
  mode: RelayMode;      // consult = the CLI's own read-only mode where it has one
  when: string;         // tells the model when to summon it
  brief?: string;       // appended to every task
  enabled: boolean;
}

export const PERSONA_MAX = 32;
const NAME_MAX = 48;
const TEXT_MAX = 2000;

// `Codex Review` → `codex-review`; empty when nothing ascii is left
export function personaSlug(name: string): string {
  return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
}

// Mirror of sanitize_personas: unnamed / agentless entries drop, text is clipped, ids are unique slugs
export function sanitizePersonas(value: unknown): SubagentPersona[] {
  const out: SubagentPersona[] = [];
  const str = (v: unknown, max: number) => (typeof v === 'string' ? [...v.trim()].slice(0, max).join('') : '');
  for (const item of Array.isArray(value) ? value.slice(0, PERSONA_MAX) : []) {
    if (!item || typeof item !== 'object') continue;
    const o = item as Record<string, unknown>;
    const name = str(o.name, NAME_MAX);
    const agent = str(o.agent, 200);
    if (!name || !agent) continue;
    const base = personaSlug(str(o.id, 200)) || personaSlug(name) || 'agent';
    let id = base;
    for (let n = 2; out.some(p => p.id === id); n++) id = `${base}-${n}`;
    const opt = (v: unknown, max: number) => str(v, max) || undefined;
    out.push({
      id, name, agent,
      model: opt(o.model, 200), effort: opt(o.effort, 200),
      mode: o.mode === 'work' ? 'work' : 'consult',
      when: str(o.when, TEXT_MAX),
      brief: opt(o.brief, TEXT_MAX),
      enabled: o.enabled !== false,
    });
  }
  return out;
}

export interface SubagentRecord extends Omit<SubagentSummary, 'permissions' | 'question'> {
  turns: Turn[];
  rev?: number;   // the node's dirty counter at write time; absent on records written before it was persisted
}
