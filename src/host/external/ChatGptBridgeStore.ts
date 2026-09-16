import { lstat, mkdir, readFile, readdir, realpath, stat } from 'node:fs/promises';
import { dirname, isAbsolute, join } from 'node:path';
import { chatgptBinding } from './chatgptBinding';
import type { SessionSummary, SessionView } from '@shared/transcript';
import { sortIndex } from '../store/TranscriptStore';
import { withFileLock, writeAtomic } from '../store/fileLock';
import { applyChatGptEvent, chatgptSessionId, chatgptSummary, chatgptView, isChatGptId, type ChatGptRecord } from './chatgptEvents';

const POLL_MS = 750;
const UNDO_MS = 30_000;
const RECORD_LIMIT = 64 * 1024 * 1024;

// External sessions have a separate writer-owned store: opening a mirror never creates an ACP
// process or lets a viewer overwrite newer events. Every host observes the same atomic records.
export class ChatGptBridgeStore {
  private records = new Map<string, ChatGptRecord>();
  private stamps = new Map<string, string>();
  private published = new Map<string, number>();
  private listeners = new Set<(ids: string[]) => void>();
  private timer?: ReturnType<typeof setInterval>;
  private scanning?: Promise<void>;
  private disposed = false;

  constructor(readonly dir: string, private log: (line: string) => void = () => {}, private now: () => number = Date.now, private cliPath?: string) {}

  async init() {
    await mkdir(this.dir, { recursive: true, mode: 0o700 });
    await this.refresh();
    if (!this.timer && !this.disposed) {
      // Polling is intentional: fs.watch misses writes on some external macOS volumes.
      this.timer = setInterval(() => { if (!this.scanning) void this.refresh().catch(e => this.log(`ChatGPT mirror refresh: ${String(e)}`)); }, POLL_MS);
      this.timer.unref?.();
    }
  }
  subscribe(fn: (ids: string[]) => void): () => void { this.listeners.add(fn); return () => this.listeners.delete(fn); }
  available(): boolean { return !!this.cliPath; }
  has(id: string): boolean { const r = this.records.get(id); return !!r && r.deletedAt === undefined; }
  owns(id: string): boolean { return isChatGptId(id); }
  view(id: string): SessionView | undefined {
    const r = this.records.get(id);
    if (!r || r.deletedAt !== undefined) return undefined;
    const v = chatgptView(r, this.now());
    if (this.cliPath) v.external!.connectionPrompt = chatgptBinding(v, this.cliPath, dirname(dirname(this.dir)));
    return v;
  }
  summaries(): SessionSummary[] {
    const out = [...this.records.values()].filter(r => r.deletedAt === undefined).map(r => chatgptSummary(r, this.now()));
    sortIndex(out); return out;
  }
  private file(id: string): string {
    if (!isChatGptId(id)) throw new Error('Invalid ChatGPT mirror ID');
    return join(this.dir, `${id}.json`);
  }
  private async read(id: string): Promise<ChatGptRecord | undefined> {
    const file = this.file(id);
    let info;
    try { info = await lstat(file); }
    catch (e) { if ((e as NodeJS.ErrnoException).code === 'ENOENT') return undefined; throw e; }
    if (!info.isFile() || info.isSymbolicLink() || info.size > RECORD_LIMIT) throw new Error('Invalid or oversized ChatGPT mirror file');
    const r = JSON.parse(await readFile(file, 'utf8')) as ChatGptRecord;
    if (r.version !== 1 || r.id !== id || typeof r.sourceKey !== 'string' || chatgptSessionId(r.sourceKey) !== id
      || !isAbsolute(r.cwd) || typeof r.title !== 'string' || !Number.isSafeInteger(r.revision)
      || !Number.isFinite(Date.parse(r.lastEventAt)) || !Array.isArray(r.turns) || !r.receipts || typeof r.receipts !== 'object') {
      throw new Error('Invalid ChatGPT mirror record');
    }
    return r;
  }
  private async save(r: ChatGptRecord) {
    const body = JSON.stringify(r);
    if (Buffer.byteLength(body) > RECORD_LIMIT) throw new Error('Mirror exceeds 64 MiB; start a new mirror');
    await writeAtomic(this.file(r.id), body, 0o600);
  }

