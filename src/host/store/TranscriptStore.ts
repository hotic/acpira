import { createHash } from 'node:crypto';
import { access, mkdir, readFile, readdir, realpath, rename, rm, stat, utimes, writeFile } from 'node:fs/promises';
import { basename, dirname, join, sep } from 'node:path';
import type { AgentId, SessionSummary, TurnSettings } from '@shared/transcript';
import type { SessionRecord } from '../acp/AcpSession';
import type { BlobStore } from '../acp/attachments';
import { msg } from '../errors';
import { t } from '../i18n';
import { withFileLock, writeAtomic } from './fileLock';

// Cross-session memory that is not a setting: the mode / config values last chosen per agent, replayed onto new sessions
export interface SessionPrefs {
  lastSettings: Record<AgentId, TurnSettings>;
}

const META_FILES = new Set(['index.json', 'prefs.json']);
const TRASH_DIR = 'trash';
const RESERVED_IDS = new Set(['index', 'prefs', TRASH_DIR]);

// Streamed updates call save() every few milliseconds; one write per session this often is plenty.
// Trailing debounce is capped so a continuous stream still lands within SAVE_MAX_WAIT_MS.
const SAVE_DEBOUNCE_MS = 400;
const SAVE_MAX_WAIT_MS = 2_000;

// Session ids name files and directories under the store root. Anything that is not a plain token
// (UUID-shaped, same as blob dirs) must not reach path.join / rm
export function isSessionId(id: string): boolean {
  return /^[\w-]+$/.test(id) && !RESERVED_IDS.has(id);
}

export interface TranscriptStoreOpts {
  saveDebounceMs?: number;
  saveMaxWaitMs?: number;
  onSaveError?: (id: string, error: string) => void;
}

interface PendingWrite {
  timer: NodeJS.Timeout;
  record: SessionRecord;
  // First save() in this burst; later calls cannot push the write past this + saveMaxWaitMs
  deadline: number;
}

// Session persistence: <dir>/index.json caches the summary list, <dir>/prefs.json the per-agent memory, <dir>/<id>.json the full record,
// <dir>/<id>/ its attachment blobs, <dir>/trash/ the soft-deleted ones during their undo window. Record writes are debounced per session.
//
// The directory is shared by every extension host (each VS Code / Cursor window runs its own), so the index is never trusted blindly:
// syncIndex re-reads it and reconciles it with the record files on disk before writing, and every file is written atomically
// (tmp + rename) so another window can never read a half-written record. A record that leaves the live directory under this store's
// feet was deleted by another window: write refuses to put it back (see knew)
export class TranscriptStore implements BlobStore {
  private pending = new Map<string, PendingWrite>();
  // Writes that have left the debounce but not yet reached the directory, one chain per id: concurrent writes of a record would share
  // its temp file, and a directory listing taken while a first write is mid-flight would report the record missing
  private inflight = new Map<string, Promise<void>>();
  // Ids whose record this store has read from or written to the live directory
  private known = new Set<string>();
  private readonly saveDebounceMs: number;
  private readonly saveMaxWaitMs: number;
  private readonly onSaveError?: (id: string, error: string) => void;

  constructor(private dir: string, private log: (line: string) => void = () => {}, opts: TranscriptStoreOpts = {}) {
    this.saveDebounceMs = opts.saveDebounceMs ?? SAVE_DEBOUNCE_MS;
    this.saveMaxWaitMs = opts.saveMaxWaitMs ?? SAVE_MAX_WAIT_MS;
    this.onSaveError = opts.onSaveError;
  }

  private async ensure() { await mkdir(this.dir, { recursive: true }); }

  // The list as the disk knows it: the cached index reconciled with the record files (see syncIndex)
  loadIndex(): Promise<SessionSummary[]> { return this.syncIndex([], new Set()); }

  async loadPrefs(): Promise<SessionPrefs> {
    try { return { lastSettings: {}, ...(JSON.parse(await readFile(join(this.dir, 'prefs.json'), 'utf8')) as Partial<SessionPrefs>) }; }
    catch { return { lastSettings: {} }; }
  }

