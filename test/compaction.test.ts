import { describe, expect, it } from 'vitest';
import { CompactionCompletion, isCompactCommand } from '../src/host/acp/compaction';

const text = (value: string) => ({ sessionUpdate: 'agent_message_chunk' as const, content: { type: 'text' as const, text: value } });

describe('compaction completion signals', () => {
  it.each([
    ['devin', 'Context compacted'],
    ['devin', 'Nothing to compact.'],
    ['devin', 'Compaction canceled.'],
    ['devin', 'Force compaction failed: unavailable'],
    ['devin', 'Compaction failed: unavailable'],
    ['kimi', 'Compaction completed.\n- Tokens after: 1234'],
    ['kimi', 'Compaction cancelled.'],
    ['kimi', 'Compaction is blocked by the current turn; retry when the turn is idle.'],
    ['kimi', '/compact failed: No messages to compact in current history.'],
  ])('%s releases on fragmented terminal text: %s', async (agent, message) => {
    const c = new CompactionCompletion(agent);
    const waiting = c.wait();
    expect(waiting).toBeInstanceOf(Promise);
    for (const character of message) c.update(text(character));
    await waiting;
    expect(c.wait()).toBeUndefined();
  });

  it('a completion received before the RPC acknowledgement needs no later wait', () => {
    const c = new CompactionCompletion('devin');
    c.update(text('Nothing to compact.'));
    expect(c.wait()).toBeUndefined();
  });

  it('ordinary model prose and synchronous peers do not arm a text latch', () => {
    for (const agent of [undefined, 'grok', 'custom']) {
      const c = new CompactionCompletion(agent);
      c.update(text('Compacting context…'));
      expect(c.wait()).toBeUndefined();
    }
  });

  it('structured events replace the text fallback and await every active compaction ID', async () => {
    const c = new CompactionCompletion('devin');
    for (const id of ['a', 'b']) c.update({ sessionUpdate: 'compaction_update', compactionId: id, status: 'in_progress' });
    let released = false;
    const waiting = c.wait()!.then(() => { released = true; });
    c.update(text('Context compacted'));
    c.update({ sessionUpdate: 'compaction_update', compactionId: 'a', status: 'completed' });
    await Promise.resolve();
    expect(released).toBe(false);
    c.update({ sessionUpdate: 'compaction_update', compactionId: 'b', status: 'failed' });
    await waiting;
  });

  it('process failure or disposal releases an outstanding wait', async () => {
    const c = new CompactionCompletion('kimi');
    const waiting = c.wait();
    c.close();
    await waiting;
    expect(c.wait()).toBeUndefined();
  });

  it('captures the self-reported post-compaction token count, even past the latch release', async () => {
    const c = new CompactionCompletion('kimi');
    const waiting = c.wait();
    for (const character of 'Compaction completed.\n- Messages compacted: 3\n- Tokens after: 24,898') c.update(text(character));
    await waiting;
    expect(c.tokensAfter).toBe(24898);
  });

  it('reports no token count for cancellations, failures, or prose without a result line', () => {
    for (const message of ['Compaction cancelled.', '/compact failed: No messages to compact in current history.', 'Context compacted']) {
      const c = new CompactionCompletion('kimi');
      for (const character of message) c.update(text(character));
      expect(c.tokensAfter).toBeUndefined();
    }
  });

  it('never mines token counts from ordinary prose on a non-compact turn', () => {
    const c = new CompactionCompletion(undefined);
    for (const character of 'Compaction completed.\n- Tokens after: 42') c.update(text(character));
    expect(c.tokensAfter).toBeUndefined();
  });

  it('recognizes compact instructions without matching unrelated slash commands', () => {
    expect(isCompactCommand(' /compact focus on the tests ')).toBe(true);
    expect(isCompactCommand('/compact')).toBe(true);
    expect(isCompactCommand('/compactor')).toBe(false);
    expect(isCompactCommand('describe /compact')).toBe(false);
  });
});
