// The wire the app speaks with the engine: the envelopes of packages/shared/sidecar.ts around the HostMsg / WebviewMsg
// of packages/shared/protocol.ts, and the transcript shapes of packages/shared/transcript.ts. Only the fields this app
// reads are declared; JSON.parse keeps the rest. Same contract as apps/ios AcpiraModels and apps/android :model.

/** Must equal SIDECAR_PROTOCOL_VERSION in packages/shared/sidecar.ts; the engine refuses any other value at hello */
export const SIDECAR_PROTOCOL_VERSION: number = 1;

export interface AgentInfo {
  id: string;
  name: string;
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
}

/** Every HostMsg in one shape: `type` says which fields carry it */
export interface HostMsg {
  type: string;
  state?: InitState;
  sessions?: SessionSummary[];
  agents?: AgentInfo[];
  session?: SessionView;
  text?: string;
}

export interface SessionPatchMsg {
  type: string;
  patch: SessionPatch;
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
    const merged: Turn = {
      role: head.role, id: head.id, stop: head.stop, startedAt: head.startedAt, endedAt: head.endedAt, blocks: kept.slice(0, p.keepBlocks).concat(p.blocks ?? [])
    };
    tail[0] = merged;
  }
  const view: SessionView = {
    id: p.view.id, agent: p.view.agent, title: p.view.title, cwd: p.view.cwd, status: p.view.status,
    running: p.view.running, rev: p.view.rev, turns: current.turns.slice(0, p.keep).concat(tail)
  };
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
