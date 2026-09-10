import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { describe, expect, it } from 'vitest';
import type { SessionRecord } from '../src/host/acp/AcpSession';
import { TranscriptStore, isSessionId, summarize } from '../src/host/store/TranscriptStore';

function record(id: string, title = 'T'): SessionRecord {
  const now = '2026-01-01T00:00:00.000Z';
  return { id, agent: 'fake', cwd: '/tmp', title, createdAt: now, updatedAt: now, turns: [], controls: { modes: [], options: [] }, commands: [] };
}

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), 'acpira-store-'));
  const logs: string[] = [];
  return { dir, logs, store: new TranscriptStore(dir, l => logs.push(l)) };
}

describe('TranscriptStore', () => {
  it('dispose writes what is still debounced', async () => {
    const { dir, store } = fixture();
    store.save(record('a', 'first'), 10_000);
    store.save(record('b', 'second'), 10_000);
    await store.dispose();
    expect(JSON.parse(readFileSync(join(dir, 'a.json'), 'utf8')).title).toBe('first');
    expect(JSON.parse(readFileSync(join(dir, 'b.json'), 'utf8')).title).toBe('second');
  });

  it('a debounced save that cannot reach the disk is logged, not thrown', async () => {
    const { dir, logs, store } = fixture();
    // A regular file where the record's directory must go makes the write fail
    writeFileSync(join(dir, 'x.json'), '');
    const broken = new TranscriptStore(join(dir, 'x.json'), l => logs.push(l));
    broken.save(record('x'), 0);
    await new Promise(r => setTimeout(r, 50));
    expect(logs.some(l => l.includes('save failed'))).toBe(true);
    expect(store).toBeDefined();
  });

  it('unreadable or malformed records load as missing and leave a log line', async () => {
    const { dir, logs, store } = fixture();
    writeFileSync(join(dir, 'bad.json'), '{ not json');
    writeFileSync(join(dir, 'shape.json'), JSON.stringify({ id: 'shape' }));
    expect(await store.load('bad')).toBeNull();
    expect(await store.load('shape')).toBeNull();
    expect(await store.load('absent')).toBeNull();
    expect(logs.filter(l => l.includes('record unreadable'))).toHaveLength(2);
    await store.flush(record('good'));
    expect((await store.load('good'))?.id).toBe('good');
  });

  it('rebuilding a lost index skips the broken records', async () => {
    const { dir, store } = fixture();
    await store.flush(record('ok'));
    writeFileSync(join(dir, 'bad.json'), 'nope');
    expect((await store.loadIndex()).map(s => s.id)).toEqual(['ok']);
  });

  // The failure that lost sessions in the wild: every window holds its own copy of the list and used to write it back whole
  it('two hosts over one directory: neither clobbers the other’s sessions, and a valid but incomplete index is healed from the record files', async () => {
    const { dir, store: a } = fixture();
    const b = new TranscriptStore(dir);
    await a.flush(record('a1', 'from A'));
    const listA = await a.syncIndex([summarize(record('a1', 'from A'))], new Set(['a1']));
    expect(listA.map(s => s.id)).toEqual(['a1']);

    // B booted earlier with an empty list and never saw a1; its write must not drop a1
    await b.flush(record('b1', 'from B'));
    const listB = await b.syncIndex([summarize(record('b1', 'from B'))], new Set(['b1']));
    expect(listB.map(s => s.id).sort()).toEqual(['a1', 'b1']);
    // A's next write, still ignorant of b1, keeps it too
    expect((await a.syncIndex(listA, new Set(['a1']))).map(s => s.id).sort()).toEqual(['a1', 'b1']);

    // An index hand-truncated to nothing (or clobbered by an old build) comes back from the files
    writeFileSync(join(dir, 'index.json'), '[]');
    expect((await a.loadIndex()).map(s => s.title).sort()).toEqual(['from A', 'from B']);
    // An entry whose file is gone drops out
    rmSync(join(dir, 'b1.json'));
    expect((await a.loadIndex()).map(s => s.id)).toEqual(['a1']);
  });

  it('own ids take this host’s summary, the rest the disk’s (another window may have renamed them); legacy entries get cwd backfilled', async () => {
    const { dir, store } = fixture();
    await store.flush(record('x', 'renamed elsewhere'));
    await store.flush(record('y', 'mine'));
    // Disk index: x already renamed by another window, y stale; this host's list has both under their old titles
    const stale = { ...summarize(record('y', 'old')), cwd: undefined as unknown as string };
    writeFileSync(join(dir, 'index.json'), JSON.stringify([summarize(record('x', 'renamed elsewhere')), stale]));
    const out = await store.syncIndex([summarize(record('x', 'old')), summarize(record('y', 'mine'))], new Set(['y']));
    expect(out.find(s => s.id === 'x')?.title).toBe('renamed elsewhere');
    expect(out.find(s => s.id === 'y')).toMatchObject({ title: 'mine', cwd: '/tmp' });
  });

  it('trash moves the record and its blobs out of the live directory and back; sweep removes only entries older than the window', async () => {
    const { dir, store } = fixture();
    await store.flush(record('t'));
    await store.saveBlob('t', '.png', new Uint8Array([1, 2, 3]));
    await store.trash('t');
    expect(existsSync(join(dir, 't.json'))).toBe(false);
    expect(existsSync(join(dir, 'trash', 't.json'))).toBe(true);
    expect(readdirSync(join(dir, 'trash', 't'))).toHaveLength(1);
    expect((await store.loadIndex()).map(s => s.id)).toEqual([]);

    await store.sweepTrash(60_000);
    expect(existsSync(join(dir, 'trash', 't.json'))).toBe(true);
    await store.restore('t');
    expect(existsSync(join(dir, 't.json'))).toBe(true);
    expect(readdirSync(join(dir, 't'))).toHaveLength(1);
    expect((await store.loadIndex()).map(s => s.id)).toEqual(['t']);

    await store.trash('t');
    await store.sweepTrash(0);
    expect(existsSync(join(dir, 'trash', 't.json'))).toBe(false);
    expect(existsSync(join(dir, 'trash', 't'))).toBe(false);
    await store.remove('t');
  });

  // The same session live in two windows: one deletes it, the other's debounced save lands afterwards and used to recreate the record
  it('a save of a record another store took out of the directory is dropped, a first write and a write after restore go through', async () => {
    const { dir, logs, store: a } = fixture();
    const b = new TranscriptStore(dir, l => logs.push(l));
    await a.flush(record('s', 'v1'));
    expect((await b.load('s'))?.title).toBe('v1');
    // B's stale save is pending when A trashes; flushing it must not bring the record back into the live directory
    b.save(record('s', 'v2 from B'), 10_000);
    await a.trash('s');
    await b.dispose();
    expect(existsSync(join(dir, 's.json'))).toBe(false);
    expect(JSON.parse(readFileSync(join(dir, 'trash', 's.json'), 'utf8')).title).toBe('v1');
    expect(logs.some(l => l.includes('not written back'))).toBe(true);
    expect((await b.syncIndex([summarize(record('s'))], new Set(['s']))).map(s => s.id)).toEqual([]);
    expect(b.knew('s')).toBe(true);
    // Removed for good: still refused. Undone instead: the file is back, so B's next save applies again
    await a.remove('s');
    await b.flush(record('s', 'v3 from B'));
    expect(existsSync(join(dir, 's.json'))).toBe(false);
    await a.flush(record('s', 'v1'));
    await a.trash('s');
    await a.restore('s');
    await b.flush(record('s', 'v4 from B'));
    expect(JSON.parse(readFileSync(join(dir, 's.json'), 'utf8')).title).toBe('v4 from B');
    // A record the store never had on disk is created as usual
    await b.flush(record('fresh'));
    expect(existsSync(join(dir, 'fresh.json'))).toBe(true);
  });

  // The save and index debounces are equal, so a fresh session's first write is often still in flight (timer fired, file not yet renamed
  // into place) when the index reconcile lists the directory. The list must wait for it: a live id missing from the list reads as
  // "deleted by another window" to the manager, which closed the brand-new session and toasted about it
  it('syncIndex waits for writes already in flight, so a fresh record never reads as deleted', async () => {
    const { dir, store } = fixture();
    for (let i = 0; i < 20; i++) {
      const id = `fresh-${i}`;
      store.save(record(id), 0);
      // Let the timer fire: the write has started but has not reached the directory yet
      await new Promise(r => setTimeout(r, 0));
      expect((await store.syncIndex([summarize(record(id))], new Set([id]))).map(s => s.id)).toContain(id);
      expect(existsSync(join(dir, `${id}.json`))).toBe(true);
    }
  });

  it('concurrent writes of one record are serialized: the last one wins and no write fails on the shared temp file', async () => {
    const { dir, logs, store } = fixture();
    await Promise.all([store.flush(record('c', 'v1')), store.flush(record('c', 'v2')), store.flush(record('c', 'v3'))]);
    expect(JSON.parse(readFileSync(join(dir, 'c.json'), 'utf8')).title).toBe('v3');
    expect(logs).toEqual([]);
    // remove waits for an in-flight write instead of letting it recreate the file afterwards
    const w = store.flush(record('c', 'v4'));
    await store.remove('c');
    await w;
    expect(existsSync(join(dir, 'c.json'))).toBe(false);
  });

  it('a record still debounced is readable through load, and writing the index lands it first so the index never names a missing file', async () => {
    const { dir, store } = fixture();
    store.save(record('p', 'pending'), 10_000);
    expect((await store.load('p'))?.title).toBe('pending');
    expect(existsSync(join(dir, 'p.json'))).toBe(false);
    expect((await store.syncIndex([summarize(record('p', 'pending'))], new Set(['p']))).map(s => s.id)).toEqual(['p']);
    expect(existsSync(join(dir, 'p.json'))).toBe(true);
    await store.dispose();
    // No temp files are left behind by the atomic writes
    expect(readdirSync(dir).filter(f => f.endsWith('.tmp'))).toEqual([]);
  });

  it('rejects path-like ids and does not touch files outside the store', async () => {
    const { dir, logs, store } = fixture();
    const marker = join(dirname(dir), `keep-${Date.now()}`);
    writeFileSync(marker, 'keep');
    const outside = record('..');
    (outside as { id: string }).id = '..';
    store.save(outside as SessionRecord, 0);
    await store.flush(record('/tmp/x'));
    await store.trash('..');
    await store.remove('..');
    await store.remove('/etc/passwd');
    expect(await store.load('..')).toBeNull();
    expect(await store.load('/tmp/x')).toBeNull();
    expect(existsSync(marker)).toBe(true);
    expect(existsSync(dir)).toBe(true);
    expect(logs.some(l => l.includes('illegal id'))).toBe(true);
    rmSync(marker, { force: true });
  });

  it('a record whose inner id does not match the filename is missing', async () => {
    const { dir, store } = fixture();
    writeFileSync(join(dir, 'good.json'), JSON.stringify(record('other')));
    expect(await store.load('good')).toBeNull();
  });

  it('refuses to follow a session directory that is a symlink out of the store', async () => {
    const { dir, store } = fixture();
    const outside = mkdtempSync(join(tmpdir(), 'acpira-out-'));
    writeFileSync(join(outside, 'secret.txt'), 'secret');
    try { symlinkSync(outside, join(dir, 'link')); }
    catch { rmSync(outside, { recursive: true, force: true }); return; }
    await store.remove('link');
    expect(existsSync(join(outside, 'secret.txt'))).toBe(true);
    await expect(store.saveBlob('link', '.txt', new Uint8Array([1]))).rejects.toThrow();
    rmSync(outside, { recursive: true, force: true });
  });

  it('a stream of saves still flushes within the max wait', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-store-'));
    const store = new TranscriptStore(dir, () => {}, { saveDebounceMs: 10_000, saveMaxWaitMs: 80 });
    store.save(record('a', 'v1'));
    await new Promise(r => setTimeout(r, 25));
    store.save(record('a', 'v2'));
    await new Promise(r => setTimeout(r, 25));
    store.save(record('a', 'v3'));
    await new Promise(r => setTimeout(r, 80));
    expect(JSON.parse(readFileSync(join(dir, 'a.json'), 'utf8')).title).toBe('v3');
    await store.dispose();
  });

  it('a failed save notifies onSaveError', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-store-'));
    writeFileSync(join(dir, 'x.json'), '');
    const errors: string[] = [];
    const broken = new TranscriptStore(join(dir, 'x.json'), () => {}, { onSaveError: (id, error) => errors.push(`${id}:${error}`) });
    broken.save(record('x'), 0);
    await new Promise(r => setTimeout(r, 50));
    expect(errors.some(e => e.startsWith('x:'))).toBe(true);
  });
});

describe('isSessionId', () => {
  it('accepts uuid-like tokens and rejects path components', () => {
    expect(isSessionId('a1b2')).toBe(true);
    expect(isSessionId('550e8400-e29b-41d4-a716-446655440000')).toBe(true);
    expect(isSessionId('..')).toBe(false);
    expect(isSessionId('../secrets')).toBe(false);
    expect(isSessionId('/tmp/x')).toBe(false);
    expect(isSessionId('index')).toBe(false);
    expect(isSessionId('trash')).toBe(false);
  });
});
