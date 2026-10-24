import { chmodSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { SIDECAR, Shell } from './sidecarShell';

// SSH hosts through the real sidecar: the connection messages of packages/shared/protocol.ts against
// rust/crates/acpira-host/src/remote/connections.rs. ssh is a fake that runs the remote command under a temp "host" home
// (ACPIRA_SSH), and the helper it deploys is a wrapper around this sidecar binary (ACPIRA_REMOTE_BIN)

function fakeSsh(dir: string, hostHome: string) {
  const ssh = join(dir, 'fake-ssh');
  writeFileSync(ssh, `#!/bin/sh
for a; do last="$a"; done
case " $* " in *" -G "*) printf 'hostname 10.0.0.9\\nuser hotic\\nport 2222\\n'; exit 0;; esac
HOME='${hostHome}'; ACPIRA_REMOTE_MACHINE_ID=machine-a; export HOME ACPIRA_REMOTE_MACHINE_ID
exec /bin/sh -c "$last"
`);
  chmodSync(ssh, 0o755);
  const bin = join(dir, 'helper-bin');
  writeFileSync(bin, `#!/bin/sh\nexec '${SIDECAR}' "$@" --no-login-path\n`);
  chmodSync(bin, 0o755);
  return { ssh, bin };
}

describe.skipIf(process.platform === 'win32')('sidecar SSH connections', () => {
  const cleanups: (() => Promise<void> | void)[] = [];
  afterEach(async () => { for (const c of cleanups.splice(0)) await c(); });

  it('lists ~/.ssh/config hosts, connects, browses the host and disconnects', async () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'acpira-remote-')));
    const data = join(root, 'data'), home = join(root, 'home'), hostHome = join(root, 'host');
    for (const d of [data, join(home, '.ssh'), join(hostHome, 'work')]) mkdirSync(d, { recursive: true });
    writeFileSync(join(home, '.ssh', 'config'), 'Host gen1\n  HostName 10.0.0.9\n  User hotic\n  Port 2222\n\nHost *\n  ServerAliveInterval 30\n');
    const { ssh, bin } = fakeSsh(root, hostHome);
    const s = new Shell(data, '', undefined, { HOME: home, ACPIRA_SSH: ssh, ACPIRA_REMOTE_BIN: bin });
    cleanups.push(async () => { await s.kill(); rmSync(root, { recursive: true, force: true }); });
    const ok = await s.hello({ env: { hostLanguage: 'en', projects: true, blobBase: 'https://acpira.local/blobs' } });
    if (ok.type !== 'helloOk') throw new Error('handshake refused');
    expect(ok.features).toContain('remote');
    await s.open('V', 'editor');

    s.view('V', { type: 'connections' });
    expect((await s.hostMsg('V', 'connections')).connections).toEqual([]);
    s.view('V', { type: 'sshHosts' });
    expect((await s.hostMsg('V', 'sshHosts')).hosts).toEqual([{ alias: 'gen1', address: 'hotic@10.0.0.9:2222' }]);

    s.view('V', { type: 'connectionOp', op: { op: 'add', alias: 'gen1', mode: 'local' } });
    const up = await s.hostMsg('V', 'connections', m => m.connections[0]?.state === 'connected', 30_000);
    expect(up.connections[0]).toMatchObject({ alias: 'gen1', mode: 'local', address: 'hotic@10.0.0.9:2222', projects: [] });
    expect(up.connections[0]?.remoteVersion).toMatch(/^\d+\.\d+\.\d+/);
    // The steps went by on the way
    const steps = new Set(s.hostMsgs('V').flatMap(m => (m.type === 'connections' ? m.connections.map(c => c.step) : [])));
    expect([...steps]).toEqual(expect.arrayContaining(['ssh', 'probe', 'upload', 'start']));

    s.view('V', { type: 'remoteDir', alias: 'gen1', seq: 7 });
    const dir = await s.hostMsg('V', 'remoteDir', m => m.seq === 7, 30_000);
    expect(dir.error).toBeUndefined();
    expect(dir.path).toBe(hostHome);
    expect(dir.entries).toEqual(expect.arrayContaining([{ name: 'work', dir: true }]));
    s.view('V', { type: 'remoteDir', alias: 'gen1', path: '/no/such/folder', seq: 8 });
    expect((await s.hostMsg('V', 'remoteDir', m => m.seq === 8)).error).toBeTruthy();

    s.view('V', { type: 'connectionOp', op: { op: 'setMode', alias: 'gen1', mode: 'remote' } });
    await s.hostMsg('V', 'connections', m => m.connections[0]?.mode === 'remote');
    s.view('V', { type: 'connectionOp', op: { op: 'disconnect', alias: 'gen1' } });
    await s.hostMsg('V', 'connections', m => m.connections[0]?.state === 'off');
    s.view('V', { type: 'connectionOp', op: { op: 'remove', alias: 'gen1' } });
    await s.hostMsg('V', 'connections', m => m.connections.length === 0 && s.hostMsgs('V').filter(x => x.type === 'connections').length > 3);
  });
});
