import { mkdtempSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { AccountAction, AddAccountVia, HostMsg, InitState, WebviewMsg } from '@shared/protocol';
import type { SidecarMsg } from '@shared/sidecar';
import type { AgentId, NativeSessionInfo, SessionSummary, SessionView, Turn } from '@shared/transcript';
import { Shell } from '../../test/sidecarShell';
import { sidecarBin } from './sidecarBin';

// The probes' view of the real host: a Rust sidecar driven over the envelope protocol the IDE shells speak, with each view offering the
// calls the webview makes. Nothing here reaches into the engine, so a probe exercises exactly the path a user's click takes

export interface HostOpts {
  // The workspace folder the session works in
  cwd: string;
  // ~/.acpira stand-in; a fresh temp directory by default
  home?: string;
  defaultAgent: AgentId;
  // acpira.agents: custom definitions (a built-in id here replaces the built-in entry)
  agents?: Record<string, unknown>;
  // Further acpira.* settings for the hello snapshot
  settings?: Record<string, unknown>;
}

type SubagentMsg = Extract<HostMsg, { type: 'subagent' }>;

export class Host {
  readonly shell: Shell;
  sessionsDir = '';
  private views = 0;

  private constructor(readonly home: string, readonly cwd: string) {
    this.shell = new Shell(home, cwd, sidecarBin({ build: true }));
    this.shell.answers = {
      toast: r => { if (r.method === 'toast') console.log(`toast ${r.level}: ${r.text}`); return null; },
      searchFiles: () => [],
      writeSetting: () => null,
      revealInOS: () => null,
      openResolvedFile: () => null,
      openPlanDocument: () => null,
    };
  }

  static async start(o: HostOpts): Promise<Host> {
    const host = new Host(o.home ?? mkdtempSync(join(tmpdir(), 'acpira-probe-home-')), o.cwd);
    const ok = await host.shell.hello({
      client: { name: 'acpira-probe', version: '0', capabilities: ['toast', 'searchFiles', 'writeSetting', 'revealInOS', 'openResolvedFile', 'openPlanDocument'] },
      env: { hostLanguage: 'en', cwd: o.cwd },
      settings: { defaultAgent: o.defaultAgent, ...(o.agents ? { agents: o.agents } : {}), ...o.settings },
    });
    if (ok.type !== 'helloOk') throw new Error(`sidecar refused the handshake: ${ok.reason}`);
    host.sessionsDir = ok.sessionsDir;
    return host;
  }

  // The sidecar's log (stderr), for probes that look for a line such as `session/resume ok`
  get logs(): string[] { return this.shell.stderr; }

  async view(initial?: string): Promise<View> {
    const v = new View(this.shell, `probe-${++this.views}`);
    await v.open(initial);
    return v;
  }

  blobPath(sessionId: string, name: string): string { return join(this.sessionsDir, sessionId, name); }

  // The persisted record of a session, as the store wrote it
  async record(sessionId: string): Promise<Record<string, unknown> | null> {
    return readFile(join(this.sessionsDir, `${sessionId}.json`), 'utf8').then(t => JSON.parse(t) as Record<string, unknown>, () => null);
  }

  // Graceful shutdown: the sidecar ends its agent processes before it exits
  async dispose() { await this.shell.kill(); }
}

export class View {
  private session?: SessionView;
  private list: SessionSummary[] = [];
  private subagentListeners: ((m: SubagentMsg) => void)[] = [];
  private changed: (() => void)[] = [];

  constructor(private shell: Shell, readonly id: string) {
    shell.onMessage(m => this.onMessage(m));
  }

  private onMessage(m: SidecarMsg) {
    if (m.type !== 'hostMessage' || m.viewId !== this.id) return;
    const msg = m.message;
    if (msg.type === 'init') this.apply(msg.state);
    else if (msg.type === 'session') this.session = msg.session;
    else if (msg.type === 'sessions') this.list = msg.sessions;
    else if (msg.type === 'subagent') for (const l of this.subagentListeners) l(msg);
    for (const c of this.changed.splice(0)) c();
  }

  private apply(state: InitState) {
    this.session = state.active;
    this.list = state.sessions;
  }

  async open(initial?: string) {
    const init = await this.shell.open(this.id, 'sidebar', initial);
    this.apply(init.state);
  }

  active(): SessionView | undefined { return this.session; }
  get activeId(): string | undefined { return this.session?.id; }
  sessions(): SessionSummary[] { return this.list; }

  onSubagent(listener: (m: SubagentMsg) => void) { this.subagentListeners.push(listener); }

  post(message: WebviewMsg) { this.shell.view(this.id, message); }

  // Resolves on the next host message for this view, or after `ms` when none comes
  private nextChange(ms: number): Promise<void> {
    return new Promise(resolve => {
      const t = setTimeout(resolve, ms);
      this.changed.push(() => { clearTimeout(t); resolve(); });
    });
  }

  async until(pred: () => boolean, ms: number, what: string) {
    const t0 = Date.now();
    while (!pred()) {
      if (Date.now() - t0 > ms) throw new Error(`timeout waiting for ${what}`);
      await this.nextChange(Math.max(1, Math.min(250, ms - (Date.now() - t0))));
    }
  }

  // What the webview's messages do, awaited the way SessionManager.handle resolved: a prompt at the end of its turn, a session switch
  // once the view shows it, anything else at the view's next update
  async handle(message: WebviewMsg, ms = 600_000): Promise<void> {
    const before = this.session;
    this.post(message);
    if (message.type === 'send' || message.type === 'retryTurn' || message.type === 'retry') {
      const id = message.sessionId ?? before?.id;
      const count = message.type === 'send' ? (before?.turns.length ?? 0) : 0;
      await this.until(() => {
        const v = this.session;
        if (!v || v.id !== id || v.running) return false;
        const last: Turn | undefined = v.turns[v.turns.length - 1];
        return v.status === 'error' || (v.turns.length > count && last?.role === 'agent' && last.stop !== undefined);
      }, ms, `the turn to end (${message.type})`);
    } else if (message.type === 'newSession') {
      // An untouched session of the same agent is reused rather than replaced, so the id may stay
      const agent = message.agent ?? before?.agent;
      await this.until(() => !!this.session && this.session.agent === agent && (this.session.id !== before?.id || before.turns.length === 0), 30_000, 'the new session');
    } else if (message.type === 'selectSession') {
      await this.until(() => this.session?.id === message.id, 30_000, `session ${message.id}`);
    } else {
      await this.nextChange(3000);
    }
  }

  newSession(agent: AgentId) { return this.handle({ type: 'newSession', agent }); }
  selectSession(id: string) { return this.handle({ type: 'selectSession', id }); }

  // The account menu's add: resolves with the host's final action state for that agent (`success`, `missing`, `error`, …)
  async addAccount(agent: AgentId, via: AddAccountVia): Promise<AccountAction> {
    const settled = (a: AccountAction) => a.agent === agent && a.via === via && a.status !== 'pending';
    const seen = this.shell.hostMsgs(this.id).length;
    this.post({ type: 'addAccount', agent, via });
    const reply = await this.shell.next((m): m is Extract<SidecarMsg, { type: 'hostMessage' }> =>
      m.type === 'hostMessage' && m.viewId === this.id && m.message.type === 'accountActions' && m.message.actions.some(settled)
      && this.shell.hostMsgs(this.id).length > seen, 120_000);
    return (reply.message as Extract<HostMsg, { type: 'accountActions' }>).actions.find(settled)!;
  }

  async listNativeSessions(agent: AgentId): Promise<NativeSessionInfo[]> {
    const seen = this.shell.hostMsgs(this.id).filter(m => m.type === 'nativeSessions').length;
    this.post({ type: 'listNativeSessions', agent });
    const reply = await this.shell.next((m): m is Extract<SidecarMsg, { type: 'hostMessage' }> =>
      m.type === 'hostMessage' && m.viewId === this.id && m.message.type === 'nativeSessions' && m.message.agent === agent
      && this.shell.hostMsgs(this.id).filter(x => x.type === 'nativeSessions').length > seen, 120_000);
    const msg = reply.message as Extract<HostMsg, { type: 'nativeSessions' }>;
    if (msg.error) throw new Error(msg.error);
    return msg.sessions;
  }

  async importNativeSession(agent: AgentId, s: NativeSessionInfo) {
    const before = this.session?.id;
    this.post({ type: 'importNativeSession', agent, sessionId: s.sessionId, cwd: s.cwd, title: s.title, updatedAt: s.updatedAt });
    // One already imported just switches to the record holding it
    await this.until(() => !!this.session && (s.localId ? this.session.id === s.localId : this.session.id !== before), 60_000, 'the imported session');
  }
}
