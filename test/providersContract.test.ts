import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer, type IncomingHttpHeaders } from 'node:http';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { newModel, type Provider } from '../src/shared/providers';
import { Shell } from './sidecarShell';

// The built-in agent's model sources against the Rust sidecar: the page edits providers.json through actions, the key
// lands in secrets.json and never comes back, and the agent itself (spawned by a fresh controls probe, no prompt sent)
// offers the saved models

describe('providers contract', () => {
  const shells: Shell[] = [];
  const dirs: string[] = [];
  afterEach(async () => {
    for (const s of shells.splice(0)) await s.kill();
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  });

  it('saves a source with its key apart, lists it to the agent and deletes both', async () => {
    const temp = (p: string) => { const d = mkdtempSync(join(tmpdir(), p)); dirs.push(d); return d; };
    const data = temp('acpira-providers-data-');
    const s = new Shell(data, temp('acpira-providers-ws-'));
    shells.push(s);
    await s.hello({ client: { name: 'contract', version: '0', capabilities: [] } });
    await s.open('V');

    s.view('V', { type: 'providers' });
    const empty = await s.hostMsg('V', 'providers');
    expect(empty.view.providers).toEqual([]);

    const draft: Provider = { id: '', name: 'Deep Seek', preset: 'custom', format: 'openai-chat', baseUrl: 'https://api.deepseek.com/v1/', fullUrl: false, enabled: true, models: [newModel('deepseek-chat')] };
    s.view('V', { type: 'providerAction', action: { kind: 'save', provider: draft, key: 'sk-contract' } });
    const saved = await s.hostMsg('V', 'providers', m => m.view.providers.length === 1);
    expect(saved.error).toBeUndefined();
    expect(saved.view.providers[0]).toMatchObject({ id: 'deep-seek', baseUrl: 'https://api.deepseek.com/v1', hasKey: true });
    expect(JSON.stringify(saved)).not.toContain('sk-contract');
    expect(readFileSync(join(data, 'providers.json'), 'utf8')).not.toContain('sk-contract');
    expect(JSON.parse(readFileSync(join(data, 'secrets.json'), 'utf8'))['acpira.provider.deep-seek']).toBe('sk-contract');

    // A refused edit keeps the view and says why
    s.view('V', { type: 'providerAction', action: { kind: 'save', provider: { ...draft, baseUrl: 'api.test' } } });
    expect((await s.hostMsg('V', 'providers', m => !!m.error)).error).toContain('http');

    s.view('V', { type: 'controls', agent: 'acpira', fresh: true });
    const controls = await s.hostMsg('V', 'controls', m => m.agent === 'acpira');
    const model = controls.controls.find(c => c.id === 'model');
    expect(model?.value).toBe('deep-seek/deepseek-chat');
    expect(model?.options.map(o => o.id)).toEqual(['deep-seek/deepseek-chat']);

    s.view('V', { type: 'providerAction', action: { kind: 'delete', id: 'deep-seek' } });
    // hostMsg scans everything received so far: the first, empty view must not stand in for the delete's reply
    await s.hostMsg('V', 'providers', m => m !== empty && m.view.providers.length === 0 && !m.error);
    expect(JSON.parse(readFileSync(join(data, 'secrets.json'), 'utf8'))).not.toHaveProperty(['acpira.provider.deep-seek']);
  });

  it('carries the presets and families, and answers a model fetch with the stored key', async () => {
    const temp = (p: string) => { const d = mkdtempSync(join(tmpdir(), p)); dirs.push(d); return d; };
    const data = temp('acpira-providers-data-');
    const s = new Shell(data, temp('acpira-providers-ws-'));
    shells.push(s);
    // A stand-in model list: ids only, so everything past the id comes from the catalogue or the defaults
    const seen: IncomingHttpHeaders[] = [];
    const server = createServer((req, res) => {
      seen.push(req.headers);
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify({ object: 'list', data: [{ id: 'deepseek-v4-flash' }, { id: 'text-embedding-v4' }, { id: 'my-finetune' }] }));
    });
    await new Promise<void>(r => server.listen(0, '127.0.0.1', r));
    try {
      const port = (server.address() as AddressInfo).port;
      await s.hello({ client: { name: 'contract', version: '0', capabilities: [] } });
      await s.open('V');
      const draft: Provider = { id: '', name: 'Stub', preset: 'custom', format: 'openai-chat', baseUrl: `http://127.0.0.1:${port}/v1`, fullUrl: false, enabled: true, models: [] };
      s.view('V', { type: 'providerAction', action: { kind: 'save', provider: draft, key: 'sk-stored' } });
      const view = (await s.hostMsg('V', 'providers', m => m.view.providers.length === 1)).view;
      expect(view.presets.map(p => p.id)).toEqual(expect.arrayContaining(['deepseek', 'ollama', 'custom']));
      expect(view.families[0]).toBe('generic');

      s.view('V', { type: 'providerProbe', id: 'fetch-1', probe: { kind: 'models', provider: view.providers[0]! } });
      const got = await s.hostMsg('V', 'providerProbed', m => m.id === 'fetch-1');
      expect(got.outcome.kind).toBe('models');
      const models = got.outcome.kind === 'models' ? got.outcome.models : [];
      expect(models.map(m => m.id)).toEqual(['deepseek-v4-flash', 'my-finetune']);
      expect(models[0]!.estimated).toEqual(expect.arrayContaining(['context', 'output']));
      expect(models[1]).toMatchObject({ context: 128000, estimated: ['context', 'input'] });
      expect(seen[0]!.authorization).toBe('Bearer sk-stored');

      // A key typed into the form wins over the stored one; a closed port is a failed outcome, not a dropped reply
      s.view('V', { type: 'providerProbe', id: 'check-1', probe: { kind: 'check', provider: view.providers[0]!, key: 'sk-typed' } });
      expect((await s.hostMsg('V', 'providerProbed', m => m.id === 'check-1')).outcome).toEqual({ kind: 'check', count: 3 });
      expect(seen[1]!.authorization).toBe('Bearer sk-typed');
      s.view('V', { type: 'providerProbe', id: 'check-2', probe: { kind: 'check', provider: { ...draft, baseUrl: 'http://127.0.0.1:9/v1' } } });
      expect((await s.hostMsg('V', 'providerProbed', m => m.id === 'check-2')).outcome.kind).toBe('failed');
    } finally {
      server.close();
    }
  });
});
