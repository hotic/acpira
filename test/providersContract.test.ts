import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
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
});
