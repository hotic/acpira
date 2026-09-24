import { chmodSync, mkdirSync, mkdtempSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import type { HostMsg } from '../src/shared/protocol';
import type { PlatformRequest } from '../src/shared/sidecar';
import { SidecarClient, type SidecarCommand, type SidecarState, type ShellView } from '../src/host/shell/SidecarClient';
import { sidecarCommands } from '../src/host/shell/sidecarLocator';
import { FAKE, SIDECAR, TSX } from './sidecarShell';

// The VS Code shell's sidecar client against the real Rust sidecar: handshake, outbox, platform RPCs, restart with re-attach,
// fallback to the next command and shutdown

const dirs: string[] = [];
const clients: SidecarClient[] = [];
afterEach(async () => {
  for (const c of clients.splice(0)) await c.dispose();
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
});

function tmp(prefix: string) {
  const d = mkdtempSync(join(tmpdir(), prefix));
  dirs.push(d);
  return d;
}

function engine(home: string): SidecarCommand {
  return { command: SIDECAR, args: ['--home', home], env: { ACPIRA_HOME: '' }, label: 'rust' };
}

class View implements ShellView {
  readonly messages: HostMsg[] = [];
  readonly states: SidecarState[] = [];
  private waiters: { pred: (m: HostMsg) => boolean; resolve: (m: HostMsg) => void }[] = [];
  constructor(readonly viewId: string, readonly host: 'sidebar' | 'editor' = 'sidebar', readonly initial?: string | { mostRecent: true }, readonly blobBase?: string) {}
  onHostMessage(m: HostMsg) {
    this.messages.push(m);
    for (const w of this.waiters.splice(0)) { if (w.pred(m)) w.resolve(m); else this.waiters.push(w); }
  }
  onState(s: SidecarState) { this.states.push(s); }
  next<T extends HostMsg['type']>(type: T, pred: (m: Extract<HostMsg, { type: T }>) => boolean = () => true, after = 0): Promise<Extract<HostMsg, { type: T }>> {
    const match = (m: HostMsg) => m.type === type && pred(m as Extract<HostMsg, { type: T }>);
    const have = this.messages.slice(after).find(match);
    if (have) return Promise.resolve(have as Extract<HostMsg, { type: T }>);
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`timed out waiting for ${type}`)), 15_000);
      this.waiters.push({ pred: match, resolve: m => { clearTimeout(t); resolve(m as Extract<HostMsg, { type: T }>); } });
    });
  }
}

function client(commands: SidecarCommand[], over: { requests?: PlatformRequest[]; logs?: string[]; states?: SidecarState[]; blobBase?: string } = {}) {
  const cwd = tmp('acpira-client-ws-');
  const c = new SidecarClient({
    commands: () => commands,
    cwd: () => cwd,
    hello: () => ({
      client: { name: 'client-test', version: '0', capabilities: ['toast'] },
      env: { hostLanguage: 'en', cwd, blobBase: 'blobBase' in over ? over.blobBase : 'https://blobs.test' },
      settings: { defaultAgent: 'fake', agents: { fake: { name: 'Fake', command: TSX, args: [FAKE] } } },
    }),
    onRequest: r => { over.requests?.push(r); },
    log: line => over.logs?.push(line),
    onState: s => over.states?.push(s),
    backoff: () => 50,
  });
  clients.push(c);
  return c;
}

