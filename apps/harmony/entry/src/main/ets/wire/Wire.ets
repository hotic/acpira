// The wire the app speaks with the engine: the envelopes of packages/shared/sidecar.ts around the HostMsg / WebviewMsg
// of packages/shared/protocol.ts, and the transcript shapes of packages/shared/transcript.ts. Only the fields this app
// reads are declared; JSON.parse keeps the rest. Same contract as apps/ios AcpiraModels and apps/android :model.

/** Must equal SIDECAR_PROTOCOL_VERSION in packages/shared/sidecar.ts; the engine refuses any other value at hello */
export const SIDECAR_PROTOCOL_VERSION: number = 1;

export interface AgentInfo {
  id: string;
  name: string;
  /** The CLI was found (false greys it out); undefined: not probed yet */
  available?: boolean;
  /** Switched off in the settings: no new sessions start on it */
  disabled?: boolean;
}

export interface SessionSummary {
  id: string;
  title: string;
  agent: string;
  cwd: string;
  updatedAt: string;
  pinned?: boolean;
  archived?: boolean;
  state?: string;
  unread?: boolean;
}

export interface PermissionOption {
  id: string;
  label: string;
  kind: string;
}

/** Every agent block in one shape: `type` says which fields carry it (transcript.ts AgentBlock) */
export interface AgentBlock {
  type: string;
  id?: string;
  // text
  markdown?: string;
  streaming?: boolean;
  // thought
  text?: string;
  // tool_call
  kind?: string;
  verb?: string;
  target?: string;
  status?: string;
  // permission
  title?: string;
  command?: string;
  description?: string;
  options?: PermissionOption[];
  // composer: the dock reads these (question, plan, plan approval, todo tool)
  planId?: string;
  message?: string;
  questions?: Question[];
  outcome?: string;
  entries?: PlanEntry[];
  verbKey?: string;
  todoEntries?: PlanEntry[];
}

/** A user or agent turn (transcript.ts Turn): `role` says which */
export interface Turn {
  role: string;
  id?: string;
  // user
  text?: string;
  // agent
  blocks?: AgentBlock[];
  stop?: string;
  /** Epoch ms: the turn's start and end, for the fold's duration */
  startedAt?: number;
  endedAt?: number;
  // composer: a prompt's attachments and selections, and why an agent turn stopped short
  attachments?: Attachment[];
  settings?: TurnSettings;
  auto?: boolean;
  error?: TurnError;
}

/**
 * transcript.ts ViewEarlier: what a windowed view leaves out ahead of `turns[0]`: `turns` whole turns, and the first
 * `blocks` blocks of `turns[0]` itself (an agent turn cut inside its blocks)
 */
export interface ViewEarlier {
  turns: number;
  blocks: number;
}

export interface SessionView {
  id: string;
  agent: string;
  title: string;
  cwd: string;
  status: string;
  turns: Turn[];
  running: boolean;
  rev?: number;
  /**
   * Set while the view starts at the session's tail (the app says `window` in ready); absent once it is whole. Turn
   * indices on the wire (editTurn, forkSession, a subagent's turnIndex) count from the session's first turn: an index
   * into `turns` plus `earlier.turns` (`earlierTurns`)
   */
  earlier?: ViewEarlier;
  // composer: what the composer's pickers, ring and dock read
  controls?: SessionControls;
  usage?: Usage;
  commands?: SlashCommand[];
  queued?: QueuedPrompt[];
  canSteer?: boolean;
  goal?: SessionGoal;
  goalActions?: string[];
  // Transcript (transcript agent): the subagent tree, typed in wire/Blocks.ets SessionViewX
  subagents?: Object[];
}

/** sessionPatch.ts SessionPatch: turns[0..keep) of the view at rev `base` stay, `turns` follow them */
export interface SessionPatch {
  id: string;
  base: number;
  view: SessionView;
  keep: number;
  turns: Turn[];
  keepBlocks?: number;
  blocks?: AgentBlock[];
}

/** protocol.ts SessionsPatch: runs of the new list, `[from, count]` copied from the held list or a new entry */
export interface SessionsPatch {
  base: number;
  rev: number;
  size: number;
  runs: Array<number[] | SessionSummary>;
}