  // prefs.json is shared by every host: the file is re-read under its lock and only the given agents' entries are replaced, so a
  // window that just remembered Kimi's mode does not undo what another window remembered for Grok. Returns the merged result
  async savePrefs(prefs: SessionPrefs, agents: AgentId[] = Object.keys(prefs.lastSettings)): Promise<SessionPrefs> {
    await this.ensure();
    const file = join(this.dir, 'prefs.json');
    return withFileLock(file, async () => {
      const disk = await this.loadPrefs();
      for (const agent of agents) {
        const v = prefs.lastSettings[agent];
        if (v) disk.lastSettings[agent] = v; else delete disk.lastSettings[agent];
      }
      await writeAtomic(file, JSON.stringify(disk, null, 2));
      return disk;
    });
  }

  // Merge this host's view of the list with what is on disk, write the result, and return it.
  // The record files are the truth: an id whose file is gone (deleted or trashed by another window) drops out, a file no index knows
  // (created by another window, or left behind by a lost index) is loaded and summarized. Where the disk index and `mine` both have an
  // entry, `mine` wins only for the ids in `own` (sessions this host has live or has just patched); for the rest the disk is fresher,
  // since another window may have renamed or pinned them. Entries from older builds lacking cwd are backfilled from their record once.
  // Debounced records are written first: an index must never name a record another window cannot find on disk
  async syncIndex(mine: SessionSummary[], own: Set<string>): Promise<SessionSummary[]> {
    await this.ensure();
    await this.flushPending();
    const disk = new Map((await this.readIndex()).map(s => [s.id, s]));
    const local = new Map(mine.map(s => [s.id, s]));
    const out: SessionSummary[] = [];
    for (const id of await this.recordIds()) {
      let s = own.has(id) ? local.get(id) ?? disk.get(id) : disk.get(id) ?? local.get(id);
      if (!s || !s.cwd) {
        const r = await this.load(id);
        if (!r) continue;
        s = { ...s, ...summarize(r) };
      }
      out.push(s);
    }
    sortIndex(out);
    await writeAtomic(join(this.dir, 'index.json'), JSON.stringify(out, null, 2));
    return out;
  }

  private async readIndex(): Promise<SessionSummary[]> {
    try {
      const v = JSON.parse(await readFile(join(this.dir, 'index.json'), 'utf8')) as unknown;
      return Array.isArray(v) ? v.filter((s): s is SessionSummary => !!s && typeof s === 'object' && typeof (s as SessionSummary).id === 'string') : [];
    } catch { return []; }
  }

  // Ids with a record file in the live directory (not the trash)
  private async recordIds(): Promise<string[]> {
    return (await readdir(this.dir)).filter(f => f.endsWith('.json') && !META_FILES.has(f)).map(f => f.slice(0, -5)).filter(isSessionId);
  }

  // A record that fails to parse, or lacks the fields every reader relies on, counts as missing: better an empty entry than a crash mid-restore
  async load(id: string): Promise<SessionRecord | null> {
    if (!isSessionId(id)) return null;
    const pending = this.pending.get(id);
    if (pending) return pending.record;
    const path = await this.confined(this.dir, `${id}.json`);
    if (!path) return null;
    let raw: string;
    try { raw = await readFile(path, 'utf8'); }
    catch { return null; }
    try {
      const r = JSON.parse(raw) as unknown;
      if (!isRecord(r) || r.id !== id) throw new Error('not a session record');
      this.known.add(id);
      return r;
    } catch (e) {
      this.log(`session ${id}: record unreadable (${msg(e)})`);
      return null;
    }
  }

  save(record: SessionRecord, delay = this.saveDebounceMs) {
    if (!isSessionId(record.id)) {
      this.log(`session ${record.id}: illegal id, not saved`);
      return;
    }
    const now = Date.now();
    const prev = this.pending.get(record.id);
    if (prev) {
      clearTimeout(prev.timer);
      const wait = Math.min(delay, Math.max(0, prev.deadline - now));
      prev.record = record;
      prev.timer = setTimeout(() => this.flushPendingId(record.id), wait);
      return;
    }
    const deadline = now + this.saveMaxWaitMs;
    const timer = setTimeout(() => this.flushPendingId(record.id), Math.min(delay, this.saveMaxWaitMs));
    this.pending.set(record.id, { timer, record, deadline });
  }

  async flush(record: SessionRecord) {
    if (!isSessionId(record.id)) return;
    this.cancelPending(record.id);
    await this.write(record);
  }

