import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { FileVault, MemoryVault, accountSecretKey } from '../src/accounts/AccountStore';
import { acpiraHome, displayHome, migrateOnce } from '../src/store/dataDir';

function tmp() { return mkdtempSync(join(tmpdir(), 'acpira-home-')); }

describe('displayHome', () => {
  it('spells a folder under the home with ~ on Unix only', () => {
    expect(displayHome('/home/u/.acpira', '/home/u', true)).toBe('~/.acpira');
    expect(displayHome('/home/u2/.acpira', '/home/u', true)).toBe('/home/u2/.acpira');
    expect(displayHome('/home/u/.acpira', '/home/u', false)).toBe('/home/u/.acpira');
  });
});

describe('acpiraHome', () => {
  it('uses ACPIRA_HOME when non-empty, otherwise ~/.acpira', () => {
    expect(acpiraHome({}, '/home/u')).toBe(join('/home/u', '.acpira'));
    expect(acpiraHome({ ACPIRA_HOME: '' }, '/home/u')).toBe(join('/home/u', '.acpira'));
    expect(acpiraHome({ ACPIRA_HOME: '  ' }, '/home/u')).toBe(join('/home/u', '.acpira'));
    expect(acpiraHome({ ACPIRA_HOME: '/tmp/iso' }, '/home/u')).toBe('/tmp/iso');
    expect(acpiraHome({ ACPIRA_HOME: '  /tmp/iso  ' }, '/home/u')).toBe('/tmp/iso');
    expect(acpiraHome({ ACPIRA_HOME: 'rel' }, '/home/u')).toBe(resolve('rel'));
  });
});