  async open(sourceKey: string, cwd: string, title = 'ChatGPT'): Promise<SessionView> {
    const id = chatgptSessionId(sourceKey);
    if (!isAbsolute(cwd)) throw new Error('An absolute project directory is required');
    const resolved = await realpath(cwd);
    if (!(await stat(resolved)).isDirectory()) throw new Error('Project path is not a directory');
    await mkdir(this.dir, { recursive: true, mode: 0o700 });
    await withFileLock(this.file(id), async () => {
      const old = await this.read(id);
      if (old) {
        if (old.deletedAt !== undefined) throw new Error('Mirror was deleted; use a new session key');
        if (old.cwd !== resolved) throw new Error('Session key is already bound to another project');
        return;
      }
      const at = new Date(this.now()).toISOString();
      await this.save({ version: 1, id, sourceKey, cwd: resolved, title: title.trim().slice(0, 160) || 'ChatGPT',
        createdAt: at, updatedAt: at, lastEventAt: at, revision: 1, turns: [], receipts: {} });
    });
    await this.refresh();
    return this.view(id)!;
  }

  private async mutate(id: string, apply: (r: ChatGptRecord) => ChatGptRecord): Promise<void> {
    await mkdir(this.dir, { recursive: true, mode: 0o700 });
    await withFileLock(this.file(id), async () => {
      const r = await this.read(id);
      if (!r) throw new Error('Unknown ChatGPT mirror; connect this conversation first');
      const next = apply(r);
      if (next !== r) await this.save(next);
    });
    await this.refresh();
  }
  async accept(id: string, event: unknown): Promise<void> { await this.mutate(id, r => applyChatGptEvent(r, event, this.now())); }
  async rename(id: string, title: string) {
    if (!title.trim()) return;
    await this.mutate(id, r => {
      if (r.deletedAt !== undefined) throw new Error('Mirror was deleted');
      return { ...r, title: title.trim().slice(0, 160), revision: r.revision + 1 };
    });
  }
  async pin(id: string, pinned: boolean) {
    await this.mutate(id, r => {
      if (r.deletedAt !== undefined) throw new Error('Mirror was deleted');
      return { ...r, pinned: pinned || undefined, revision: r.revision + 1 };
    });
  }
  async delete(id: string) {
    await this.mutate(id, r => r.deletedAt !== undefined ? r : { ...r, deletedAt: this.now(), revision: r.revision + 1 });
  }
  async restore(id: string) {
    await this.mutate(id, r => {
      if (r.deletedAt === undefined) return r;
      if (this.now() - r.deletedAt > UNDO_MS) throw new Error('The undo window has expired');
      const next = { ...r, revision: r.revision + 1 }; delete next.deletedAt; return next;
    });
  }

  async refresh(): Promise<void> {
    if (this.disposed) return;
    const run = (this.scanning ?? Promise.resolve()).catch(() => {}).then(() => this.scan());
    this.scanning = run;
    try { await run; } finally { if (this.scanning === run) this.scanning = undefined; }
  }
  private async scan() {
    let files: string[];
    try { files = await readdir(this.dir); }
    catch (e) { if ((e as NodeJS.ErrnoException).code === 'ENOENT') files = []; else throw e; }
    const found = new Set<string>();
    for (const file of files) {
      const id = file.endsWith('.json') ? file.slice(0, -5) : '';
      if (!isChatGptId(id)) continue;
      found.add(id);
      try {
        const s = await lstat(this.file(id));
        const stamp = `${s.ino}:${s.mtimeMs}:${s.size}`;
        if (stamp === this.stamps.get(id)) continue;
        const r = await this.read(id);
        if (r) { this.records.set(id, r); this.stamps.set(id, stamp); }
      } catch (e) { this.records.delete(id); this.stamps.delete(id); this.log(`ChatGPT ${id}: ${String(e)}`); }
    }
    for (const id of this.records.keys()) if (!found.has(id)) { this.records.delete(id); this.stamps.delete(id); }
    const next = new Map([...this.records].filter(([, r]) => r.deletedAt === undefined).map(([id, r]) => [id, chatgptView(r, this.now()).rev!]));
    const changed = [...new Set([...this.published.keys(), ...next.keys()])].filter(id => this.published.get(id) !== next.get(id));
    this.published = next;
    if (changed.length && !this.disposed) for (const fn of this.listeners) fn(changed);
    // Keep a tiny tombstone after undo expires so a delayed writer cannot resurrect deleted history.
    for (const r of this.records.values()) if (r.deletedAt !== undefined && this.now() - r.deletedAt > UNDO_MS && r.turns.length) {
      await withFileLock(this.file(r.id), async () => {
        const latest = await this.read(r.id);
        if (latest?.deletedAt !== undefined && this.now() - latest.deletedAt > UNDO_MS) {
          await this.save({ ...latest, turns: [], receipts: {}, activeTurnId: undefined, revision: latest.revision + 1 });
        }
      });
    }
  }
  async dispose() {
    this.disposed = true;
    clearInterval(this.timer); this.timer = undefined;
    await this.scanning?.catch(() => {});
    this.listeners.clear();
  }
}
