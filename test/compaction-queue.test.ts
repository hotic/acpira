import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AcpSession } from '../src/host/acp/AcpSession';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));

function fixture(agent: string, auto = false, env: Record<string, string> = {}) {
  const logs: string[] = [];
  const session = AcpSession.fresh(agent, '/tmp', {
    registry: new AgentRegistry({ [agent]: { command: TSX, args: [FAKE], env: { FAKE_COMPACTION: agent, ...env } } }),
    log: line => logs.push(line), onChange: () => {},
    blobs: { saveBlob: async () => { throw new Error('No attachments'); }, readBlob: async () => { throw new Error('No attachments'); } },
    compaction: () => ({ auto, atTokens: 300_000 }),
  });
  return { session, logs };
}

async function until(predicate: () => boolean) {
  const deadline = Date.now() + 5_000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error('Timed out waiting for the fake ACP peer');
    await new Promise(resolve => setTimeout(resolve, 10));
  }
}

describe('background compaction queue', () => {
  it('kimi: cancellation releases a pending usage refresh without triggering late compaction', async () => {
    const { session, logs } = fixture('kimi', true);
    try {
      await session.start();
      const prompt = session.prompt('delayed-usage');
      await until(() => logs.some(line => line.includes('prompt done:')));
      await session.cancel();
      await prompt;
      expect(session.isRunning).toBe(false);
      await until(() => session.view().usage?.used === 350_000);
      expect(session.view().turns.some(turn => turn.role === 'user' && turn.auto)).toBe(false);
      expect(session.view().turns.at(-1)).toMatchObject({ role: 'agent', stop: 'cancelled' });
    } finally { session.dispose(); }
  });
  it('kimi: waits for the post-response usage before dispatching a queued follow-up', async () => {
    const { session, logs } = fixture('kimi', true);
    try {
      await session.start();
      const prompt = session.prompt('delayed-usage');
      await until(() => logs.some(line => line.includes('prompt done:')));
      await session.prompt('follow-up');
      expect(session.view().turns.filter(turn => turn.role === 'user').map(turn => turn.text)).toEqual(['delayed-usage']);
      await until(() => logs.filter(line => line.includes('prompt done:')).length === 2);
      await session.setConfig('effort', 'high');
      await prompt;
      await until(() => !session.isRunning);
      expect(session.view().turns.filter(turn => turn.role === 'user').map(turn => turn.text)).toEqual(['delayed-usage', '/compact', 'follow-up']);
    } finally { session.dispose(); }
  });
  it.each(['devin', 'kimi'])('%s: evaluates usage arriving after the prompt acknowledgement', async agent => {
    const { session, logs } = fixture(agent, true);
    try {
      await session.start();
      await session.prompt('delayed-usage');
      if (agent === 'devin') expect(session.isRunning).toBe(false);
      await until(() => logs.some(line => line.includes('auto /compact')));
      expect(session.view().turns.filter(turn => turn.role === 'user' && turn.auto)).toHaveLength(1);
      await until(() => logs.filter(line => line.includes('prompt done:')).length === 2);
      await session.setConfig('effort', 'high');
      await until(() => !session.isRunning);
    } finally { session.dispose(); }
  });
  it.each(['devin', 'kimi', 'structured'])('%s: holds a follow-up after the compact RPC returns until compaction completes', async agent => {
    const { session: s, logs } = fixture(agent);
    try {
      await s.start();
      await s.prompt('hi');
      logs.length = 0;
      const compact = s.compact();
      await until(() => logs.some(l => l.includes('prompt done:')));
      expect(s.isRunning).toBe(true);
      await s.prompt('follow-up');
      expect(s.view().queued?.map(q => q.text)).toEqual(['follow-up']);
      expect(s.view().turns).toHaveLength(4);
      await s.setConfig('effort', 'low');
      expect(s.isRunning).toBe(true);
      await s.setConfig('effort', 'high');
      await compact;
      await until(() => !s.isRunning);
      expect(s.view().queued).toBeUndefined();
      expect(s.view().turns).toHaveLength(6);
      expect(s.view().turns[4]).toMatchObject({ role: 'user', text: 'follow-up' });
      expect(s.view().turns[5]).toMatchObject({ role: 'agent', stop: 'end_turn', blocks: expect.arrayContaining([expect.objectContaining({ type: 'text', markdown: 'hello world' })]) });
      expect(logs.some(l => /\] cancel$/.test(l))).toBe(false);
    } finally { s.dispose(); }
  });

  it.each(['devin', 'kimi'])('%s: auto-compaction keeps the queue and records usage after completion', async agent => {
    const { session: s, logs } = fixture(agent, true);
    try {
      await s.start();
      await s.prompt('big');
      await until(() => logs.filter(l => l.includes('prompt done:')).length === 2);
      expect(s.isRunning).toBe(true);
      expect(s.view().turns[2]).toEqual({ role: 'user', text: '/compact', auto: true });
      // 401234 from the "big" prompt; the fake compaction drops it to a fifth
      const before = s.view().usage!.used;
      expect(before).toBe(401234);
      await s.prompt('follow-up');
      await s.setConfig('effort', 'high');
      await until(() => !s.isRunning);
      expect(s.view().turns).toHaveLength(6);
      // Kimi pushes no usage_update here: the reading is adopted from its completion prose
      expect(s.view().usage?.used).toBe(Math.round(before * 0.2));
    } finally { s.dispose(); }
  });

  it('cancel releases a background wait even when the agent never confirms the cancellation in prose', async () => {
    const { session: s, logs } = fixture('devin', false, { FAKE_SILENT_CANCEL: '1' });
    try {
      await s.start();
      await s.prompt('hi');
      logs.length = 0;
      const compact = s.compact();
      await until(() => logs.some(l => l.includes('waiting for compaction completion')));
      expect(s.isRunning).toBe(true);
      await s.cancel();
      await compact;
      expect(s.isRunning).toBe(false);
      expect(s.view().status).toBe('ready');
      await s.prompt('after');
      expect(s.view().turns.at(-2)).toMatchObject({ role: 'user', text: 'after' });
    } finally { s.dispose(); }
  });

  it('dispose releases a background wait without dispatching the queued prompt', async () => {
    const { session: s, logs } = fixture('devin');
    try {
      await s.start();
      await s.prompt('hi');
      logs.length = 0;
      const compact = s.compact();
      await until(() => logs.some(l => l.includes('prompt done:')));
      await s.prompt('follow-up');
      s.dispose();
      await compact;
      expect(s.view().status).toBe('closed');
      expect(s.view().turns).toHaveLength(4);
    } finally { s.dispose(); }
  });
});
