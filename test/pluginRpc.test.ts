import { chmodSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { Shell } from './sidecarShell';

// A page calling a method a plugin's resident process offers (`runtime.methods`) through the Rust sidecar: enabled
// from the plugin page, answered by id, and an undeclared method refused with the reason

const RATES = `
import { createInterface } from 'node:readline';
const send = m => process.stdout.write(JSON.stringify(m) + '\\n');
createInterface({ input: process.stdin }).on('close', () => process.exit(0)).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'rpc/rates.get') return send({ jsonrpc: '2.0', id: m.id, result: { eur: 0.92, from: m.params.from, asked: m.params.params } });
  if (m.id !== undefined) send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no ' + m.method } });
});
`;

// Frozen plugin copies are read-only, folders included: open them up so the temp dir can go
function writable(dir: string): void {
  chmodSync(dir, 0o755);
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) writable(p);
    else if (e.isFile()) chmodSync(p, 0o644);
  }
}

describe('plugin rpc contract', () => {
  const shells: Shell[] = [];
  const dirs: string[] = [];
  afterEach(async () => {
    for (const s of shells.splice(0)) await s.kill();
    for (const d of dirs.splice(0)) {
      writable(d);
      rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
    }
  });

  it('answers a page call to a declared method and refuses the rest', async () => {
    const temp = (p: string) => { const d = mkdtempSync(join(tmpdir(), p)); dirs.push(d); return d; };
    const data = temp('acpira-plugin-rpc-data-');
    const dir = join(data, 'plugins', 'rates');
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'rpc.mjs'), RATES);
    writeFileSync(join(dir, 'plugin.json'), JSON.stringify({
      schema: 1, id: 'rates', version: '1.0.0', description: 'Exchange rates',
      contributes: { runtime: { command: ['node', 'rpc.mjs'], methods: ['rates.get'] } },
    }));
    const s = new Shell(data, temp('acpira-plugin-rpc-ws-'));
    shells.push(s);
    await s.hello({ client: { name: 'contract', version: '0', capabilities: [] } });
    await s.open('V');

    s.view('V', { type: 'pluginAction', action: { op: 'enable', id: 'rates' } });
    const enabled = await s.hostMsg('V', 'plugins', m => m.view.plugins.some(p => p.id === 'rates' && p.enabled));
    expect(enabled.error).toBeUndefined();

    // The process starts right after the commit; until it answered initialize a call says it is not running
    let answer: { result?: unknown; error?: string } = {};
    for (let i = 0; i < 50; i++) {
      s.view('V', { type: 'pluginCall', id: `c${i}`, plugin: 'rates', method: 'rates.get', params: { to: 'EUR' } });
      answer = await s.hostMsg('V', 'pluginCalled', m => m.id === `c${i}`);
      if (!answer.error?.includes('no running process')) break;
      await new Promise(r => setTimeout(r, 100));
    }
    expect(answer).toEqual({ type: 'pluginCalled', id: expect.any(String), result: { eur: 0.92, from: 'page', asked: { to: 'EUR' } } });

    s.view('V', { type: 'pluginCall', id: 'x', plugin: 'rates', method: 'secret' });
    expect((await s.hostMsg('V', 'pluginCalled', m => m.id === 'x')).error).toContain('no method `secret`');
  });
});
