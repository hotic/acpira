import { randomUUID } from 'node:crypto';
import { chmod, mkdir, readFile } from 'node:fs/promises';
import { dirname } from 'node:path';
import type { AccountInfo, AgentId } from '@shared/transcript';
import type { AccountCredential, AccountDraft } from './types';
import { msg } from '../errors';
import { withFileLock, writeAtomic } from '../store/fileLock';

// Secret vault: FileVault (secrets.json, mode 600) in the app, in-memory in tests
export interface SecretVault {
  get(key: string): Promise<string | undefined>;
  store(key: string, value: string): Promise<void>;
  delete(key: string): Promise<void>;
}

export class MemoryVault implements SecretVault {
  private m = new Map<string, string>();
  async get(key: string) { return this.m.get(key); }
  async store(key: string, value: string) { this.m.set(key, value); }
  async delete(key: string) { this.m.delete(key); }
}

// Persistent vault at secrets.json, shared by every host on the machine: each change re-reads the file under its lock and writes the
// whole table back, so a secret another host stored meanwhile survives. A corrupt file is logged and never overwritten
export class FileVault implements SecretVault {
  private data = new Map<string, string>();
  private frozen = false;

  constructor(private file: string, private log: (line: string) => void = () => {}) {}

  // Always re-read under the lock: another host can replace the secret for a key this process already cached
  async get(key: string) {
    return withFileLock(this.file, async () => {
      await this.read();
      return this.data.get(key);
    });
  }

  store(key: string, value: string) { return this.mutate(d => { d.set(key, value); }); }

  delete(key: string) { return this.mutate(d => { d.delete(key); }); }

  private mutate(fn: (data: Map<string, string>) => void) {
    return withFileLock(this.file, async () => {
      await this.read();
      if (this.frozen) return;
      const data = new Map(this.data);
      fn(data);
      await mkdir(dirname(this.file), { recursive: true });
      await writeAtomic(this.file, JSON.stringify(Object.fromEntries(data), null, 2), 0o600);
      try { await chmod(this.file, 0o600); } catch { /* Windows */ }
      this.data = data;
    });
  }

  private async read() {
    let raw: string;
    try { raw = await readFile(this.file, 'utf8'); }
    catch { this.data = new Map(); return; }
    try {
      const parsed = JSON.parse(raw) as unknown;
      if (!isPlainObject(parsed)) throw new Error('not an object');
      this.data = new Map(Object.entries(parsed).filter((e): e is [string, string] => typeof e[1] === 'string'));
      this.frozen = false;
    } catch (e) {
      if (!this.frozen) this.log(`secrets.json unreadable, leaving file untouched (${msg(e)})`);
      this.frozen = true;
    }
  }
}

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === 'object' && v !== null && !Array.isArray(v);
}

interface StoredAccount extends AccountInfo {
  meta?: Record<string, string>;
}

const SECRET_PREFIX = 'acpira.account.';

export function accountSecretKey(id: string): string {
  return SECRET_PREFIX + id;
}

// Account metadata goes to <file> (JSON); secrets go into the vault keyed by id; the two sides are linked only by id.
// The file is shared by every host on the machine (VS Code and Cursor windows, IDEA sidecars), so `items` is a cache: every change
// re-reads the file under its lock, applies itself to what is there and writes the result, and `reload` picks up what other hosts did
export class AccountStore {
  private items: StoredAccount[] = [];
  private tail: Promise<unknown> = Promise.resolve();

  constructor(private file: string, private vault: SecretVault, private log: (line: string) => void = () => {}) {}

  // Reload and mutate share this queue so a focus-driven refresh cannot replace `items` while add/remove awaits the vault
  private enqueue<T>(fn: () => Promise<T>): Promise<T> {
    const run = this.tail.then(fn, fn);
    this.tail = run.then(() => undefined, () => undefined);
    return run;
  }

