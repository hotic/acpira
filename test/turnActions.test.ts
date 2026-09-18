import { describe, expect, it } from 'vitest';
import type { AgentTurn, SessionControls, ToolCallBlock } from '../src/shared/transcript';
import { modelLabel, replyMarkdown, toolCallCount } from '../src/webview/chat/turnActionHelpers';

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

describe('toolCallCount', () => {
  it('counts only tool_call blocks', () => {
    expect(toolCallCount(turn([text('hi'), read, run, { type: 'thought', text: 't' }]))).toBe(2);
    expect(toolCallCount(turn([text('hi')]))).toBe(0);
  });
});
