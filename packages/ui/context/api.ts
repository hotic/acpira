import type { ContextContent, ContextRaw, ContextReport } from '@acpira/shared/context';
import type { HostMsg } from '@acpira/shared/protocol';
import { requestTable } from '../requestTable';
import { vscodeApi } from '../vscodeApi';

// A session's context split (@acpira/shared/context), asked of the engine with a seq and answered with the same one
// (bridge_core.rs forwards to the agent). Only an agent that splits its own requests answers with one; for every other
// agent, and from an engine older than these messages (after the timeout), there is none

type ContextReply = Extract<HostMsg, { type: 'sessionContext' | 'sessionContextItem' | 'sessionContextRaw' }>;

const requests = requestTable<ContextReply | undefined>(10000, undefined);

// The page's message switch hands every reply here
export const settleContext = (m: ContextReply) => requests.settle(m.seq, m);

const post = (m: object) => vscodeApi().postMessage(m as never);

// The last split per session, so the ring's card opens with its parts at once and refreshes behind them
export const lastSplit = new Map<string, ContextReport | null>();

export const contextApi = {
  async report(sessionId: string, detail = false): Promise<ContextReport | undefined> {
    const m = await requests.ask(seq => post({ type: 'sessionContext', sessionId, seq, ...(detail ? { detail } : {}) }));
    return m?.type === 'sessionContext' ? m.report : undefined;
  },
  async item(sessionId: string, rev: string, id: string): Promise<ContextContent | undefined> {
    const m = await requests.ask(seq => post({ type: 'sessionContextItem', sessionId, rev, id, seq }));
    return m?.type === 'sessionContextItem' ? m.content : undefined;
  },
  async raw(sessionId: string): Promise<{ raw?: ContextRaw; error?: string }> {
    const m = await requests.ask(seq => post({ type: 'sessionContextRaw', sessionId, seq }));
    if (m?.type !== 'sessionContextRaw') return { error: 'timeout' };
    return { ...(m.raw ? { raw: m.raw } : {}), ...(m.error ? { error: m.error } : {}) };
  },
};
