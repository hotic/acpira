import { execFile } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { withFileLock, writeAtomic } from '../src/host/store/fileLock';
import { TranscriptStore } from '../src/host/store/TranscriptStore';

const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));
const WORKER = fileURLToPath(new URL('./lock-worker.ts', import.meta.url));

function tmp() { return mkdtempSync(join(tmpdir(), 'acpira-lock-')); }

describe('withFileLock', () => {
  it('serializes callers in one process and leaves no lock file behind', async () => {
    const file = join(tmp(), 'data.json');
    const order: string[] = [];
    await Promise.all([
      withFileLock(file, async () => { order.push('a-in'); await new Promise(r => setTimeout(r, 30)); order.push('a-out'); }),
      withFileLock(file, async () => { order.push('b-in'); order.push('b-out'); }),
    ]);
    expect(order).toEqual(['a-in', 'a-out', 'b-in', 'b-out']);
    expect(() => readFileSync(`${file}.lock`)).toThrow();
  });

  it('a failing body releases the lock; a lock left by a dead host is taken over once stale', async () => {
    const file = join(tmp(), 'data.json');
    await expect(withFileLock(file, async () => { throw new Error('boom'); })).rejects.toThrow('boom');
    expect(() => readFileSync(`${file}.lock`)).toThrow();
    writeFileSync(`${file}.lock`, '2147483000');
    const old = (Date.now() - 60_000) / 1000;
    utimesSync(`${file}.lock`, old, old);
    const t0 = Date.now();
    await withFileLock(file, async () => { await writeAtomic(file, 'x'); });
    expect(Date.now() - t0).toBeLessThan(1000);
    expect(readFileSync(file, 'utf8')).toBe('x');
  });

  it('waits for a lock another host holds right now', async () => {
    const file = join(tmp(), 'data.json');
    writeFileSync(`${file}.lock`, String(process.pid));
    setTimeout(() => { writeFileSync(file, 'theirs'); rmSync(`${file}.lock`); }, 120);
    const t0 = Date.now();
    await withFileLock(file, async () => { await writeAtomic(file, `${readFileSync(file, 'utf8')}+mine`); });
    expect(Date.now() - t0).toBeGreaterThanOrEqual(100);
    expect(readFileSync(file, 'utf8')).toBe('theirs+mine');
  });

  it('read-increment-write from four processes at once loses nothing', async () => {
    const file = join(tmp(), 'counter');
    const run = () => new Promise<void>((resolve, reject) => execFile(TSX, [WORKER, file, '25'], (err, _out, stderr) => (err ? reject(new Error(`${err.message}\n${stderr}`)) : resolve())));
    await Promise.all([run(), run(), run(), run()]);
    expect(Number(readFileSync(file, 'utf8'))).toBe(100);
    expect(() => readFileSync(`${file}.lock`)).toThrow();
  }, 30_000);

  it('two processes reclaiming the same dead lock both run, neither increment is lost', async () => {
    const file = join(tmp(), 'counter');
    const lock = `${file}.lock`;
    const token = 'ab'.repeat(16);
    writeFileSync(lock, `2147483000\n${token}`);
    writeFileSync(`${lock}.${token}`, `2147483000\n${token}`);
    const old = (Date.now() - 60_000) / 1000;
    utimesSync(lock, old, old);
    utimesSync(`${lock}.${token}`, old, old);
    const run = () => new Promise<void>((resolve, reject) => execFile(TSX, [WORKER, file, '1'], (err, _out, stderr) => (err ? reject(new Error(`${err.message}\n${stderr}`)) : resolve())));
    await Promise.all([run(), run()]);
    expect(Number(readFileSync(file, 'utf8'))).toBe(2);
    expect(() => readFileSync(lock)).toThrow();
  }, 30_000);
});

describe('prefs.json shared by two windows', () => {
  it('each window writes only its own agent; the file keeps both', async () => {
    const dir = tmp();
    const a = new TranscriptStore(dir);
    const b = new TranscriptStore(dir);
    const prefsA = await a.loadPrefs();
    const prefsB = await b.loadPrefs();
    prefsA.lastSettings.grok = { modeId: 'plan', config: { model: 'grok-4.6' } };
    await a.savePrefs(prefsA, ['grok']);
    // b's snapshot predates a's write; writing kimi must not drop grok
    prefsB.lastSettings.kimi = { modeId: 'default', config: {} };
    const merged = await b.savePrefs(prefsB, ['kimi']);
    expect(Object.keys(merged.lastSettings).sort()).toEqual(['grok', 'kimi']);
    expect((await a.loadPrefs()).lastSettings.kimi).toEqual({ modeId: 'default', config: {} });
    // a newer choice for the same agent wins, whatever the other window's stale snapshot said
    prefsA.lastSettings.grok = { modeId: 'default', config: { model: 'grok-4.5' } };
    await a.savePrefs(prefsA, ['grok']);
    expect((await b.loadPrefs()).lastSettings.grok).toEqual({ modeId: 'default', config: { model: 'grok-4.5' } });
    // the default (no agent list) writes every entry the caller has, as before
    await b.savePrefs({ lastSettings: { devin: { config: {} } } });
    expect(Object.keys((await a.loadPrefs()).lastSettings).sort()).toEqual(['devin', 'grok', 'kimi']);
  });
});