  // Whether this store has had the record on disk. A live session whose id this store knew but whose file is gone from the directory
  // (absent from the list syncIndex returns) was deleted by another window; a fresh session whose first write failed is not
  knew(id: string) { return this.known.has(id); }

  // Removes the record and its blob directory for good, wherever they are (live or trash)
  async remove(id: string) {
    if (!isSessionId(id)) return;
    this.cancelPending(id);
    await this.settleInflight(id);
    this.known.delete(id);
    for (const dir of [this.dir, join(this.dir, TRASH_DIR)]) {
      await this.rmConfined(dir, `${id}.json`);
      await this.rmConfined(dir, id, true);
    }
  }

  // Soft deletion: move the record and its blobs into trash/ so the live directory (what syncIndex trusts) no longer lists it, while an
  // undo can still bring it back. Unlike an in-memory trash, this survives a crash: sweepTrash cleans up whatever is left on the next start
  async trash(id: string) {
    if (!isSessionId(id)) return;
    this.cancelPending(id);
    await this.settleInflight(id);
    const trash = join(this.dir, TRASH_DIR);
    await mkdir(trash, { recursive: true });
    await this.move(this.dir, trash, id);
    // rename keeps the record's mtime; stamp the moment it was trashed so sweepTrash can tell a fresh undo window from a leftover
    const now = new Date();
    const stamped = await this.confined(trash, `${id}.json`);
    if (stamped) await utimes(stamped, now, now).catch(() => {});
  }

  async restore(id: string) {
    if (!isSessionId(id)) return;
    await this.move(join(this.dir, TRASH_DIR), this.dir, id);
  }

  // Remove what was trashed more than `olderThanMs` ago: its undo window closed with the host that trashed it. Anything younger may still
  // be undone in another window and is left alone
  async sweepTrash(olderThanMs = 0) {
    const trash = join(this.dir, TRASH_DIR);
    let files: string[];
    try { files = await readdir(trash); } catch { return; }
    const cutoff = Date.now() - olderThanMs;
    for (const f of files) {
      if (!f.endsWith('.json')) continue;
      const id = f.slice(0, -5);
      if (!isSessionId(id)) continue;
      const mtime = await stat(join(trash, f)).then(s => s.mtimeMs).catch(() => 0);
      if (mtime > cutoff) continue;
      await this.rmConfined(trash, f);
      await this.rmConfined(trash, id, true);
    }
  }

  private async move(from: string, to: string, id: string) {
    if (!isSessionId(id)) return;
    const fileFrom = await this.confined(from, `${id}.json`);
    const fileTo = await this.confined(to, `${id}.json`);
    if (fileFrom && fileTo) await rename(fileFrom, fileTo).catch(() => {});
    const dirFrom = await this.confined(from, id);
    const dirTo = await this.confined(to, id);
    if (dirFrom && dirTo) await rename(dirFrom, dirTo).catch(() => {});
  }

  // Writes whatever is still debounced; called when the extension host goes down so the last few seconds of a transcript are not lost
  async dispose() { await this.flushPending(); }

  // Every debounced record is on its way and every write already on its way has landed (or failed, logged) when this resolves
  private async flushPending() {
    const queued = [...this.pending.values()];
    this.pending.clear();
    const results = await Promise.allSettled(queued.map(p => { clearTimeout(p.timer); return this.write(p.record); }));
    for (let i = 0; i < results.length; i++) {
      const r = results[i]!;
      if (r.status !== 'rejected') continue;
      const err = msg(r.reason);
      const id = queued[i]!.record.id;
      this.log(`session ${id}: save failed (${err})`);
      this.onSaveError?.(id, err);
    }
    await Promise.allSettled([...this.inflight.values()]);
  }

  // Wait for the write of one record that is already on its way; its failure is the writer's to log
  private async settleInflight(id: string) {
    await this.inflight.get(id)?.catch(() => {});
  }

  private cancelPending(id: string) {
    const p = this.pending.get(id);
    if (!p) return;
    clearTimeout(p.timer);
    this.pending.delete(id);
  }

