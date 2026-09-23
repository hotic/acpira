import { chmodSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { AgentRegistry, resolveCommand } from '../src/host/acp/AgentRegistry';

// A fresh directory with an optional executable, so a CLI can be "installed" and "removed" under the registry's nose
function sandbox() {
  const dir = mkdtempSync(join(tmpdir(), 'acpira-reg-'));
  const bin = join(dir, 'ghost-cli');
  const install = () => { writeFileSync(bin, '#!/bin/sh\nexit 0\n'); chmodSync(bin, 0o755); };
  const remove = () => rmSync(bin, { force: true });
  return { dir, bin, install, remove };
}

describe('AgentRegistry', () => {
  it('probeAll reports whether the available set changed and notifies subscribers; an executable appearing later is picked up without a new registry', async () => {
    const { bin, install } = sandbox();
    const r = new AgentRegistry({ ghost: { name: 'Ghost', command: bin } });
    let notified = 0;
    r.subscribe(() => notified++);
    expect(r.list().find(a => a.id === 'ghost')?.available).toBeUndefined();
    expect(await r.probeAll()).toBe(false);
    expect(r.list().find(a => a.id === 'ghost')?.available).toBe(false);
    expect(r.missing()).toBe(true);
    expect(notified).toBe(0);

    install();
    expect(await r.probeAll()).toBe(true);
    expect(notified).toBe(1);
    expect(r.list().find(a => a.id === 'ghost')?.available).toBe(true);
    // Nothing changed: no second notification
    expect(await r.probeAll()).toBe(false);
    expect(notified).toBe(1);
  });

  it('a cached path is re-verified: removing the binary flips the agent back to unavailable', async () => {
    const { bin, install, remove } = sandbox();
    install();
    const r = new AgentRegistry({ ghost: { name: 'Ghost', command: bin } });
    await r.probeAll();
    expect(await r.resolveBinary('ghost')).toBe(bin);
    remove();
    let notified = 0;
    r.subscribe(() => notified++);
    expect(await r.resolveBinary('ghost')).toBeNull();
    expect(notified).toBe(1);
    expect(r.list().find(a => a.id === 'ghost')?.available).toBe(false);
  });

  it('a single resolveBinary that finds a freshly installed CLI notifies like a probe pass (the settings-page rescan path)', async () => {
    const { bin, install } = sandbox();
    const r = new AgentRegistry({ ghost: { name: 'Ghost', command: bin } });
    await r.probeAll();
    let notified = 0;
    r.subscribe(() => notified++);
    install();
    expect(await r.resolveBinary('ghost')).toBe(bin);
    expect(notified).toBe(1);
    expect(r.list().find(a => a.id === 'ghost')?.available).toBe(true);
    // Built-ins stay on the registry, so missing() still follows whether this machine has grok / kimi / devin
  });

  it('on Windows a PATH entry resolves through PATHEXT extensions; on POSIX a non-executable misses', async () => {
    const { dir } = sandbox();
    const cmd = join(dir, 'foo.CMD');
    writeFileSync(cmd, '@echo off\r\n');
    chmodSync(cmd, 0o755);
    const env = { PATH: dir, PATHEXT: '.COM;.EXE;.BAT;.CMD' };
    expect(await resolveCommand('foo', [], 'win32', env)).toBe(cmd);
    expect(await resolveCommand('foo', [], 'linux', { PATH: dir })).toBeNull();
  });

  it('an agent whose required helper is missing counts as unavailable and reports what is missing', async () => {
    const { bin, install } = sandbox();
    install();
    const r = new AgentRegistry({ pi: { name: 'Pi', command: bin, requires: ['acpira-definitely-missing-helper'] } });
    await r.probeAll();
    const info = r.list().find(a => a.id === 'pi');
    expect(info?.available).toBe(false);
    expect(info?.missing).toEqual(['acpira-definitely-missing-helper']);
    expect(await r.resolveBinary('pi')).toBeNull();
  });

  it('OpenCode / DSH / Pi are built in, and a custom entry overrides the builtin of the same id', () => {
    const ids = new AgentRegistry().list().map(a => a.id);
    expect(ids).toEqual(expect.arrayContaining(['grok', 'devin', 'kimi', 'codex', 'claude', 'opencode', 'dsh', 'pi']));
    const r = new AgentRegistry({ opencode: { name: 'OC Fork', command: '/x/oc-fork' } });
    const info = r.list().find(a => a.id === 'opencode');
    expect(info?.name).toBe('OC Fork');
    expect(r.get('opencode').command).toBe('/x/oc-fork');
  });

  it('Codex / Claude are npm adapter definitions, and a custom entry of the same id replaces them wholesale', () => {
    const r = new AgentRegistry();
    expect(r.get('codex')).toMatchObject({
      command: 'codex-acp', requires: ['node'],
      login: { command: 'codex-acp', args: ['cli', 'login'] },
      adapter: { package: '@agentclientprotocol/codex-acp', engine: { package: '@openai/codex', overrideEnv: 'CODEX_PATH' } },
    });
    expect(r.get('claude')).toMatchObject({
      command: 'claude-agent-acp', requires: ['node'],
      login: { command: 'claude-agent-acp', args: ['--cli', 'auth', 'login'] },
      adapter: { package: '@agentclientprotocol/claude-agent-acp', engine: { package: '@anthropic-ai/claude-agent-sdk', overrideEnv: 'CLAUDE_CODE_EXECUTABLE' } },
    });
    const custom = new AgentRegistry({ codex: { name: 'CX', command: '/x/cx' }, claude: { command: '/x/cl' } });
    expect(custom.get('codex').command).toBe('/x/cx');
    expect(custom.get('codex').adapter).toBeUndefined();
    expect(custom.get('claude').adapter).toBeUndefined();
  });

  it('devin opts out of the terminal-auth capability; a custom agent does the same via terminalAuth: false', () => {
    expect(new AgentRegistry().get('devin').auth).toEqual({ terminal: false });
    const r = new AgentRegistry({ mine: { command: '/x/mine', terminalAuth: false }, other: { command: '/x/other' } });
    expect(r.get('mine').auth).toEqual({ terminal: false });
    expect(r.get('other').auth).toBeUndefined();
  });

  it('the pi builtin reports the missing pi helper when only pi-acp resolves', async () => {
    const { dir } = sandbox();
    const adapter = join(dir, 'pi-acp');
    writeFileSync(adapter, '#!/bin/sh\nexit 0\n');
    chmodSync(adapter, 0o755);
    // locate() reads process.env.PATH — swap it so `pi` cannot resolve anywhere else on this machine
    const realPath = process.env.PATH;
    process.env.PATH = dir;
    try {
      const r = new AgentRegistry({}, 'darwin');
      await r.probeAll();
      const info = r.list().find(a => a.id === 'pi');
      expect(info?.available).toBe(false);
      expect(info?.missing).toEqual(['pi']);
    } finally { process.env.PATH = realPath; }
  });

  it('install info follows the platform: POSIX line on darwin / linux, PowerShell line on win32, docs everywhere', () => {
    const posix = new AgentRegistry({}, 'darwin');
    const win = new AgentRegistry({}, 'win32');
    expect(posix.install('grok')).toEqual({ command: 'curl -fsSL https://x.ai/cli/install.sh | bash', docs: 'https://docs.x.ai/build/overview' });
    expect(win.install('grok')).toEqual({ command: 'irm https://x.ai/cli/install.ps1 | iex', docs: 'https://docs.x.ai/build/overview' });
    expect(posix.list().find(a => a.id === 'kimi')?.install?.command).toContain('code.kimi.com');
  });

  it('custom agents: an install line applies to every platform, docs alone is enough, nothing declared means no install info', () => {
    const r = new AgentRegistry({
      a: { command: '/x/a', install: { command: 'brew install a' } },
      b: { command: '/x/b', install: { docs: 'https://example.com/b' } },
      c: { command: '/x/c' },
    }, 'win32');
    expect(r.install('a')).toEqual({ command: 'brew install a' });
    expect(r.install('b')).toEqual({ docs: 'https://example.com/b' });
    expect(r.install('c')).toBeUndefined();
    expect(r.list().find(a => a.id === 'c')).not.toHaveProperty('install');
  });
});
