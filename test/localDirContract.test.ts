import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { Shell } from './sidecarShell';

// The paired phone's add-project picker against the real sidecar: `localDir` of packages/shared/protocol.ts lists the
// computer's folders (project_files.rs `local_folders`): the home when no path is given, folders only, dot-folders
// left out, work trees marked, and helloOk.features announcing it so an older engine falls back to a typed path

describe.skipIf(process.platform === 'win32')('sidecar localDir', () => {
  const cleanups: (() => Promise<void> | void)[] = [];
  afterEach(async () => { for (const c of cleanups.splice(0)) await c(); });

  it('lists the home and a typed folder, folders only', async () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'acpira-localdir-')));
    const data = join(root, 'data'), home = join(root, 'home');
    for (const d of [data, join(home, 'code', 'acpira', '.git'), join(home, 'Notes'), join(home, '.cache')]) mkdirSync(d, { recursive: true });
    writeFileSync(join(home, 'todo.txt'), 'x');
    const s = new Shell(data, '', undefined, { HOME: home });
    cleanups.push(async () => { await s.kill(); rmSync(root, { recursive: true, force: true }); });
    const ok = await s.hello({ env: { hostLanguage: 'en', projects: true, blobBase: 'https://acpira.local/blobs' } });
    if (ok.type !== 'helloOk') throw new Error('handshake refused');
    expect(ok.features).toEqual(expect.arrayContaining(['localDir', 'providers']));
    await s.open('V', 'editor');

    s.view('V', { type: 'localDir', seq: 1 });
    const top = await s.hostMsg('V', 'localDir', m => m.seq === 1);
    expect(top).toMatchObject({ path: home, parent: dirname(home) });
    expect(top.entries).toEqual([{ name: 'code', dir: true }, { name: 'Notes', dir: true }]);

    s.view('V', { type: 'localDir', path: '~/code', seq: 2 });
    const code = await s.hostMsg('V', 'localDir', m => m.seq === 2);
    expect(code.path).toBe(join(home, 'code'));
    expect(code.entries).toEqual([{ name: 'acpira', dir: true, git: true }]);

    s.view('V', { type: 'localDir', path: join(home, 'missing'), seq: 3 });
    const missing = await s.hostMsg('V', 'localDir', m => m.seq === 3);
    expect(missing.error).toBeTruthy();
    expect(missing.entries).toEqual([]);
  });
});
