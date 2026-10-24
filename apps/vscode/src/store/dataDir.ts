import { access, chmod, copyFile, cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';
import { accountSecretKey, type SecretVault } from '../accounts/AccountStore';
import { msg } from '../errors';

const MARKER = '.migrated';
const INFLIGHT = '.migrating';
const META_FILES = new Set(['index.json', 'prefs.json']);

// ACPIRA_HOME when set (tests, isolated profiles); otherwise ~/.acpira. Shared by Cursor, VS Code, and a future standalone app.
export function acpiraHome(env: NodeJS.ProcessEnv = process.env, home = homedir()): string {
  const override = env.ACPIRA_HOME?.trim();
  if (override) return resolve(override);
  return join(home, '.acpira');
}

export interface MigrateOnceOpts {
  from: string;
  to: string;
  oldVault: SecretVault;
  newVault: SecretVault;
  log?: (line: string) => void;
}

// Merge one IDE's legacy globalStorage + SecretStorage into ~/.acpira, once per source. The old tree is left in place.
// VS Code and Cursor each have their own globalStorage but share ~/.acpira, so the marker lists the sources already merged rather than
// saying "done": the second IDE to start still brings its sessions and accounts along. Nothing in the destination is ever overwritten
// (it is the live copy); only records and accounts it lacks are added, and the session index rebuilds itself from the files.
// .migrating stays if a copy fails so the source is retried on the next launch
export async function migrateOnce(opts: MigrateOnceOpts): Promise<void> {
  const { from, to, oldVault, newVault, log = () => {} } = opts;
  await mkdir(to, { recursive: true, mode: 0o700 });
  try { await chmod(to, 0o700); } catch { /* Windows */ }

  const done = await readMarker(to);
  if (done.has(from)) return;

  if (!await exists(join(from, 'accounts.json')) && !await exists(join(from, 'sessions'))) {
    await stamp(to, done, from);
    return;
  }

  try {
    // A marker from an earlier build did not say which source it copied; a source whose data is already here is that one
    if (await alreadyMerged(from, to)) {
      await stamp(to, done, from);
      return;
    }
    await writeFile(join(to, INFLIGHT), '');
    const n = await mergeData(from, to, oldVault, newVault);
    await stamp(to, done, from);
    await rm(join(to, INFLIGHT), { force: true });
    log(`Merged ${n.sessions} session(s) and ${n.accounts} account(s) from ${from} into ${to}`);
  } catch (e) {
    log(`Migration from ${from} to ${to} failed (${msg(e)}); will retry on next start`);
  }
}

// The marker is a JSON object listing merged sources; older builds wrote a timestamp line, which reads as "some source, unknown which"
async function readMarker(to: string): Promise<Set<string>> {
  try {
    const parsed = JSON.parse(await readFile(join(to, MARKER), 'utf8')) as unknown;
    const sources = (parsed as { sources?: unknown })?.sources;
    return new Set(Array.isArray(sources) ? sources.filter((s): s is string => typeof s === 'string') : []);
  } catch { return new Set(); }
}

async function stamp(to: string, done: Set<string>, from: string) {
  await writeFile(join(to, MARKER), `${JSON.stringify({ sources: [...done, from], at: new Date().toISOString() }, null, 2)}\n`);
}

// Session and account ids are UUIDs, so any overlap with the destination means this source was copied before. Sessions decide when the
// source has any (the user may have deleted some of them since, so "all present" is not the test); accounts only for a sessions-less tree
async function alreadyMerged(from: string, to: string): Promise<boolean> {
  const ids = await sessionIds(join(from, 'sessions'));
  if (ids.length) {
    for (const id of ids) if (await exists(join(to, 'sessions', `${id}.json`))) return true;
    return false;
  }
  const fromIds = new Set((await readAccounts(join(from, 'accounts.json'))).map(a => a.id));
  return (await readAccounts(join(to, 'accounts.json'))).some(a => fromIds.has(a.id));
}

async function mergeData(from: string, to: string, oldVault: SecretVault, newVault: SecretVault): Promise<{ sessions: number; accounts: number }> {
  let sessions = 0;
  const sessFrom = join(from, 'sessions');
  const sessTo = join(to, 'sessions');
  const ids = await sessionIds(sessFrom);
  if (ids.length || await exists(join(sessFrom, 'prefs.json'))) await mkdir(sessTo, { recursive: true });
  for (const id of ids) {
    if (await exists(join(sessTo, `${id}.json`))) continue;
    await copyFile(join(sessFrom, `${id}.json`), join(sessTo, `${id}.json`));
    if (await exists(join(sessFrom, id)) && !await exists(join(sessTo, id))) await cp(join(sessFrom, id), join(sessTo, id), { recursive: true });
    sessions++;
  }
  if (await exists(join(sessFrom, 'prefs.json')) && !await exists(join(sessTo, 'prefs.json'))) await copyFile(join(sessFrom, 'prefs.json'), join(sessTo, 'prefs.json'));

  const accFrom = join(from, 'accounts.json');
  const mine = await readAccounts(join(to, 'accounts.json'));
  const have = new Set(mine.map(a => a.id));
  const added = (await readAccounts(accFrom)).filter(a => !have.has(a.id));
  if (added.length) {
    await writeFile(join(to, 'accounts.json'), JSON.stringify([...mine, ...added], null, 2), { mode: 0o600 });
    for (const a of added) {
      const secret = await oldVault.get(accountSecretKey(a.id));
      if (secret !== undefined) await newVault.store(accountSecretKey(a.id), secret);
    }
  }
  return { sessions, accounts: added.length };
}

// No sessions directory is a normal legacy tree (accounts only); one that cannot be read is a failure worth retrying, not an empty source
async function sessionIds(dir: string): Promise<string[]> {
  try { return (await readdir(dir)).filter(f => f.endsWith('.json') && !META_FILES.has(f)).map(f => f.slice(0, -5)); }
  catch (e) {
    if ((e as NodeJS.ErrnoException).code === 'ENOENT') return [];
    throw e;
  }
}

// Whatever the file holds, only entries with a string id take part in the merge
async function readAccounts(file: string): Promise<{ id: string }[]> {
  try {
    const parsed = JSON.parse(await readFile(file, 'utf8')) as unknown;
    return Array.isArray(parsed) ? parsed.filter((a): a is { id: string } => !!a && typeof a === 'object' && typeof (a as { id?: unknown }).id === 'string') : [];
  } catch { return []; }
}

async function exists(path: string): Promise<boolean> {
  try { await access(path); return true; }
  catch { return false; }
}
