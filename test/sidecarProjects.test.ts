import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import type { HostMsg } from '../src/shared/protocol';
import { NODE, FAKE, Shell } from './sidecarShell';

// A project shell (`ShellEnv.projects`, the desktop app) against the real sidecar: no workspace in hello, so init, @ search
// and the settings page's project files follow the session the view shows, and nothing starts before a project exists

describe('sidecar project shell', () => {
  const cleanups: (() => Promise<void> | void)[] = [];
  afterEach(async () => { for (const c of cleanups.splice(0)) await c(); });

  it('starts without a session, then searches and scans the shown session\'s project', async () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'acpira-projects-')));
    const data = join(root, 'data'), home = join(root, 'home'), p1 = join(root, 'p1');
    for (const d of [data, home, p1]) mkdirSync(d, { recursive: true });
    execFileSync('git', ['init', '-q', p1]);
    writeFileSync(join(p1, 'alpha.ts'), '');
    // HOME is the platform's home: the "no project" fallback must land there, never in the real one
    const s = new Shell(data, '', undefined, { HOME: home });
    cleanups.push(async () => { await s.kill(); rmSync(root, { recursive: true, force: true }); });
    const ok = await s.hello({
      env: { hostLanguage: 'en', projects: true, blobBase: 'https://acpira.local/blobs' },
      settings: { defaultAgent: 'fake', agents: { fake: { name: 'Fake', command: NODE, args: [FAKE] } } },
    });
    if (ok.type !== 'helloOk') throw new Error('handshake refused');
    expect(ok.features).toContain('projects');

    const init = await s.open('V', 'editor');
    expect(init.state.active).toBeUndefined();
    expect(init.state.cwd).toBe(home);
    // Nothing comes up on its own: the page shows its empty state
    await new Promise(r => setTimeout(r, 300));
    expect(s.hostMsgs('V').some(m => m.type === 'session')).toBe(false);
    s.view('V', { type: 'searchFiles', query: 'alpha', seq: 1 });
    expect((await s.hostMsg('V', 'files', m => m.seq === 1)).files).toEqual([]);
    s.view('V', { type: 'shared' });
    expect((await s.hostMsg('V', 'shared')).view.root).toBeUndefined();

    s.view('V', { type: 'categoryOp', op: 'addProject', cwd: p1 });
    const listed = await s.hostMsg('V', 'categories', m => !!m.categories.projects?.length);
    expect(listed.categories.projects).toEqual([p1]);
    s.view('V', { type: 'newSession', cwd: p1 });
    const shown = await s.hostMsg('V', 'session');
    expect(shown.session.cwd).toBe(p1);

    s.view('V', { type: 'searchFiles', query: 'alpha', seq: 2 });
    expect((await s.hostMsg('V', 'files', m => m.seq === 2)).files.map(f => f.path)).toEqual(['alpha.ts']);
    s.view('V', { type: 'shared' });
    const shared = await s.hostMsg('V', 'shared', (m: Extract<HostMsg, { type: 'shared' }>) => m.view.root !== undefined);
    expect(realpathSync(shared.view.root!)).toBe(p1);
  });
});
