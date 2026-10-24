import { randomBytes } from 'node:crypto';
import { open, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';

// A holder that died (killed IDE, crashed sidecar) leaves its lock behind; past this age another host takes it over
const STALE_MS = 10_000;
// Give up on a lock that stays held this long: the file is written by hand in the meantime rather than blocking the UI forever
const WAIT_MS = 5_000;

// Callers in this process queue on a promise chain per path, so the O_EXCL file only ever arbitrates between processes
const chains = new Map<string, Promise<unknown>>();

// Cross-process mutex around a shared file: `<file>.lock` is created with O_EXCL, so exactly one host (VS Code window, Cursor window,
// IDEA sidecar) is inside `fn` at a time. Every store that keeps a whole-file snapshot in memory re-reads the file inside the lock
// before it writes, so two hosts editing accounts or preferences no longer overwrite each other's changes.
//
// The published lock carries `pid\ntoken`. A sibling `<file>.lock.<token>` is the generation that stale waiters rename: they never
// rename the published path, so a check made against a dead holder cannot move a lock a live host just created. Release deletes the
// published path only when it still holds our token.
export function withFileLock<T>(file: string, fn: () => Promise<T>): Promise<T> {
  const prev = chains.get(file) ?? Promise.resolve();
  const run = prev.catch(() => {}).then(() => locked(file, fn));
  chains.set(file, run);
  run.finally(() => { if (chains.get(file) === run) chains.delete(file); }).catch(() => {});
  return run;
}

async function locked<T>(file: string, fn: () => Promise<T>): Promise<T> {
  const lock = `${file}.lock`;
  const deadline = Date.now() + WAIT_MS;
  const token = randomBytes(16).toString('hex');
  const gen = `${lock}.${token}`;
  try {
    for (;;) {
      await writeFile(gen, `${process.pid}\n${token}`);
      try {
        const h = await open(lock, 'wx');
        try { await h.writeFile(`${process.pid}\n${token}`); } finally { await h.close(); }
        break;
      } catch (e) {
        await rm(gen, { force: true });
        if ((e as NodeJS.ErrnoException).code !== 'EEXIST') throw e;
        if (await claimStale(lock)) continue;
        if (Date.now() > deadline) throw new Error(`${file} is locked by another host (${lock})`);
        const age = await stat(lock).then(s => Date.now() - s.mtimeMs, () => -1);
        if (age >= 0) await new Promise(r => setTimeout(r, 5 + Math.random() * 20));
      }
    }
    return await fn();
  } finally {
    await release(lock, token);
  }
}

async function release(lock: string, token: string) {
  const body = await readFile(lock, 'utf8').catch(() => '');
  if (parseLock(body).token === token) await rm(lock, { force: true });
  await rm(`${lock}.${token}`, { force: true });
}

function parseLock(body: string): { pid: number; token?: string } {
  const [pidLine, tokenLine] = body.split(/\r?\n/);
  const pid = Number(pidLine?.trim());
  const token = tokenLine?.trim();
  return { pid, token: token && /^[0-9a-f]{16,}$/i.test(token) ? token : undefined };
}

async function claimStale(lock: string): Promise<boolean> {
  const body = await readFile(lock, 'utf8').catch(() => '');
  if (!body) return false;
  const { pid, token } = parseLock(body);
  const age = await stat(lock).then(s => Date.now() - s.mtimeMs, () => -1);
  if (age <= STALE_MS) return false;
  if (await pidAlive(pid)) return false;

  if (token) {
    const gen = `${lock}.${token}`;
    const dead = `${gen}.dead-${process.pid}-${randomBytes(4).toString('hex')}`;
    // Unique source: a second waiter renaming the same generation loses, and a newly published lock uses a different token
    if (!await rename(gen, dead).then(() => true, () => false)) return false;
    const still = await readFile(lock, 'utf8').catch(() => '');
    if (parseLock(still).token === token) await rm(lock, { force: true });
    await rm(dead, { force: true });
    return true;
  }

  // Legacy lock (pid only, no generation file). Keep the moved bytes only when they are still the observation we checked
  const claim = `${lock}.stale-${process.pid}-${randomBytes(4).toString('hex')}`;
  if (!await rename(lock, claim).then(() => true, () => false)) return false;
  const moved = await readFile(claim, 'utf8').catch(() => '');
  if (moved === body) {
    await rm(claim, { force: true });
    return true;
  }
  await rename(claim, lock).catch(() => {});
  return false;
}

async function pidAlive(pid: number): Promise<boolean> {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try { process.kill(pid, 0); return true; }
  catch (e) { return (e as NodeJS.ErrnoException).code === 'EPERM'; }
}

// tmp + rename so a reader in another host never sees a half-written file; mode applies to the temp file and travels with the rename
export async function writeAtomic(path: string, data: string, mode?: number) {
  const tmp = `${path}.${process.pid}.tmp`;
  await writeFile(tmp, data, mode === undefined ? undefined : { mode });
  await rename(tmp, path);
}