  // No file yet is the normal first run; a file that will not parse is worth a log line, since the UI then shows no accounts while the secrets still exist
  async load() {
    await this.enqueue(() => this.read());
    // Older drafts stored detail as '{tier} · {name}'; the name says nothing the label doesn't — keep the leading segment
    if (this.items.some(a => a.detail !== legacyDetail(a.detail))) await this.mutate(items => { for (const a of items) a.detail = legacyDetail(a.detail); });
  }

  // Re-read what other hosts wrote; true when the list differs from what this store had
  async reload(): Promise<boolean> {
    return this.enqueue(async () => {
      const before = JSON.stringify(this.items);
      await this.read();
      return JSON.stringify(this.items) !== before;
    });
  }

  private async read() {
    let raw: string;
    try { raw = await readFile(this.file, 'utf8'); }
    catch { this.items = []; return; }
    try {
      const parsed = JSON.parse(raw) as unknown;
      if (!Array.isArray(parsed)) throw new Error('not an array');
      this.items = parsed as StoredAccount[];
    } catch (e) {
      this.log(`accounts.json unreadable, starting with no accounts (${msg(e)})`);
      this.items = [];
    }
  }

  // Read → change → write under the file lock; `fn` sees a private snapshot of the disk list, published to `items` only after the write
  private mutate(fn: (items: StoredAccount[]) => void | Promise<void>) {
    return this.enqueue(() => withFileLock(this.file, async () => {
      await this.read();
      const items = this.items.map(copyAccount);
      await fn(items);
      await mkdir(dirname(this.file), { recursive: true });
      await writeAtomic(this.file, JSON.stringify(items, null, 2), 0o600);
      this.items = items;
    }));
  }

  list(agent?: AgentId): AccountInfo[] {
    return this.items.filter(a => !agent || a.agent === agent).map(({ meta: _, ...info }) => info);
  }

  get(id: string): AccountInfo | undefined {
    const a = this.items.find(x => x.id === id);
    if (!a) return undefined;
    const { meta: _, ...info } = a;
    return info;
  }

  // The most recently used one is the default; if none has ever been used, take the earliest added
  defaultFor(agent: AgentId): AccountInfo | undefined {
    const list = this.list(agent);
    return list.sort((a, b) => (b.lastUsedAt ?? '').localeCompare(a.lastUsedAt ?? '') || a.addedAt.localeCompare(b.addedAt))[0];
  }

  // Same label under the same agent counts as the same account: re-login just swaps the secret instead of growing a duplicate
  async add(agent: AgentId, draft: AccountDraft): Promise<AccountInfo> {
    let added!: StoredAccount;
    await this.mutate(async items => {
      const now = new Date().toISOString();
      let a = items.find(x => x.agent === agent && x.label === draft.label);
      if (a) { a.detail = draft.detail; a.meta = draft.meta; }
      else { a = { id: randomUUID(), agent, label: draft.label, detail: draft.detail, meta: draft.meta, addedAt: now }; items.push(a); }
      await this.vault.store(accountSecretKey(a.id), draft.secret);
      added = a;
    });
    const { meta: _, ...info } = added;
    return info;
  }

  async remove(id: string) {
    if (!this.items.some(x => x.id === id)) return;
    await this.mutate(async items => {
      const i = items.findIndex(x => x.id === id);
      if (i >= 0) items.splice(i, 1);
      await this.vault.delete(accountSecretKey(id));
    });
  }

  async credential(id: string): Promise<AccountCredential | undefined> {
    const a = this.items.find(x => x.id === id);
    if (!a) return undefined;
    const secret = await this.vault.get(accountSecretKey(id));
    return secret ? { secret, meta: a.meta } : undefined;
  }

  // Marks the account used; one another host removed meanwhile is not resurrected
  async touch(id: string) {
    if (!this.items.some(x => x.id === id)) return;
    await this.mutate(items => {
      const a = items.find(x => x.id === id);
      if (a) a.lastUsedAt = new Date().toISOString();
    });
  }
}

function copyAccount(a: StoredAccount): StoredAccount {
  return { ...a, meta: a.meta ? { ...a.meta } : undefined };
}

function legacyDetail(detail: string | undefined): string | undefined {
  return detail?.split(' · ')[0] || undefined;
}
