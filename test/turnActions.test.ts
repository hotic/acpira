import { describe, expect, it } from 'vitest';
import type { AgentInfo, AgentTurn, SessionControls, ToolCallBlock } from '../src/shared/transcript';
import { forkTargets, modelLabel, replyMarkdown, turnOutputTps, turnTtft } from '../src/webview/chat/turnActionHelpers';

const text = (markdown: string) => ({ type: 'text' as const, markdown });
const read: ToolCallBlock = { type: 'tool_call', id: 'r1', kind: 'read', verb: 'Read', target: 'a.ts', status: 'completed' };
const run: ToolCallBlock = { type: 'tool_call', id: 'r2', kind: 'execute', verb: 'Run', target: 'pnpm test', status: 'completed' };
const turn = (blocks: AgentTurn['blocks']): AgentTurn => ({ role: 'agent', blocks });

const CONTROLS: SessionControls = {
  modes: [],
  options: [
    { id: 'model', name: 'Model', category: 'model', options: [{ id: 'm-1', name: 'Model One' }, { id: 'm-2', name: 'Model Two' }] },
    { id: 'thinking', name: 'Thinking', category: 'thought_level', options: [] },
  ],
};

describe('replyMarkdown', () => {
  it('copies only the trailing reply when prose precedes a tool call', () => {
    expect(replyMarkdown(turn([text('commentary'), read, text('the answer')]))).toBe('the answer');
  });

  it('joins consecutive prose with a blank line when nothing is foldable', () => {
    expect(replyMarkdown(turn([text('first'), text('second')]))).toBe('first\n\nsecond');
  });

  it('falls back to every text block when the reply heuristic finds none', () => {
    // A trailing thought joins the tail, so no text qualifies as reply; the prose still copies
    expect(replyMarkdown(turn([text('the answer'), { type: 'thought', text: 'recheck' }]))).toBe('the answer');
  });

  it('returns an empty string for a tools-only turn', () => {
    expect(replyMarkdown(turn([read, run]))).toBe('');
  });
});

describe('modelLabel', () => {
  it('resolves the usage model id to its option name', () => {
    expect(modelLabel({ model: 'm-2' }, undefined, CONTROLS)).toBe('Model Two');
  });

  it('falls back to the model captured in the user turn settings', () => {
    expect(modelLabel(undefined, { config: { model: 'm-1' } }, CONTROLS)).toBe('Model One');
  });

  it('prefers the peer-reported model over the captured setting', () => {
    expect(modelLabel({ model: 'm-2' }, { config: { model: 'm-1' } }, CONTROLS)).toBe('Model Two');
  });

  it('shows an unknown id as-is', () => {
    expect(modelLabel({ model: 'grok-4.6' }, undefined, CONTROLS)).toBe('grok-4.6');
    expect(modelLabel(undefined, { config: { model: 'nightly-build' } }, CONTROLS)).toBe('nightly-build');
  });

  it('is undefined when neither the peer nor the settings name a model', () => {
    expect(modelLabel(undefined, undefined, CONTROLS)).toBeUndefined();
    expect(modelLabel(undefined, { config: {} }, CONTROLS)).toBeUndefined();
    expect(modelLabel(undefined, { config: { thinking: 'high' } }, CONTROLS)).toBeUndefined();
  });
});

describe('turnTtft', () => {
  it('formats the latency from startedAt to respondedAt', () => {
    expect(turnTtft({ ...turn([text('hi')]), startedAt: 1_000, respondedAt: 1_580 })).toBe('0.6s');
    expect(turnTtft({ ...turn([text('hi')]), startedAt: 1_000, respondedAt: 1_040 })).toBe('<0.1s');
  });

  it('is undefined when either timestamp is missing', () => {
    expect(turnTtft({ ...turn([text('hi')]), startedAt: 1_000 })).toBeUndefined();
    expect(turnTtft({ ...turn([text('hi')]), respondedAt: 1_500 })).toBeUndefined();
  });
});

