import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { SIDECAR_PROTOCOL_VERSION, type SidecarMsg } from '../src/shared/sidecar';
import { Shell } from './sidecarShell';

describe('sidecar (host-server)', () => {
  const shells: Shell[] = [];
  const dirs: string[] = [];
  afterEach(async () => {
    for (const s of shells.splice(0)) await s.kill();
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  });

  function shell(cwdSuffix = '') {
    const home = mkdtempSync(join(tmpdir(), 'acpira-sidecar-home-'));
    const cwd = mkdtempSync(join(tmpdir(), `acpira-sidecar-ws-${cwdSuffix}`));
    dirs.push(home, cwd);
    const s = new Shell(home, cwd);
    shells.push(s);
    return s;
  }

  it('rejects a protocol version mismatch at hello and exits with code 2', async () => {
    const s = shell();
    const reply = await s.hello({ protocolVersion: SIDECAR_PROTOCOL_VERSION + 1 });
    expect(reply).toMatchObject({ type: 'helloReject', requestId: 'h1', protocolVersion: SIDECAR_PROTOCOL_VERSION });
    expect(await s.exited()).toBe(2);
  });

  it('two viewers on different sessions: each gets its own init and only its own session pushes', async () => {
    const s = shell();
    const ok = await s.hello();
    expect(ok).toMatchObject({ type: 'helloOk', sessionsDir: join(s.home, 'sessions'), sidecar: { pid: expect.any(Number) } });

    const a = await s.open('A', 'sidebar', { mostRecent: true });
    expect(a.state).toMatchObject({ host: 'sidebar', blobBase: 'https://acpira.local/blobs', cwd: s.cwd, locale: 'en' });
    const sessionA = a.state.active!.id;
    const b = await s.open('B', 'editor');
    const sessionB = b.state.active!.id;
    expect(b.state.host).toBe('editor');
    expect(sessionB).not.toBe(sessionA);

    // A turn on B streams to B only; A sees the list change (a global event) but no session frame for B
    s.view('B', { type: 'send', text: 'hi from B' });
    const doneB = await s.hostMsg('B', 'session', m => m.session.id === sessionB && !m.session.running && m.session.turns.length === 2);
    expect(doneB.session.turns[1]).toMatchObject({ role: 'agent' });
    expect(s.hostMsgs('A').filter(m => m.type === 'session' && m.session.id === sessionB)).toHaveLength(0);
    await s.hostMsg('A', 'sessions', m => m.sessions.some(x => x.id === sessionB && x.title === 'Fake title'));

    // Detaching A stops its pushes; B keeps working
    s.send({ type: 'detachView', viewId: 'A' });
    const before = s.hostMsgs('A').length;
    s.view('B', { type: 'send', text: 'again' });
    await s.hostMsg('B', 'session', m => !m.session.running && m.session.turns.length === 4);
    expect(s.hostMsgs('A').length).toBe(before);
    // A message for a detached / unknown view is dropped, not fatal
    s.view('A', { type: 'send', text: 'ghost' });
    s.view('B', { type: 'send', text: 'still here' });
    await s.hostMsg('B', 'session', m => !m.session.running && m.session.turns.length === 6);
    expect(s.stderr.some(l => l.includes('unknown view A'))).toBe(true);
  });

  it('dense session pushes coalesce per view and the idle frame arrives promptly', async () => {
    const s = shell();
    await s.hello();
    const init = await s.open('V');
    const id = init.state.active!.id;
    const t0 = Date.now();
    s.view('V', { type: 'send', text: 'hello' });
    const done = await s.hostMsg('V', 'session', m => m.session.id === id && !m.session.running && m.session.turns.length === 2);
    const elapsed = Date.now() - t0;
    const frames = s.hostMsgs('V').filter(m => m.type === 'session' && m.session.id === id);
    // The fake streams a thought, a plan, two text chunks, a title and a command list within a couple of ms: far more updates than frames
    expect(frames.length).toBeLessThan(6);
    const agent = done.session.turns[1];
    expect(agent?.role === 'agent' && agent.blocks.some(b => b.type === 'text' && b.markdown === 'hello world')).toBe(true);
    expect(elapsed).toBeLessThan(10_000);
  });

  it('platform RPC: openFile resolves against the session cwd, openPath reveals directories, searchFiles falls back to a workspace walk', async () => {
    const s = shell();
    s.answers.openResolvedFile = () => null;
    s.answers.revealInOS = () => null;
    await s.hello({ client: { name: 'test-shell', version: '0', capabilities: ['openResolvedFile', 'revealInOS', 'openExternal', 'toast'] } });
    const init = await s.open('V');
    const id = init.state.active!.id;
    mkdirSync(join(s.cwd, 'src'));
    writeFileSync(join(s.cwd, 'src', 'alpha.ts'), '');
    writeFileSync(join(s.cwd, 'beta.ts'), '');
    mkdirSync(join(s.cwd, 'node_modules', 'x'), { recursive: true });
    writeFileSync(join(s.cwd, 'node_modules', 'x', 'alpha.ts'), '');

    s.view('V', { type: 'openFile', sessionId: id, path: 'src/alpha.ts', line: 3 });
    const open = await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'openResolvedFile');
    expect(open.request).toEqual({ method: 'openResolvedFile', path: join(s.cwd, 'src', 'alpha.ts'), line: 3 });
    expect(open.requestId).toBeDefined();

    s.view('V', { type: 'openPath', path: join(s.cwd, 'src') });
    const reveal = await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'revealInOS');
    expect(reveal.request).toEqual({ method: 'revealInOS', path: join(s.cwd, 'src') });

    s.view('V', { type: 'openExternal', url: 'https://example.com/' });
    const ext = await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'openExternal');
    expect(ext.requestId).toBeUndefined();
    s.view('V', { type: 'openExternal', url: 'javascript:alert(1)' });

    // No searchFiles capability: the sidecar walks the workspace itself, skipping node_modules
    s.view('V', { type: 'searchFiles', query: 'alpha', seq: 1 });
    const files = await s.hostMsg('V', 'files', m => m.seq === 1);
    expect(files.files.map(f => f.path)).toEqual(['src/alpha.ts']);
    expect(s.requests.filter(r => r.request.method === 'openExternal')).toHaveLength(1);
  });

  it('platform RPC: a shell with searchFiles answers the @ search itself; an unanswered RPC does not block shutdown', async () => {
    const s = shell();
    s.answers.searchFiles = r => (r.method === 'searchFiles' ? [{ uri: 'file:///w/idx.ts', path: `idx-${r.query}.ts` }, { bogus: true }] : []);
    await s.hello({ client: { name: 'test-shell', version: '0', capabilities: ['searchFiles', 'openResolvedFile'] } });
    const init = await s.open('V');
    s.view('V', { type: 'searchFiles', query: 'q', seq: 2 });
    const files = await s.hostMsg('V', 'files', m => m.seq === 2);
    expect(files.files).toEqual([{ uri: 'file:///w/idx.ts', path: 'idx-q.ts' }]);

    // openResolvedFile is declared but never answered here: the request stays pending until shutdown fails it
    s.view('V', { type: 'openFile', sessionId: init.state.active!.id, path: 'x.ts' });
    await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'openResolvedFile');
    s.send({ type: 'shutdown' });
    await s.next((m): m is Extract<SidecarMsg, { type: 'shutdownOk' }> => m.type === 'shutdownOk');
    expect(await s.exited()).toBe(0);
  });

  it('login falls back to the shell terminal when the agent refuses authenticate; installAgent runs the install line there', async () => {
    const s = shell('needs-auth-');
    await s.hello(
      { client: { name: 'test-shell', version: '0', capabilities: ['runInTerminal', 'toast'] } },
      { env: { FAKE_AUTH_REJECT: '1' }, login: 'fake-login --now', install: { command: 'curl -fsSL https://example.com/i.sh | sh' } },
    );
    const init = await s.open('V');
    expect(init.state.active?.status).toBe('auth_required');
    s.view('V', { type: 'login' });
    const term = await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'runInTerminal');
    expect(term.request).toMatchObject({ method: 'runInTerminal', command: 'fake-login', args: ['--now'] });
    await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'toast');

    s.view('V', { type: 'installAgent', agent: 'fake' });
    const install = await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'runInTerminal' && m.request.command === 'bash');
    expect(install.request).toMatchObject({ command: 'bash', args: ['-c', 'curl -fsSL https://example.com/i.sh | sh'] });
  });

  it('settings: writes go to the shell and the snapshot updates at once; a settingsChanged event re-pushes appearance and settings', async () => {
    const s = shell();
    s.answers.writeSetting = () => null;
    await s.hello({ client: { name: 'test-shell', version: '0', capabilities: ['writeSetting'] } });
    await s.open('V');
    s.view('V', { type: 'setSetting', key: 'sessionScope', value: 'all' });
    const write = await s.next((m): m is Extract<SidecarMsg, { type: 'platformRequest' }> => m.type === 'platformRequest' && m.request.method === 'writeSetting');
    expect(write.request).toEqual({ method: 'writeSetting', key: 'sessionScope', value: 'all' });
    await s.hostMsg('V', 'settings', m => m.settings.sessionScope === 'all');

    s.send({ type: 'platformEvent', event: { type: 'settingsChanged', keys: ['appearance.motion', 'language'], settings: { defaultAgent: 'fake', 'appearance.motion': 'none', language: 'zh-CN' } } });
    await s.hostMsg('V', 'appearance', m => m.appearance.motion === 'none');
    await s.hostMsg('V', 'settings', m => m.locale === 'zh-CN');
  });

  it('stdout carries only envelopes; garbage on stdin is skipped; the shell closing its end shuts the sidecar down cleanly', async () => {
    const s = shell();
    await s.hello();
    s.raw('not json at all');
    s.raw('{"no":"type"}');
    await s.open('V');
    s.view('V', { type: 'send', text: 'hi' });
    await s.hostMsg('V', 'session', m => !m.session.running && m.session.turns.length === 2);
    for (const line of s.stdoutLines) expect(() => JSON.parse(line)).not.toThrow();
    expect(s.stderr.some(l => l.includes('ignoring non-JSON line'))).toBe(true);
    expect(s.stderr.some(l => l.includes('without a type'))).toBe(true);
    s.proc.stdin.end();
    expect(await s.exited()).toBe(0);
    expect(s.out.at(-1)?.type).not.toBe('shutdownOk');
  });
});
