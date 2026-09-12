import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { grokContextUsage } from '../src/host/acp/grokUsage';
import { AcpSession } from '../src/host/acp/AcpSession';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';

const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));
const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));

function fixture(style = 'context', auto = false) {
  const logs: string[] = [];
  const registry = new AgentRegistry({ grok: { command: TSX, args: [FAKE], env: { FAKE_GROK_USAGE: style } } });
  const session = AcpSession.fresh('grok', '/tmp', {
    registry, log: line => logs.push(line), onChange: () => {}, compaction: () => ({ auto, atTokens: 300_000 }),
    blobs: { saveBlob: async () => { throw new Error('No attachments expected'); }, readBlob: async () => new Uint8Array() },
  });
  return { session, logs };
}

describe('Grok context snapshots', () => {
  it('accepts exact, zero and overfull windows without adding spend or cache buckets', () => {
    for (const used of [0, 16378, 260000]) {
      expect(grokContextUsage({ result: { sessionId: 's1', context: { used, total: 250000 } }, _meta: { usage: { totalTokens: 999999 } } }, 's1'))
        .toEqual({ used, size: 250000 });
    }
  });

  it.each([null, {}, { _meta: { usage: { inputTokens: 100, totalTokens: 120 } } },
    { result: { sessionId: 'other', context: { used: 1, total: 2 } } },
    ...[-1, NaN, Infinity, '100', 1.5].map(used => ({ result: { sessionId: 's1', context: { used, total: 250000 } } })),
    ...[0, -1, Infinity, '250000'].map(total => ({ result: { sessionId: 's1', context: { used: 10, total } } })),
  ])('rejects unknown, mismatched or invalid usage: %j', value => {
    expect(grokContextUsage(value, 's1')).toBeUndefined();
  });

  it('publishes and persists live context, refreshes model size, and ignores prompt spend', async () => {
    const { session: s } = fixture();
    try {
      await s.start();
      expect(s.view().usage).toEqual({ used: 1234, size: 1_000_000 });
      await s.prompt('big');
      expect(s.view().usage).toEqual({ used: 401234, size: 1_000_000 });
      expect(s.toRecord().usage).toEqual(s.view().usage);
      await s.setConfig('model', 'm2');
      expect(s.view().usage).toEqual({ used: 401234, size: 250_000 });
      await s.compact();
      expect(s.view().usage).toEqual({ used: 80247, size: 250_000 });
    } finally { s.dispose(); }
  });

  it('drives automatic compaction from the live snapshot and refreshes after compaction', async () => {
    const { session: s } = fixture('context', true);
    try {
      await s.start();
      await s.prompt('big');
      await expect.poll(() => s.view().usage?.used).toBe(80247);
      await expect.poll(() => s.isRunning).toBe(false);
      expect(s.view().turns.filter(t => t.role === 'user' && t.auto)).toHaveLength(1);
      await s.prompt('hi');
      expect(s.view().turns.filter(t => t.role === 'user' && t.auto)).toHaveLength(1);
    } finally { s.dispose(); }
  });

  it.each(['unsupported', 'malformed'])('keeps %s context unknown without breaking prompts', async style => {
    const { session: s, logs } = fixture(style, true);
    try {
      await s.start();
      await s.prompt('big');
      expect(s.view().status).toBe('ready');
      expect(s.view().usage).toBeUndefined();
      expect(s.view().turns).toHaveLength(2);
      if (style === 'unsupported') expect(logs.filter(l => l.includes('context unavailable'))).toHaveLength(1);
    } finally { s.dispose(); }
  });

  it('refreshes context while a turn is still on the wire', async () => {
    const { session: s } = fixture();
    try {
      await s.start();
      expect(s.view().usage).toEqual({ used: 1234, size: 1_000_000 });
      const prompt = s.prompt('slow');
      await expect.poll(() => (s.view().usage?.used ?? 0) > 1234, { timeout: 4_000 }).toBe(true);
      expect(s.isRunning).toBe(true);
      await prompt;
      expect(s.view().usage?.used).toBeGreaterThan(1234);
      expect(s.view().usage?.size).toBe(1_000_000);
    } finally { s.dispose(); }
  });

  it('preserves standard usage notifications over the Grok fallback', async () => {
    const { session: s } = fixture();
    try {
      await s.start();
      const prompt = s.prompt('tool');
      await expect.poll(() => s.view().turns.some(t => t.role === 'agent' && t.blocks.some(b => b.type === 'permission'))).toBe(true);
      const permission = s.view().turns.flatMap(t => t.role === 'agent' ? t.blocks : []).find(b => b.type === 'permission')!;
      s.resolvePermission(permission.id, 'reject');
      await prompt;
      expect(s.view().usage).toEqual({ used: 1234, size: 100000, cost: 0.01 });
    } finally { s.dispose(); }
  });

  it('continues polling during quiet model work without text events', async () => {
    const { session } = fixture();
    try {
      await session.start();
      const prompt = session.prompt('quiet-context');
      await expect.poll(() => session.view().usage?.used, { timeout: 2_650 }).toBe(42_000);
      expect(session.isRunning).toBe(true);
      await prompt;
    } finally { session.dispose(); }
  });
});
