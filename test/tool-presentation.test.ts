import { afterEach, describe, expect, it } from 'vitest';
import { setLocale } from '../src/webview/i18n';
import { foldActivity, toolVerb } from '../src/webview/chat/folding';
import type { AgentTurn, ToolCallBlock } from '../src/shared/transcript';
import { fileReference, groupReadCalls, isLineCount, toolFiles } from '../src/webview/chat/toolDetails';
import { agentTurn } from './fixtures/engine';

afterEach(() => setLocale('en'));

describe('ACP tool presentation', () => {
  it.each([
    ['/repo/my file.ts:408–420', { path: '/repo/my file.ts', line: 408 }],
    ['src/a.ts:12:8', { path: 'src/a.ts', line: 12 }],
    ['C:\\repo\\a.ts:27-30', { path: 'C:\\repo\\a.ts', line: 27 }],
    ['file:///repo/my%20file.ts:9', { path: 'file:///repo/my%20file.ts', line: 9 }],
    ['src/a.ts', { path: 'src/a.ts' }],
  ])('preserves the editor destination for %s', (hit, expected) => {
    expect(fileReference(hit)).toEqual(expected);
  });
  it('groups consecutive reads while preserving output and action boundaries', () => {
    const a: ToolCallBlock = { type: 'tool_call', id: 'a', kind: 'read', verb: 'Read', status: 'completed', target: 'a.ts', content: { type: 'text', text: 'source A' } };
    const b = { ...a, id: 'b', target: 'b.ts', content: { type: 'text' as const, text: 'source B' } };
    const failed = { ...a, id: 'failed', status: 'failed' as const };
    const prose = { type: 'text' as const, markdown: 'Next step' };
    expect(groupReadCalls([a, b, failed, a, prose, b])).toEqual([[a, b], failed, [a], prose, [b]]);
    expect(groupReadCalls([a, { ...b, status: 'in_progress' }])).toEqual([[a], { ...b, status: 'in_progress' }]);
  });
  it('supports title-first notifications followed by typed actions and raw file paths', () => {
    const turn = agentTurn('read-title-first', 1);
    expect(foldActivity(turn)).toMatchObject({ kind: 'read', label: 'Read…', target: 'a.ts', active: true });
    expect(toolFiles(turn.blocks[0] as ToolCallBlock)).toEqual(['/repo/a.ts']);
    expect(foldActivity(agentTurn('read-title-first', 2))).toMatchObject({ kind: 'read', label: 'Read', target: 'a.ts' });
  });

  it('shows search file hits without interpreting ordinary output as filenames', () => {
    const search: ToolCallBlock = { type: 'tool_call', id: 's', kind: 'search', verb: 'Search', status: 'completed',
      target: 'activity|fold', content: { type: 'text', text: 'src/a.ts:12:const activity = 1;\nsrc/b.ts:3:fold();\n2 matches found\n' } };
    expect(toolFiles(search)).toEqual(['src/a.ts:12', 'src/b.ts:3']);
    expect(toolFiles({ ...search, content: { type: 'text', text: 'No matches found' } })).toEqual([]);
    const read: ToolCallBlock = { ...search, kind: 'read', target: 'a.ts', content: { type: 'text', text: '90 lines' } };
    expect(toolFiles(read)).toEqual(['a.ts']);
    expect(isLineCount(read)).toBe(true);
    expect(isLineCount({ ...read, content: { type: 'text', text: 'const lines = 90;' } })).toBe(false);
  });
  it('retains every file location across sparse tool updates', () => {
    expect(agentTurn('read-locations').blocks[0]).toMatchObject({ locations: [{ path: '/repo/a.ts', line: 12 }, { path: '/repo/b.ts' }] });
  });

  it('normalizes file URI search results to editor paths', () => {
    const search: ToolCallBlock = { type: 'tool_call', id: 's', kind: 'search', verb: 'Search', status: 'completed',
      content: { type: 'text', text: 'file:///repo/AcpSession.ts\nfile:///repo/my%20file.ts\n' } };
    expect(toolFiles(search)).toEqual(['/repo/AcpSession.ts', '/repo/my file.ts']);
  });

  // line_offset + n_lines, start_line + end_line, offset + limit
  it.each([0, 1, 2])('preserves read ranges through later location-only updates (form %i)', form => {
    const block = agentTurn(`read-range-${form}`).blocks[0] as ToolCallBlock;
    expect(toolFiles(block)).toEqual(['/repo/AcpSession.ts:120–199']);
  });

  it('keeps partial and unknown read ranges honest', () => {
    expect((agentTurn('read-partial').blocks as ToolCallBlock[]).map(toolFiles)).toEqual([
      ['/repo/a.ts:120'], ['/repo/a.ts'], ['/repo/a.ts'],
    ]);
  });

  it('renders stored verbs in the current UI language', () => {
    const block: ToolCallBlock = { type: 'tool_call', id: 'r', kind: 'read', verb: 'Read', status: 'completed' };
    setLocale('zh-CN');
    expect(toolVerb(block)).toBe('已读取');
    setLocale('en');
    expect(toolVerb({ ...block, verb: '读取' })).toBe('Read');
  });

  it('labels a todo-list tool by name whatever kind the agent filed it under', () => {
    setLocale('zh-CN');
    expect(foldActivity(agentTurn('todo-label', 0))).toMatchObject({ label: '正在更新待办', target: undefined, active: true });
    const turn = agentTurn('todo-label', 1);
    expect(foldActivity(turn)).toMatchObject({ label: '已更新待办' });
    setLocale('en');
    expect(toolVerb(turn.blocks[0] as ToolCallBlock)).toBe('Update todos');
  });

  it('infers the kind of well-known tool names when the agent omits or grab-bags it', () => {
    const [web, sh] = agentTurn('infer-kinds').blocks as ToolCallBlock[];
    expect(web).toMatchObject({ kind: 'search', target: 'acp spec' });
    expect(sh).toMatchObject({ kind: 'execute', target: 'ls -la' });
    setLocale('zh-CN');
    expect(toolVerb(web!)).toBe('正在搜索');
    expect(toolVerb(sh!)).toBe('正在运行');
  });

  it('shows the file for delete/move and the URL for fetch', () => {
    const [del, fet] = agentTurn('delete-fetch').blocks as ToolCallBlock[];
    expect(del).toMatchObject({ target: 'old.ts', locations: [{ path: '/repo/old.ts' }] });
    expect(fet).toMatchObject({ target: 'https://example.com/spec' });
  });

  it('keeps a specific kind even when the tool name suggests another', () => {
    expect(agentTurn('specific-kind').blocks[0]).toMatchObject({ kind: 'edit' });
  });

  it('shows the latest finished action between tool completion and the next thought', () => {
    setLocale('zh-CN');
    const turn: AgentTurn = { role: 'agent', blocks: [
      { type: 'tool_call', id: 'e', kind: 'edit', verb: 'Edit', target: 'a.ts', status: 'completed' },
    ], activity: { kind: 'think', label: 'Thinking' } };
    expect(foldActivity(turn)).toMatchObject({ kind: 'edit', label: '已编辑', target: 'a.ts' });
    turn.blocks.push({ type: 'thought', text: 'Check the result.', streaming: true });
    expect(foldActivity(turn)).toMatchObject({ kind: 'think', label: '正在处理' });
    turn.blocks.push({ type: 'text', markdown: 'Done.', streaming: true });
    expect(foldActivity(turn)).toMatchObject({ kind: 'other', label: '正在回复' });
  });
});
