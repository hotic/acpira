import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer, type IncomingHttpHeaders } from 'node:http';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { newModel, type Provider } from '../packages/shared/providers';
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

    // The fast model takes only a usable pick, lands in the file as `fastModel`, and an absent pick turns it off
    s.view('V', { type: 'providerAction', action: { kind: 'fastModel', pick: 'deep-seek/gone' } });
    expect((await s.hostMsg('V', 'providers', m => !!m.error?.includes('gone'))).view.fastModel).toBeUndefined();
    s.view('V', { type: 'providerAction', action: { kind: 'fastModel', pick: 'deep-seek/deepseek-chat' } });
    await s.hostMsg('V', 'providers', m => m.view.fastModel === 'deep-seek/deepseek-chat');
    expect(JSON.parse(readFileSync(join(data, 'providers.json'), 'utf8')).fastModel).toBe('deep-seek/deepseek-chat');
    // Only a reply that arrives after the unset counts: earlier views have no fastModel either
    const earlier = new Set(s.hostMsgs('V'));
    s.view('V', { type: 'providerAction', action: { kind: 'fastModel' } });
    await s.hostMsg('V', 'providers', m => !earlier.has(m) && m.view.fastModel === undefined && !m.error);

    // Web search: Jina with a key stores the key in the vault (never in the view), and back to default keeps it
    s.view('V', { type: 'providerAction', action: { kind: 'webSearch', backend: 'jina', key: 'jina-contract' } });
    const jina = await s.hostMsg('V', 'providers', m => m.view.webSearch === 'jina');
    expect(jina.view.webKeys).toEqual(['jina']);
    expect(JSON.stringify(jina)).not.toContain('jina-contract');
    expect(JSON.parse(readFileSync(join(data, 'providers.json'), 'utf8')).webSearch).toBe('jina');
    expect(JSON.parse(readFileSync(join(data, 'secrets.json'), 'utf8'))['acpira.web.jina']).toBe('jina-contract');
    const beforeDefault = new Set(s.hostMsgs('V'));
    s.view('V', { type: 'providerAction', action: { kind: 'webSearch', backend: 'default' } });
    const back = await s.hostMsg('V', 'providers', m => !beforeDefault.has(m) && m.view.webSearch === undefined && !m.error);
    expect(back.view.webKeys).toEqual(['jina']);
    expect(JSON.parse(readFileSync(join(data, 'providers.json'), 'utf8'))).not.toHaveProperty('webSearch');
    // DeepSeek through a model source lands as `webSearchVia`; an empty via goes back to DeepSeek's own API
    s.view('V', { type: 'providerAction', action: { kind: 'webSearch', backend: 'deepseek', via: 'deep-seek' } });
    const via = await s.hostMsg('V', 'providers', m => m.view.webSearch === 'deepseek' && m.view.webSearchVia === 'deep-seek');
    expect(via.error).toBeUndefined();
    expect(JSON.parse(readFileSync(join(data, 'providers.json'), 'utf8'))).toMatchObject({ webSearch: 'deepseek', webSearchVia: 'deep-seek' });
    s.view('V', { type: 'providerAction', action: { kind: 'webSearch', backend: 'deepseek', via: '' } });
    await s.hostMsg('V', 'providers', m => m.view.webSearch === 'deepseek' && m.view.webSearchVia === undefined && !m.error);
    expect(JSON.parse(readFileSync(join(data, 'providers.json'), 'utf8'))).not.toHaveProperty('webSearchVia');

    // Compaction: chunk is written, the default summary leaves no entry
    s.view('V', { type: 'providerAction', action: { kind: 'compaction', strategy: 'chunk' } });
    await s.hostMsg('V', 'providers', m => m.view.compaction === 'chunk');
    expect(JSON.parse(readFileSync(join(data, 'providers.json'), 'utf8')).compaction).toBe('chunk');
    const beforeSummary = new Set(s.hostMsgs('V'));
    s.view('V', { type: 'providerAction', action: { kind: 'compaction', strategy: 'summary' } });
    await s.hostMsg('V', 'providers', m => !beforeSummary.has(m) && m.view.compaction === undefined && !m.error);
    expect(JSON.parse(readFileSync(join(data, 'providers.json'), 'utf8'))).not.toHaveProperty('compaction');

    s.view('V', { type: 'providerAction', action: { kind: 'delete', id: 'deep-seek' } });
    // hostMsg scans everything received so far: the first, empty view must not stand in for the delete's reply
    await s.hostMsg('V', 'providers', m => m !== empty && m.view.providers.length === 0 && !m.error);
    expect(JSON.parse(readFileSync(join(data, 'secrets.json'), 'utf8'))).not.toHaveProperty(['acpira.provider.deep-seek']);
  });

  it('carries the families, and answers a model fetch with the stored key', async () => {
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
