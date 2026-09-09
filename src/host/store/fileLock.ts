import { open, readFile, rename, rm, stat, writeFile } from 'node:fs/promises';

// A holder that died (killed IDE, crashed sidecar) leaves its lock behind; past this age another host takes it over
const STALE_MS = 10_000;
// Give up on a lock that stays held this long: the file is written by hand in the meantime rather than blocking the UI forever
const WAIT_MS = 5_000;

// Callers in this process queue on a promise chain per path, so the O_EXCL file only ever arbitrates between processes
const chains = new Map<string, Promise<unknown>>();

// Cross-process mutex around a shared file: `<file>.lock` is created with O_EXCL, so exactly one host (VS Code window, Cursor window,
// IDEA sidecar) is inside `fn` at a time. Every store that keeps a whole-file snapshot in memory re-reads the file inside the lock
// before it writes, so two hosts editing accounts or preferences no longer overwrite each other's changes
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
  for (;;) {
    try {
      const h = await open(lock, 'wx');
      try { await h.writeFile(String(process.pid)); } finally { await h.close(); }
      break;
    } catch (e) {
      if ((e as NodeJS.ErrnoException).code !== 'EEXIST') throw e;
      // Whoever holds it may be gone. A lock older than STALE_MS whose holder pid is dead is claimed by renaming it first: only one
      // waiter wins the rename, so nobody ever removes a lock a live host created a moment ago (a lock that vanished between open and
      // stat is just retried)
      const age = await stat(lock).then(s => Date.now() - s.mtimeMs, () => -1);
      if (age > STALE_MS && !(await holderAlive(lock))) {
        const claim = `${lock}.stale-${process.pid}-${Math.random().toString(36).slice(2)}`;
        if (await rename(lock, claim).then(() => true, () => false)) await rm(claim, { force: true });
        continue;
      }
      if (Date.now() > deadline) throw new Error(`${file} is locked by another host (${lock})`);
      if (age >= 0) await new Promise(r => setTimeout(r, 5 + Math.random() * 20));
    }
  }
  try { return await fn(); } finally { await rm(lock, { force: true }); }
}

// The lock file holds the holder's pid; signal 0 tells whether that process still exists (EPERM means it does, under another user)
async function holderAlive(lock: string): Promise<boolean> {
  const pid = Number(await readFile(lock, 'utf8').catch(() => ''));
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
