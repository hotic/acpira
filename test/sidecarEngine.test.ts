import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import type { HostMsg } from '../src/shared/protocol';
import type { AgentTurn, SessionView } from '../src/shared/transcript';
import { SidecarClient, type SidecarCommand, type SidecarState, type ShellView } from '../src/host/shell/SidecarClient';
import { engineEndpoint, type EngineEndpoint } from '../src/host/shell/engine';
import { FAKE, SIDECAR, NODE } from './sidecarShell';

// The persistent engine (`acpira serve --socket`) through the VS Code shell's client: a turn outlives the window that started
// it, the next window for the workspace reconnects to it, an idle engine with no window ends on its own, and a session another
// engine has open is mirrored read-only until it is taken over or that engine ends

const dirs: string[] = [];
const clients: SidecarClient[] = [];
const engines = new Set<number>();

const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };

afterEach(async () => {
  for (const c of clients.splice(0)) await c.dispose();
  // A failed test must not leave an engine (and its agents) behind: SIGTERM runs the engine's normal teardown
  for (const pid of engines) if (alive(pid)) process.kill(pid, 'SIGTERM');
  await expect.poll(() => [...engines].filter(alive), { timeout: 10_000 }).toEqual([]);
  engines.clear();
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
});

function tmp(prefix: string) {
  const d = mkdtempSync(join(tmpdir(), prefix));
  dirs.push(d);
  return d;
}

// One second of idleness ends the engine, so the tests can watch it go
function command(home: string): SidecarCommand {
  return {
    command: SIDECAR,
    args: ['--home', home, '--idle-grace', '1'],
    env: { ACPIRA_HOME: '', ACPIRA_CATALOG_REFRESH: '0', ACPIRA_LOGIN_PATH: '0' },
    label: 'rust',
  };
}

class View implements ShellView {
  readonly messages: HostMsg[] = [];
  readonly states: SidecarState[] = [];
  private waiters: { pred: (m: HostMsg) => boolean; resolve: (m: HostMsg) => void }[] = [];
  constructor(readonly viewId: string, readonly initial?: string | { mostRecent: true }) {}
  readonly host = 'sidebar' as const;
  onHostMessage(m: HostMsg) {
    this.messages.push(m);
    for (const w of this.waiters.splice(0)) { if (w.pred(m)) w.resolve(m); else this.waiters.push(w); }
  }
  onState(s: SidecarState) { this.states.push(s); }
  // The session as the page would hold it: full pushes only (the client is not a patching page)
  session(pred: (s: SessionView) => boolean, ms = 20_000): Promise<SessionView> {
    const pick = (m: HostMsg) => m.type === 'session' ? m.session : m.type === 'init' ? m.state.active : undefined;
    const have = this.messages.map(pick).find(s => s && pred(s));
    if (have) return Promise.resolve(have);
    return new Promise((resolve, reject) => {
      const seen = () => this.messages.map(pick).filter(Boolean).slice(-3).map(s => `${s!.status}/${s!.running}/${s!.turns.length}/${s!.error ?? ''}`).join(' | ');
      const t = setTimeout(() => reject(new Error(`timed out waiting for the session; last: ${seen()}; types: ${this.messages.slice(-5).map(m => m.type).join(',')}`)), ms);
      this.waiters.push({ pred: m => { const s = pick(m); return !!s && pred(s); }, resolve: m => { clearTimeout(t); resolve(pick(m)!); } });
    });
  }
}

function client(ep: EngineEndpoint, cwd: string, home: string, logs: string[] = [], agentEnv: Record<string, string> = {}) {
  const c = new SidecarClient({
    commands: () => [command(home)],
    engine: () => ep,
    cwd: () => cwd,
    hello: () => ({
      client: { name: 'engine-test', version: '0', capabilities: ['toast'] },
      env: { hostLanguage: 'en', cwd },
      settings: { defaultAgent: 'fake', agents: { fake: { name: 'Fake', command: NODE, args: [FAKE], env: agentEnv } } },
    }),
    onRequest: () => undefined,
    log: line => {
      logs.push(line);
      const pid = /sidecar ready: pid (\d+)/.exec(line)?.[1];
      if (pid) engines.add(Number(pid));
    },
    backoff: () => 50,
  });
  clients.push(c);
  return c;
}