describe('turnOutputTps', () => {
  it('computes decode throughput after respondedAt using (tokens - 1) / decodeSeconds', () => {
    // 167 tokens over 1100 ms decode window -> 166 / 1.1s ≈ 151 tok/s
    expect(turnOutputTps({
      ...turn([text('hi')]),
      startedAt: 1_000, respondedAt: 1_600, endedAt: 2_700,
      usage: { input: 18_951, output: 167, total: 19_118 },
    })).toBe(151);
  });

  it('includes separate reasoning tokens without double-counting inclusive ones', () => {
    // OpenAI-style: input (100) + output (101) === total (201), reasoning (60) is already inside output
    expect(turnOutputTps({
      ...turn([text('hi')]),
      startedAt: 0, respondedAt: 500, endedAt: 1_500,
      usage: { input: 100, output: 101, reasoning: 60, total: 201 },
    })).toBe(100);
    // Separate thoughtTokens: reasoning (150) > output (51) -> 201 generated tokens over 2s = 100 tok/s
    expect(turnOutputTps({
      ...turn([text('hi')]),
      startedAt: 0, respondedAt: 500, endedAt: 2_500,
      usage: { input: 100, output: 51, reasoning: 150 },
    })).toBe(100);
  });

  it('subtracts merged foreground tool execution spans and ignores background tools', () => {
    // Decode window [1000, 7000] = 6000 ms, minus merged foreground tools [2000, 5000] U [3000, 6000] = 4000 ms -> 2000 ms active
    expect(turnOutputTps({
      role: 'agent',
      startedAt: 500, respondedAt: 1_000, endedAt: 7_000,
      blocks: [
        { ...read, startedAt: 2_000, endedAt: 5_000 },
        { ...run, startedAt: 3_000, endedAt: 6_000 },
        { ...run, id: 'bg', background: true, startedAt: 1_000, endedAt: 7_000 },
        text('done'),
      ],
      usage: { output: 201 },
    })).toBe(100);
  });

  it('measures only after the last foreground tool when the tally covers just the last model call (Devin)', () => {
    const blocks = [
      { ...read, startedAt: 2_000, endedAt: 5_000 },
      { ...run, startedAt: 3_000, endedAt: 6_000 },
      { ...run, id: 'bg', background: true, startedAt: 1_000, endedAt: 7_000 },
      text('done'),
    ];
    // input + output equals the end-of-turn context: one call's tally, window [6000, 8000] = 2000 ms
    expect(turnOutputTps({
      role: 'agent', startedAt: 500, respondedAt: 1_000, endedAt: 8_000, blocks,
      usage: { input: 99_799, output: 201, total: 100_000, context: { used: 100_000, size: 200_000 } },
    })).toBe(100);
    // An explicit single model call does the same without a context snapshot
    expect(turnOutputTps({
      role: 'agent', startedAt: 500, respondedAt: 1_000, endedAt: 8_000, blocks,
      usage: { output: 201, modelCalls: 1 },
    })).toBe(100);
    // A per-turn tally far above the context keeps the whole window minus tools: 7000 - 4000 = 3000 ms
    expect(turnOutputTps({
      role: 'agent', startedAt: 500, respondedAt: 1_000, endedAt: 8_000, blocks,
      usage: { input: 400_000, output: 301, total: 400_301, context: { used: 100_000, size: 200_000 } },
    })).toBe(100);
  });

  it('is undefined for tiny token counts, sub-200ms single-chunk dumps, or missing respondedAt', () => {
    expect(turnOutputTps({ ...turn([text('ok')]), startedAt: 0, respondedAt: 500, endedAt: 1_500, usage: { output: 3 } })).toBeUndefined();
    expect(turnOutputTps({ ...turn([text('buffered')]), startedAt: 0, respondedAt: 1_450, endedAt: 1_500, usage: { output: 100 } })).toBeUndefined();
    expect(turnOutputTps({ ...turn([text('legacy')]), startedAt: 0, endedAt: 2_000, usage: { output: 100 } })).toBeUndefined();
  });
});

describe('forkTargets', () => {
  const agents: AgentInfo[] = [
    { id: 'claude', name: 'Claude Code' },
    { id: 'codex', name: 'Codex', available: false },
    { id: 'devin', name: 'Devin' },
    { id: 'kimi', name: 'Kimi', disabled: true },
    { id: 'chatgpt', name: 'ChatGPT', external: true },
  ];

  it('puts the session agent first and keeps the plus menu order after it', () => {
    expect(forkTargets(agents, 'devin').map(a => a.id)).toEqual(['devin', 'claude', 'codex']);
  });

  it('keeps an unavailable agent in the list for the menu to grey out', () => {
    expect(forkTargets(agents, 'claude').find(a => a.id === 'codex')?.available).toBe(false);
  });

  it('still offers a switched-off session agent first, but no other switched-off or external agent', () => {
    expect(forkTargets(agents, 'kimi').map(a => a.id)).toEqual(['kimi', 'claude', 'codex', 'devin']);
  });
});
