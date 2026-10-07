import { describe, expect, it } from 'vitest';
import { shortPath } from '../src/webview/settings/paths';

describe('shortPath', () => {
  it('shortens POSIX paths under the workspace and home', () => {
    const env = { home: '/Users/me', cwd: '/Users/me/work' };
    expect(shortPath('/Users/me/work/AGENTS.md', env)).toBe('AGENTS.md');
    expect(shortPath('/Users/me/work', env)).toBe('.');
    expect(shortPath('/Users/me/.codex/AGENTS.md', env)).toBe('~/.codex/AGENTS.md');
    expect(shortPath('/Users/meow/x', env)).toBe('/Users/meow/x');
    expect(shortPath('/opt/x', env)).toBe('/opt/x');
  });

  it('shows Windows paths with backslashes only, matching home and workspace case-insensitively', () => {
    const env = { home: 'C:\\Users\\Spark', cwd: 's:\\Projects\\acpira' };
    // The sidecar's older spelling joined a template's `/` tail onto a native home
    expect(shortPath('C:\\Users\\Spark\\.codex/AGENTS.md', env)).toBe('~\\.codex\\AGENTS.md');
    expect(shortPath('c:/users/spark/.claude/CLAUDE.md', env)).toBe('~\\.claude\\CLAUDE.md');
    expect(shortPath('S:\\Projects\\acpira\\.claude/skills\\x', env)).toBe('.claude\\skills\\x');
    expect(shortPath('C:\\Users\\Sparkle\\x', env)).toBe('C:\\Users\\Sparkle\\x');
    expect(shortPath('D:\\data/AGENTS.md', env)).toBe('D:\\data\\AGENTS.md');
  });
});
