import { mkdirSync, mkdtempSync, readFileSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import * as acp from '@agentclientprotocol/sdk';
import { describe, expect, it, vi } from 'vitest';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AcpSession } from '../src/host/acp/AcpSession';
import type { AgentProcess } from '../src/host/acp/AgentProcess';
import { AccountManager } from '../src/host/accounts/AccountManager';
import { AccountStore, FileVault, MemoryVault, accountSecretKey } from '../src/host/accounts/AccountStore';
import type { AccountCredential, AccountDraft, AccountProvider, LoginFlow } from '../src/host/accounts/types';
import { DevinAccountProvider, parseStatus, parseUserStatus, readCredentials, tomlOf } from '../src/host/accounts/devin';
import { SessionManager } from '../src/host/SessionManager';
import { TranscriptStore } from '../src/host/store/TranscriptStore';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));

// Fake provider: puts the key into authenticate's _meta.api_key like Devin does; import always returns one account.
// Quota: one weekly window whose remaining share drops on every read, so refreshes are observable
class FakeProvider implements AccountProvider {
  readonly agent = 'fake';
  importDraft: AccountDraft | undefined = { label: 'one@example.com', detail: 'Max', secret: 'good-key' };
  quotaReads = 0;
  async importLocal() { return this.importDraft; }
  async login(): Promise<LoginFlow> { throw new Error('not in test'); }
  async authenticate(proc: AgentProcess, cred: AccountCredential) {
    const req: acp.AuthenticateRequest = { methodId: 'fake.login', _meta: { api_key: cred.secret } };
    await proc.agent.request(acp.methods.agent.authenticate, req);
  }
  async quota(cred: AccountCredential) {
    if (cred.secret !== 'good-key') throw new Error('invalid api key');
    this.quotaReads++;
    return { windows: [{ id: 'weekly', remaining: 1 - this.quotaReads / 10, resetsAt: '2026-09-14T00:00:00.000Z' }], fetchedAt: new Date().toISOString() };
  }
}

function tmp() { return mkdtempSync(join(tmpdir(), 'acpira-acc-')); }

// The fake agent gates authentication on this marker; spawn also requires an existing cwd.
function authCwd() { return mkdtempSync(join(tmpdir(), 'acpira-needs-auth-')); }

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
});

describe('Devin terminal login flow', () => {
  it('waits for the toml in an isolated XDG dir → collects credentials (falls back to the key suffix as label if identity lookup fails) → cleans up the dir', async () => {
    const { existsSync } = await import('node:fs');
    const { mkdir, writeFile } = await import('node:fs/promises');
    const scratch = tmp();
    const p = new DevinAccountProvider(scratch, async () => '/nonexistent/devin');
    const flow = await p.login();
    expect(flow).toMatchObject({ command: '/nonexistent/devin', args: ['auth', 'login'] });
    expect(flow.env.ACP_BACKEND).toBeNull();
    const dir = flow.env.XDG_DATA_HOME as string;
    expect(dir.startsWith(scratch)).toBe(true);
    const ctrl = new AbortController();
    const collecting = flow.collect(ctrl.signal);
    // simulate the CLI login writing to disk
    await new Promise(r => setTimeout(r, 300));
    await mkdir(join(dir, 'devin'), { recursive: true });
    await writeFile(join(dir, 'devin', 'credentials.toml'), tomlOf({ secret: 'devin-key-wxyz', meta: { api_server_url: 'https://s' } }));
    const draft = await collecting;
    expect(draft).toEqual({ secret: 'devin-key-wxyz', meta: { api_server_url: 'https://s' }, label: 'Devin …wxyz', detail: undefined });
    expect(existsSync(dir)).toBe(false);
    // abort: nothing written, collect returns undefined and the dir is cleaned up
    const flow2 = await p.login();
    const ctrl2 = new AbortController();
    setTimeout(() => ctrl2.abort(), 50);
    expect(await flow2.collect(ctrl2.signal)).toBeUndefined();
    expect(existsSync(flow2.env.XDG_DATA_HOME as string)).toBe(false);
  });
});

