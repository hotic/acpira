import { execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { terminalScript } from '../src/terminal';

describe('terminalScript', () => {
  it('quotes the line and applies the environment edits before it', () => {
    const script = terminalScript({ title: "Claude's login", command: '/opt/bin/claude', args: ['auth', 'login', 'a b'], env: { CLAUDE_CONFIG_DIR: '/tmp/x y', DROP_ME: null, 'BAD NAME': 'x' } });
    expect(script.split('\n')).toEqual([
      '#!/bin/sh',
      'rm -f "$0"; rmdir "$(dirname "$0")" 2>/dev/null',
      "printf '\\033]0;%s\\007' 'Claude'\\''s login'",
      "export CLAUDE_CONFIG_DIR='/tmp/x y'",
      'unset DROP_ME',
      "/opt/bin/claude auth login 'a b'",
      '',
    ]);
  });

  // The script really runs under sh: arguments with quotes and spaces arrive whole, and the file and its folder go
  it.skipIf(process.platform === 'win32')('runs under sh with its arguments intact and removes itself', () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-term-test-'));
    try {
      const file = join(dir, 'run.sh');
      writeFileSync(file, terminalScript({ title: 't', command: 'printf', args: ['%s|%s|%s', "it's", 'two words', '${HOME}'], env: { GONE: null } }), { mode: 0o700 });
      const out = execFileSync('sh', [file], { encoding: 'utf8', env: { ...process.env, GONE: 'x' } });
      expect(out.replace(/^\u001b\]0;t\u0007/, '')).toBe("it's|two words|${HOME}");
      expect(() => execFileSync('test', ['-e', dir])).toThrow();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
