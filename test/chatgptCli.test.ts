import { execFile, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { SIDECAR } from './sidecarShell';

// `acpira bridge …` is what the copied connection prompt runs; every case goes through the real binary
const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

function fixture(prompt = true) {
  const root = realpathSync(mkdtempSync(join(tmpdir(), 'acpira-chatgpt-cli-'))); roots.push(root);
  const home = join(root, 'profile');
  const invoke = (args: string[], input?: string) => spawnSync(SIDECAR, ['bridge', ...args, '--home', home], { encoding: 'utf8', input, timeout: 10_000 });
  const opened = invoke(['open', '--key', 'test-only', '--cwd', root]);
  if (opened.status !== 0) throw new Error(opened.stderr);
  const id = (JSON.parse(opened.stdout) as { sessionId: string }).sessionId;
  const scope = ['--session', id, '--turn', 'test-turn'];
  const call = (action: string, args: string[] = [], input?: string) => invoke([action, ...scope, ...args], input);
  if (prompt) expect(call('prompt', ['--text', 'Synthetic CLI integration test']).status).toBe(0);
  const record = () => JSON.parse(readFileSync(join(home, 'bridges', 'chatgpt', `${id}.json`), 'utf8'));
  return { root, home, id, scope, call, record };
}
const quote = (s: string) => process.platform === 'win32' ? `"${s.replaceAll('"', '\\"')}"` : `'${s.replaceAll("'", "'\\''")}'`;
const command = (js: string) => `${quote(process.execPath)} -e ${quote(js)}`;

describe('ChatGPT bridge CLI real execution', () => {
  it('records the real command, output and success receipt, and preserves nonzero exit codes', () => {
    const { call, record } = fixture();
    const ok = call('exec', ['--command', command("console.log('bridge-output')")]);
    expect(ok.status, ok.stderr).toBe(0); expect(ok.stdout).toContain('bridge-output');
    const bad = call('exec', ['--command', command("console.error('bridge-error'); process.exit(7)")]);
    expect(bad.status).toBe(7);
    const blocks = record().turns[1].blocks;
    expect(blocks[0]).toMatchObject({ type: 'tool_call', status: 'completed', meta: 'exit 0' });
    expect(blocks[0].content.text).toContain('bridge-output');
    expect(blocks[1]).toMatchObject({ status: 'failed', meta: 'exit 7' });
    expect(blocks[1].content.text).toContain('bridge-error');
    expect(call('finish').status).toBe(0);
    expect(record().turns[1].stop).toBe('end_turn');
  });

  it('fails closed before executing when no active turn can record the operation', () => {
    const { call, root } = fixture(false);
    const target = join(root, 'must-not-exist');
    const result = call('exec', ['--command', command(`require('node:fs').writeFileSync(${JSON.stringify(target)}, 'bad')`)]);
    expect(result.status).not.toBe(0);
    expect(() => readFileSync(target)).toThrow();
  });

  it('protects edits with an expected hash and emits the actual before/after diff', () => {
    const { call, root, record } = fixture();
    const target = join(root, 'sample.txt'); writeFileSync(target, 'before\n');
    const bad = call('write', ['--file', target, '--expect', 'wrong'], 'after\n');
    expect(bad.status).not.toBe(0); expect(readFileSync(target, 'utf8')).toBe('before\n');
    const hash = createHash('sha256').update('before\n').digest('hex');
    const ok = call('write', ['--file', target, '--expect', hash], 'after\n');
    expect(ok.status, ok.stderr).toBe(0); expect(readFileSync(target, 'utf8')).toBe('after\n');
    expect(record().turns[1].blocks.at(-1)).toMatchObject({ status: 'completed', content: { type: 'diff', source: { path: target, oldText: 'before\n', newText: 'after\n' } } });
  });

  it('refuses file operations that escape the bound project through a symlink', () => {
    const { call, root } = fixture();
    const outside = realpathSync(mkdtempSync(join(tmpdir(), 'acpira-chatgpt-outside-'))); roots.push(outside);
    writeFileSync(join(outside, 'private.txt'), 'outside fixture');
    symlinkSync(outside, join(root, 'escape'), process.platform === 'win32' ? 'junction' : 'dir');
    const result = call('read', ['--file', 'escape/private.txt']);
    expect(result.status).not.toBe(0); expect(result.stderr).toContain('outside the bound project');
    expect(result.stdout).not.toContain('outside fixture');
  });

  it('publishes stdout before process completion so an open session shows the live loop', async () => {
    const { home, id, scope, record } = fixture();
    let done = false;
    const child = new Promise<void>((resolve, reject) => execFile(SIDECAR,
      ['bridge', 'exec', ...scope, '--home', home, '--command', command("console.log('stream-before-exit'); setTimeout(() => console.log('stream-done'), 1500)")],
      (error) => { done = true; if (error) reject(error); else resolve(); }));
    // The mirror record on disk is what every window's store reads
    const live = () => {
      const turn = record().turns.at(-1);
      return turn?.role === 'agent' && turn.blocks.some((b: { type: string; status?: string; content?: { type: string; text: string } }) =>
        b.type === 'tool_call' && b.status === 'in_progress' && b.content?.type === 'text' && b.content.text.includes('Output:\nstream-before-exit'));
    };
    try {
      const start = Date.now(); let sawOutput = false;
      while (Date.now() - start < 1400 && !done) {
        sawOutput = live();
        if (sawOutput) break;
        await new Promise(r => setTimeout(r, 25));
      }
      expect(sawOutput).toBe(true); expect(done).toBe(false);
      await child;
      expect(record().turns.at(-1).blocks[0]).toMatchObject({ status: 'completed' });
    } finally { await child.catch(() => {}); }
  });
});