describe('migrateOnce', () => {
  it('copies accounts, sessions (not scratch) and secrets into an empty dest and writes .migrated', async () => {
    const from = tmp();
    const to = tmp();
    mkdirSync(join(from, 'sessions', 's1'), { recursive: true });
    mkdirSync(join(from, 'scratch', 'login-x'), { recursive: true });
    const accounts = [{ id: 'a1', agent: 'devin', label: 'a@x.io', addedAt: '2026-01-01T00:00:00.000Z' }];
    writeFileSync(join(from, 'accounts.json'), JSON.stringify(accounts));
    writeFileSync(join(from, 'sessions', 'index.json'), JSON.stringify([{ id: 's1' }]));
    writeFileSync(join(from, 'sessions', 'prefs.json'), JSON.stringify({ lastSettings: {} }));
    writeFileSync(join(from, 'sessions', 's1.json'), JSON.stringify({ id: 's1', title: 'hello' }));
    writeFileSync(join(from, 'sessions', 's1', 'blob.txt'), 'img');
    writeFileSync(join(from, 'scratch', 'login-x', 'keep.txt'), 'no');
    const oldVault = new MemoryVault();
    await oldVault.store(accountSecretKey('a1'), 'sec');
    const logs: string[] = [];
    const newVault = new FileVault(join(to, 'secrets.json'));
    await migrateOnce({ from, to, oldVault, newVault, log: l => logs.push(l) });
    expect(JSON.parse(readFileSync(join(to, '.migrated'), 'utf8')).sources).toEqual([from]);
    expect(JSON.parse(readFileSync(join(to, 'accounts.json'), 'utf8'))).toEqual(accounts);
    expect(JSON.parse(readFileSync(join(to, 'sessions', 's1.json'), 'utf8')).title).toBe('hello');
    expect(readFileSync(join(to, 'sessions', 's1', 'blob.txt'), 'utf8')).toBe('img');
    expect(existsSync(join(to, 'sessions', 'prefs.json'))).toBe(true);
    // The index is not copied: the store rebuilds it from the record files
    expect(existsSync(join(to, 'sessions', 'index.json'))).toBe(false);
    expect(existsSync(join(to, 'scratch'))).toBe(false);
    expect(await newVault.get(accountSecretKey('a1'))).toBe('sec');
    expect(existsSync(join(from, 'accounts.json'))).toBe(true);
    expect(logs.some(l => l.includes('Merged'))).toBe(true);

    await migrateOnce({ from, to, oldVault, newVault, log: l => logs.push(l) });
    expect(logs.filter(l => l.includes('Merged'))).toHaveLength(1);
  });

  // VS Code and Cursor have separate globalStorage trees but share ~/.acpira: the second IDE to start must bring its own sessions along
  it('a second source merges into a populated dest: new records and accounts are added, existing ones are never overwritten', async () => {
    const vscode = tmp();
    const cursor = tmp();
    const to = tmp();
    const acc = (id: string, label: string) => ({ id, agent: 'devin', label, addedAt: '2026-01-01T00:00:00.000Z' });
    for (const [from, id, title] of [[vscode, 'v1', 'from vscode'], [cursor, 'c1', 'from cursor']] as const) {
      mkdirSync(join(from, 'sessions', id), { recursive: true });
      writeFileSync(join(from, 'sessions', `${id}.json`), JSON.stringify({ id, title }));
      writeFileSync(join(from, 'sessions', `${id}`, 'blob.txt'), title);
      writeFileSync(join(from, 'sessions', 'index.json'), JSON.stringify([{ id }]));
      writeFileSync(join(from, 'sessions', 'prefs.json'), JSON.stringify({ lastSettings: { [id]: {} } }));
    }
    // The same login imported in both IDEs gets a different id each time (ids are UUIDs), so both copies come along
    writeFileSync(join(vscode, 'accounts.json'), JSON.stringify([acc('a-vscode', 'me@x.io')]));
    writeFileSync(join(cursor, 'accounts.json'), JSON.stringify([acc('a-cursor', 'me@x.io')]));
    const vault = new MemoryVault();
    await vault.store(accountSecretKey('a-vscode'), 'v-sec');
    await vault.store(accountSecretKey('a-cursor'), 'c-sec');
    const newVault = new MemoryVault();

    await migrateOnce({ from: vscode, to, oldVault: vault, newVault });
    // The user edits the merged copy before the other IDE starts: that edit must survive
    writeFileSync(join(to, 'sessions', 'v1.json'), JSON.stringify({ id: 'v1', title: 'renamed in acpira' }));
    await migrateOnce({ from: cursor, to, oldVault: vault, newVault });

    expect(JSON.parse(readFileSync(join(to, '.migrated'), 'utf8')).sources).toEqual([vscode, cursor]);
    expect(JSON.parse(readFileSync(join(to, 'sessions', 'v1.json'), 'utf8')).title).toBe('renamed in acpira');
    expect(JSON.parse(readFileSync(join(to, 'sessions', 'c1.json'), 'utf8')).title).toBe('from cursor');
    expect(readFileSync(join(to, 'sessions', 'c1', 'blob.txt'), 'utf8')).toBe('from cursor');
    // prefs: first source wins, the second does not overwrite
    expect(JSON.parse(readFileSync(join(to, 'sessions', 'prefs.json'), 'utf8')).lastSettings).toEqual({ v1: {} });
    expect(JSON.parse(readFileSync(join(to, 'accounts.json'), 'utf8')).map((a: { id: string }) => a.id)).toEqual(['a-vscode', 'a-cursor']);
    expect(await newVault.get(accountSecretKey('a-vscode'))).toBe('v-sec');
    expect(await newVault.get(accountSecretKey('a-cursor'))).toBe('c-sec');
  });

  it('a legacy timestamp marker: the source whose records are already here is only recorded, an unknown source still merges', async () => {
    const first = tmp();
    const second = tmp();
    const to = tmp();
    mkdirSync(join(first, 'sessions'), { recursive: true });
    mkdirSync(join(second, 'sessions'), { recursive: true });
    mkdirSync(join(to, 'sessions'), { recursive: true });
    writeFileSync(join(first, 'sessions', 'f1.json'), JSON.stringify({ id: 'f1', title: 'old copy' }));
    writeFileSync(join(first, 'sessions', 'f2.json'), JSON.stringify({ id: 'f2', title: 'deleted since' }));
    writeFileSync(join(to, 'sessions', 'f1.json'), JSON.stringify({ id: 'f1', title: 'live copy' }));
    writeFileSync(join(second, 'sessions', 's1.json'), JSON.stringify({ id: 's1', title: 'never merged' }));
    writeFileSync(join(to, '.migrated'), '2026-09-01T00:00:00.000Z\n');

    await migrateOnce({ from: first, to, oldVault: new MemoryVault(), newVault: new MemoryVault() });
    // f2 was deleted in acpira after the original migration; it must not come back
    expect(existsSync(join(to, 'sessions', 'f2.json'))).toBe(false);
    expect(JSON.parse(readFileSync(join(to, 'sessions', 'f1.json'), 'utf8')).title).toBe('live copy');
    await migrateOnce({ from: second, to, oldVault: new MemoryVault(), newVault: new MemoryVault() });
    expect(existsSync(join(to, 'sessions', 's1.json'))).toBe(true);
    expect(JSON.parse(readFileSync(join(to, '.migrated'), 'utf8')).sources).toEqual([first, second]);
  });

  it('a source already listed in the marker is skipped without touching the dest', async () => {
    const from = tmp();
    const to = tmp();
    writeFileSync(join(from, 'accounts.json'), JSON.stringify([{ id: 'a1' }]));
    writeFileSync(join(to, '.migrated'), JSON.stringify({ sources: [from] }));
    await migrateOnce({ from, to, oldVault: new MemoryVault(), newVault: new MemoryVault() });
    expect(existsSync(join(to, 'accounts.json'))).toBe(false);
    expect(readFileSync(join(to, '.migrated'), 'utf8')).toBe(JSON.stringify({ sources: [from] }));
  });

  it('stamps .migrated without copying when there is no legacy data', async () => {
    const from = tmp();
    const to = tmp();
    await migrateOnce({ from, to, oldVault: new MemoryVault(), newVault: new MemoryVault() });
    expect(existsSync(join(to, '.migrated'))).toBe(true);
    expect(existsSync(join(to, 'accounts.json'))).toBe(false);
  });

  it('does not write .migrated when the copy fails, then retries', async () => {
    const from = tmp();
    const to = tmp();
    mkdirSync(join(from, 'sessions'), { recursive: true });
    writeFileSync(join(from, 'accounts.json'), JSON.stringify([{ id: 'a1', agent: 'devin', label: 'a', addedAt: '2026-01-01T00:00:00.000Z' }]));
    writeFileSync(join(from, 'sessions', 's1.json'), JSON.stringify({ id: 's1' }));
    chmodSync(join(from, 'sessions'), 0o000);
    const logs: string[] = [];
    try {
      await migrateOnce({ from, to, oldVault: new MemoryVault(), newVault: new MemoryVault(), log: l => logs.push(l) });
      expect(existsSync(join(to, '.migrated'))).toBe(false);
      expect(logs.some(l => l.includes('failed'))).toBe(true);
    } finally {
      chmodSync(join(from, 'sessions'), 0o755);
    }
    await migrateOnce({ from, to, oldVault: new MemoryVault(), newVault: new MemoryVault() });
    expect(existsSync(join(to, '.migrated'))).toBe(true);
    expect(existsSync(join(to, 'sessions', 's1.json'))).toBe(true);
    expect(existsSync(join(to, 'accounts.json'))).toBe(true);
  });
});
