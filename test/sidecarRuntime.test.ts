import { execFileSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { afterEach, describe, expect, it } from 'vitest';
import type { EditTurnRequest, HostMsg } from '../src/shared/protocol';
import type { PlatformRequest, SidecarMsg } from '../src/shared/sidecar';
import { captureTurnSettings } from '../src/shared/turnSettings';
import { FAKE, SIDECAR, Shell, TSX } from './sidecarShell';

// The runtime behind the envelope protocol (settings snapshot and events, platform RPCs, view routing) as a shell sees it:
// what the VS Code and IntelliJ shells both rely on

const CAPABILITIES = ['openResolvedFile', 'openPlanDocument', 'revealInOS', 'searchFiles', 'writeSetting', 'openExternal', 'openInEditor', 'runInTerminal', 'toast'] as const;

async function until(pred: () => boolean, ms = 5000) {
  const start = Date.now();
  while (!pred()) {
    if (Date.now() - start > ms) throw new Error('timed out waiting');
    await new Promise(r => setTimeout(r, 10));
  }
}

describe('sidecar runtime', () => {
  const cleanups: (() => Promise<void> | void)[] = [];
  afterEach(async () => { for (const c of cleanups.splice(0)) await c(); });

  async function setup(extraAgents: Record<string, unknown> = {}) {
    const home = mkdtempSync(join(tmpdir(), 'acpira-runtime-'));
    const cwd = mkdtempSync(join(tmpdir(), 'acpira-runtime-ws-'));
    const settings: Record<string, unknown> = {
      agents: { fake: { name: 'Fake', command: TSX, args: [FAKE], install: { command: 'curl -fsSL https://example.com/install.sh | sh' } }, ...extraAgents },
      defaultAgent: 'fake',
    };
    const s = new Shell(home, cwd);
    cleanups.push(async () => { await s.kill(); rmSync(home, { recursive: true, force: true }); rmSync(cwd, { recursive: true, force: true }); });
    // The shell owns the settings store: a write lands there and comes back as a settingsChanged event, like the IDE shells do
    const change = (keys: string[]) => s.send({ type: 'platformEvent', event: { type: 'settingsChanged', keys, settings: { ...settings } } });
    s.answers = {
      writeSetting: r => { if (r.method === 'writeSetting') { settings[r.key] = r.value; setTimeout(() => change([r.key]), 0); } return null; },
      searchFiles: r => r.method === 'searchFiles' ? [{ uri: `file://${cwd}/${r.query}.ts`, path: `${r.query}.ts` }] : [],
      openResolvedFile: () => null,
      openPlanDocument: () => null,
      revealInOS: () => null,
    };
    const ok = await s.hello({ client: { name: 'runtime-test', version: '0', capabilities: [...CAPABILITIES] }, settings });
    if (ok.type !== 'helloOk') throw new Error('handshake refused');
    const init = await s.open('V');
    const posted = () => s.hostMsgs('V');
    const requests = (method: PlatformRequest['method']) => s.requests.filter(r => r.request.method === method).map(r => r.request);
    const logs = () => s.stderr;
    return { s, home, cwd, settings, change, init, posted, requests, logs, sessionsDir: ok.sessionsDir };
  }

  const toasts = (s: Shell) => s.requests.map(r => r.request).filter((r): r is Extract<PlatformRequest, { method: 'toast' }> => r.method === 'toast');
  const bridge = (home: string, args: string[]) => execFileSync(SIDECAR, ['bridge', ...args, '--home', home], { encoding: 'utf8' });

  it('shows ChatGPT as an external conversation and forwards live events without spawning another agent', async () => {
    const { s, home, cwd, init, posted, logs } = await setup();
    const nativeId = init.state.active!.id;
    await s.hostMsg('V', 'session', m => m.session.id === nativeId && m.session.status === 'ready');
    const opened = JSON.parse(bridge(home, ['open', '--key', 'runtime-test-source', '--cwd', cwd, '--title', 'ChatGPT runtime test'])) as { sessionId: string };
    const mirror = opened.sessionId;
    // A window focus reconciles the index, which is where the mirror written by the CLI shows up
    s.send({ type: 'platformEvent', event: { type: 'windowFocus' } });
    await s.hostMsg('V', 'sessions', m => m.sessions.some(x => x.id === mirror));
    const spawnsBefore = logs().filter(l => l.includes('spawn ')).length;
    s.view('V', { type: 'selectSession', id: mirror });
    const shown = await s.hostMsg('V', 'session', m => m.session.id === mirror);
    expect(shown.session).toMatchObject({ id: mirror, agent: 'chatgpt', status: 'readonly' });
    expect(init.state.agents.some(a => a.id === 'chatgpt') || posted().some(m => m.type === 'agents' && m.agents.some(a => a.id === 'chatgpt' && a.external))).toBe(true);
    bridge(home, ['prompt', '--session', mirror, '--turn', 'test-turn', '--text', 'Visible test prompt']);
    bridge(home, ['message', '--session', mirror, '--turn', 'test-turn', '--message', 'm', '--text', 'Visible test progress']);
    s.send({ type: 'platformEvent', event: { type: 'windowFocus' } });
    await s.hostMsg('V', 'session', m => m.session.id === mirror && m.session.turns.length === 2);
    s.view('V', { type: 'send', sessionId: mirror, text: 'must not be sent to an ACP' });
    await until(() => toasts(s).some(t => t.level === 'error' && t.text.includes('ChatGPT')));
    s.view('V', { type: 'renameSession', id: mirror, title: 'Renamed mirror' });
    s.view('V', { type: 'pinSession', id: mirror, pinned: true });
    await s.hostMsg('V', 'sessions', m => m.sessions.some(x => x.id === mirror && x.title === 'Renamed mirror' && x.pinned && x.external));
    s.view('V', { type: 'selectSession', id: nativeId });
    const back = await s.hostMsg('V', 'session', m => m.session.id === nativeId && posted().filter(x => x.type === 'session').length > 0);
    expect(back.session.agent).toBe('fake');
    expect(logs().filter(l => l.includes('spawn ')).length).toBe(spawnsBefore);
  });

  it('reports project mirror receipts separately from unverifiable cloud pairing', async () => {
    const { s, home, cwd } = await setup();
    const { sessionId } = JSON.parse(bridge(home, ['open', '--key', 'status-fixture', '--cwd', cwd, '--title', 'Status'])) as { sessionId: string };
    bridge(home, ['prompt', '--session', sessionId, '--turn', 't', '--text', 'Status fixture']);
    s.view('V', { type: 'chatgptStatus' });
    const result = await s.hostMsg('V', 'chatgptStatus');
    expect(result).toMatchObject({ type: 'chatgptStatus', status: {
      desktopCommander: { pairing: 'unknown' },
      project: { mirrors: 1, observedMirrors: 1, latestSessionId: sessionId },
    } });
  });

  it('answers ready with the full init state built from the hello snapshot', async () => {
    const { init, cwd, home, sessionsDir } = await setup();
    expect(init.state).toMatchObject({ host: 'sidebar', blobBase: 'https://acpira.local/blobs', cwd, home: homedir(), locale: 'en' });
    expect(init.state.agents.map(a => a.id)).toContain('fake');
    expect(init.state.active).toMatchObject({ agent: 'fake', cwd });
    expect(init.state.settings.defaultAgent).toBe('fake');
    expect(sessionsDir).toBe(join(home, 'sessions'));
    expect(init.state.sessions.length).toBe(1);
  });

  it('resolves openFile against the session cwd before handing it to the shell, and ignores another session', async () => {
    const { s, init, cwd, requests } = await setup();
    const id = init.state.active!.id;
    const opened = () => requests('openResolvedFile') as Extract<PlatformRequest, { method: 'openResolvedFile' }>[];
    s.view('V', { type: 'openFile', sessionId: id, path: 'src/a.ts', line: 12 });
    await until(() => opened().length === 1);
    expect(opened()[0]).toMatchObject({ path: resolve(cwd, 'src/a.ts'), line: 12 });
    s.view('V', { type: 'openFile', sessionId: id, path: pathToFileURL(join(cwd, 'b.ts')).href, line: 0 });
    await until(() => opened().length === 2);
    expect(opened()[1]!.path).toBe(join(cwd, 'b.ts'));
    expect(opened()[1]!.line).toBeUndefined();
    s.view('V', { type: 'openFile', sessionId: 'someone-else', path: 'src/a.ts' });
    // A shell failure surfaces as an error toast, not an exception
    s.answers.openResolvedFile = () => { throw new Error('no editor'); };
    s.view('V', { type: 'openFile', sessionId: id, path: 'c.ts' });
    await until(() => toasts(s).some(t => t.level === 'error' && t.text.includes('no editor')));
    expect(opened().length).toBe(3);
  });

  it('only forwards allowlisted external URLs', async () => {
    const { s, requests, logs } = await setup();
    s.view('V', { type: 'openExternal', url: 'javascript:alert(1)' });
    s.view('V', { type: 'openExternal', url: 'file:///etc/passwd' });
    s.view('V', { type: 'openExternal', url: 'https://example.com/x' });
    await until(() => requests('openExternal').length === 1);
    expect(requests('openExternal')[0]).toMatchObject({ url: 'https://example.com/x' });
    await until(() => logs().filter(l => l.includes('openExternal refused')).length === 2);
  });

  it('openPath reveals directories and opens files; openInEditor defaults to the view\'s own session', async () => {
    const { s, cwd, init, requests } = await setup();
    const dir = join(cwd, 'skills');
    mkdirSync(dir);
    writeFileSync(join(dir, 'SKILL.md'), '# x');
    s.view('V', { type: 'openPath', path: dir });
    await until(() => requests('revealInOS').length === 1);
    expect(requests('revealInOS')[0]).toMatchObject({ path: dir });
    s.view('V', { type: 'openPath', path: join(dir, 'SKILL.md') });
    await until(() => requests('openResolvedFile').length === 1);
    expect(requests('openResolvedFile')[0]).toMatchObject({ path: join(dir, 'SKILL.md') });
    s.view('V', { type: 'openInEditor' });
    s.view('V', { type: 'openInEditor', sessionId: 'other' });
    await until(() => requests('openInEditor').length === 2);
    expect(requests('openInEditor')).toEqual([{ method: 'openInEditor', sessionId: init.state.active!.id }, { method: 'openInEditor', sessionId: 'other' }]);
  });

  it('answers searchFiles with the shell\'s hits and the request seq, empty on failure', async () => {
    const { s } = await setup();
    s.view('V', { type: 'searchFiles', query: 'foo', seq: 7 });
    expect(await s.hostMsg('V', 'files', m => m.seq === 7)).toMatchObject({ type: 'files', seq: 7, files: [{ path: 'foo.ts' }] });
    s.answers.searchFiles = () => { throw new Error('index down'); };
    s.view('V', { type: 'searchFiles', query: 'bar', seq: 8 });
    expect(await s.hostMsg('V', 'files', m => m.seq === 8)).toEqual({ type: 'files', seq: 8, files: [] });
  });

  it('installAgent runs the registry install line through the shell terminal', async () => {
    const { s, requests } = await setup();
    s.view('V', { type: 'installAgent', agent: 'fake' });
    await until(() => requests('runInTerminal').length === 1);
    expect(requests('runInTerminal')[0]).toMatchObject({ title: expect.stringContaining('Fake'),
      command: process.platform === 'win32' ? 'powershell' : 'bash', args: [process.platform === 'win32' ? '-Command' : '-c', 'curl -fsSL https://example.com/install.sh | sh'] });
    await until(() => toasts(s).some(t => t.level === 'info'));
  });

  it('setSetting writes through the shell and re-pushes the settings view to every attached view', async () => {
    const { s, requests } = await setup();
    await s.open('E', 'editor');
    const settingsSeen = (viewId: string) => s.hostMsgs(viewId).filter(m => m.type === 'settings').length;
    const [before, beforeE] = [settingsSeen('V'), settingsSeen('E')];
    s.view('V', { type: 'setSetting', key: 'defaultAgent', value: 'fake' });
    await until(() => requests('writeSetting').length === 1);
    expect(requests('writeSetting')[0]).toMatchObject({ key: 'defaultAgent', value: 'fake' });
    await until(() => settingsSeen('V') > before && settingsSeen('E') > beforeE);
  });

  it('uses live compaction settings and publishes identical usage to sidebar and editor', async () => {
    const { s, init } = await setup();
    const sessionId = init.state.active!.id;
    await s.open('E', 'editor', sessionId);
    const latest = (viewId: string) => s.hostMsgs(viewId).filter((m): m is Extract<HostMsg, { type: 'session' }> => m.type === 'session' && m.session.id === sessionId).at(-1)?.session;
    s.view('V', { type: 'setSetting', key: 'compactAtTokens', value: 500_000 });
    await s.next((m): m is SidecarMsg => m.type === 'platformRequest' && m.request.method === 'writeSetting');
    await new Promise(r => setTimeout(r, 50));
    s.view('V', { type: 'send', sessionId, text: 'big' });
    await until(() => latest('V')?.usage?.used === 401234 && latest('E')?.usage?.used === 401234);
    expect(latest('V')?.turns.some(t => t.role === 'user' && t.auto)).toBe(false);
    s.view('E', { type: 'setSetting', key: 'compactAtTokens', value: 300_000 });
    await until(() => s.requests.filter(r => r.request.method === 'writeSetting').length === 2);
    await new Promise(r => setTimeout(r, 50));
    s.view('E', { type: 'send', sessionId, text: 'hi' });
    await until(() => latest('V')?.usage?.used === 80247 && latest('E')?.usage?.used === 80247, 10_000);
    expect(latest('V')?.turns.filter(t => t.role === 'user' && t.auto)).toHaveLength(1);
    expect(latest('E')?.usage).toEqual(latest('V')?.usage);
  });

  it('follows settings change events: appearance pushes, agents swap the registry, other keys re-emit settings', async () => {
    const { s, settings, change, posted } = await setup();
    settings['appearance.motion'] = 'none';
    change(['appearance.motion']);
    await until(() => posted().some(m => m.type === 'appearance' && m.appearance.motion === 'none'));
    settings.hiddenOptions = { fake: { model: ['x'] } };
    change(['hiddenOptions']);
    await until(() => posted().some(m => m.type === 'hidden'));
    settings.agents = { fake: { name: 'Renamed', command: TSX, args: [FAKE] } };
    change(['agents']);
    await until(() => posted().some(m => m.type === 'agents' && m.agents.find(a => a.id === 'fake')?.name === 'Renamed'));
    settings.language = 'zh-CN';
    change(['language']);
    await until(() => posted().some(m => m.type === 'settings' && m.locale === 'zh-CN'));
    expect(s.exitCode).toBeNull();
  });

  it('window focus re-probes executables and reconciles the session index', async () => {
    const bin = join(mkdtempSync(join(tmpdir(), 'acpira-focus-')), 'ghost-cli');
    const { s, home, cwd, posted, init } = await setup({ ghost: { name: 'Ghost', command: bin } });
    const ghost = () => (posted().filter(m => m.type === 'agents').at(-1)?.agents ?? init.state.agents).find(a => a.id === 'ghost')?.available;
    await until(() => ghost() === false);
    // A CLI installed meanwhile, and a session another window created in the shared directory
    writeFileSync(bin, '#!/bin/sh\nexit 0\n');
    chmodSync(bin, 0o755);
    const other = new Shell(home, cwd);
    cleanups.push(() => other.kill());
    await other.hello();
    const otherInit = await other.open('O');
    const otherId = otherInit.state.active!.id;
    other.view('O', { type: 'send', sessionId: otherId, text: 'hi' });
    await other.hostMsg('O', 'session', m => m.session.id === otherId && !m.session.running && m.session.turns.length === 2);
    await other.kill();
    s.send({ type: 'platformEvent', event: { type: 'windowFocus' } });
    await until(() => ghost() === true);
    await s.hostMsg('V', 'sessions', m => m.sessions.some(x => x.id === otherId));
  });

  it('a detached view stops receiving messages', async () => {
    const { s } = await setup();
    await s.open('E', 'editor');
    s.send({ type: 'detachView', viewId: 'V' });
    const n = s.hostMsgs('V').length;
    s.view('E', { type: 'newSession' });
    await s.hostMsg('E', 'session', m => m.session.status === 'ready' && m.session.turns.length === 0);
    await new Promise(r => setTimeout(r, 50));
    expect(s.hostMsgs('V').length).toBe(n);
  });

  it('routes a continue editTurn: acknowledged, appended natively, a stale repeat rejected', async () => {
    const { s, init } = await setup();
    const sessionId = init.state.active!.id;
    const latest = () => s.hostMsgs('V').filter((m): m is Extract<HostMsg, { type: 'session' }> => m.type === 'session' && m.session.id === sessionId).at(-1)?.session;
    const lastStop = () => { const last = latest()?.turns.at(-1); return last?.role === 'agent' ? last.stop : undefined; };
    s.view('V', { type: 'send', sessionId, text: 'original' });
    await until(() => latest()?.turns.length === 2 && lastStop() === 'end_turn');
    const view = latest()!;
    const turn = view.turns[0]!;
    if (turn.role !== 'user') throw new Error('Missing user turn');
    const edit: EditTurnRequest = { sessionId, turnIndex: 0, turnCount: view.turns.length, originalText: turn.text, turnId: turn.id,
      text: 'inspect-history', retainedAttachments: [], attachments: [], settings: captureTurnSettings(view.controls), intent: 'continue' };
    s.view('V', { type: 'editTurn', requestId: 'continue-request', edit });
    expect(await s.hostMsg('V', 'editTurnResult', m => m.requestId === 'continue-request')).toEqual({ type: 'editTurnResult', requestId: 'continue-request' });
    await until(() => latest()?.turns.length === 4 && lastStop() === 'end_turn');
    const after = latest()!;
    expect(after.turns.slice(0, 2)).toEqual(view.turns);
    expect(after.turns[2]).toMatchObject({ role: 'user', text: 'inspect-history' });
    expect(after.turns[2]).not.toHaveProperty('edited');
    s.view('V', { type: 'editTurn', requestId: 'stale-repeat', edit });
    expect(await s.hostMsg('V', 'editTurnResult', m => m.requestId === 'stale-repeat')).toMatchObject({ error: expect.any(String) });
    expect(latest()!.turns).toHaveLength(4);
  });

  it('exportSession writes the file under exports/ and opens it; an unknown id toasts the error', async () => {
    const { s, init, requests } = await setup();
    const sessionId = init.state.active!.id;
    s.view('V', { type: 'send', sessionId, text: 'hi' });
    await s.hostMsg('V', 'session', m => m.session.id === sessionId && !m.session.running && m.session.turns.length === 2);
    s.view('V', { type: 'exportSession', id: sessionId, format: 'markdown' });
    await until(() => requests('openResolvedFile').length === 1);
    const opened = (requests('openResolvedFile')[0] as Extract<PlatformRequest, { method: 'openResolvedFile' }>).path;
    expect(opened).toMatch(/exports[/\\][^/\\]+\.md$/);
    expect(readFileSync(opened, 'utf8')).toContain('hello world');
    await until(() => toasts(s).some(t => t.level === 'info' && t.text.includes(opened)));
    s.view('V', { type: 'exportSession', id: 'no-such-session', format: 'json' });
    await until(() => toasts(s).some(t => t.level === 'error'));
  });
});
