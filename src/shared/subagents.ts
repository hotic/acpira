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
  // the agent's own identifiers (diagnostics + replay re-association); sidechainId = a Claude workflow agent's own agentId,
  // which names the CLI's sidechain transcript its process is read from (agentId is then the run-and-position key)
  peer: { sessionId?: string; agentId?: string; toolCallId?: string; sidechainId?: string };
  activity?: string;          // latest activity label maintained host-side
  toolCount: number;
  result?: string;            // final result text the parent received (receipt content / Devin summary); for 'session' visibility the child's own last text
  hasProcess?: boolean;       // a receipt node whose transcript has content of its own (a Claude workflow agent's sidechain log)
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
  effort?: string;     // the reasoning effort the child session really runs (the CLI's option name); absent = none reported
  group?: string;      // the group the round was summoned through (SubagentGroup.id)
}

// bypass = work without approvals: the CLI's own full-access mode where it has one, and its permission requests are
// answered by the host instead of shown as cards; inherit (a persona setting only, never a node's round mode) = each
// round takes the caller's current access: read-only / plan → consult, full access → bypass, anything else work
export type RelayMode = 'consult' | 'work' | 'bypass' | 'inherit';

// What a persona is for. Picking one fills in its default access and standing instructions (dutyDefaults, both still the
// user's to change) and decides how a call without `thread` continues: scouts and executors keep their idle conversation
// within a session, a reviewer starts fresh every time, general continues only when the call names the thread
export type SubagentDuty = 'general' | 'scout' | 'executor' | 'reviewer';
export const DUTIES: SubagentDuty[] = ['general', 'scout', 'executor', 'reviewer'];

// Mirror of the Rust SCOUT_BRIEF / EXECUTOR_BRIEF / REVIEWER_BRIEF (both checked against test/fixtures/duty-presets.json).
// Written for the model, so in English whatever the UI language
export const DUTY_BRIEFS: Record<Exclude<SubagentDuty, 'general'>, string> = {
  scout: 'You are a scout: find things out and report them; do not change anything. Back every claim with '
    + 'where it comes from as path:line (or path:start-end), quoting the line when it matters, so the caller can act on your '
    + 'report without opening the files again. Say what you looked for and did not find as well as what you found, and mark '
    + 'guesses as guesses.',
  executor: 'You carry out a plan another agent made. If a step looks wrong, would break something, or '
    + 'misses a case, stop before doing it and say so, with the reason and what you would do instead: pushing back early is part '
    + 'of the job. Otherwise do what was asked and no more, and run the checks the task names (or the obvious ones).',
  reviewer: 'You are a reviewer. Look for real defects in the change: bugs, regressions, missed cases, '
    + 'broken contracts or docs that no longer match. Read the changed code, its callers and its tests yourself, and judge the '
    + 'change against the requirement, not against how its author describes it. Leave out style preferences.',
};

// The access and standing instructions a duty fills in when picked; general leaves both as they are
export function dutyDefaults(duty: SubagentDuty): { mode: RelayMode; brief: string } | undefined {
  if (duty === 'general') return undefined;
  return { mode: duty === 'executor' ? 'inherit' : 'consult', brief: DUTY_BRIEFS[duty] };
}

export const keepsThread = (duty: SubagentDuty | undefined) => duty === 'scout' || duty === 'executor';
export const freshThread = (duty: SubagentDuty | undefined) => duty === 'reviewer';

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
  duty?: SubagentDuty;  // absent = general
  changes?: boolean;    // every new conversation starts with the handoff pack of the session's changes
}

export const PERSONA_MAX = 32;
// The id the built-in agent's sidekick goes by (mirror of SIDEKICK_ID): no persona or group of the roster takes it
export const SIDEKICK_ID = 'sidekick';
const NAME_MAX = 48;
const TEXT_MAX = 2000;

// `Codex Review` → `codex-review`; empty when nothing ascii is left
export function personaSlug(name: string): string {
  return name.trim().toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
}

// Mirror of sanitize_personas: unnamed / agentless entries drop, text is clipped, ids are unique slugs (never SIDEKICK_ID)
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
    for (let n = 2; id === SIDEKICK_ID || out.some(p => p.id === id); n++) id = `${base}-${n}`;
    const opt = (v: unknown, max: number) => str(v, max) || undefined;
    out.push({
      id, name, agent,
      model: opt(o.model, 200), effort: opt(o.effort, 200),
      mode: o.mode === 'work' || o.mode === 'bypass' || o.mode === 'inherit' ? o.mode : 'consult',
      when: str(o.when, TEXT_MAX),
      brief: opt(o.brief, TEXT_MAX),
      enabled: o.enabled !== false,
      ...(o.duty === 'scout' || o.duty === 'executor' || o.duty === 'reviewer' ? { duty: o.duty } : {}),
      ...(o.changes === true ? { changes: true } : {}),
    });
  }
  return out;
}

// Personas summoned together (subagents.json `groups`): one ask_agent call naming the group starts a round for each
// enabled member, same task and handoff pack, each on its own; the replies come back together. Mirror of
// acpira_shared::subagents::SubagentGroup
export interface SubagentGroup {
  id: string;           // stable slug, unique among groups and never a persona's id (the tool's `agent` names either)
  name: string;
  members: string[];    // persona ids, in the order their rounds start
  when: string;         // tells the model when to summon the group
  enabled: boolean;
}

export const GROUP_MAX = 16;
// The summoned rounds a session runs at once
export const GROUP_MEMBERS_MAX = 6;

// Mirror of sanitize_groups: unnamed entries drop, text is clipped, ids are unique slugs no persona has, members are
// the ids of `personas` it names, each once, in order, at most GROUP_MEMBERS_MAX
export function sanitizeGroups(value: unknown, personas: SubagentPersona[]): SubagentGroup[] {
  const out: SubagentGroup[] = [];
  const str = (v: unknown, max: number) => (typeof v === 'string' ? [...v.trim()].slice(0, max).join('') : '');
  for (const item of Array.isArray(value) ? value.slice(0, GROUP_MAX) : []) {
    if (!item || typeof item !== 'object') continue;
    const o = item as Record<string, unknown>;
    const name = str(o.name, NAME_MAX);
    if (!name) continue;
    const base = personaSlug(str(o.id, 200)) || personaSlug(name) || 'group';
    const taken = (id: string) => id === SIDEKICK_ID || out.some(g => g.id === id) || personas.some(p => p.id === id);
    let id = base;
    for (let n = 2; taken(id); n++) id = `${base}-${n}`;
    const members: string[] = [];
    for (const m of Array.isArray(o.members) ? o.members : []) {
      if (typeof m !== 'string') continue;
      const want = m.trim();
      if (members.length < GROUP_MEMBERS_MAX && personas.some(p => p.id === want) && !members.includes(want)) members.push(want);
    }
    out.push({ id, name, members, when: str(o.when, TEXT_MAX), enabled: o.enabled !== false });
  }
  return out;
}

export interface SubagentRecord extends Omit<SubagentSummary, 'permissions' | 'question'> {
  turns: Turn[];
  rev?: number;   // the node's dirty counter at write time; absent on records written before it was persisted
}
