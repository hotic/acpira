import type { GitMark, HostMsg, ProjectBase, ProjectChange, ProjectEntry, ProjectHunk, WebviewMsg } from '@shared/protocol';
import { vscodeApi } from '../vscodeApi';

// The desktop tool panel's reads of a session's project, answered by the engine (rust/.../project_files.rs) with the
// request's seq. A read nobody answers (an engine from before these messages) settles empty after the timeout, with
// `timeout` as its error

type ProjectReply = Extract<HostMsg, { type: 'projectDir' | 'projectStatus' | 'projectFile' | 'sessionChanges' }>;
type ProjectAsk = Extract<WebviewMsg, { type: ProjectReply['type'] }>;

// A git status of a large tree or a file near the size cap can take a while; nothing waits on it but the panel
const TIMEOUT = 20000;
let seq = 0;
const waits = new Map<number, (m: ProjectReply | undefined) => void>();

// App's message switch hands every reply here
export function settleProject(m: ProjectReply) {
  const wait = waits.get(m.seq);
  waits.delete(m.seq);
  wait?.(m);
}

function ask<T extends ProjectReply['type']>(type: T, rest: Omit<Extract<ProjectAsk, { type: T }>, 'seq' | 'type'>): Promise<Extract<ProjectReply, { type: T }> | undefined> {
  return new Promise(resolve => {
    const mine = ++seq;
    waits.set(mine, m => resolve(m as Extract<ProjectReply, { type: T }> | undefined));
    setTimeout(() => { if (waits.delete(mine)) resolve(undefined); }, TIMEOUT);
    vscodeApi().postMessage({ type, ...rest, seq: mine });
  });
}

export interface DirListing { entries: ProjectEntry[]; error?: string }
export interface GitStatus { root?: string; marks: Record<string, GitMark> }
export interface FileRead { text?: string; hunks: ProjectHunk[]; error?: string }

// The last answers, per project root (folder listings, git status) and per session (its changes): a Files view mounted
// again (a session switch remounts the tool panel) paints them at once and refreshes behind them, instead of flashing an
// empty tree and then unmarked rows. A few small objects per project or session viewed in this window
export const lastRead = {
  dirs: new Map<string, Record<string, DirListing>>(),
  status: new Map<string, GitStatus>(),
  changes: new Map<string, ProjectChange[]>(),
};
// A fresh answer equal to the one shown keeps the shown object, so nothing re-renders for it
export const sameRead = (a: unknown, b: unknown) => a === b || JSON.stringify(a) === JSON.stringify(b);

export const project = {
  async dir(sessionId: string, dir: string): Promise<DirListing> {
    const m = await ask('projectDir', { sessionId, dir });
    return m ? { entries: m.entries, ...(m.error ? { error: m.error } : {}) } : { entries: [], error: 'timeout' };
  },
  async status(sessionId: string): Promise<GitStatus> {
    const m = await ask('projectStatus', { sessionId });
    return { ...(m?.root ? { root: m.root } : {}), marks: m?.marks ?? {} };
  },
  async file(sessionId: string, path: string, base?: ProjectBase): Promise<FileRead> {
    const m = await ask('projectFile', { sessionId, path, ...(base ? { base } : {}) });
    if (!m) return { hunks: [], error: 'timeout' };
    return { ...(m.text !== undefined ? { text: m.text } : {}), hunks: m.hunks ?? [], ...(m.error ? { error: m.error } : {}) };
  },
  async changes(sessionId: string): Promise<ProjectChange[]> {
    return (await ask('sessionChanges', { sessionId }))?.files ?? [];
  },
};
