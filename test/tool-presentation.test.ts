import { afterEach, describe, expect, it } from 'vitest';
import { applyUpdate, emptyState } from '../src/host/acp/normalize';
import { setLocale } from '../src/webview/i18n';
import { foldActivity, toolVerb } from '../src/webview/chat/folding';
import type { AgentTurn, ToolCallBlock } from '../src/shared/transcript';
import { fileReference, groupReadCalls, isLineCount, toolFiles } from '../src/webview/chat/toolDetails';

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
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'r', title: 'read_file' });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'r', kind: 'read', status: 'in_progress', rawInput: { path: '/repo/a.ts' } });
    const turn = s.turns[0] as AgentTurn;
    expect(foldActivity(turn)).toMatchObject({ kind: 'read', label: 'Read…', target: 'a.ts', active: true });
    expect(toolFiles(turn.blocks[0] as ToolCallBlock)).toEqual(['/repo/a.ts']);
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'r', title: 'read_file', status: 'completed' });
    expect(foldActivity(turn)).toMatchObject({ kind: 'read', label: 'Read', target: 'a.ts' });
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
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'r', title: 'Read files', kind: 'read',
      status: 'in_progress', locations: [{ path: '/repo/a.ts', line: 12 }, { path: '/repo/b.ts' }] });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'r', status: 'completed',
      content: [{ type: 'content', content: { type: 'text', text: '90 lines' } }] });
    const turn = s.turns[0] as AgentTurn;
    expect(turn.blocks[0]).toMatchObject({ locations: [{ path: '/repo/a.ts', line: 12 }, { path: '/repo/b.ts' }] });
  });

  it('normalizes file URI search results to editor paths', () => {
    const search: ToolCallBlock = { type: 'tool_call', id: 's', kind: 'search', verb: 'Search', status: 'completed',
      content: { type: 'text', text: 'file:///repo/AcpSession.ts\nfile:///repo/my%20file.ts\n' } };
    expect(toolFiles(search)).toEqual(['/repo/AcpSession.ts', '/repo/my file.ts']);
  });

  it.each([
    { line_offset: 120, n_lines: 80 },
    { start_line: 120, end_line: 199 },
    { offset: 120, limit: 80 },
  ])('preserves read ranges through later location-only updates: %j', params => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'r', title: 'read_file', kind: 'read',
      rawInput: { path: '/repo/AcpSession.ts', ...params } });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'r', status: 'completed', locations: [{ path: '/repo/AcpSession.ts' }] });
    const block = (s.turns[0] as AgentTurn).blocks[0] as ToolCallBlock;
    expect(toolFiles(block)).toEqual(['/repo/AcpSession.ts:120–199']);
  });

  it('keeps partial and unknown read ranges honest', () => {
    const s = emptyState();
    for (const [id, params] of Object.entries({ start: { line_offset: 120 }, invalid: { start_line: -1, end_line: 20 }, unknown: {} })) {
      applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: id, title: 'Read', kind: 'read', rawInput: { path: '/repo/a.ts', ...params } });
    }
    expect(((s.turns[0] as AgentTurn).blocks as ToolCallBlock[]).map(toolFiles)).toEqual([
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
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'td', title: 'todo_write', kind: 'think', status: 'in_progress' });
    const turn = s.turns[0] as AgentTurn;
    setLocale('zh-CN');
    expect(foldActivity(turn)).toMatchObject({ label: '正在更新待办', target: undefined, active: true });
    applyUpdate(s, { sessionUpdate: 'tool_call_update', toolCallId: 'td', status: 'completed' });
    expect(foldActivity(turn)).toMatchObject({ label: '已更新待办' });
    setLocale('en');
    expect(toolVerb(turn.blocks[0] as ToolCallBlock)).toBe('Update todos');
  });

  it('infers the kind of well-known tool names when the agent omits or grab-bags it', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'w', title: 'web_search', status: 'in_progress', rawInput: { query: 'acp spec' } });
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'b', title: 'bash', kind: 'other', status: 'in_progress', rawInput: { command: 'ls -la' } });
    const [web, sh] = (s.turns[0] as AgentTurn).blocks as ToolCallBlock[];
    expect(web).toMatchObject({ kind: 'search', target: 'acp spec' });
    expect(sh).toMatchObject({ kind: 'execute', target: 'ls -la' });
    setLocale('zh-CN');
    expect(toolVerb(web!)).toBe('正在搜索');
    expect(toolVerb(sh!)).toBe('正在运行');
  });

  it('shows the file for delete/move and the URL for fetch', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'd', title: 'Delete', kind: 'delete', status: 'in_progress', rawInput: { file_path: '/repo/old.ts' } });
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'f', title: 'Fetch', kind: 'fetch', status: 'in_progress', rawInput: { url: 'https://example.com/spec' } });
    const [del, fet] = (s.turns[0] as AgentTurn).blocks as ToolCallBlock[];
    expect(del).toMatchObject({ target: 'old.ts', locations: [{ path: '/repo/old.ts' }] });
    expect(fet).toMatchObject({ target: 'https://example.com/spec' });
  });

  it('keeps a specific kind even when the tool name suggests another', () => {
    const s = emptyState();
    applyUpdate(s, { sessionUpdate: 'tool_call', toolCallId: 'r', title: 'read_file', kind: 'edit', status: 'in_progress', rawInput: { path: '/repo/a.ts' } });
    expect((s.turns[0] as AgentTurn).blocks[0]).toMatchObject({ kind: 'edit' });
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