describe('SidecarClient', () => {
  it('queues until the handshake, relays both ways and re-attaches after a restart', async () => {
    const home = tmp('acpira-client-home-');
    const requests: PlatformRequest[] = [];
    const c = client([engine(home)], { requests });
    // Attached and ready before the process exists: both wait in the outbox for helloOk
    const view = new View('V', 'sidebar', { mostRecent: true });
    c.attach(view);
    c.send('V', { type: 'ready' });
    c.start();
    const init = await view.next('init');
    const id = init.state.active!.id;
    expect(init.state.blobBase).toBe('https://blobs.test');
    await view.next('session', m => m.session.id === id && m.session.status === 'ready');
    c.send('V', { type: 'send', sessionId: id, text: 'hi' });
    await view.next('session', m => m.session.id === id && !m.session.running && m.session.turns.length === 2);

    // A toast is a notification the shell carries out
    c.send('V', { type: 'exportSession', id: '00000000-0000-4000-8000-000000000000', format: 'markdown' });
    await expect.poll(() => requests.some(r => r.method === 'toast')).toBe(true);

    // The process goes away; the client starts another one and puts the view back on the session it was showing
    const seen = view.messages.length;
    process.kill((c as unknown as { proc: { pid: number } }).proc.pid, 'SIGTERM');
    await expect.poll(() => view.states.filter(s => s === 'ready').length, { timeout: 15_000 }).toBe(2);
    c.send('V', { type: 'ready' });
    const again = await view.next('init', () => true, seen);
    expect(again.state.active?.id).toBe(id);
    expect(again.state.active?.turns).toHaveLength(2);
  });

  // An env fact that changes while the handshake runs (the hello in flight predates it) still lands before the views attach
  it('delivers an environment change made during the handshake before the views attach', async () => {
    const home = tmp('acpira-client-home-');
    const c = client([engine(home)], { blobBase: undefined });
    const view = new View('V');
    c.start();
    c.event({ type: 'envChanged', env: { blobBase: 'https://late.test' } });
    c.attach(view);
    c.send('V', { type: 'ready' });
    expect((await view.next('init')).state.blobBase).toBe('https://late.test');
  });

  // Each VS Code webview has its own resource URI for the sessions directory: a view's blob base wins over the env's and survives
  // the re-attach after a restart, whichever view attached last
  it('keeps each view on its own blob base across a restart', async () => {
    const home = tmp('acpira-client-home-');
    const c = client([engine(home)]);
    const v1 = new View('V1', 'sidebar', undefined, 'https://view-1.test/blobs');
    const v2 = new View('V2', 'editor', undefined, 'https://view-2.test/blobs');
    c.attach(v1);
    c.attach(v2);
    c.send('V1', { type: 'ready' });
    c.send('V2', { type: 'ready' });
    c.start();
    expect((await v1.next('init')).state.blobBase).toBe('https://view-1.test/blobs');
    expect((await v2.next('init')).state.blobBase).toBe('https://view-2.test/blobs');
    const seen = [v1.messages.length, v2.messages.length];
    process.kill((c as unknown as { proc: { pid: number } }).proc.pid, 'SIGTERM');
    await expect.poll(() => v1.states.filter(s => s === 'ready').length, { timeout: 15_000 }).toBe(2);
    c.send('V1', { type: 'ready' });
    c.send('V2', { type: 'ready' });
    expect((await v1.next('init', () => true, seen[0])).state.blobBase).toBe('https://view-1.test/blobs');
    expect((await v2.next('init', () => true, seen[1])).state.blobBase).toBe('https://view-2.test/blobs');
  });

  it('falls through to the next engine when one cannot start or dies before the handshake', async () => {
    const home = tmp('acpira-client-home-');
    const logs: string[] = [];
    const c = client([
      { command: join(home, 'missing-binary'), args: [], label: 'missing' },
      { command: process.execPath, args: ['-e', 'process.exit(3)'], label: 'dies' },
      engine(home),
    ], { logs });
    const view = new View('V');
    c.attach(view);
    c.send('V', { type: 'ready' });
    c.start();
    await view.next('init');
    expect(logs.some(l => l.includes('could not start (missing)'))).toBe(true);
    expect(logs.some(l => l.includes('before the handshake'))).toBe(true);
  });

  it('gives up after repeated failures and starts over on retry', async () => {
    const home = tmp('acpira-client-home-');
    const states: SidecarState[] = [];
    const commands = [{ command: process.execPath, args: ['-e', 'process.exit(1)'], label: 'dies' }];
    const c = client(commands, { states });
    c.start();
    await expect.poll(() => c.current.state, { timeout: 10_000 }).toBe('failed');
    commands.splice(0, 1, engine(home));
    const view = new View('V');
    c.attach(view);
    c.retry();
    c.send('V', { type: 'ready' });
    await view.next('init');
    expect(states.at(-1)).toBe('ready');
  });

  it('shuts the sidecar down on dispose', async () => {
    const home = tmp('acpira-client-home-');
    const c = client([engine(home)]);
    const view = new View('V');
    c.attach(view);
    c.send('V', { type: 'ready' });
    c.start();
    await view.next('init');
    const pid = (c as unknown as { proc: { pid: number } }).proc.pid;
    await c.dispose();
    expect(() => process.kill(pid, 0)).toThrow();
  });
});

describe('sidecarCommands', () => {
  function root(bin: string[] = [], mode = 0o755) {
    const r = tmp('acpira-ext-');
    for (const rel of bin) {
      mkdirSync(join(r, rel, '..'), { recursive: true });
      writeFileSync(join(r, rel), '');
      chmodSync(join(r, rel), mode);
    }
    return r;
  }

  it('starts the binary the platform package carries', () => {
    const r = root(['bin/acpira', 'dist/sidecar/mac-arm64/acpira']);
    expect(sidecarCommands({ root: r, env: {}, platform: 'darwin', arch: 'arm64' }).map(c => c.command)).toEqual([join(r, 'bin', 'acpira')]);
  });

  it('uses the build:sidecar output of a repository checkout for this machine only', () => {
    const r = root(['dist/sidecar/linux-x86_64/acpira']);
    expect(sidecarCommands({ root: r, env: {}, platform: 'linux', arch: 'x64' }).map(c => c.command)).toEqual([join(r, 'dist', 'sidecar', 'linux-x86_64', 'acpira')]);
    expect(sidecarCommands({ root: r, env: {}, platform: 'linux', arch: 'arm64' })).toEqual([]);
  });

  it('has nothing to start without a binary', () => {
    expect(sidecarCommands({ root: root(), env: {}, platform: 'win32', arch: 'x64' })).toEqual([]);
  });

  it('restores a lost executable bit', () => {
    const r = root(['bin/acpira'], 0o644);
    expect(sidecarCommands({ root: r, env: {}, platform: 'linux', arch: 'x64' })[0]?.command).toBe(join(r, 'bin', 'acpira'));
    expect(statSync(join(r, 'bin', 'acpira')).mode & 0o111).not.toBe(0);
  });

  it('honours ACPIRA_SIDECAR_BIN', () => {
    const r = root(['bin/acpira']);
    expect(sidecarCommands({ root: r, env: { ACPIRA_SIDECAR_BIN: '/opt/acpira' }, platform: 'darwin', arch: 'arm64' }).map(c => c.command)).toEqual(['/opt/acpira']);
  });
});
