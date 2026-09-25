import { describe, expect, it } from 'vitest';
import type { AgentTurn } from '../src/shared/transcript';
import { compactionForDisplay } from '../src/webview/chat/compactionDisplay';

const reply = (markdown: string, stop: AgentTurn['stop'] = 'end_turn'): AgentTurn => ({ role: 'agent', stop, blocks: [{ type: 'text', markdown }] });

describe('compaction presentation', () => {
  it.each([
    'Compacting context...Context compacted',
    'Compacting context…Context compacted',
    'Context compaction started — it runs in the background and the compacted context applies once it finishes.Compaction completed.\n- Messages compacted: 3',
    'Nothing to compact.',
  ])('collapses known CLI output into one completed status: %s', text => {
    expect(compactionForDisplay(reply(text), false).blocks).toEqual([{ type: 'compaction', id: 'compact-display', status: 'completed' }]);
  });

  it('keeps background progress pending after an RPC acknowledgement', () => {
    expect(compactionForDisplay(reply('Compacting context…'), true).blocks[0]).toMatchObject({ status: 'in_progress' });
    expect(compactionForDisplay(reply('Compac'), true).blocks[0]).toMatchObject({ status: 'in_progress' });
    const incomplete = reply('Compacting context…');
    expect(compactionForDisplay(incomplete, false)).toBe(incomplete);
  });

  it('retains the original transcript and preserves failure details', () => {
    const original = reply('Compaction failed: service unavailable');
    const view = compactionForDisplay(original, false);
    expect(view.blocks[0]).toMatchObject({ status: 'failed' });
    expect(view.blocks[1]).toEqual(original.blocks[0]);
    expect(original.blocks).toHaveLength(1);
    expect(compactionForDisplay(reply('Compaction cancelled.'), false).blocks[0]).toMatchObject({ status: 'cancelled' });
  });

  it('deduplicates structured status and text without hiding unrelated blocks', () => {
    const original: AgentTurn = { ...reply('Context compacted'), blocks: [
      { type: 'compaction', id: 'c', status: 'completed' },
      { type: 'text', markdown: 'Context compacted' },
      { type: 'thought', text: 'Preserved detail' },
    ] };
    expect(compactionForDisplay(original, false).blocks).toEqual([
      { type: 'compaction', id: 'c', status: 'completed' },
      { type: 'thought', text: 'Preserved detail' },
    ]);
  });

  it('keeps the host-recorded failure reason on the collapsed status', () => {
    const turn: AgentTurn = { role: 'agent', stop: 'end_turn', blocks: [{ type: 'compaction', id: 'text-compaction-1', status: 'failed', error: 'Compaction failed: too large' }] };
    expect(compactionForDisplay(turn, false).blocks).toEqual([{ type: 'compaction', id: 'text-compaction-1', status: 'failed', error: 'Compaction failed: too large' }]);
  });

  it('does not turn unrecognized output into a successful compaction', () => {
    const original = reply('The command is not supported.');
    expect(compactionForDisplay(original, false)).toBe(original);
  });
});