describe('Devin credentials file and auth status parsing', () => {
  it('toml write/read round-trip; status output yields the email label and the tier detail', async () => {
    const dir = tmp();
    const cred: AccountCredential = { secret: 'devin-abc', meta: { api_server_url: 'https://server.codeium.com', devin_webapp_host: 'app.devin.ai', devin_api_url: 'https://api.devin.ai' } };
    const { writeFile } = await import('node:fs/promises');
    await writeFile(join(dir, 'credentials.toml'), tomlOf(cred));
    expect(await readCredentials(join(dir, 'credentials.toml'))).toEqual(cred);
    expect(await readCredentials(join(dir, 'nope.toml'))).toBeUndefined();
    const out = 'Logged in (via Devin).\n\nUser:\n  Name:              Someone\n  Email:             someone@example.com\n\nAccount:\n  Tier:              Devin Max\n  Plan:              Max\n';
    expect(parseStatus(out)).toEqual({ label: 'someone@example.com', detail: 'Devin Max' });
    expect(parseStatus('Not logged in.')).toBeUndefined();
  });

  // Shape of GetUserStatus in JSON encoding as the seat-management service returned it for a Max seat (2026-09): whole-number percents,
  // int64 reset times as strings, hideDailyQuota on the plan
  it('GetUserStatus → windows the plan exposes: Max hides daily, Pro has both, credit plans have none', () => {
    const max = { userStatus: { planStatus: { planInfo: { planName: 'Max', billingStrategy: 'BILLING_STRATEGY_QUOTA', hideDailyQuota: true },
      dailyQuotaRemainingPercent: 100, weeklyQuotaRemainingPercent: 94, dailyQuotaResetAtUnix: '1788854400', weeklyQuotaResetAtUnix: '1789286400' } } };
    const q = parseUserStatus(max)!;
    expect(q.windows).toEqual([{ id: 'weekly', remaining: 0.94, resetsAt: '2026-09-13T08:00:00.000Z' }]);
    expect(Date.parse(q.fetchedAt)).not.toBeNaN();
    // Pro: both windows; the weekly one is exhausted, which proto3 JSON expresses by omitting the zero-valued percent (and reset time)
    const pro = { userStatus: { planStatus: { planInfo: { planName: 'Pro', billingStrategy: 'BILLING_STRATEGY_QUOTA' }, dailyQuotaRemainingPercent: 37, dailyQuotaResetAtUnix: '1788854400' } } };
    expect(parseUserStatus(pro)!.windows).toEqual([
      { id: 'daily', remaining: 0.37, resetsAt: '2026-09-08T08:00:00.000Z' },
      { id: 'weekly', remaining: 0, resetsAt: undefined },
    ]);
    // Not billed by quota and no percents at all → nothing to show; an explicit zero on such a plan still counts
    expect(parseUserStatus({ userStatus: { planStatus: { planInfo: { billingStrategy: 'BILLING_STRATEGY_CREDITS' } } } })).toBeUndefined();
    expect(parseUserStatus({ userStatus: { planStatus: { planInfo: {}, weeklyQuotaRemainingPercent: 0 } } })!.windows).toEqual([{ id: 'weekly', remaining: 0, resetsAt: undefined }]);
    expect(parseUserStatus({})).toBeUndefined();
    expect(parseUserStatus(undefined)).toBeUndefined();
  });
});

function setup(loadOnly = false) {
  const dir = tmp();
  const cwd = join(dir, 'needs-auth');
  mkdirSync(cwd);
  const provider = new FakeProvider();
  const vault = new MemoryVault();
  const store = new AccountStore(join(dir, 'accounts.json'), vault);
  const toasts: string[] = [];
  const accounts = new AccountManager({ store, providers: [provider], log: () => {}, runInTerminal: () => {}, toast: (_l, t) => toasts.push(t) });
  const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], env: { FAKE_SESSION_DIR: dir, ...(loadOnly ? { FAKE_LOAD_ONLY: '1' } : {}) } } });
  const m = new SessionManager({
    registry, store: new TranscriptStore(join(dir, 'sessions')), log: () => {}, cwd: () => cwd, defaultAgent: () => 'fake',
    runInTerminal: () => {}, toast: (_l, t) => toasts.push(t), accounts,
  });
  return { m, accounts, store, provider, toasts, registry, dir, vault };
}

