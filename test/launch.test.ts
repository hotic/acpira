import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { resolveExecutable, spawnSpec } from '../src/host/acp/launch';
import { resolveCommand } from '../src/host/acp/AgentRegistry';

describe('spawnSpec', () => {
  it('routes a .cmd through the Windows command shell with cross-spawn escaping', () => {
    const spec = spawnSpec('C:\\x\\pi-acp.cmd', ['--a', 'b c', 'q"t'], 'win32', {});
    expect(spec.command).toBe('cmd.exe');
    expect(spec.windowsVerbatimArguments).toBe(true);
    expect(spec.args.slice(0, 3)).toEqual(['/d', '/s', '/c']);
    // One quoted string for the whole command line; the argument with a space arrives escaped
    expect(spec.args[3]).toMatch(/^".*"$/);
    expect(spec.args[3]).toContain('^"b c^"');
  });

  it('honours ComSpec when set', () => {
    const spec = spawnSpec('C:\\x\\a.bat', [], 'win32', { ComSpec: 'C:\\Windows\\System32\\cmd.exe' });
    expect(spec.command).toBe('C:\\Windows\\System32\\cmd.exe');
    expect(spec.args).toEqual(['/d', '/s', '/c', '"C:\\x\\a.bat"']);
  });

  it('matches .cmd / .bat case-insensitively but leaves .exe alone', () => {
    expect(spawnSpec('C:\\x\\tool.CMD', ['x'], 'win32', {}).command).toBe('cmd.exe');
    expect(spawnSpec('C:\\x\\tool.exe', ['x'], 'win32', {})).toEqual({ command: 'C:\\x\\tool.exe', args: ['x'] });
  });

  it('a directory named like the command does not resolve; the .cmd next to it does', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-launch-'));
    mkdirSync(join(dir, 'foo'));
    writeFileSync(join(dir, 'foo.cmd'), '@echo off\r\n');
    const env = { PATH: dir, PATHEXT: '.COM;.EXE;.BAT;.CMD' };
    // The PATHEXT variant may come back upper-cased (case-insensitive filesystems match either); what matters
    // is that the bare directory was skipped and the .cmd was found
    const cmd = join(dir, 'foo.cmd').toLowerCase();
    expect((await resolveExecutable(join(dir, 'foo'), 'win32', env))?.toLowerCase()).toBe(cmd);
    expect((await resolveCommand('foo', [], 'win32', env))?.toLowerCase()).toBe(cmd);
  });

  it('passes everything through unchanged off Windows', () => {
    expect(spawnSpec('/usr/local/bin/pi-acp', ['--a', 'b c'], 'darwin', {})).toEqual({ command: '/usr/local/bin/pi-acp', args: ['--a', 'b c'] });
    expect(spawnSpec('C:\\x\\pi-acp.cmd', [], 'linux', {})).toEqual({ command: 'C:\\x\\pi-acp.cmd', args: [] });
  });
});
