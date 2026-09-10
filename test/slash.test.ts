import { describe, expect, it } from 'vitest';
import type { SlashCommand, Turn } from '@shared/transcript';
import { commandAt, commandHint, matchCommands } from '../src/webview/chat/slashCommands';

import { presentCommand } from '../src/shared/commandPresentation';
import { commandName, namedCommand, restoreCommandReceipts } from '../src/shared/slashCommands';
import { turnOutcome } from '../src/webview/chat/turnOutcome';

const COMMANDS: SlashCommand[] = [
  { name: 'compact', description: 'Compact the conversation' },
  { name: 'review', description: 'Review the current changes', input: { hint: 'files or scope' } },
  { name: 'research_codebase', description: 'Explore before planning' },
  { name: 'init', description: 'Write an AGENTS.md; runs a full scan' },
];

describe('commandAt', () => {
  it('opens on a leading slash while the caret is inside the first token', () => {
    expect(commandAt('/', 1)).toEqual({ query: '' });
    expect(commandAt('/rev', 4)).toEqual({ query: 'rev' });
    expect(commandAt('/review files', 3)).toEqual({ query: 're' });
  });

  it('is plain text once the caret leaves the token, for slashes elsewhere, and for paths', () => {
    expect(commandAt('/review ', 8)).toBeUndefined();
    expect(commandAt('/review files', 13)).toBeUndefined();
    expect(commandAt('look at /tmp/file.ts', 20)).toBeUndefined();
    expect(commandAt(' /review', 8)).toBeUndefined();
    expect(commandAt('', 0)).toBeUndefined();
    // /tmp/file.ts typed at the start still yields a span; the list simply has no hit for it and stays closed
    expect(matchCommands(COMMANDS, 'tmp/file.ts')).toEqual([]);
  });
});

describe('matchCommands', () => {
  it('lists everything for an empty query in the agent order', () => {
    expect(matchCommands(COMMANDS, '').map(c => c.name)).toEqual(['compact', 'review', 'research_codebase', 'init']);
  });

  it('ranks name prefixes before name substrings and description word starts, case-insensitively', () => {
    expect(matchCommands(COMMANDS, 're').map(c => c.name)).toEqual(['review', 'research_codebase']);
    expect(matchCommands(COMMANDS, 'Re').map(c => c.name)).toEqual(['review', 'research_codebase']);
    expect(matchCommands(COMMANDS, 'code').map(c => c.name)).toEqual(['research_codebase']);
    expect(matchCommands(COMMANDS, 'scan').map(c => c.name)).toEqual(['init']);
    expect(matchCommands(COMMANDS, 'plann').map(c => c.name)).toEqual(['research_codebase']);
    // A description only matches at a word start: `he` does not hit "the", `ion` does not hit "conversation"
    expect(matchCommands(COMMANDS, 'he')).toEqual([]);
    expect(matchCommands(COMMANDS, 'ion')).toEqual([]);
    expect(matchCommands(COMMANDS, 'zzz')).toEqual([]);
  });
});

describe('commandHint', () => {
  it('shows the hint only while the text is exactly the command with empty arguments', () => {
    expect(commandHint(COMMANDS, '/review')).toBe('files or scope');
    expect(commandHint(COMMANDS, '/review ')).toBe('files or scope');
    expect(commandHint(COMMANDS, '/review src')).toBeUndefined();
    expect(commandHint(COMMANDS, '/compact')).toBeUndefined();
    expect(commandHint(COMMANDS, '/rev')).toBeUndefined();
    expect(commandHint(COMMANDS, 'review')).toBeUndefined();
  });
});

describe('command presentation and feedback', () => {
  it('repairs historical empty slash receipts without changing the saved record or inventing mode changes', () => {
    const turns: Turn[] = [
      { role: 'user', text: '/always-approve on' }, { role: 'agent', blocks: [], stop: 'end_turn' },
      { role: 'user', text: '/tmp/file.ts' }, { role: 'agent', blocks: [], stop: 'end_turn' },
      { role: 'user', text: '/plan' }, { role: 'agent', blocks: [], stop: 'error' },
    ];
    const restored = restoreCommandReceipts(turns);
    expect(restored[1]).toMatchObject({ command: { name: 'always-approve' } });
    expect(turns[1]).not.toHaveProperty('command');
    expect(restored[1]).not.toHaveProperty('command.mode');
    expect(restored[3]).toBe(turns[3]);
    expect(restored[5]).toBe(turns[5]);
  });

  it('localizes known source wording, searches Chinese, and preserves custom descriptions and wire names', () => {
    const source = { name: 'status', description: 'Check authentication status', input: { hint: '[question]' } };
    expect(presentCommand(source, 'zh-CN')).toEqual({ name: 'status', description: '查看登录状态', input: { hint: '[问题]' } });
    expect(presentCommand(source, 'en')).toBe(source);
    expect(source.description).toBe('Check authentication status');
    expect(matchCommands([source], '查看', 'zh-CN')).toEqual([source]);
    expect(matchCommands([source], 'auth', 'zh-CN')).toEqual([source]);
    expect(commandHint([source], '/status ', 'zh-CN')).toBe('[问题]');
    const custom = { name: 'status', description: 'Show the status of a custom deployment' };
    expect(presentCommand(custom, 'zh-CN').description).toBe(custom.description);
    expect(presentCommand({ ...source, input: { hint: 'on|off' } }, 'zh-CN').input?.hint).toBe('on|off');
  });

  it('recognizes complete advertised tokens without highlighting partial names, paths or ordinary prose', () => {
    const commands = [{ name: 'agents:ui-pick', description: '选型' }];
    expect(namedCommand(commands, '/agents:ui-pick 做一个页面')?.name).toBe('agents:ui-pick');
    for (const text of ['/agents:ui', '/agents:ui-pick-more', ' /agents:ui-pick', '使用 /agents:ui-pick']) {
      expect(namedCommand(commands, text)).toBeUndefined();
    }
    expect(commandName('/unknown argument')).toBe('unknown');
    for (const text of ['/tmp/file.ts', '//server/share', '/tmp/', '/', 'hello']) expect(commandName(text)).toBeUndefined();
  });

  it('distinguishes an empty receipt, observed settings, content, and failed requests', () => {
    const base = { role: 'agent' as const, blocks: [], stop: 'end_turn' as const };
    expect(turnOutcome(base, 'zh-CN')).toBe('没有回复');
    const receipt = { ...base, command: { name: 'context' } };
    expect(turnOutcome(receipt, 'zh-CN')).toBe('请求已结束，CLI 未返回文字反馈');
    expect(turnOutcome({ ...receipt, command: { name: 'plan', mode: 'Plan' } }, 'zh-CN')).toBe('已切换至 Plan 模式');
    expect(turnOutcome({ ...receipt, blocks: [{ type: 'text', markdown: '   ' }] }, 'en')).toContain('no text feedback');
    expect(turnOutcome({ ...receipt, blocks: [{ type: 'text', markdown: 'Native reply' }] }, 'zh-CN')).toBeUndefined();
    expect(turnOutcome({ ...receipt, stop: 'error' }, 'zh-CN')).toBe('请求失败');
    expect(turnOutcome({ ...receipt, stop: 'cancelled' }, 'en')).toBe('Stopped');
    expect(turnOutcome({ ...receipt, stop: undefined }, 'zh-CN')).toBeUndefined();
  });
});