export interface InitState {
  agents: AgentInfo[];
  sessions: SessionSummary[];
  active?: SessionView;
  home: string;
  categories?: SessionCategories;
  settings?: SettingsView;
  /** Where chats' folders live: a session whose cwd is directly inside is a chat, not a project */
  chats?: string;
}

/** Every HostMsg in one shape: `type` says which fields carry it */
export interface HostMsg {
  type: string;
  state?: InitState;
  sessions?: SessionSummary[];
  agents?: AgentInfo[];
  session?: SessionView;
  text?: string;
  /** `sessionUnavailable`: the session picked and the engine's sentence for why it cannot be shown */
  id?: string;
  reason?: string;
}

export interface SessionPatchMsg {
  type: string;
  patch: SessionPatch;
}

/**
 * sessionWindow.ts SessionEarlier, the one answer to `loadEarlier`: `blocks` go ahead of the blocks of the held
 * `turns[0]`, then `turns` ahead of the held turns; `earlier` is what is still left out, absent once the view is whole.
 * `from` is the window it was cut for
 */
export interface SessionEarlier {
  id: string;
  from: ViewEarlier;
  turns: Turn[];
  blocks?: AgentBlock[];
  earlier?: ViewEarlier;
}

export interface SessionEarlierMsg {
  type: string;
  chunk: SessionEarlier;
}

export interface SessionsPatchMsg {
  type: string;
  patch: SessionsPatch;
}

/** One envelope from the engine (sidecar.ts SidecarMsg) */
export interface SidecarMsg {
  type: string;
  viewId?: string;
  message?: HostMsg;
  reason?: string;
  protocolVersion?: number;
}

/** The view a patch leads to, or undefined when the app does not hold the view it was computed against (then resync) */
export function applySessionPatch(current: SessionView | undefined, p: SessionPatch): SessionView | undefined {
  if (current === undefined || current.id !== p.id || current.rev !== p.base || current.turns.length < p.keep) {
    return undefined;
  }
  const tail: Turn[] = p.turns.slice();
  if (p.keepBlocks !== undefined) {
    const base: Turn | undefined = current.turns[p.keep];
    const head: Turn | undefined = tail[0];
    if (base === undefined || head === undefined || base.role !== 'agent' || head.role !== 'agent') {
      return undefined;
    }
    const kept: AgentBlock[] = base.blocks ?? [];
    if (kept.length < p.keepBlocks) {
      return undefined;
    }
    // The parsed head and view are this patch's own objects: completed in place, so every field the wire sent (error,
    // usage, the view's commands and subagents …) reaches the transcript, not only the ones declared here
    head.blocks = kept.slice(0, p.keepBlocks).concat(p.blocks ?? []);
  }
  const view: SessionView = p.view;
  view.turns = current.turns.slice(0, p.keep).concat(tail);
  return view;
}

/** Turns the view leaves out ahead of its first: what a row's index adds to count from the session's first turn */
export function earlierTurns(view: SessionView | undefined): number {
  return view?.earlier?.turns ?? 0;
}

/** A parsed wire object's own copy, one level deep, with every field the wire sent (ArkTS has no object spread) */
function copyOf(source: Object): Record<string, Object> {
  const from = source as Record<string, Object>;
  const out: Record<string, Object> = {};
  Object.keys(from).forEach((key: string) => {
    out[key] = from[key];
  });
  return out;
}

/**
 * The view with the chunk in front (sessionWindow.ts applySessionEarlier), or undefined when the chunk was cut for
 * another window (a whole view landed since it was asked for): then it is dropped. The rev stays. The held turns keep
 * their objects, so their rows stay (store/Thread.ets turnKey); only a cut first turn that gains its leading blocks is
 * a new object, and its row alone is built again
 */
export function applySessionEarlier(current: SessionView | undefined, c: SessionEarlier): SessionView | undefined {
  if (current === undefined || current.id !== c.id) {
    return undefined;
  }
  if ((current.earlier?.turns ?? 0) !== c.from.turns || (current.earlier?.blocks ?? 0) !== c.from.blocks) {
    return undefined;
  }
  let held: Turn[] = current.turns;
  const lead: AgentBlock[] = c.blocks ?? [];
  if (lead.length > 0) {
    const first: Turn | undefined = held[0];
    if (first === undefined || first.role !== 'agent') {
      return undefined;
    }
    const grown = copyOf(first) as Object as Turn;
    grown.blocks = lead.concat(first.blocks ?? []);
    const front: Turn[] = [grown];
    held = front.concat(held.slice(1));
  }
  const view = copyOf(current) as Object as SessionView;
  view.earlier = c.earlier;
  view.turns = c.turns.concat(held);
  return view;
}

