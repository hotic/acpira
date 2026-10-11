import { existsSync, mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, expect, it } from 'vitest';
import { Host } from '../scripts/lib/host';
import { newModel, type Provider } from '../src/shared/providers';

const hosts: Host[] = [];
const dirs: string[] = [];
const temp = () => { const p = mkdtempSync(join(tmpdir(), 'acpira-review-')); dirs.push(p); return p; };
const delay = (ms: number) => new Promise(r => setTimeout(r, ms));
const provider = (baseUrl: string): Provider => ({ id: 'mock', name: 'Mock', preset: 'custom', format: 'openai-chat', baseUrl, fullUrl: false, enabled: true, models: [newModel('m1')] });

afterEach(async () => {
  for (const h of hosts.splice(0)) await h.dispose();
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

it('finishes the native session probe when the host replies', async () => {
  const h = await Host.start({ cwd: temp(), home: temp(), defaultAgent: 'acpira', settings: { proxy: 'off' } });
  hosts.push(h);
  const v = await h.view();
  let replies = 0;
  h.shell.onMessage(m => { if (m.type === 'hostMessage' && m.message.type === 'nativeSessions') replies++; });
  // Keep the production predicate, shortening only its two-minute timeout.
  const upcoming = h.shell.upcoming.bind(h.shell);
  h.shell.upcoming = (pred, ms) => upcoming(pred, ms === 120000 ? 1500 : ms);
  const result = await v.listNativeSessions('acpira').then(() => 'resolved', () => 'timed out');
  expect({ replies, result }).toEqual({ replies: 1, result: 'resolved' });
});

it('refreshes the active session model picker after saving a provider', async () => {
  const home = temp();
  const p = provider('http://127.0.0.1:9/v1');
  writeFileSync(join(home, 'providers.json'), JSON.stringify({ version: 1, providers: [p] }));
  const h = await Host.start({ cwd: temp(), home, defaultAgent: 'acpira', settings: { proxy: 'off' } });
  hosts.push(h);
  const v = await h.view();
  v.post({ type: 'newSession', agent: 'acpira' });
  await v.until(() => !!v.active()?.controls.options.find(c => c.id === 'model'), 10000, 'initial model picker');
  const saved = h.shell.hostMsg(v.id, 'providers', m => m.view.providers[0]?.models.length === 2);
  v.post({ type: 'providerAction', action: { kind: 'save', provider: { ...p, models: [newModel('m1'), newModel('m2')] } } });
  await saved;
  const refreshed = h.shell.hostMsg(v.id, 'controls', m => m.agent === 'acpira');
  v.post({ type: 'controls', agent: 'acpira', fresh: true });
  const reply = await refreshed;
  expect(reply.controls.find(c => c.id === 'model')?.options.map(o => o.id)).toEqual(['mock/m1', 'mock/m2']);
  // The agent notices the saved file within a second and updates the open session's options
  const picker = () => v.active()?.controls.options.find(c => c.id === 'model')?.options.map(o => o.id);
  await v.until(() => picker()?.length === 2, 5000, 'the session model picker').catch(() => {});
  expect(picker()).toEqual(['mock/m1', 'mock/m2']);
});

it('stops built-in agent background jobs when the sidecar shuts down', async () => {
  const cwd = temp();
  const home = temp();
  let requests = 0;
  let jobPid: number | undefined;
  const server = createServer((req, res) => {
    req.resume();
    req.on('end', () => {
      const first = requests++ === 0;
      const delta = first ? { tool_calls: [{ index: 0, id: 'c1', type: 'function', function: { name: 'bash', arguments: JSON.stringify({ command: 'echo $$ > job.pid; while [ ! -e shutdown-complete ]; do sleep 0.1; done; echo still-running > after-close; sleep 60', background: true }) } }] } : { content: 'done' };
      const chunks = [
        { choices: [{ index: 0, delta, finish_reason: null }] },
        { choices: [{ index: 0, delta: {}, finish_reason: first ? 'tool_calls' : 'stop' }] },
      ];
      res.writeHead(200, { 'Content-Type': 'text/event-stream' });
      res.end(chunks.map(c => `data: ${JSON.stringify(c)}\n\n`).join('') + 'data: [DONE]\n\n');
    });
  });
  await new Promise<void>(r => server.listen(0, '127.0.0.1', r));
  try {
    writeFileSync(join(home, 'providers.json'), JSON.stringify({ version: 1, providers: [provider(`http://127.0.0.1:${(server.address() as AddressInfo).port}/v1`)] }));
    const h = await Host.start({ cwd, home, defaultAgent: 'acpira', settings: { proxy: 'off' } });
    hosts.push(h);
    const v = await h.view();
    v.post({ type: 'newSession', agent: 'acpira' });
    await v.until(() => !!v.active()?.controls.options.find(c => c.id === 'approval'), 10000, 'approval control');
    await v.handle({ type: 'setConfig', configId: 'approval', value: 'full' });
    await v.handle({ type: 'send', text: 'local scripted fixture only' }, 10000);
    jobPid = Number(readFileSync(join(cwd, 'job.pid'), 'utf8').trim());
    await h.dispose();
    writeFileSync(join(cwd, 'shutdown-complete'), 'sidecar has exited');
    await delay(500);
    let alive = true;
    try { process.kill(jobPid, 0); } catch { alive = false; }
    expect({ requests, alive, wroteAfterShutdown: existsSync(join(cwd, 'after-close')) }).toEqual({ requests: 2, alive: false, wroteAfterShutdown: false });
  } finally {
    if (jobPid) { try { process.kill(-jobPid, 'SIGKILL'); } catch { /* The job already ended. */ } }
    await new Promise<void>(r => server.close(() => r()));
  }
});
