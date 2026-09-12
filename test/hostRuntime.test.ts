import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { HostMsg } from '../src/shared/protocol';
import type { HostPlatform, SettingsAffects } from '../src/host/platform';
import { createHostRuntime, type HostRuntime } from '../src/host/runtime';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));

// A HostPlatform made of spies and a settings map: what the sidecar and the VS Code extension both have to provide
function fakePlatform(cwd: string) {
  const settings = new Map<string, unknown>([
    ['agents', { fake: { name: 'Fake', command: TSX, args: [FAKE], install: { command: 'curl -fsSL https://example.com/install.sh | sh' } } }],
    ['defaultAgent', 'fake'],
  ]);
  let settingsListener: ((affects: SettingsAffects) => void) | undefined;
  let focusListener: (() => void) | undefined;
  const logs: string[] = [];
  const platform = {
    log: (line: string) => { logs.push(line); },
    hostLanguage: () => 'en',
    home: () => '/home/tester',
    cwd: () => cwd,
    readSetting: (key: string) => settings.get(key),
    writeSetting: vi.fn(async (key: string, value: unknown) => { settings.set(key, value); }),
    onSettingsChanged: (fn: (affects: SettingsAffects) => void) => { settingsListener = fn; return () => { settingsListener = undefined; }; },
    onWindowFocus: (fn: () => void) => { focusListener = fn; return () => { focusListener = undefined; }; },
    toast: vi.fn(),
    runInTerminal: vi.fn(),
    openResolvedFile: vi.fn(async () => {}),
    openPlanDocument: vi.fn(async () => {}),
    openExternal: vi.fn(),
    revealInOS: vi.fn(async () => {}),
    openInEditor: vi.fn(),
    searchFiles: vi.fn(async (query: string) => [{ uri: `file://${cwd}/${query}.ts`, path: `${query}.ts` }]),
  } satisfies HostPlatform;
  // Simulate the IDE: a setting changed under acpira.<key>
  const change = (key: string) => settingsListener?.(section => section === undefined || key === section || key.startsWith(`${section}.`));
  return { platform, settings, logs, change, focus: () => focusListener?.() };
}

async function until(pred: () => boolean, ms = 2000) {
  const start = Date.now();
  while (!pred()) {
    if (Date.now() - start > ms) throw new Error('timed out waiting');
    await new Promise(r => setTimeout(r, 10));
  }
}