/** `held` (at revision `heldRev`) with the patch applied; undefined when it does not fit (then ask for the whole list) */
export function applySessionsPatch(held: SessionSummary[], heldRev: number, p: SessionsPatch): SessionSummary[] | undefined {
  if (heldRev !== p.base || held.length !== p.size) {
    return undefined;
  }
  const out: SessionSummary[] = [];
  for (const run of p.runs) {
    if (run instanceof Array) {
      const from: number = run[0];
      const count: number = run[1];
      if (from < 0 || count < 0 || from > held.length - count) {
        return undefined;
      }
      for (let i = from; i < from + count; i++) {
        out.push(held[i]);
      }
    } else {
      out.push(run as SessionSummary);
    }
  }
  return out;
}

// Session list, settings, prefs (list agent): the categories, the engine settings and the list's answers

/** One of the desktop app's sections (transcript.ts SessionSection) */
export interface SessionSection {
  id: string;
  name: string;
  projects: string[];
  collapsed?: boolean;
}

/** transcript.ts SessionCategories: only the project shell's parts the phone reads */
export interface SessionCategories {
  projects?: string[];
  removedProjects?: string[];
  sections?: SessionSection[];
}

/** settings.ts SettingsView: the fields the settings pages show */
export interface SettingsView {
  defaultAgent: string;
  agentOrder: string[];
  disabledAgents: string[];
  autoCompact: boolean;
  compactAtTokens: number;
  accountSwitch: string;
  steerQueued: boolean;
  planAutoApprove: string[];
  /** agent → control id → hidden model family keys, kept out of the composer menus (settings.ts HiddenMap) */
  hiddenOptions?: Record<string, Record<string, string[]>>;
  /** Cross-harness subagents and their groups, which `@` summons (subagents.ts) */
  subagents?: SubagentPersona[];
  subagentGroups?: SubagentGroup[];
}

/** subagents.ts SubagentPersona: only what the `@` list shows */
export interface SubagentPersona {
  id: string;
  name: string;
  agent: string;
  model?: string;
  effort?: string;
  enabled: boolean;
}

/** subagents.ts SubagentGroup: only what the `@` list shows */
export interface SubagentGroup {
  id: string;
  name: string;
  members: string[];
  enabled: boolean;
}

export function defaultSettings(): SettingsView {
  return {
    defaultAgent: 'acpira', agentOrder: [], disabledAgents: [], autoCompact: true, compactAtTokens: 300000,
    accountSwitch: 'off', steerQueued: false, planAutoApprove: []
  };
}

/** protocol.ts SessionHit: a content match and the text around it */
export interface SessionHit {
  id: string;
  snippet: string;
  summary?: SessionSummary;
}

/** The list's HostMsgs: `categories`, `settings`, `sessionHits`, `sessionMarkdown` */
export interface ListHostMsg {
  type: string;
  categories?: SessionCategories;
  settings?: SettingsView;
  seq?: number;
  hits?: SessionHit[];
  markdown?: string;
  error?: string;
}

// composer: the shapes the composer and the dock under the thread read (transcript.ts SessionControls, Usage,
// SlashCommand, Attachment, QueuedPrompt, Question, PlanEntry, SessionGoal; protocol.ts Draft, FileHit,
// EditTurnRequest), and the view copy every patch and local change goes through

export interface OptionRef {
  id: string;
  name: string;
}

export interface OptionSource {
  id: string;
  name: string;
  /** `official` / `custom` */
  kind: string;
}

export interface SessionOption {
  id: string;
  name: string;
  description?: string;
  group?: OptionRef;
  source?: OptionSource;
  /** The agent's own classification (`full_access`, `auto_review`); display only */
  kind?: string;
}

export interface ConfigControl {
  id: string;
  name: string;
  /** model / thought_level / model_config / collaboration_mode / _permission_mode / _sidekick… */
  category?: string;
  /** `boolean` for an ACP boolean option; its options are the synthetic Off / On pair */
  type?: string;
  options: SessionOption[];
  value?: string;
}