  // Creates the file or replaces it while it is still there. A record this store once had on disk and that is gone now was trashed or
  // removed by another window; writing it back would undo that deletion, so the save is dropped (the manager learns of it from syncIndex)
  private write(record: SessionRecord): Promise<void> {
    const prev = this.inflight.get(record.id)?.catch(() => {}) ?? Promise.resolve();
    const run: Promise<void> = prev.then(() => this.writeNow(record)).finally(() => {
      if (this.inflight.get(record.id) === run) this.inflight.delete(record.id);
    });
    this.inflight.set(record.id, run);
    return run;
  }

  private flushPendingId(id: string) {
    const p = this.pending.get(id);
    if (!p) return;
    this.pending.delete(id);
    this.write(p.record).catch(e => {
      const err = msg(e);
      this.log(`session ${id}: save failed (${err})`);
      this.onSaveError?.(id, err);
    });
  }

  private async writeNow(record: SessionRecord) {
    if (!isSessionId(record.id)) return;
    await this.ensure();
    const path = await this.confined(this.dir, `${record.id}.json`);
    if (!path) return;
    if (this.known.has(record.id) && !(await exists(path))) {
      this.log(`session ${record.id}: deleted by another window, not written back`);
      return;
    }
    await writeAtomic(path, JSON.stringify(record));
    this.known.add(record.id);
  }

  // Resolves `root/name` and refuses anything that is not still under `root` after following symlinks.
  // Missing targets are allowed when the parent stays inside the root (create / force-rm).
  private async confined(root: string, name: string): Promise<string | undefined> {
    let base: string;
    try { base = await realpath(root); } catch { return undefined; }
    const target = join(root, name);
    let resolved: string;
    try { resolved = await realpath(target); }
    catch {
      let parent: string;
      try { parent = await realpath(dirname(target)); } catch { return undefined; }
      resolved = join(parent, basename(target));
    }
    if (resolved !== base && !resolved.startsWith(base + sep)) return undefined;
    return resolved;
  }

  private async rmConfined(root: string, name: string, recursive = false) {
    const path = await this.confined(root, name);
    if (path) await rm(path, { recursive, force: true });
  }

  // Blob names are content hashes, so pasting the same image twice yields one file. The session id names the directory, so it must be a plain token
  // (fresh ids are UUIDs; a hand-edited record could hold anything)
  async saveBlob(sessionId: string, ext: string, bytes: Uint8Array): Promise<{ name: string; path: string }> {
    if (!isSessionId(sessionId) || !/^\.\w+$/.test(ext)) throw new Error(t('host.blobIllegal', { path: `${sessionId}/*${ext}` }));
    const name = `${createHash('sha256').update(bytes).digest('hex').slice(0, 16)}${ext}`;
    await mkdir(join(this.dir, sessionId), { recursive: true });
    const dir = await this.confined(this.dir, sessionId);
    if (!dir) throw new Error(t('host.blobIllegal', { path: `${sessionId}/*${ext}` }));
    const path = join(dir, name);
    await writeFile(path, bytes);
    return { name, path };
  }

  async readBlob(sessionId: string, name: string): Promise<Uint8Array> {
    if (!isSessionId(sessionId) || !/^[\w-]+\.\w+$/.test(name)) throw new Error(t('host.blobIllegal', { path: `${sessionId}/${name}` }));
    const dir = await this.confined(this.dir, sessionId);
    const file = dir ? await this.confined(dir, name) : undefined;
    if (!file) throw new Error(t('host.blobIllegal', { path: `${sessionId}/${name}` }));
    return readFile(file);
  }
}

export function summarize(r: SessionRecord): SessionSummary {
  return { id: r.id, title: r.title, agent: r.agent, accountId: r.accountId, cwd: r.cwd, updatedAt: r.updatedAt, pinned: r.pinned };
}

// Pinned first, then newest first: the order the list shows
export function sortIndex(list: SessionSummary[]) {
  list.sort((a, b) => Number(!!b.pinned) - Number(!!a.pinned) || b.updatedAt.localeCompare(a.updatedAt));
}

function exists(path: string) { return access(path).then(() => true, () => false); }

// The minimum shape the manager and the session constructor dereference without checks
function isRecord(v: unknown): v is SessionRecord {
  if (typeof v !== 'object' || v === null) return false;
  const r = v as Record<string, unknown>;
  return typeof r.id === 'string' && isSessionId(r.id) && typeof r.agent === 'string' && typeof r.cwd === 'string'
    && typeof r.updatedAt === 'string' && Array.isArray(r.turns);
}