describe('account layer wired into sessions', () => {
  it('publishes import progress immediately and ignores repeated clicks until it finishes', async () => {
    const { m, provider } = setup();
    let complete!: (draft: AccountDraft | undefined) => void;
    const importing = vi.spyOn(provider, 'importLocal').mockImplementation(() => new Promise(resolve => { complete = resolve; }));
    const events: unknown[] = [];
    m.subscribe(ev => { if (ev.type === 'accountActions') events.push(ev.actions); });
    const first = m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    expect(events).toEqual([[{ agent: 'fake', via: 'import', status: 'pending' }]]);
    await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    expect(importing).toHaveBeenCalledTimes(1);
    complete(provider.importDraft);
    await first;
    expect(events.at(-1)).toEqual([{ agent: 'fake', via: 'import', status: 'success' }]);
    await m.dispose();
  });

  it('reports missing local login and import failures, and allows another attempt', async () => {
    const { m, provider } = setup();
    const importing = vi.spyOn(provider, 'importLocal');
    importing.mockResolvedValueOnce(undefined);
    await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    expect(m.accountActions()).toEqual([{ agent: 'fake', via: 'import', status: 'missing' }]);
    importing.mockRejectedValueOnce(new Error('keychain unavailable'));
    await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    expect(m.accountActions()).toEqual([{ agent: 'fake', via: 'import', status: 'error', error: 'keychain unavailable' }]);
    await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    expect(m.accountActions()).toEqual([{ agent: 'fake', via: 'import', status: 'success' }]);
    await m.dispose();
  });

  it('reload keeps the imported account binding and authenticates the replacement process', async () => {
    const { m, dir, vault, registry } = setup();
    await m.init();
    await m.newSession();
    await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    const record = m.active()!;
    await m.dispose();
    // Empty Devin sessions can disappear when the process exits; the fake's gone cwd models that response.
    const transcripts = new TranscriptStore(join(dir, 'sessions'));
    const saved = (await transcripts.load(record.id))!;
    const cwd = join(dir, 'needs-auth-gone');
    mkdirSync(cwd);
    await transcripts.flush({ ...saved, cwd });
    const store = new AccountStore(join(dir, 'accounts.json'), vault);
    await store.load();
    const authenticate = vi.spyOn(FakeProvider.prototype, 'authenticate');
    const accounts = new AccountManager({ store, providers: [new FakeProvider()], log: () => {}, runInTerminal: () => {}, toast: () => {} });
    const restored = new SessionManager({ registry, store: transcripts, accounts, log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {} });
    try {
      await restored.init();
      await restored.ensureActive();
      expect(authenticate).toHaveBeenCalledTimes(1);
      expect(restored.active()).toMatchObject({ id: record.id, accountId: record.accountId, status: 'ready' });
    } finally {
      authenticate.mockRestore();
      await restored.dispose();
    }
  });

  it('Devin generic missing-credential stderr uses login guidance instead of a raw log line', async () => {
    const s = AcpSession.fresh('fake', authCwd(), {
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], env: { FAKE_AUTH_HINT: 'devin' } } }),
      log: () => {}, onChange: () => {}, blobs: { saveBlob: async () => ({ name: 'x', path: '/tmp/x' }), readBlob: async () => new Uint8Array() },
    });
    try {
      await s.start();
      expect(s.view()).toMatchObject({ status: 'auth_required', error: undefined });
    } finally { s.dispose(); }
  });

  it.each([false, true])('account switching preserves native history through resume/load (load only: %s)', async loadOnly => {
    const { m, accounts, store, dir } = setup(loadOnly);
    await m.init();
    expect(m.agents().find(a => a.id === 'fake')?.accounts).toBe(true);
    await m.newSession();
    expect(m.active()?.status).toBe('auth_required');
    const empty = m.activeId!;

    await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    const [one] = accounts.list();
    expect(one).toMatchObject({ agent: 'fake', label: 'one@example.com' });
    // the empty session stuck on login is rebound, not replaced
    expect(m.activeId).toBe(empty);
    expect(m.sessions().map(s => s.id)).toEqual([empty]);
    expect(m.active()).toMatchObject({ status: 'ready', accountId: one!.id });
    await m.handle({ type: 'send', text: 'hi' });
    await m.handle({ type: 'setConfig', configId: 'model', value: 'm2' });
    await m.handle({ type: 'setMode', id: 'plan' });
    const before = m.active()!;
    const transcripts = new TranscriptStore(join(dir, 'sessions'));
    // Flush through disposal below as well; the manager's debounced save must
    // retain the same native ID after every account change.
    await vi.waitFor(async () => expect((await transcripts.load(before.id))?.acpSessionId).toBeTruthy());
    const nativeId = (await transcripts.load(before.id))!.acpSessionId;
    const history = structuredClone(before.turns);

    // second account: same session, new credential; the default account switches too
    const two = await store.add('fake', { label: 'two@example.com', secret: 'good-key' });
    const first = m.activeId!;
    const turns = m.active()!.turns.length;
    await m.handle({ type: 'selectAccount', id: two.id });
    expect(m.activeId).toBe(first);
    expect(m.sessions()).toHaveLength(1);
    expect(m.active()).toMatchObject({ status: 'ready', accountId: two.id });
    expect(m.active()!.turns).toHaveLength(turns);
    expect(m.active()!.turns).toEqual(history);
    expect(m.active()!.controls.modeId).toBe('plan');
    expect(m.active()!.controls.options.find(o => o.id === 'model')?.value).toBe('m2');
    expect(accounts.defaultFor('fake')?.id).toBe(two.id);

    await m.handle({ type: 'send', text: 'inspect-native-history' });
    const reply = m.active()!.turns.at(-1);
    if (reply?.role !== 'agent' || reply.blocks[0]?.type !== 'text') throw new Error('Missing reply');
    expect(JSON.parse(reply.blocks[0].markdown).prompts).toEqual([
      [{ type: 'text', text: 'hi' }], [{ type: 'text', text: 'inspect-native-history' }],
    ]);
    await m.handle({ type: 'selectAccount', id: one!.id });
    expect(m.active()).toMatchObject({ id: first, status: 'ready', accountId: one!.id });
    await m.dispose();
    expect((await transcripts.load(first))?.acpSessionId).toBe(nativeId);
  }, 20_000);

  it('switching during a running turn leaves the session and default account unchanged', async () => {
    const { m, accounts, store } = setup();
    try {
      await m.init();
      await m.newSession();
      await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
      const one = m.active()!.accountId;
      const two = await store.add('fake', { label: 'two@example.com', secret: 'good-key' });
      const sending = m.handle({ type: 'send', text: 'slow' });
      await vi.waitFor(() => expect(m.active()?.running).toBe(true));
      await m.handle({ type: 'selectAccount', id: two.id });
      expect(m.active()?.accountId).toBe(one);
      expect(accounts.defaultFor('fake')?.id).toBe(one);
      await m.handle({ type: 'stop' });
      await sending;
    } finally { await m.dispose(); }
  });

  it('quota: fetched after the hand-off and again when a turn ends, served from memory when asked again soon after, dropped with the account; a bad key only logs', async () => {
    const { m, accounts, store, provider } = setup();
    const pushed: number[] = [];
    m.subscribe(ev => { if (ev.type === 'accounts') pushed.push(ev.accounts[0]?.quota?.windows[0]?.remaining ?? -1); });
    await m.init();
    await m.newSession();
    await m.handle({ type: 'addAccount', agent: 'fake', via: 'import' });
    // the session is ready before the vendor answers; the quota lands on the account list afterwards
    await vi.waitFor(() => expect(accounts.list()[0]?.quota?.windows).toEqual([{ id: 'weekly', remaining: 0.9, resetsAt: '2026-09-14T00:00:00.000Z' }]));
    expect(pushed).toContain(0.9);
    // the webview asking right away costs no request
    await m.handle({ type: 'refreshQuota', agent: 'fake' });
    expect(provider.quotaReads).toBe(1);
    // a finished turn forces a re-read
    await m.handle({ type: 'send', text: 'hi' });
    await vi.waitFor(() => expect(accounts.list()[0]?.quota?.windows[0]?.remaining).toBe(0.8));
    expect(m.active()?.accountId).toBe(accounts.list()[0]!.id);
    // an account whose key the vendor rejects has no quota, and nothing is thrown
    const bad = await store.add('fake', { label: 'bad@example.com', secret: 'bad-key' });
    await accounts.refreshQuota(bad.id, true);
    expect(accounts.get(bad.id)?.quota).toBeUndefined();
    const good = accounts.list()[0]!.id;
    await m.handle({ type: 'removeAccount', id: good });
    expect(accounts.list().some(a => a.quota)).toBe(false);
    await m.dispose();
  }, 20_000);

  it('quota: fetched as soon as an account is stored, even without a session hand-off', async () => {
    const { accounts } = setup();
    const a = await accounts.add('fake');
    expect(a?.quota).toBeUndefined();
    await vi.waitFor(() => expect(accounts.get(a!.id)?.quota?.windows[0]?.remaining).toBe(0.9));
  });

  it('"+" auto-decides: import when the local login was never imported; fall back to terminal login when already imported or not logged in locally', async () => {
    const { accounts, provider, toasts } = setup();
    // first time: the local login is not in the list yet → import directly
    expect(await accounts.add('fake')).toMatchObject({ label: 'one@example.com' });
    expect(toasts.at(-1)).toContain('one@example.com');
    // second time: already imported → goes to login (the fake provider's login throws, proving that path was taken)
    await expect(accounts.add('fake')).rejects.toThrow('not in test');
    // not logged in locally → also goes to login
    provider.importDraft = undefined;
    await expect(accounts.add('fake')).rejects.toThrow('not in test');
    expect(accounts.list()).toHaveLength(1);
  });

  it('invalid key → auth_required with a reason; removing the account removes its credential', async () => {
    const { m, accounts, store } = setup();
    await m.init();
    const bad = await store.add('fake', { label: 'bad@example.com', secret: 'bad-key' });
    await m.newSession('fake', bad.id);
    expect(m.active()?.status).toBe('auth_required');
    await m.handle({ type: 'removeAccount', id: bad.id });
    expect(accounts.list()).toEqual([]);
    expect(await store.credential(bad.id)).toBeUndefined();
    await m.dispose();
  }, 20_000);

  it('AcpSession using the hooks directly: a missing credential raises AccountAuthError, enters auth_required and keeps the reason', async () => {
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } });
    const s = AcpSession.fresh('fake', authCwd(), {
      registry, log: () => {}, onChange: () => {}, blobs: { saveBlob: async () => ({ name: 'x', path: '/tmp/x' }), readBlob: async () => new Uint8Array() },
      accounts: { spawnEnv: async () => undefined, authenticate: async () => { throw new Error('账号 x 的凭据不在了'); } },
    }, 'missing');
    await s.start();
    expect(s.view()).toMatchObject({ status: 'auth_required', error: '账号 x 的凭据不在了', accountId: 'missing' });
    s.dispose();
  });

  // Regression: the account hand-off can fail transiently (Devin timed out fetching team settings right after a window reload)
  // while the process stays alive; retry must re-hand the credential instead of bouncing session/load off -32000 forever
  it('transient hand-off failure → auth_required with the reason; retry re-authenticates on the same process and becomes ready', async () => {
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } });
    let calls = 0;
    const s = AcpSession.fresh('fake', authCwd(), {
      registry, log: () => {}, onChange: () => {}, blobs: { saveBlob: async () => ({ name: 'x', path: '/tmp/x' }), readBlob: async () => new Uint8Array() },
      accounts: {
        spawnEnv: async () => undefined,
        authenticate: async (_agent, _id, proc) => {
          calls++;
          if (calls === 1) throw new Error('Authentication failed: Failed to fetch team settings: fetch timed out after 10000ms');
          await proc.agent.request(acp.methods.agent.authenticate, { methodId: 'fake.login', _meta: { api_key: 'good-key' } });
        },
      },
    }, 'acc1');
    await s.start();
    expect(s.view()).toMatchObject({ status: 'auth_required', error: 'Authentication failed: Failed to fetch team settings: fetch timed out after 10000ms' });
    expect(s.alive).toBe(true);
    await s.retry();
    expect(calls).toBe(2);
    expect(s.view()).toMatchObject({ status: 'ready', error: undefined });
    s.dispose();
  });
});
