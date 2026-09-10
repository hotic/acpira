import { describe, expect, it } from 'vitest';
import type { SlashCommand } from '@shared/transcript';
import { commandAt, commandHint, matchCommands } from '../src/webview/chat/slashCommands';

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
