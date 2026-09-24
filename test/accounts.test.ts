import { mkdtempSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { AccountStore, FileVault, MemoryVault, accountSecretKey, type SecretVault } from '../src/host/accounts/AccountStore';

// The TypeScript account store survives only for the VS Code shell's one-time migration of legacy accounts (SecretStorage is
// readable there alone); the engine's own store is covered in rust/crates/acpira-host/tests/engine/accounts.rs

function tmp() { return mkdtempSync(join(tmpdir(), 'acpira-acc-')); }


describe('AccountStore', () => {
  it('metadata goes to JSON, secrets go to the vault; re-login with the same label only replaces the secret; the default account is the most recently used', async () => {
    const dir = tmp();
    const vault = new MemoryVault();
    const store = new AccountStore(join(dir, 'accounts.json'), vault);
    await store.load();
    const a = await store.add('devin', { label: 'a@x.io', detail: 'Max', secret: 's1', meta: { api_server_url: 'https://s' } });
    const b = await store.add('devin', { label: 'b@x.io', secret: 's2' });
    expect(store.list('devin').map(x => x.label)).toEqual(['a@x.io', 'b@x.io']);
    expect(JSON.parse(readFileSync(join(dir, 'accounts.json'), 'utf8'))).not.toContain('s1');
    expect(readFileSync(join(dir, 'accounts.json'), 'utf8')).not.toMatch(/s1|s2/);
    expect(await store.credential(a.id)).toEqual({ secret: 's1', meta: { api_server_url: 'https://s' } });
    // the earliest added is the default; after b is used once, the default switches to b
    expect(store.defaultFor('devin')?.id).toBe(a.id);
    await store.touch(b.id);
    expect(store.defaultFor('devin')?.id).toBe(b.id);
    // re-login with the same label: id unchanged, secret replaced
    const a2 = await store.add('devin', { label: 'a@x.io', secret: 's1-new' });
    expect(a2.id).toBe(a.id);
    expect(store.list('devin')).toHaveLength(2);
    expect((await store.credential(a.id))?.secret).toBe('s1-new');
    await store.remove(a.id);
    expect(store.list('devin').map(x => x.id)).toEqual([b.id]);
    expect(await store.credential(a.id)).toBeUndefined();
    // still there after a reload
    const store2 = new AccountStore(join(dir, 'accounts.json'), vault);
    await store2.load();
    expect(store2.list().map(x => x.id)).toEqual([b.id]);
  });

  it('two hosts on one accounts.json: an account added in one is not erased by a touch or add in the other, a removal is not resurrected, reload picks up the difference', async () => {
    const dir = tmp();
    const file = join(dir, 'accounts.json');
    const vaultA = new FileVault(join(dir, 'secrets.json'));
    const vaultB = new FileVault(join(dir, 'secrets.json'));
    const a = new AccountStore(file, vaultA);
    const b = new AccountStore(file, vaultB);
    await a.load();
    await b.load();
    const one = await a.add('devin', { label: 'one@x.io', secret: 's1' });
    // b never saw `one`; its own add must land next to it, not over it
    const two = await b.add('devin', { label: 'two@x.io', secret: 's2' });
    expect(JSON.parse(readFileSync(file, 'utf8')).map((x: { id: string }) => x.id).sort()).toEqual([one.id, two.id].sort());
    expect(JSON.parse(readFileSync(join(dir, 'secrets.json'), 'utf8'))).toEqual({ [accountSecretKey(one.id)]: 's1', [accountSecretKey(two.id)]: 's2' });
    // a's cache still lists only `one`; a touch writes through what is on disk
    expect(a.list().map(x => x.id)).toEqual([one.id]);
    await a.touch(one.id);
    expect(JSON.parse(readFileSync(file, 'utf8'))).toHaveLength(2);
    expect(a.list().map(x => x.id).sort()).toEqual([one.id, two.id].sort());
    expect(await a.reload()).toBe(false);
    // b removes `two`; a still knows it but must not put it back when it touches `one`
    await b.remove(two.id);
    await a.touch(one.id);
    expect(JSON.parse(readFileSync(file, 'utf8')).map((x: { id: string }) => x.id)).toEqual([one.id]);
    expect(await b.credential(two.id)).toBeUndefined();
    // the other host's secret is readable without a restart
    expect((await vaultB.get(accountSecretKey(one.id)))).toBe('s1');
    // concurrent adds from both hosts all survive
    await Promise.all([a.add('devin', { label: 'p@x.io', secret: 'p' }), b.add('devin', { label: 'q@x.io', secret: 'q' }), a.add('devin', { label: 'r@x.io', secret: 'r' })]);
    expect(JSON.parse(readFileSync(file, 'utf8'))).toHaveLength(4);
    expect(Object.keys(JSON.parse(readFileSync(join(dir, 'secrets.json'), 'utf8')))).toHaveLength(4);
    expect(await b.reload()).toBe(true);
    expect(b.list()).toHaveLength(4);
  });

  it('reload cannot wipe an add or restore a remove that is waiting on the vault', async () => {
    const dir = tmp();
    const file = join(dir, 'accounts.json');
    const inner = new MemoryVault();
    let block!: () => void;
    const blocked = new Promise<void>(r => { block = r; });
    let entered!: () => void;
    const inVault = new Promise<void>(r => { entered = r; });
    const vault: SecretVault = {
      get: k => inner.get(k),
      async store(k, v) { entered(); await blocked; return inner.store(k, v); },
      async delete(k) { entered(); await blocked; return inner.delete(k); },
    };
    const store = new AccountStore(file, vault);
    await store.load();
    const adding = store.add('devin', { label: 'a@x.io', secret: 's1' });
    await inVault;
    const reloadingAdd = store.reload();
    block();
    const added = await adding;
    await reloadingAdd;
    expect(JSON.parse(readFileSync(file, 'utf8')).map((x: { id: string }) => x.id)).toEqual([added.id]);
    expect(store.list().map(x => x.id)).toEqual([added.id]);
    expect(await inner.get(accountSecretKey(added.id))).toBe('s1');

    let blockRemove!: () => void;
    const blockedRemove = new Promise<void>(r => { blockRemove = r; });
    let enteredRemove!: () => void;
    const inDelete = new Promise<void>(r => { enteredRemove = r; });
    vault.store = (k, v) => inner.store(k, v);
    vault.delete = async k => { enteredRemove(); await blockedRemove; return inner.delete(k); };
    const removing = store.remove(added.id);
    await inDelete;
    const reloadingRemove = store.reload();
    blockRemove();
    await removing;
    await reloadingRemove;
    expect(JSON.parse(readFileSync(file, 'utf8'))).toEqual([]);
    expect(store.list()).toEqual([]);
    expect(await inner.get(accountSecretKey(added.id))).toBeUndefined();
  });

  it('load drops the legacy "· name" tail from stored details and rewrites the file', async () => {
    const dir = tmp();
    const { mkdir, writeFile } = await import('node:fs/promises');
    await mkdir(dir, { recursive: true });
    const file = join(dir, 'accounts.json');
    await writeFile(file, JSON.stringify([
      { id: 'x', agent: 'devin', label: 'a@x.io', detail: 'Devin Max · Someone', addedAt: '2026-01-01T00:00:00.000Z' },
      { id: 'y', agent: 'devin', label: 'b@x.io', detail: 'Devin Max', addedAt: '2026-01-01T00:00:00.000Z' },
    ]));
    const store = new AccountStore(file, new MemoryVault());
    await store.load();
    expect(store.list().map(a => a.detail)).toEqual(['Devin Max', 'Devin Max']);
    expect(readFileSync(file, 'utf8')).not.toContain('Someone');
  });
});

describe('FileVault', () => {
  it('round-trips secrets to secrets.json with mode 600', async () => {
    const dir = tmp();
    const file = join(dir, 'secrets.json');
    const vault = new FileVault(file);
    await vault.store(accountSecretKey('a'), 's1');
    expect(await vault.get(accountSecretKey('a'))).toBe('s1');
    expect(JSON.parse(readFileSync(file, 'utf8'))).toEqual({ [accountSecretKey('a')]: 's1' });
    expect(statSync(file).mode & 0o777).toBe(0o600);
    await vault.delete(accountSecretKey('a'));
    expect(await vault.get(accountSecretKey('a'))).toBeUndefined();
    expect(JSON.parse(readFileSync(file, 'utf8'))).toEqual({});
    const again = new FileVault(file);
    await again.store(accountSecretKey('b'), 's2');
    expect(await again.get(accountSecretKey('b'))).toBe('s2');
  });

  it('a missing file is an empty table', async () => {
    const vault = new FileVault(join(tmp(), 'secrets.json'));
    expect(await vault.get('k')).toBeUndefined();
  });

  it('corrupt JSON is logged and never overwritten', async () => {
    const dir = tmp();
    const file = join(dir, 'secrets.json');
    writeFileSync(file, '{ nope');
    const logs: string[] = [];
    const vault = new FileVault(file, l => logs.push(l));
    await vault.store('k', 'v');
    expect(readFileSync(file, 'utf8')).toBe('{ nope');
    expect(await vault.get('k')).toBeUndefined();
    expect(logs.some(l => l.includes('unreadable'))).toBe(true);
  });

  it('get sees a secret another vault wrote to the same file', async () => {
    const file = join(tmp(), 'secrets.json');
    const a = new FileVault(file);
    const b = new FileVault(file);
    await a.store('k', 'v1');
    expect(await b.get('k')).toBe('v1');
    await a.store('k', 'v2');
    expect(await b.get('k')).toBe('v2');
    await a.delete('k');
    expect(await b.get('k')).toBeUndefined();
  });

  it('get sees a rotated secret for a key this vault already cached', async () => {
    const file = join(tmp(), 'secrets.json');
    const a = new FileVault(file);
    const b = new FileVault(file);
    await a.store(accountSecretKey('same'), 'old-key');
    expect(await b.get(accountSecretKey('same'))).toBe('old-key');
    await a.store(accountSecretKey('same'), 'new-key');
    expect(await b.get(accountSecretKey('same'))).toBe('new-key');
  });
});