export interface SessionControls {
  modes: SessionOption[];
  modeId?: string;
  modeConfigId?: string;
  options: ConfigControl[];
}

export interface Usage {
  used: number;
  size: number;
  cost?: number;
  /** A compaction finished and the agent has not reported since: `used` predates it, this is that figure */
  compactedFrom?: number;
}

export interface SlashCommandInput {
  hint: string;
}

export interface SlashCommand {
  name: string;
  description: string;
  input?: SlashCommandInput;
}

export interface TextContext {
  sourceId: string;
  sourceName: string;
  uri?: string;
  comment?: string;
}

/**
 * An attachment in every shape (transcript.ts Attachment, `kind` says which). In the composer an image's or a pasted
 * text's `blob` holds its base64 until the engine stores it and names the blob (`<hash>.<ext>`, which base64 never
 * holds); a file picked on the phone keeps its bytes in `data` (base64), since its path means nothing on the computer
 */
export interface Attachment {
  kind: string;
  blob?: string;
  mimeType?: string;
  name?: string;
  uri?: string;
  startLine?: number;
  endLine?: number;
  text?: string;
  comment?: string;
  context?: TextContext;
  /** The phone's own copy of a picked file, base64 (composer only, never on the wire as an Attachment) */
  data?: string;
  /** A picked photo's local file, for its thumbnail in the strip (composer only) */
  thumb?: string;
}

/** What the composer hands the engine with a prompt (protocol.ts Draft): images and texts carry their payload */
export interface Draft {
  kind: string;
  mimeType?: string;
  data?: string;
  name?: string;
  text?: string;
  uri?: string;
  startLine?: number;
  endLine?: number;
  comment?: string;
  context?: TextContext;
}

export interface QueuedPrompt {
  id: string;
  text: string;
  attachments?: Attachment[];
  sending?: boolean;
}

export interface TurnSettings {
  modeId?: string;
  config: Record<string, string>;
}

export interface TurnError {
  message: string;
  code?: number;
  kind?: string;
  retryable?: boolean;
  /** retry / login / new_session: the remedies an AIR failure declared */
  actions?: string[];
}

export interface QuestionOption {
  id: string;
  label: string;
  description?: string;
}

export interface Question {
  id: string;
  title?: string;
  text: string;
  /** single / multiple / text */
  kind: string;
  options: QuestionOption[];
  other?: boolean;
  numeric?: boolean;
  required?: boolean;
}

export type QuestionAnswer = string | string[];

export interface PlanEntry {
  title: string;
  /** pending / in_progress / completed */
  status: string;
  priority?: string;
}

export interface SessionGoal {
  objective: string;
  /** active / paused / blocked / limited / complete */
  status: string;
  iterations?: number;
  tokenBudget?: number;
  tokensUsed?: number;
  timeUsedSeconds?: number;
}

export interface FileHit {
  uri: string;
  path: string;
}

export interface EditTurnRequest {
  sessionId: string;
  turnIndex: number;
  turnCount: number;
  originalText: string;
  turnId?: string;
  text: string;
  retainedAttachments: number[];
  attachments: Draft[];
  settings: TurnSettings;
}

/** The composer's HostMsg replies: `files` (searchFiles) and `editTurnResult` (editTurn) */
export interface ComposerHostMsg {
  type: string;
  seq?: number;
  files?: FileHit[];
  requestId?: string;
  error?: string;
  /** `controls`: what a new session of `agent` offers (in `cwd` on a remote host) */
  agent?: string;
  controls?: ConfigControl[];
  commands?: SlashCommand[];
  cwd?: string;
}

/** `view` with `turns` in place of its own, every other field kept (ArkTS has no object spread) */
export function viewWithTurns(view: SessionView, turns: Turn[]): SessionView {
  const out: SessionView = {
    id: view.id, agent: view.agent, title: view.title, cwd: view.cwd, status: view.status, running: view.running,
    rev: view.rev, turns: turns, controls: view.controls, usage: view.usage, commands: view.commands, queued: view.queued,
    canSteer: view.canSteer, goal: view.goal, goalActions: view.goalActions, subagents: view.subagents,
    earlier: view.earlier
  };
  return out;
}
