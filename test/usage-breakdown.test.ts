import { describe, expect, it } from 'vitest';
import { compactBudget, liveUsage, overCompactBudget, usageWindow } from '../src/webview/chat/usageBreakdown';
import type { Turn, Usage } from '../src/shared/transcript';

const latin = (n: number) => 'a'.repeat(n);

describe('usageWindow', () => {
  it('uses the reported model window, independently of the compact budget', () => {
    expect(usageWindow(1_000_000)).toBe(1_000_000);
    expect(usageWindow(200_000)).toBe(200_000);
  });
});

describe('compactBudget', () => {
  it('only marks a budget strictly inside the agent window', () => {
    expect(compactBudget(1_000_000, 300_000)).toBe(300_000);
    expect(compactBudget(200_000, 300_000)).toBeUndefined();
    expect(compactBudget(300_000, 300_000)).toBeUndefined();
    expect(compactBudget(1_000_000)).toBeUndefined();
  });

  it('treats usage at or above the threshold as over budget', () => {
    expect(overCompactBudget(345_000, 300_000)).toBe(true);
    expect(overCompactBudget(300_000, 300_000)).toBe(true);
    expect(overCompactBudget(299_999, 300_000)).toBe(false);
    expect(overCompactBudget(345_000)).toBe(false);
  });
});

describe('liveUsage', () => {
  const usage: Usage = { used: 100, size: 10_000 };
  const turns: Turn[] = [
    { role: 'user', text: latin(400) },
    { role: 'agent', blocks: [{ type: 'text', markdown: latin(400) }] },
  ];

  it('keeps the agent total when idle even if the transcript estimate is larger', () => {
    expect(liveUsage(usage, turns)).toBe(usage);
    expect(liveUsage(usage, turns, false)).toBe(usage);
  });

  it('never substitutes retained history for the live window while running', () => {
    expect(liveUsage(usage, turns, true)).toBe(usage);
  });

  it('does not resurrect nearly a million tokens of retained tools after compaction', () => {
    const compacted = { used: 24_000, size: 200_000 };
    const history: Turn[] = [{ role: 'agent', blocks: [{ type: 'tool_call', id: 'large', kind: 'read', verb: 'Read', status: 'completed',
      content: { type: 'text', text: latin(3_984_000) } }] }];
    expect(liveUsage(compacted, history, true)).toBe(compacted);
  });

  it('does not drop below the agent snapshot', () => {
    expect(liveUsage({ used: 50_000, size: 10_000 }, turns, true)).toEqual({ used: 50_000, size: 10_000 });
  });
});