describe('HostRuntime + BridgeCore', () => {
  const cleanups: (() => Promise<void> | void)[] = [];
  afterEach(async () => { for (const c of cleanups.splice(0)) await c(); });

  async function setup() {
    const home = mkdtempSync(join(tmpdir(), 'acpira-runtime-'));
    const cwd = mkdtempSync(join(tmpdir(), 'acpira-runtime-ws-'));
    const fake = fakePlatform(cwd);
    const runtime: HostRuntime = await createHostRuntime(fake.platform, { home });
    const posted: HostMsg[] = [];
    const core = runtime.attachView({ host: 'sidebar', blobBase: 'https://acpira.local/blobs', post: m => posted.push(m) });
    cleanups.push(async () => { await runtime.dispose(); rmSync(home, { recursive: true, force: true }); rmSync(cwd, { recursive: true, force: true }); });
    await core.handle({ type: 'ready' });
    const init = posted.find(m => m.type === 'init');
    if (init?.type !== 'init') throw new Error('no init');
    return { runtime, core, posted, init, cwd, home, ...fake };
  }

  it('builds the runtime from the platform and answers ready with the full init state', async () => {
    const { init, cwd, runtime, home } = await setup();
    expect(init.state).toMatchObject({ host: 'sidebar', blobBase: 'https://acpira.local/blobs', cwd, home: '/home/tester', locale: 'en' });
    expect(init.state.agents.map(a => a.id)).toContain('fake');
    expect(init.state.active).toMatchObject({ agent: 'fake', cwd });
    expect(init.state.settings.defaultAgent).toBe('fake');
    expect(runtime.sessionsDir).toBe(join(home, 'sessions'));
    // No legacy tree on this platform: nothing was migrated, the data dir is the one we gave
    expect(init.state.sessions.length).toBe(1);
  });

  it('resolves openFile against the session cwd before handing it to the platform, and ignores another session', async () => {
    const { core, init, cwd, platform } = await setup();
    const id = init.state.active!.id;
    await core.handle({ type: 'openFile', sessionId: id, path: 'src/a.ts', line: 12 });
    expect(platform.openResolvedFile).toHaveBeenLastCalledWith(resolve(cwd, 'src/a.ts'), 12);
    await core.handle({ type: 'openFile', sessionId: id, path: pathToFileURL(join(cwd, 'b.ts')).href, line: 0 });
    expect(platform.openResolvedFile).toHaveBeenLastCalledWith(join(cwd, 'b.ts'), undefined);
    platform.openResolvedFile.mockClear();
    await core.handle({ type: 'openFile', sessionId: 'someone-else', path: 'src/a.ts' });
    expect(platform.openResolvedFile).not.toHaveBeenCalled();
    // A platform failure surfaces as an error toast, not an exception
    platform.openResolvedFile.mockRejectedValueOnce(new Error('no editor'));
    await core.handle({ type: 'openFile', sessionId: id, path: 'c.ts' });
    expect(platform.toast).toHaveBeenLastCalledWith('error', 'no editor');
  });

  it('only forwards allowlisted external URLs', async () => {
    const { core, platform, logs } = await setup();
    await core.handle({ type: 'openExternal', url: 'https://example.com/x' });
    expect(platform.openExternal).toHaveBeenLastCalledWith('https://example.com/x');
    await core.handle({ type: 'openExternal', url: 'javascript:alert(1)' });
    await core.handle({ type: 'openExternal', url: 'file:///etc/passwd' });
    expect(platform.openExternal).toHaveBeenCalledTimes(1);
    expect(logs.filter(l => l.startsWith('openExternal refused'))).toHaveLength(2);
  });

  it('openPath reveals directories and opens files; openInEditor defaults to the view\'s own session', async () => {
    const { core, platform, cwd, init } = await setup();
    const dir = join(cwd, 'skills');
    mkdirSync(dir);
    writeFileSync(join(dir, 'SKILL.md'), '# x');
    await core.handle({ type: 'openPath', path: dir });
    expect(platform.revealInOS).toHaveBeenLastCalledWith(dir);
    await core.handle({ type: 'openPath', path: join(dir, 'SKILL.md') });
    expect(platform.openResolvedFile).toHaveBeenLastCalledWith(join(dir, 'SKILL.md'));
    await core.handle({ type: 'openInEditor' });
    expect(platform.openInEditor).toHaveBeenLastCalledWith(init.state.active!.id);
    await core.handle({ type: 'openInEditor', sessionId: 'other' });
    expect(platform.openInEditor).toHaveBeenLastCalledWith('other');
  });

  it('answers searchFiles with the platform\'s hits and the request seq, empty on failure', async () => {
    const { core, posted, platform } = await setup();
    await core.handle({ type: 'searchFiles', query: 'foo', seq: 7 });
    expect(posted.at(-1)).toMatchObject({ type: 'files', seq: 7, files: [{ path: 'foo.ts' }] });
    platform.searchFiles.mockRejectedValueOnce(new Error('index down'));
    await core.handle({ type: 'searchFiles', query: 'bar', seq: 8 });
    expect(posted.at(-1)).toEqual({ type: 'files', seq: 8, files: [] });
  });

  it('installAgent runs the registry install line through the platform terminal', async () => {
    const { core, platform } = await setup();
    await core.handle({ type: 'installAgent', agent: 'fake' });
    expect(platform.runInTerminal).toHaveBeenCalledWith(expect.stringContaining('Fake'), 'bash', ['-c', 'curl -fsSL https://example.com/install.sh | sh']);
    expect(platform.toast).toHaveBeenCalledWith('info', expect.any(String));
  });

  it('setSetting writes through the platform and re-pushes the settings view to every attached view', async () => {
    const { core, posted, platform, runtime } = await setup();
    const second: HostMsg[] = [];
    const other = runtime.attachView({ host: 'editor', post: m => second.push(m) });
    await other.handle({ type: 'ready' });
    await core.handle({ type: 'setSetting', key: 'defaultAgent', value: 'fake' });
    expect(platform.writeSetting).toHaveBeenCalledWith('defaultAgent', 'fake');
    // SettingsCenter.set emits at once; the bridge batches it for 30 ms
    await until(() => posted.some(m => m.type === 'settings') && second.some(m => m.type === 'settings'));
    runtime.detachView(other);
  });

  it('uses live platform compaction settings and publishes identical usage to sidebar and editor', async () => {
    const { core, posted, runtime, init } = await setup();
    const second: HostMsg[] = [];
    const other = runtime.attachView({ host: 'editor', post: message => second.push(message) });
    await other.handle({ type: 'ready' });
    const sessionId = init.state.active!.id;
    await other.handle({ type: 'selectSession', id: sessionId });
    await core.handle({ type: 'setSetting', key: 'compactAtTokens', value: 500_000 });
    await core.handle({ type: 'send', sessionId, text: 'big' });
    const latest = (messages: HostMsg[]) => messages.filter((message): message is Extract<HostMsg, { type: 'session' }> => message.type === 'session' && message.session.id === sessionId).at(-1)?.session;
    await until(() => latest(posted)?.usage?.used === 401234 && latest(second)?.usage?.used === 401234);
    expect(latest(posted)?.turns.some(turn => turn.role === 'user' && turn.auto)).toBe(false);
    await other.handle({ type: 'setSetting', key: 'compactAtTokens', value: 300_000 });
    await other.handle({ type: 'send', sessionId, text: 'hi' });
    await until(() => latest(posted)?.usage?.used === 80247 && latest(second)?.usage?.used === 80247);
    expect(latest(posted)?.turns.filter(turn => turn.role === 'user' && turn.auto)).toHaveLength(1);
    expect(latest(second)?.usage).toEqual(latest(posted)?.usage);
    runtime.detachView(other);
  });

  it('follows the platform\'s settings change events: appearance pushes, agents swap the registry, other keys re-emit settings', async () => {
    const { core, posted, settings, change, runtime } = await setup();
    settings.set('appearance.motion', 'none');
    change('appearance.motion');
    expect(posted.at(-1)).toMatchObject({ type: 'appearance', appearance: { motion: 'none' } });
    const before = posted.length;
    settings.set('hiddenOptions', { fake: { model: ['x'] } });
    change('hiddenOptions');
    await until(() => posted.slice(before).some(m => m.type === 'hidden'));
    settings.set('agents', { fake: { name: 'Renamed', command: TSX, args: [FAKE] } });
    change('agents');
    expect(runtime.manager.agents().find(a => a.id === 'fake')?.name).toBe('Renamed');
    const before2 = posted.length;
    settings.set('language', 'zh-CN');
    change('language');
    await until(() => posted.slice(before2).some(m => m.type === 'settings' && m.locale === 'zh-CN'));
    expect(core.viewer.activeId).toBeDefined();
  });

  it('window focus re-probes executables and reconciles the session index', async () => {
    const { runtime, focus } = await setup();
    const reprobe = vi.spyOn(runtime.manager, 'reprobe');
    const refresh = vi.spyOn(runtime.manager, 'refreshIndex');
    focus();
    expect(reprobe).toHaveBeenCalled();
    expect(refresh).toHaveBeenCalled();
  });

  it('detaching a view stops its messages; disposing the runtime tears every view down', async () => {
    const { runtime, core, posted } = await setup();
    runtime.detachView(core);
    const n = posted.length;
    await runtime.manager.newSession();
    await new Promise(r => setTimeout(r, 50));
    expect(posted.length).toBe(n);
    await runtime.dispose();
    await runtime.dispose();
  });
});