const lastAgent = (s: SessionView) => s.turns.at(-1) as AgentTurn | undefined;
const textOf = (s: SessionView) => JSON.stringify(lastAgent(s)?.blocks ?? []);
const pidOf = (logs: string[]) => Number(logs.map(l => /sidecar ready: pid (\d+)/.exec(l)?.[1]).find(Boolean));

describe('persistent engine', { timeout: 60_000 }, () => {
  it('keeps a turn running after the window goes, hands it to the next window and ends once idle', async () => {
    const home = tmp('acpira-engine-home-');
    const cwd = tmp('acpira-engine-ws-');
    const ep = engineEndpoint({ binary: SIDECAR, home, cwd });
    const logs: string[] = [];
    const first = client(ep, cwd, home, logs);
    const v = new View('V', { mostRecent: true });
    first.attach(v);
    first.send('V', { type: 'ready' });
    first.start();
    const id = (await v.session(s => s.status === 'ready')).id;
    first.send('V', { type: 'send', sessionId: id, text: 'slow' });
    await v.session(s => s.running && textOf(s).includes(' 3 '));
    const pid = pidOf(logs);
    expect(alive(pid)).toBe(true);

    // The window closes mid-turn: only the connection goes
    await first.dispose();
    await new Promise(r => setTimeout(r, 300));
    expect(alive(pid)).toBe(true);

    // The next window reaches the same engine and finds the turn still running, then sees it finish normally
    const logs2: string[] = [];
    const second = client(ep, cwd, home, logs2);
    const w = new View('W', id);
    second.attach(w);
    second.send('W', { type: 'ready' });
    second.start();
    await w.session(s => s.id === id && s.running);
    expect(pidOf(logs2)).toBe(pid);
    const done = await w.session(s => s.id === id && !s.running && lastAgent(s)?.stop !== undefined);
    expect(lastAgent(done)?.stop).toBe('end_turn');
    expect(textOf(done)).toContain('49 ');

    // No window and no turn: the engine ends after its grace and takes its socket with it
    await second.dispose();
    await expect.poll(() => alive(pid), { timeout: 10_000 }).toBe(false);
    expect(existsSync(ep.socket)).toBe(false);
  });

  it('mirrors a session another engine is mid-turn on, and takes it over on request', async () => {
    const home = tmp('acpira-engine-home-');
    // Two workspaces on one data dir: two engines, the same session list
    const [cwdA, cwdB] = [tmp('acpira-engine-a-'), tmp('acpira-engine-b-')];
    // A turn of about 7.5 s: long enough for A's debounced save (at most 2 s behind) and B's start to land inside it. Both
    // agents share a native session store on disk, as real CLIs do, so B's agent can resume what A's started
    const agentEnv = { FAKE_SLOW_STEP_MS: '150', FAKE_SESSION_DIR: tmp('acpira-engine-native-') };
    const a = client(engineEndpoint({ binary: SIDECAR, home, cwd: cwdA }), cwdA, home, [], agentEnv);
    const va = new View('A', { mostRecent: true });
    a.attach(va);
    a.send('A', { type: 'ready' });
    a.start();
    const id = (await va.session(s => s.status === 'ready')).id;
    a.send('A', { type: 'send', sessionId: id, text: 'slow' });
    await va.session(s => s.running && textOf(s).includes(' 2 '));
    // The record is on disk once A's debounced save ran: what a second engine opens
    await expect.poll(() => existsSync(join(home, 'sessions', `${id}.json`)), { timeout: 5000 }).toBe(true);

    const b = client(engineEndpoint({ binary: SIDECAR, home, cwd: cwdB }), cwdB, home, [], agentEnv);
    const vb = new View('B', id);
    b.attach(vb);
    b.send('B', { type: 'ready' });
    b.start();
    // Read-only while engine A holds the lease, and the copy follows the record A keeps saving (the running turn is not
    // marked interrupted the way an ordinary reopen would)
    const mirrored = await vb.session(s => s.id === id && s.status === 'readonly');
    expect(mirrored.error).toContain('Another Acpira engine');
    expect(lastAgent(mirrored)?.stop).toBeUndefined();
    await vb.session(s => s.status === 'readonly' && textOf(s).includes(' 20 '));

    // B takes it over: A's turn stops and A keeps a read-only copy it can take back; B has the session live with A's turn
    expect(mirrored.canTakeOver).toBe(true);
    b.send('B', { type: 'takeOverSession', sessionId: id });
    const opened = await vb.session(s => s.id === id && s.status === 'ready');
    expect(opened.turns).toHaveLength(2);
    expect(textOf(opened)).toContain(' 20 ');
    const left = await va.session(s => s.id === id && s.status === 'readonly');
    expect(left.canTakeOver).toBe(true);
  });

  // Blue-green: an engine whose windows are all gone (the extension was upgraded, the next window talks to a new engine) keeps
  // every session it has open until it ends, so the new engine never opens a native session beside the old agent
  it('lets a newer engine take a session only once the old engine, idle and windowless, has ended', async () => {
    const home = tmp('acpira-engine-home-');
    const cwd = tmp('acpira-engine-ws-');
    const agentEnv = { FAKE_SESSION_DIR: tmp('acpira-engine-native-') };
    const oldLogs: string[] = [];
    // Different socket = different binary identity: what an upgrade does
    const old = client({ ...engineEndpoint({ binary: SIDECAR, home, cwd }), socket: engineEndpoint({ binary: SIDECAR, home, cwd: `${cwd}/old` }).socket }, cwd, home, oldLogs, agentEnv);
    const v = new View('V', { mostRecent: true });
    old.attach(v);
    old.send('V', { type: 'ready' });
    old.start();
    const id = (await v.session(s => s.status === 'ready')).id;
    old.send('V', { type: 'send', sessionId: id, text: 'hi' });
    await v.session(s => !s.running && lastAgent(s)?.stop === 'end_turn');
    await expect.poll(() => existsSync(join(home, 'sessions', `${id}.json`)), { timeout: 5000 }).toBe(true);
    const oldPid = pidOf(oldLogs);
    await old.dispose();

    const fresh = client(engineEndpoint({ binary: SIDECAR, home, cwd }), cwd, home, [], agentEnv);
    const w = new View('W', id);
    fresh.attach(w);
    fresh.send('W', { type: 'ready' });
    fresh.start();
    const opened = await w.session(s => s.id === id && s.status === 'ready');
    // The old engine was gone by then: it ended after its one-second grace, and only then did the new one start an agent
    expect(alive(oldPid)).toBe(false);
    // The read-only copy can reach the page inside init, which no longer waits for the agent, or as a later push
    const shown = w.messages.map(m => m.type === 'session' ? m.session : m.type === 'init' ? m.state.active : undefined);
    expect(shown.some(s => s?.id === id && s.status === 'readonly')).toBe(true);
    expect(opened.turns).toHaveLength(2);
  });
});

describe('engineEndpoint', () => {
  it('keys the socket by binary, data dir and workspace, and moves a too-long path under the temp dir', () => {
    const home = tmp('acpira-engine-key-');
    const a = engineEndpoint({ binary: SIDECAR, home: '/h', cwd: '/w1' });
    expect(engineEndpoint({ binary: SIDECAR, home: '/h', cwd: '/w1' })).toEqual(a);
    expect(engineEndpoint({ binary: SIDECAR, home: '/h', cwd: '/w2' }).socket).not.toBe(a.socket);
    expect(engineEndpoint({ binary: `${home}/missing`, home: '/h', cwd: '/w1' }).socket).not.toBe(a.socket);
    expect(a.socket).toMatch(/^\/h\/run\/engine-[0-9a-f]{16}\.sock$/);
    const deep = `/${'d'.repeat(120)}`;
    const far = engineEndpoint({ binary: SIDECAR, home: deep, cwd: '/w1', tmp: '/t', uid: 7 });
    expect(far.socket).toMatch(/^\/t\/acpira-7\/engine-[0-9a-f]{16}\.sock$/);
    expect(far.log.startsWith(`${deep}/run/`)).toBe(true);
  });
});
