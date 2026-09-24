import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, utimesSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it, vi } from 'vitest';
import type { HiddenMap } from '../src/shared/settings';
import type { ConfigControl } from '../src/shared/transcript';
import { AgentRegistry } from '../src/host/acp/AgentRegistry';
import { AcpSession } from '../src/host/acp/AcpSession';
import { probeAgentControls } from '../src/host/acp/probeControls';
import { SessionManager } from '../src/host/SessionManager';
import { TranscriptStore, summarize } from '../src/host/store/TranscriptStore';
import { LocalAccounts } from '../src/host/accounts/local';

const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));

function manager() {
  const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
  const toasts: string[] = [];
  const m = new SessionManager({
    registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
    store: new TranscriptStore(dir),
    log: () => {},
    cwd: () => '/tmp',
    defaultAgent: () => 'fake',
    runInTerminal: () => {},
    toast: (_l, t) => toasts.push(t),
  });
  return { m, dir, toasts };
}

describe('SessionManager', () => {
  it('persists a stopped turn before shutdown releases the session', async () => {
    const { m, dir } = manager();
    try {
      await m.init();
      await m.newSession();
      const id = m.activeId!;
      const prompt = m.handle({ type: 'send', text: 'slow' });
      await vi.waitFor(() => expect(m.active()?.turns.at(-1)).toMatchObject({ role: 'agent', blocks: expect.arrayContaining([expect.objectContaining({ streaming: true })]) }), { timeout: 5000 });
      await m.dispose();
      await prompt;
      const store = new TranscriptStore(dir);
      const record = await store.load(id);
      expect(record?.turns.at(-1)).toMatchObject({ stop: 'cancelled', endedAt: expect.any(Number), blocks: expect.arrayContaining([expect.objectContaining({ streaming: false })]) });
      await store.dispose();
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  });

  it('orders the agent list and starts new sessions on the first enabled agent once the default is switched off', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-order-'));
    const prefs = { order: ['fake2'], disabled: [] as string[] };
    const m = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] }, fake2: { name: 'Fake 2', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir), agentPrefs: () => prefs,
      log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    const pushed: string[][] = [];
    m.subscribe(ev => { if (ev.type === 'agents') pushed.push(ev.agents.filter(a => a.disabled).map(a => a.id)); });
    try {
      await m.init();
      expect(m.agents()[0]?.id).toBe('fake2');
      prefs.disabled = ['fake'];
      m.emitAgents();
      expect(pushed.at(-1)).toEqual(['fake']);
      await m.newSession();
      expect(m.active()?.agent).toBe('fake2');
      // An explicit pick (a disabled agent's own Retry / new session) is still honoured
      await m.newSession('fake');
      expect(m.active()?.agent).toBe('fake');
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  });

  it('publishes local quota updates and refreshes after a turn without binding an imported account', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-local-manager-'));
    const local = new LocalAccounts({ home: dir, env: () => ({ KIMI_CODE_API_KEY: 'test-code-key' }),
      fetch: vi.fn<typeof fetch>(async () => new Response(JSON.stringify({ usage: { limit: 100, used: 25 } }))),
    });
    const refresh = vi.spyOn(local, 'refresh');
    const m = new SessionManager({
      registry: new AgentRegistry({ kimi: { name: 'Kimi Code', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir), localAccounts: local,
      log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'kimi', runInTerminal: () => {}, toast: () => {},
    });
    const updates: unknown[] = [];
    m.subscribe(ev => { if (ev.type === 'agents') updates.push(ev.agents); });
    try {
      await m.init();
      await m.handle({ type: 'refreshQuota', agent: 'kimi' });
      expect(m.agents().find(a => a.id === 'kimi')).toMatchObject({ localAccount: { status: 'ready', quota: { windows: [{ remaining: 0.75 }] } } });
      expect(updates.length).toBeGreaterThan(0);
      expect(m.accounts()).toEqual([]);
      await m.newSession();
      expect(m.active()?.accountId).toBeUndefined();
      refresh.mockClear();
      await m.handle({ type: 'send', text: 'hi' });
      expect(refresh).toHaveBeenCalledWith('kimi', true);
    } finally {
      await m.dispose();
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it('delete is soft: leaves the list, switches the active session, file kept; restore brings it back; rename / pin land in the index', async () => {
    const { m, dir } = manager();
    await m.init();
    await m.newSession();
    const a = m.activeId!;
    await m.handle({ type: 'send', text: 'hi' });
    await m.newSession();
    const b = m.activeId!;
    expect(m.sessions().map(s => s.id)).toEqual([b, a]);

    await m.handle({ type: 'renameSession', id: a, title: '第一条' });
    await m.handle({ type: 'pinSession', id: a, pinned: true });
    expect(m.sessions()[0]).toMatchObject({ id: a, title: '第一条', pinned: true });

    // delete the current session b → active switches to a, b's file moved to the trash (out of the live directory, so no other window lists it)
    await m.handle({ type: 'deleteSession', id: b });
    expect(m.sessions().map(s => s.id)).toEqual([a]);
    expect(m.activeId).toBe(a);
    expect(m.active()?.title).toBe('第一条');
    expect(existsSync(join(dir, `${b}.json`))).toBe(false);
    expect(existsSync(join(dir, 'trash', `${b}.json`))).toBe(true);

    await m.handle({ type: 'restoreSession', id: b });
    expect(m.sessions().map(s => s.id).sort()).toEqual([a, b].sort());
    expect(existsSync(join(dir, `${b}.json`))).toBe(true);

    // delete a again, then reopen the manager: only b left in the index
    await m.handle({ type: 'deleteSession', id: a });
    expect(m.activeId).toBe(b);
    await m.dispose();
    expect(existsSync(join(dir, `${a}.json`))).toBe(false);
    expect(existsSync(join(dir, 'trash', `${a}.json`))).toBe(false);
    const m2 = new SessionManager({
      registry: new AgentRegistry(), store: new TranscriptStore(dir), log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    await m2.init();
    expect(m2.sessions().map(s => s.id)).toEqual([b]);
  }, 20_000);

  // Several webviews (sidebar + editor tabs) each hold their own active session over the shared list; only the viewers showing a session get its updates
  it('viewers: independent active sessions, session events only to the viewers showing it, deletion / empty-drop respect the other viewers', async () => {
    const { m } = manager();
    await m.init();
    const a = m.attach();
    const b = m.attach();
    const seenA: string[] = [];
    const seenB: string[] = [];
    a.subscribe(ev => { if (ev.type === 'session') seenA.push(ev.session.id); });
    b.subscribe(ev => { if (ev.type === 'session') seenB.push(ev.session.id); });

    // a fresh viewer opens a fresh session; a second fresh viewer gets its own, not a's
    await a.ensureActive();
    await b.ensureActive();
    const sa = a.activeId!;
    const sb = b.activeId!;
    expect(sa).not.toBe(sb);
    expect(m.sessions().map(s => s.id).sort()).toEqual([sa, sb].sort());
    // both viewers can look at the same session; b moving on to a new one leaves a where it was
    await a.handle({ type: 'send', text: 'hi' });
    await b.selectSession(sa);
    expect(b.active()?.turns.length).toBe(2);
    await b.newSession();
    expect(m.sessions().map(s => s.id)).toContain(sa);
    expect(a.activeId).toBe(sa);
    expect(b.activeId).not.toBe(sa);

    // updates route by active session: a's turn reached a and (while b showed sa) b, but b's fresh session never reached a
    seenA.length = 0; seenB.length = 0;
    await a.handle({ type: 'send', text: 'again' });
    expect(seenA).toContain(sa);
    expect(seenB).not.toContain(sa);

    // deleting a's session moves only a; b stays where it was
    const sb2 = b.activeId!;
    await b.handle({ type: 'deleteSession', id: sa });
    expect(b.activeId).toBe(sb2);
    expect(a.activeId).toBeDefined();
    expect(a.activeId).not.toBe(sa);

    // a disposed viewer no longer hears anything
    seenB.length = 0;
    b.dispose();
    await a.handle({ type: 'send', text: 'quiet' });
    expect(seenB).toEqual([]);
    await m.dispose();
  }, 30_000);

  it('after probing binaries, agents() carries available: the fake agent is present, an uninstalled one is not', async () => {
    const { m } = manager();
    expect(m.agents().find(a => a.id === 'fake')?.available).toBeUndefined();
    await m.init();
    expect(m.agents().find(a => a.id === 'fake')?.available).toBe(true);
    await m.dispose();
    const m2 = new SessionManager({
      registry: new AgentRegistry({ ghost: { name: 'Ghost', command: '/nonexistent/ghost-cli' } }), store: new TranscriptStore(mkdtempSync(join(tmpdir(), 'acpira-mgr-'))),
      log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'ghost', runInTerminal: () => {}, toast: () => {},
    });
    await m2.init();
    expect(m2.agents().find(a => a.id === 'ghost')?.available).toBe(false);
    await m2.dispose();
  });

  // A CLI installed while the window is open: the registry's notification re-pushes agents to every viewer, and the poll keeps looking while
  // something is missing (fake timers drive it); the install action runs the vendor line through the shell in a host terminal
  it('a CLI appearing after init reaches the viewers as an agents event, via the poll or a direct lookup; installAgent runs the vendor line in a terminal', async () => {
    vi.useFakeTimers();
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const bin = join(dir, 'ghost-cli');
    const terminal: { command: string; args: string[] }[] = [];
    const m = new SessionManager({
      // `never` stays missing so the poll keeps its timer armed regardless of which built-in CLIs this machine has.
      // Warm that stub, not ghost: init's background spawn would otherwise race the test's resolveBinary on the same id.
      registry: new AgentRegistry({ ghost: { name: 'Ghost', command: bin, install: { command: 'curl -fsSL https://example.com/i.sh | bash' } }, never: { name: 'Never', command: '/nonexistent/never-cli' } }),
      store: new TranscriptStore(mkdtempSync(join(tmpdir(), 'acpira-mgr-'))),
      log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'never', runInTerminal: (_t, command, args) => terminal.push({ command, args }), toast: () => {},
    });
    try {
      await m.init();
      const v = m.attach();
      const seen: (boolean | undefined)[] = [];
      const last = () => seen.at(-1);
      v.subscribe(ev => { if (ev.type === 'agents') seen.push(ev.agents.find(a => a.id === 'ghost')?.available); });
      expect(m.agents().find(a => a.id === 'ghost')).toMatchObject({ available: false, install: { command: 'curl -fsSL https://example.com/i.sh | bash' } });

      // The settings page's rescan path: a direct lookup finds the new binary and the list is pushed at once
      writeFileSync(bin, '#!/bin/sh\nexit 0\n'); chmodSync(bin, 0o755);
      expect(await m.registry.resolveBinary('ghost')).toBe(bin);
      expect(last()).toBe(true);

      // Removed again: the next poll tick notices (the tick starts real fs lookups, so waitFor lets them land)
      rmSync(bin);
      await vi.advanceTimersByTimeAsync(10_000);
      await vi.waitFor(() => expect(last()).toBe(false));
      // …and the poll keeps running while it is missing, so a reinstall shows up on its own
      writeFileSync(bin, '#!/bin/sh\nexit 0\n'); chmodSync(bin, 0o755);
      await vi.advanceTimersByTimeAsync(10_000);
      await vi.waitFor(() => expect(last()).toBe(true));

      await v.handle({ type: 'installAgent', agent: 'ghost' });
      expect(terminal).toEqual([{ command: process.platform === 'win32' ? 'powershell' : 'bash', args: [process.platform === 'win32' ? '-Command' : '-c', 'curl -fsSL https://example.com/i.sh | bash'] }]);
    } finally {
      await m.dispose();
      vi.useRealTimers();
    }
  });

  it('hidden options: read from the host as a plain copy and re-pushed on emitHidden', () => {
    const hidden: HiddenMap = { devin: { model: ['GLM-5.2'] } };
    const events: HiddenMap[] = [];
    const m = new SessionManager({
      registry: new AgentRegistry(), store: new TranscriptStore(mkdtempSync(join(tmpdir(), 'acpira-mgr-'))), log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
      hidden: () => hidden,
    });
    m.subscribe(ev => { if (ev.type === 'hidden') events.push(ev.hidden); });
    expect(m.hidden()).toEqual(hidden);
    expect(m.hidden()).not.toBe(hidden);
    m.emitHidden();
    expect(events).toEqual([hidden]);
  });

  it('knownControls: the configOptions of the agent’s latest session, also after the process is gone; nothing for an agent never opened', async () => {
    const { m, dir } = manager();
    await m.init();
    expect(await m.knownControls('fake')).toEqual([]);
    await m.newSession();
    const live = await m.knownControls('fake');
    expect(live.map(c => c.id)).toEqual(['model', 'effort']);
    await m.dispose();
    const m2 = new SessionManager({
      registry: new AgentRegistry(), store: new TranscriptStore(dir), log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    await m2.init();
    expect((await m2.knownControls('fake')).map(c => c.id)).toEqual(['model', 'effort']);
    expect(await m2.knownControls('ghost')).toEqual([]);
  }, 20_000);

  // The refresh button: a throwaway spawn runs initialize + session/new so a CLI config change (FAKE_MODELS here) shows up without a
  // real session; knownControls prefers the probed list until the next real session reads its own
  it('probeControls: a throwaway spawn reads the CLI’s current configOptions; knownControls prefers it until the next session starts', async () => {
    const { m, dir } = manager();
    const model = (cs: ConfigControl[] | undefined) => cs?.find(c => c.id === 'model')?.options.map(o => o.id);
    try {
      await m.init();
      await m.newSession();
      expect(model(await m.knownControls('fake'))).toEqual(['m1', 'm2']);

      process.env.FAKE_MODELS = 'm3';
      const probed = await m.probeControls('fake');
      expect(model(probed)).toEqual(['m1', 'm2', 'm3']);
      expect(model(await m.knownControls('fake'))).toEqual(['m1', 'm2', 'm3']);
      // The probe's initialize fills the facts card while no session of this agent is live… but this one is live; still resolves
      expect(m.runtimeInfo('fake')).toMatchObject({ name: 'fake' });

      // Fill the current session so newSession cannot reuse it (keepEmpty), then a real session reads m3 itself and retires the probe
      await m.handle({ type: 'send', text: 'hi' });
      await m.newSession();
      expect(model(m.active()!.controls.options)).toEqual(['m1', 'm2', 'm3']);
    } finally { delete process.env.FAKE_MODELS; await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 20_000);

  it('probeControls: an agent without a binary answers empty instead of throwing', async () => {
    const m = new SessionManager({
      registry: new AgentRegistry({ ghost: { name: 'Ghost', command: '/nonexistent/ghost-cli' } }),
      store: new TranscriptStore(mkdtempSync(join(tmpdir(), 'acpira-mgr-'))),
      log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'ghost', runInTerminal: () => {}, toast: () => {},
    });
    try {
      await m.init();
      expect(await m.probeControls('ghost')).toEqual([]);
    } finally { await m.dispose(); }
  });

  // The agent page's status line: each launch stage records its own outcome — probe (spawn / handshake / session/new auth)
  // and real sessions (ready / auth_required). Newest wins, a failed probe keeps the last known controls
  it('health: a successful probe is ready, a real session start overwrites it with source session', async () => {
    const { m, dir } = manager();
    try {
      await m.init();
      await m.probeControls('fake');
      expect(m.agentHealth('fake')).toMatchObject({ stage: 'ready', source: 'probe' });
      await m.newSession();
      await vi.waitFor(() => expect(m.active()?.status).toBe('ready'));
      expect(m.agentHealth('fake')).toMatchObject({ stage: 'ready', source: 'session' });
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  });

  it('health: session/new asking for sign-in is auth_required, an initialize error is handshake_failed', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-health-'));
    const authDir = join(dir, 'needs-auth');
    mkdirSync(authDir, { recursive: true });
    let cwd = authDir;
    const m = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir),
      log: () => {}, cwd: () => cwd, defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    try {
      await m.init();
      await m.probeControls('fake');
      expect(m.agentHealth('fake')).toMatchObject({ stage: 'auth_required', source: 'probe' });
      process.env.FAKE_INIT_FAIL = '1';
      cwd = dir;
      await m.probeControls('fake');
      expect(m.agentHealth('fake')).toMatchObject({ stage: 'handshake_failed', source: 'probe' });
    } finally { delete process.env.FAKE_INIT_FAIL; await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  });

  it('health: a binary that cannot exec is spawn_failed, and a failed probe keeps the last known controls', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-health-'));
    // A path that resolves (execute bit, regular file) but cannot exec — no interpreter behind the shebang
    const bad = join(dir, 'bad-cli');
    writeFileSync(bad, '#!/nonexistent/definitely-missing-interp\n');
    chmodSync(bad, 0o755);
    const m = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] }, broken: { name: 'Broken', command: bad } }),
      store: new TranscriptStore(dir),
      log: () => {}, cwd: () => dir, defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    try {
      await m.init();
      const before = await m.probeControls('fake');
      expect(before.length).toBeGreaterThan(0);
      // Directly against the probe and through the manager both land on spawn_failed
      const def = m.registry.get('broken');
      await expect(probeAgentControls({ def, binary: bad, cwd: dir, log: () => {} }))
        .rejects.toMatchObject({ name: 'ProbeFailure', stage: 'spawn_failed' });
      await m.probeControls('broken');
      expect(m.agentHealth('broken')).toMatchObject({ stage: 'spawn_failed', source: 'probe' });
      // A failed probe falls back to the last known controls instead of emptying the list
      process.env.FAKE_INIT_FAIL = '1';
      try {
        await m.probeControls('fake');
        expect(m.agentHealth('fake')).toMatchObject({ stage: 'handshake_failed', source: 'probe' });
        expect(await m.knownControls('fake')).toEqual(before);
      } finally { delete process.env.FAKE_INIT_FAIL; }
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  });

  it('terminal auth method runs the agent binary in a terminal and never reaches authenticate', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-term-auth-'));
    const authLog = join(dir, 'auth.log');
    const terminals: { command: string; args: string[]; env?: Record<string, string | null> }[] = [];
    const m = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], env: { FAKE_TERMINAL_AUTH: authLog, FAKE_FLAG: 'agent', FAKE_OTHER: 'agent' } } }),
      store: new TranscriptStore(dir),
      log: () => {}, cwd: () => dir, defaultAgent: () => 'fake',
      runInTerminal: (_title, command, args, env) => terminals.push({ command, args, env }),
      toast: () => {},
    });
    try {
      await m.init();
      await m.newSession();
      await vi.waitFor(() => expect(m.active()?.status).toBe('auth_required'));
      // The terminal method only exists because the host advertised auth.terminal at initialize
      expect(m.active()!.authMethods).toContainEqual(expect.objectContaining({
        id: 'term-login', terminal: { args: ['--login'], env: { FAKE_LOGIN: '1', FAKE_FLAG: 'method' } },
      }));
      await m.handle({ type: 'login', methodId: 'term-login' });
      // The resolved binary + the agent's own args + the method's args; env is the agent's launch env with the method's on top
      expect(terminals).toEqual([{ command: TSX, args: [FAKE, '--login'], env: { FAKE_TERMINAL_AUTH: authLog, FAKE_OTHER: 'agent', FAKE_FLAG: 'method', FAKE_LOGIN: '1' } }]);
      // authenticate never went to the agent for the terminal method
      await new Promise(r => setTimeout(r, 150));
      expect(existsSync(authLog)).toBe(false);
      expect(m.agentHealth('fake')).toMatchObject({ stage: 'auth_required', source: 'session' });
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  });

  // The fake agent's process starts every session on model m1 / effort high / mode agent; the option values and the mode picked last
  // come back on the next new session. Only the user's own picks count: a mode the agent switches by itself is not a choice
  it('last chosen config values and the manually picked mode are remembered per agent and replayed onto new sessions', async () => {
    const { m, dir } = manager();
    await m.init();
    await m.newSession();
    const controls = () => m.active()!.controls;
    expect(controls().options.map(c => c.value)).toEqual(['m1', 'high']);
    await m.handle({ type: 'setConfig', configId: 'model', value: 'm2' });
    await m.handle({ type: 'setConfig', configId: 'effort', value: 'low' });
    expect(m.lastSettings('fake')).toEqual({ config: { model: 'm2', effort: 'low' } });
    await m.handle({ type: 'setMode', id: 'plan' });
    expect(m.lastSettings('fake')).toEqual({ modeId: 'plan', config: { model: 'm2', effort: 'low' } });
    // a mode the agent switches by itself (Devin's "switch to bypass mode" permission answer, Kimi leaving plan after approval) is
    // that session's business — the memory keeps what the user picked, and a later config pick must not overwrite it either
    await m.handle({ type: 'send', text: 'mode:agent' });
    expect(controls().modeId).toBe('agent');
    await m.handle({ type: 'setConfig', configId: 'effort', value: 'high' });
    expect(m.lastSettings('fake')).toEqual({ modeId: 'plan', config: { model: 'm2', effort: 'high' } });
    // (a session must have said something, or the next newSession replaces it instead of adding one — done above)
    await m.newSession();
    expect(controls().options.map(c => c.value)).toEqual(['m2', 'high']);
    expect(controls().modeId).toBe('plan');
    // the applied choices are what the new session's first turn records
    await m.handle({ type: 'send', text: 'inspect-history' });
    const reply = m.active()!.turns.at(-1)!;
    const markdown = reply.role === 'agent' ? reply.blocks.map(b => b.type === 'text' ? b.markdown : '').join('') : '';
    expect(markdown).toContain('"model":"m2"');
    expect(markdown).toContain('"mode":"plan"');
    await m.dispose();

    // reload: the memory is on disk; a stale value (no longer in the agent's list) is passed over while the others still apply
    const store = new TranscriptStore(dir);
    const prefs = await store.loadPrefs();
    expect(prefs.lastSettings.fake).toEqual({ modeId: 'plan', config: { model: 'm2', effort: 'high' } });
    prefs.lastSettings.fake = { modeId: 'plan', config: { model: 'gone', effort: 'low' } };
    await store.savePrefs(prefs);
    const m2 = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }), store, log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    await m2.init();
    await m2.newSession();
    expect(m2.active()!.controls.options.map(c => c.value)).toEqual(['m1', 'low']);
    expect(m2.active()!.controls.modeId).toBe('plan');
    // a remembered mode the agent no longer offers is skipped like any other stale value
    prefs.lastSettings.fake = { modeId: 'gone', config: {} };
    await store.savePrefs(prefs);
    await m2.dispose();
    const m3 = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }), store, log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    await m3.init();
    await m3.newSession();
    expect(m3.active()!.controls.modeId).toBe('agent');
    await m3.dispose();
  }, 40_000);

  // The history editor switches models locally; what each model really offers (Devin: SWE-2 drops `speed`) is learned from live
  // switches, carried on the view and kept in prefs.json for the next host
  it('learns each model\'s parameters from live switches and carries them on the view', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const make = () => new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], env: { FAKE_SPEED: '1', FAKE_MODELS: 'x3' } } }), store: new TranscriptStore(dir),
      log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    const ids = (shape?: ConfigControl[]) => shape?.map(c => `${c.id}:${c.options.map(o => o.id).join('|')}`);
    const m = make();
    try {
      await m.init();
      await m.newSession();
      expect(ids(m.active()!.modelShapes?.m1)).toEqual(['effort:low|high', 'speed:standard|fast']);
      const before = m.active()!.modelShapes;
      await m.handle({ type: 'setConfig', configId: 'effort', value: 'low' });
      // A value change is not a new shape
      expect(m.active()!.modelShapes).toBe(before);
      await m.handle({ type: 'setConfig', configId: 'model', value: 'm2' });
      expect(ids(m.active()!.modelShapes?.m2)).toEqual(['effort:high']);
      // prefs writes are fire-and-forget
      await vi.waitFor(async () => expect(Object.keys((await new TranscriptStore(dir).loadPrefs()).modelShapes?.fake ?? {}).sort()).toEqual(['m1', 'm2']));
    } finally { await m.dispose(); }
    const next = make();
    const other = make();
    try {
      await next.init();
      await next.newSession();
      expect(Object.keys(next.active()!.modelShapes ?? {}).sort()).toEqual(['m1', 'm2']);
      // Another window that was already running picks up a shape learned here on focus / webview ready, with a newer rev
      await other.init();
      await other.newSession();
      const rev = other.active()!.rev!;
      await next.handle({ type: 'setConfig', configId: 'model', value: 'x3' });
      await vi.waitFor(async () => expect(Object.keys((await new TranscriptStore(dir).loadPrefs()).modelShapes?.fake ?? {})).toContain('x3'));
      await other.refreshIndex();
      expect(ids(other.active()!.modelShapes?.x3)).toEqual(['effort:high']);
      expect(other.active()!.rev).toBeGreaterThan(rev);
    } finally { await next.dispose(); await other.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 40_000);

  // Two viewers landing on the same stored session at once used to build one AcpSession each: two processes, the second
  // shadowing the first in the live map. The shared load hands both the same session
  it('concurrent selects of the same stored session share one load — a single process is spawned', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const logs: string[] = [];
    const mk = () => new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir), log: l => logs.push(l), cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    const a = mk();
    await a.init();
    await a.newSession();
    await a.handle({ type: 'send', text: 'hi' });
    const id = a.activeId!;
    await a.dispose();
    const b = mk();
    await b.init();
    const v1 = b.attach();
    const v2 = b.attach();
    logs.length = 0;
    await Promise.all([v1.selectSession(id), v2.selectSession(id)]);
    expect(logs.filter(l => l.includes('spawn') || l.includes('reuse warm')).length).toBe(1);
    expect(v1.activeId).toBe(id);
    expect(v2.activeId).toBe(id);
    expect(b.viewOf(id)?.turns.length).toBe(2);
    await b.dispose();
  }, 30_000);

  it('newSession on an empty starting/ready session keeps the process instead of respawning', async () => {
    const { m } = manager();
    await m.init();
    await m.newSession();
    const a = m.activeId!;
    await m.newSession();
    expect(m.activeId).toBe(a);
    expect(m.sessions().map(s => s.id)).toEqual([a]);
    await m.dispose();
  }, 20_000);

  // Two extension hosts (two windows, or VS Code + Cursor) share ~/.acpira/sessions. Each used to rewrite index.json from its own memory,
  // so whichever streamed last erased the other's new sessions from the list while their records stayed on disk
  it('two managers over one directory: sessions created in one show up in the other on refresh, neither erases the other’s, deletion is honored across', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const mk = () => new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir), log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    const a = mk();
    const b = mk();
    await a.init();
    await b.init();
    await a.newSession();
    await a.handle({ type: 'send', text: 'from A' });
    const sa = a.activeId!;
    await b.newSession();
    await b.handle({ type: 'send', text: 'from B' });
    const sb = b.activeId!;
    // Each keeps streaming (index writes on both sides) — nothing is lost; a refresh (window focus) is when the other's work shows up
    await a.handle({ type: 'send', text: 'A again' });
    await b.handle({ type: 'send', text: 'B again' });
    await b.refreshIndex();
    await a.refreshIndex();
    await b.refreshIndex();
    expect(a.sessions().map(s => s.id).sort()).toEqual([sa, sb].sort());
    expect(b.sessions().map(s => s.id).sort()).toEqual([sa, sb].sort());
    // A renames its own session: B sees the new title after its refresh, not its stale copy
    await a.handle({ type: 'renameSession', id: sa, title: 'A 的会话' });
    await a.refreshIndex();
    await b.refreshIndex();
    expect(b.sessions().find(s => s.id === sa)?.title).toBe('A 的会话');
    // A's window closes; B deletes A's session (live nowhere now): a host starting meanwhile does not list it, B's undo brings it back for everyone
    await a.dispose();
    const c = mk();
    await b.handle({ type: 'deleteSession', id: sa });
    await c.init();
    expect(c.sessions().map(s => s.id)).toEqual([sb]);
    await b.handle({ type: 'restoreSession', id: sa });
    await c.refreshIndex();
    expect(c.sessions().map(s => s.id).sort()).toEqual([sa, sb].sort());
    expect(c.sessions().find(s => s.id === sa)?.title).toBe('A 的会话');
    await b.dispose();
    await c.dispose();
  }, 30_000);

  // The same session open in two windows: a deletion in one used to be undone by the other's next debounced save, which recreated the record
  it('a session live in two managers: deleting it in one closes it in the other, whose stale save does not bring it back; undo returns it as a stored session', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const toasts: string[] = [];
    const mk = () => new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir), log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: (_l, t) => toasts.push(t),
    });
    const a = mk();
    const b = mk();
    await a.init();
    await b.init();
    await a.newSession();
    await a.handle({ type: 'send', text: 'shared' });
    const id = a.activeId!;
    // A's reconcile lands its debounced record; B picks the session up from the disk and opens it too
    await a.refreshIndex();
    await b.refreshIndex();
    await b.selectSession(id);
    expect(b.active()?.id).toBe(id);
    // B changes the record (a save is now debounced) right before A deletes it
    await b.handle({ type: 'renameSession', id, title: 'renamed in B' });
    await a.handle({ type: 'deleteSession', id });
    expect(existsSync(join(dir, `${id}.json`))).toBe(false);
    // B's next reconcile (the debounce, a window focus): the pending save is dropped, the session closed, the viewer moved on
    await b.refreshIndex();
    expect(existsSync(join(dir, `${id}.json`))).toBe(false);
    expect(existsSync(join(dir, 'trash', `${id}.json`))).toBe(true);
    expect(b.sessions().map(s => s.id)).not.toContain(id);
    expect(b.activeId).toBeDefined();
    expect(b.activeId).not.toBe(id);
    expect(toasts.some(t => t.includes('another window') || t.includes('另一个窗口'))).toBe(true);
    // Undo in A: the record is back in both lists as a stored session; B does not reattach to it by itself
    await a.handle({ type: 'restoreSession', id });
    await b.refreshIndex();
    expect(existsSync(join(dir, `${id}.json`))).toBe(true);
    expect(a.sessions().map(s => s.id)).toContain(id);
    expect(b.sessions().map(s => s.id)).toContain(id);
    expect(b.viewOf(id)).toBeUndefined();
    await a.dispose();
    await b.dispose();
  }, 30_000);

  // Sessions belong to the workspace folder they were opened in (their cwd). Under the workspace scope a viewer left without a session
  // falls onto one of this folder's, never another project's; moving re-homes a session into the current folder
  it('workspace scope: summaries carry cwd, "most recent" and the post-deletion pick stay inside the folder, moveSession re-homes stored and live sessions', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    // The agent process is spawned in the session's cwd, so the project folders have to exist
    const proj = (name: string) => { const p = join(dir, 'proj', name); mkdirSync(p, { recursive: true }); return p; };
    let cwd = proj('a');
    let scope: 'workspace' | 'all' = 'workspace';
    const toasts: string[] = [];
    const mk = () => new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(join(dir, 'sessions')), log: () => {}, cwd: () => cwd, defaultAgent: () => 'fake', runInTerminal: () => {}, toast: (_l, t) => toasts.push(t),
      scope: () => scope,
    });
    // Project A: one session with a turn
    let m = mk();
    await m.init();
    await m.newSession();
    const a1 = m.activeId!;
    await m.handle({ type: 'send', text: 'in a' });
    expect(m.sessions()[0]).toMatchObject({ id: a1, cwd: proj('a') });
    await m.dispose();

    // Project B: a sidebar starting on "most recent" must not land on A's session
    cwd = proj('b');
    m = mk();
    await m.init();
    expect(m.attach({ mostRecent: true }).activeId).toBeUndefined();
    scope = 'all';
    expect(m.attach({ mostRecent: true }).activeId).toBe(a1);
    scope = 'workspace';
    await m.newSession();
    const b1 = m.activeId!;
    await m.handle({ type: 'send', text: 'in b' });
    await m.newSession();
    const b2 = m.activeId!;
    await m.handle({ type: 'send', text: 'in b too' });
    // Deleting the active one falls back to B's other session, not the newer-looking A one
    await m.handle({ type: 'deleteSession', id: b2 });
    expect(m.activeId).toBe(b1);

    // Move A's stored session into B: its record and summary change folder
    await m.handle({ type: 'moveSession', id: a1 });
    expect(m.sessions().find(s => s.id === a1)?.cwd).toBe(proj('b'));
    expect((await new TranscriptStore(join(dir, 'sessions')).load(a1))?.cwd).toBe(proj('b'));

    // Move a live idle session: it is reopened in the new folder (a fresh process; "gone" makes the fake report the old id swept —
    // the transcript already ran, so it stays read-only with its history rather than silently continuing on an empty native context);
    // a running one refuses
    cwd = proj('c-gone');
    await m.handle({ type: 'moveSession', id: b1 });
    expect(m.activeId).toBe(b1);
    expect(m.active()?.cwd).toBe(proj('c-gone'));
    expect(m.active()?.status).toBe('readonly');
    expect(m.active()?.turns.length).toBe(2);
    // A session mid-turn refuses the move; use a fresh one, since the moved b1 is read-only now
    cwd = proj('d');
    await m.newSession();
    const d1 = m.activeId!;
    const sending = m.handle({ type: 'send', text: 'slow' });
    cwd = proj('e');
    await m.handle({ type: 'moveSession', id: d1 });
    expect(toasts.some(t => t.includes('moving') || t.includes('移动'))).toBe(true);
    await sending;
    expect(m.active()?.cwd).toBe(proj('d'));
    await m.dispose();
  }, 30_000);

  it('the first session takes the warm process started at init', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const logs: string[] = [];
    const m = new SessionManager({
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(dir),
      log: line => logs.push(line),
      cwd: () => '/tmp',
      defaultAgent: () => 'fake',
      runInTerminal: () => {},
      toast: () => {},
    });
    await m.init();
    await m.newSession();
    expect(logs.some(l => l.includes('reuse warm'))).toBe(true);
    expect(m.active()?.status).toBe('ready');
    await m.dispose();
  }, 20_000);

  it('deleteSession ignores path-like ids', async () => {
    const { m, dir } = manager();
    await m.init();
    await m.newSession();
    const id = m.activeId!;
    const marker = join(dirname(dir), `keep-${Date.now()}`);
    writeFileSync(marker, 'x');
    await m.handle({ type: 'deleteSession', id: '..' });
    await m.handle({ type: 'deleteSession', id: '/etc/passwd' });
    expect(m.activeId).toBe(id);
    expect(existsSync(marker)).toBe(true);
    rmSync(marker, { force: true });
    await m.dispose();
  }, 20_000);

  it('permission answers address the named session, not whichever the viewer is showing', async () => {
    const { m } = manager();
    await m.init();
    await m.newSession();
    const until = async (pred: () => boolean, ms = 8_000) => {
      const t0 = Date.now();
      while (!pred()) {
        if (Date.now() - t0 > ms) throw new Error('timeout');
        await new Promise(r => setTimeout(r, 20));
      }
    };
    const permOf = (id: string) => m.viewOf(id)?.turns.flatMap(t => t.role === 'agent' ? t.blocks : []).find(b => b.type === 'permission');
    // handle(send) waits for the whole turn, including the permission gate — do not await it
    const sendA = m.handle({ type: 'send', text: 'use tool' });
    await until(() => !!permOf(m.activeId!));
    const a = m.activeId!;
    const permA = permOf(a)!;
    await m.newSession();
    const sendB = m.handle({ type: 'send', text: 'use tool' });
    await until(() => m.activeId !== a && !!permOf(m.activeId!));
    const b = m.activeId!;
    expect(permOf(b)).toBeTruthy();
    await m.handle({ type: 'permission', sessionId: a, blockId: permA.id, optionId: 'allow' });
    await until(() => !permOf(a) && !!m.viewOf(a)?.turns.some(t => t.role === 'agent' && t.blocks.some(x => x.type === 'tool_call' && x.status === 'completed')));
    expect(permOf(b)).toBeTruthy();
    expect(m.activeId).toBe(b);
    await sendA;
    sendB.catch(() => {});
    await m.dispose();
  }, 30_000);

  it('forks from an agent turn: copied prefix, forkedFrom, blob re-homed, retained context on first prompt', async () => {
    const { m, dir } = manager();
    try {
      await m.init();
      await m.newSession();
      const src = m.activeId!;
      await m.handle({ type: 'send', text: 'hi', attachments: [{ kind: 'image', mimeType: 'image/png', data: 'aGVsbG8=', name: 'a.png' }] });
      await m.handle({ type: 'send', text: 'hi' });
      expect(m.active()?.turns).toHaveLength(4);

      await m.handle({ type: 'forkSession', sessionId: src, turnIndex: 1 });
      const forkId = m.activeId!;
      expect(forkId).not.toBe(src);
      expect(m.active()?.turns).toHaveLength(2);
      expect(m.active()?.title).toMatch(/^Fork: /);
      expect(m.viewOf(src)?.turns).toHaveLength(4);

      const store = new TranscriptStore(dir);
      const forkRecord = await store.load(forkId);
      expect(forkRecord).toMatchObject({ historyPending: true, forkedFrom: { sessionId: src, turnIndex: 1 } });
      const first = forkRecord?.turns[0];
      if (first?.role !== 'user') throw new Error('Missing copied user turn');
      const blob = first.attachments?.filter(a => a.kind !== 'file').map(a => a.blob).find(Boolean);
      expect(blob).toBeTruthy();
      expect(existsSync(join(dir, forkId, blob!))).toBe(true);
      await store.dispose();

      await m.handle({ type: 'send', text: 'again' });
      const reply = m.active()?.turns.at(-1);
      if (reply?.role !== 'agent') throw new Error('Missing reply');
      expect(reply.blocks.filter(b => b.type === 'text').map(b => b.markdown).join('')).toContain('resource:acpira://history/');
      await m.refreshIndex();
      expect((await new TranscriptStore(dir).load(forkId))?.historyPending).toBeUndefined();
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 30_000);

  it('rejects forking a non-agent turn or the running last turn', async () => {
    const { m, dir, toasts } = manager();
    try {
      await m.init();
      await m.newSession();
      const src = m.activeId!;
      await m.handle({ type: 'send', text: 'hi' });
      await m.handle({ type: 'forkSession', sessionId: src, turnIndex: 0 });
      expect(toasts).toContain('That reply is no longer there to fork from.');
      expect(m.activeId).toBe(src);

      const sending = m.handle({ type: 'send', text: 'slow' });
      await vi.waitFor(() => expect(m.active()?.running).toBe(true));
      await m.handle({ type: 'forkSession', sessionId: src, turnIndex: (m.active()?.turns.length ?? 0) - 1 });
      expect(toasts).toContain('Wait for this reply to finish before forking from it.');
      expect(m.activeId).toBe(src);
      await sending;
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 30_000);

  it('exports a session as markdown and json under the sibling exports dir', async () => {
    const { m, dir } = manager();
    try {
      await m.init();
      await m.newSession();
      const id = m.activeId!;
      await m.handle({ type: 'send', text: 'hi' });
      const title = m.active()!.title;
      const mdPath = await m.exportSession(id, 'markdown');
      // writeExport returns the realpath'd target (confined()), so resolve the expectation the same way
      expect(mdPath.startsWith(realpathSync(join(dirname(dir), 'exports')))).toBe(true);
      const md = readFileSync(mdPath, 'utf8');
      expect(md).toContain(`# ${title}`);
      expect(md).toContain('hello world');
      const jsonPath = await m.exportSession(id, 'json');
      expect(JSON.parse(readFileSync(jsonPath, 'utf8')).id).toBe(id);
      await expect(m.exportSession('missing-id', 'json')).rejects.toThrow();
      rmSync(mdPath, { force: true });
      rmSync(jsonPath, { force: true });
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 30_000);

  // "Import from <agent>": session/list on a throwaway process marks the ids this window already holds; importing a
  // foreign one opens a record whose transcript the session/load replay fills
  it('native sessions: listing marks imported ones, import replays the transcript, re-import selects the existing record', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const nativeDir = join(dir, 'native');
    mkdirSync(nativeDir);
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], env: { FAKE_SESSION_DIR: nativeDir } } });
    const store = new TranscriptStore(join(dir, 'sessions'));
    const m = new SessionManager({
      registry, store, log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    try {
      await m.init();
      // (a) a session this manager runs is listed with localId pointing back at its record
      await m.newSession();
      await m.handle({ type: 'send', text: 'hi' });
      const ownId = m.activeId!;
      const own = (await m.listNativeSessions('fake')).find(s => s.localId === ownId);
      // The fake stores the canonical cwd (codex-acp canonicalizes thread cwd the same way)
      expect(own).toMatchObject({ cwd: realpathSync('/tmp'), title: expect.stringMatching(/^Fake /) });
      expect(own?.updatedAt).toBeTruthy();

      // (b) a native session another process owns is listed bare; importing it replays the native history
      const other = AcpSession.fresh('fake', '/tmp', { registry, log: () => {}, onChange: () => {}, blobs: store });
      await other.start();
      await other.prompt('hi');
      const nativeId = other.toRecord().acpSessionId!;
      other.dispose();
      const target = (await m.listNativeSessions('fake')).find(s => s.sessionId === nativeId)!;
      expect(target.localId).toBeUndefined();

      const count = m.sessions().length;
      const v = m.attach();
      await m.importNativeSession(v, 'fake', target);
      expect(m.sessions()).toHaveLength(count + 1);
      const importedId = v.activeId!;
      const view = m.viewOf(importedId)!;
      expect(view.status).toBe('ready');
      const said = view.turns.flatMap(t => t.role === 'agent' ? t.blocks : []).map(b => b.type === 'text' ? b.markdown : '').join('');
      expect(said).toContain('NATIVE_REPLAY');
      expect(m.sessions().find(s => s.id === importedId)?.acpSessionId).toBe(nativeId);
      const record = (await store.load(importedId))!;
      expect(record.acpSessionId).toBe(nativeId);
      expect(record.importedFrom).toEqual({ sessionId: nativeId });
      expect(record.importPending).toBeUndefined();

      // (c) importing the same native id again lands the viewer on the existing record instead of making a second one
      await m.importNativeSession(v, 'fake', target);
      expect(m.sessions()).toHaveLength(count + 1);
      expect(v.activeId).toBe(importedId);
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 30_000);

  // codex-acp stores the canonicalized thread cwd and filters each page by string compare, so a project reached
  // through a symlink produced only empty pages with cursors; the host pages through and retries with the realpath
  it('native sessions: a symlinked project dir still lists its sessions (empty pages paged through, realpath fallback)', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const target = join(dir, 'proj-real');
    mkdirSync(target);
    const projLink = join(dir, 'proj-link');
    symlinkSync(target, projLink);
    const otherDir = join(dir, 'other');
    mkdirSync(otherDir);
    const nativeDir = join(dir, 'native');
    mkdirSync(nativeDir);
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], env: { FAKE_SESSION_DIR: nativeDir, FAKE_LIST_PAGE: '1' } } });
    const store = new TranscriptStore(join(dir, 'sessions'));
    const m = new SessionManager({
      registry, store, log: () => {}, cwd: () => projLink, defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    const mk = async (cwd: string) => {
      const s = AcpSession.fresh('fake', cwd, { registry, log: () => {}, onChange: () => {}, blobs: store });
      await s.start();
      const id = s.toRecord().acpSessionId!;
      s.dispose();
      return id;
    };
    try {
      await m.init();
      const targetId = await mk(projLink);
      const foreign = [await mk(otherDir), await mk(otherDir)];
      // Deterministic order: the foreign sessions lead (filtered-out pages), the project's trails on the last page
      const stamp = (id: string, t: number) => utimesSync(join(nativeDir, `${id}.json`), t / 1000, t / 1000);
      stamp(targetId, 1000);
      foreign.forEach((id, i) => stamp(id, 2000 + i * 1000));
      const listed = await m.listNativeSessions('fake');
      expect(listed.map(s => s.sessionId)).toEqual([targetId]);
      expect(listed[0]!.cwd).toBe(realpathSync(projLink));
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 30_000);

  it('listNativeSessions: an agent without the sessionCapabilities.list capability rejects as unsupported', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const m = new SessionManager({
      // no FAKE_SESSION_DIR: the fixture advertises no list capability
      registry: new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE] } }),
      store: new TranscriptStore(join(dir, 'sessions')), log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
    });
    try {
      await m.init();
      await expect(m.listNativeSessions('fake')).rejects.toThrow('does not list its sessions');
    } finally { await m.dispose(); rmSync(dir, { recursive: true, force: true }); }
  }, 20_000);

  // An index summary written before acpSessionId existed is patched once and the debounced index write persists it,
  // so the next listing (or another window) does not re-read the record
  it('native sessions: a stale index summary gains acpSessionId and the index file is updated', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'acpira-mgr-'));
    const nativeDir = join(dir, 'native');
    mkdirSync(nativeDir);
    const registry = new AgentRegistry({ fake: { name: 'Fake', command: TSX, args: [FAKE], env: { FAKE_SESSION_DIR: nativeDir } } });
    const store = new TranscriptStore(join(dir, 'sessions'));
    try {
      const other = AcpSession.fresh('fake', '/tmp', { registry, log: () => {}, onChange: () => {}, blobs: store });
      await other.start();
      const rec = other.toRecord();
      other.dispose();
      await store.flush(rec);
      // Rewrite the index the way a build without the field left it
      const { acpSessionId, ...stale } = summarize(rec);
      writeFileSync(join(dir, 'sessions', 'index.json'), JSON.stringify([stale]));

      const m = new SessionManager({
        registry, store, log: () => {}, cwd: () => '/tmp', defaultAgent: () => 'fake', runInTerminal: () => {}, toast: () => {},
      });
      try {
        await m.init();
        await m.listNativeSessions('fake');
        await vi.waitFor(() => {
          const entry = (JSON.parse(readFileSync(join(dir, 'sessions', 'index.json'), 'utf8')) as { id: string; acpSessionId?: string }[]).find(s => s.id === rec.id);
          expect(entry?.acpSessionId).toBe(acpSessionId);
        }, { timeout: 5000 });
      } finally { await m.dispose(); }
    } finally { rmSync(dir, { recursive: true, force: true }); }
  }, 30_000);
});
