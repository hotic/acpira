import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import type { HostMsg, WebviewHost, WebviewMsg } from '@shared/protocol';
import { SIDECAR_PROTOCOL_VERSION, type PlatformEvent, type PlatformRequest, type ShellMsg, type SidecarMsg } from '@shared/sidecar';

// One way to start a sidecar (the Rust binary, or Node running host-server.cjs) and a label for the log
export interface SidecarCommand {
  command: string;
  args: string[];
  env?: Record<string, string>;
  label: string;
}

export type SidecarState = 'starting' | 'ready' | 'failed' | 'stopped';

export type HelloPayload = Pick<Extract<ShellMsg, { type: 'hello' }>, 'client' | 'env' | 'settings'>;

// A view the shell renders (a webview); its WebviewMsgs go out through send(), the sidecar's HostMsgs come back here
export interface ShellView {
  readonly viewId: string;
  readonly host: WebviewHost;
  readonly initial?: string | { mostRecent: true };
  onHostMessage(m: HostMsg): void;
  // Called once on attach with the current state, then on every change; a view that already initialized reloads on a later `ready`
  onState(state: SidecarState, detail?: string): void;
}

export interface SidecarClientOpts {
  // Candidates in preference order: one that cannot be spawned, or exits before helloOk, hands over to the next
  commands: () => SidecarCommand[];
  cwd?: () => string | undefined;
  // Read at every (re)start, so a restarted sidecar gets the current settings and environment
  hello: () => HelloPayload;
  // IDE actions the sidecar asks for; the result (or thrown error) answers RPC methods, notifications ignore it
  onRequest: (request: PlatformRequest) => unknown;
  log: (line: string) => void;
  onState?: (state: SidecarState, detail?: string) => void;
  // Delay before the nth consecutive restart
  backoff?: (attempt: number) => number;
}

const MAX_RESTARTS = 5;
// A run longer than this starts a new burst of restarts instead of adding to the current one
const STABLE_MS = 60_000;

interface Attached {
  view: ShellView;
  // The session the view was last showing; a restart re-attaches on it
  lastSessionId?: string;
}

// The shell side of the sidecar protocol (src/shared/sidecar.ts) for a Node-hosted shell: spawns the process, handshakes, relays view
// traffic and platform RPCs, and restarts with backoff. Envelopes sent before helloOk wait in an outbox; a restart re-sends hello and
// every attachView on the session each view was showing. Each process has a generation, so a replaced process's late output and exit
// cannot touch its successor. The VS Code extension uses it; nothing here imports vscode
export class SidecarClient {
  private proc?: ChildProcessWithoutNullStreams;
  private gen = 0;
  private ready = false;
  private outbox: ShellMsg[] = [];
  private views = new Map<string, Attached>();
  private state: SidecarState = 'stopped';
  private detail?: string;
  private restarts = 0;
  private lastStart = 0;
  private candidate = 0;
  private timer?: NodeJS.Timeout;
  private disposed = false;
  private helloSeq = 0;
  sessionsDir?: string;

  constructor(private opts: SidecarClientOpts) {}

  get current(): { state: SidecarState; detail?: string } { return { state: this.state, detail: this.detail }; }

  start() {
    if (this.disposed || this.proc) return;
    clearTimeout(this.timer);
    this.timer = undefined;
    const candidates = this.opts.commands();
    const cmd = candidates[this.candidate];
    if (!cmd) { this.setState('failed', 'no sidecar binary for this platform'); return; }
    const gen = ++this.gen;
    this.setState('starting');
    this.lastStart = Date.now();
    let proc: ChildProcessWithoutNullStreams;
    try {
      proc = spawn(cmd.command, cmd.args, { cwd: this.opts.cwd?.(), env: { ...process.env, ...cmd.env }, stdio: 'pipe', windowsHide: true });
    } catch (e) {
      this.spawnFailed(gen, cmd, e, candidates.length);
      return;
    }
    this.proc = proc;
    this.opts.log(`sidecar starting: ${cmd.label}`);
    proc.on('error', e => { if (proc.pid === undefined) this.spawnFailed(gen, cmd, e, candidates.length); else this.opts.log(`sidecar process error: ${String(e)}`); });
    proc.on('exit', (code, signal) => this.onExit(gen, code ?? signal ?? 'unknown', candidates.length));
    // A sidecar that died leaves a broken pipe behind; the exit handler takes it from there
    proc.stdin.on('error', e => this.opts.log(`sidecar stdin: ${String(e)}`));
    createInterface({ input: proc.stdout, crlfDelay: Infinity }).on('line', line => this.onLine(gen, line));
    createInterface({ input: proc.stderr, crlfDelay: Infinity }).on('line', line => this.opts.log(`sidecar: ${line}`));
    this.write({ type: 'hello', protocolVersion: SIDECAR_PROTOCOL_VERSION, requestId: `h${++this.helloSeq}`, ...this.opts.hello() });
  }

  // Retry after FAILED (the status notification's button): the consecutive-failure counter and the engine choice start over
  retry() {
    if (this.disposed || this.proc) return;
    this.restarts = 0;
    this.candidate = 0;
    this.start();
  }

  attach(view: ShellView): () => void {
    this.views.set(view.viewId, { view });
    view.onState(this.state, this.detail);
    if (this.ready) this.write(this.attachEnvelope(this.views.get(view.viewId)!));
    return () => this.detach(view.viewId);
  }

  private detach(viewId: string) {
    if (!this.views.delete(viewId)) return;
    if (this.ready) this.write({ type: 'detachView', viewId });
    else this.outbox = this.outbox.filter(m => !('viewId' in m && m.viewId === viewId));
  }

  send(viewId: string, message: WebviewMsg) {
    if (!this.views.has(viewId)) return;
    this.post({ type: 'webviewMessage', viewId, message });
  }

  // Facts about the IDE changed. Before helloOk the event waits too: the hello already in flight may predate it (the first webview's
  // blobBase arrives while the handshake runs), and replaying a snapshot the next hello also carries is harmless
  event(event: PlatformEvent) {
    this.post({ type: 'platformEvent', event });
  }

  private post(m: ShellMsg) {
    if (this.ready) this.write(m);
    else this.outbox.push(m);
  }

  private write(m: ShellMsg) {
    const stdin = this.proc?.stdin;
    if (stdin?.writable) stdin.write(`${JSON.stringify(m)}\n`);
  }

  private attachEnvelope(a: Attached): ShellMsg {
    const initial = a.lastSessionId ?? a.view.initial;
    return { type: 'attachView', viewId: a.view.viewId, host: a.view.host, ...(initial !== undefined ? { initial } : {}) };
  }

  private onLine(gen: number, line: string) {
    if (gen !== this.gen || !line.trim()) return;
    let m: SidecarMsg;
    try { m = JSON.parse(line) as SidecarMsg; } catch { this.opts.log(`sidecar stdout is not an envelope: ${line.slice(0, 200)}`); return; }
    switch (m.type) {
      case 'helloOk': {
        this.ready = true;
        this.candidate = 0;
        this.sessionsDir = m.sessionsDir;
        this.opts.log(`sidecar ready: pid ${m.sidecar.pid}, version ${m.sidecar.version}, sessions ${m.sessionsDir}`);
        // Queued facts first: a view reads the environment (blobBase) when it attaches
        const queued = this.outbox.splice(0);
        for (const m of queued) if (m.type === 'platformEvent') this.write(m);
        for (const a of this.views.values()) this.write(this.attachEnvelope(a));
        for (const m of queued) if (m.type !== 'platformEvent') this.write(m);
        this.setState('ready');
        return;
      }
      case 'helloReject':
        this.opts.log(`sidecar rejected hello: ${m.reason}`);
        this.setState('failed', m.reason);
        void this.stop(this.proc, 500);
        return;
      case 'hostMessage': {
        const a = this.views.get(m.viewId);
        if (!a) return;
        const session = m.message.type === 'session' ? m.message.session : m.message.type === 'init' ? m.message.state.active : undefined;
        if (session?.id) a.lastSessionId = session.id;
        a.view.onHostMessage(m.message);
        return;
      }
      case 'platformRequest': void this.onRequest(gen, m.requestId, m.request); return;
      case 'shutdownOk': return;
      default: this.opts.log(`sidecar sent an unknown envelope: ${(m as { type?: unknown }).type}`);
    }
  }

  private async onRequest(gen: number, requestId: string | undefined, request: PlatformRequest) {
    let reply: ShellMsg | undefined;
    try {
      const result = await this.opts.onRequest(request);
      if (requestId) reply = { type: 'platformResponse', requestId, result: result ?? null };
    } catch (e) {
      this.opts.log(`platform request ${request.method} failed: ${e instanceof Error ? e.message : String(e)}`);
      if (requestId) reply = { type: 'platformResponse', requestId, error: e instanceof Error ? e.message : String(e) };
    }
    if (reply && gen === this.gen) this.write(reply);
  }

  private spawnFailed(gen: number, cmd: SidecarCommand, e: unknown, total: number) {
    if (gen !== this.gen) return;
    this.proc = undefined;
    this.opts.log(`sidecar could not start (${cmd.label}): ${String(e)}`);
    if (this.candidate + 1 < total) { this.candidate++; this.start(); return; }
    this.setState('failed', String(e));
  }

  private onExit(gen: number, code: number | string, total: number) {
    if (gen !== this.gen) return;
    const reachedHello = this.ready;
    this.proc = undefined;
    this.ready = false;
    // The outbox is only ever non-empty before a handshake, so what it holds was never delivered: it goes to the next process
    if (this.disposed || this.state === 'failed') return;
    this.opts.log(`sidecar exited (${code})`);
    // An engine that never got through the handshake is not worth retrying while another one is available
    if (!reachedHello && this.candidate + 1 < total) {
      this.candidate++;
      this.opts.log('sidecar exited before the handshake; trying the next engine');
      this.start();
      return;
    }
    this.restarts = Date.now() - this.lastStart > STABLE_MS ? 1 : this.restarts + 1;
    if (this.restarts > MAX_RESTARTS) { this.setState('failed', `the host process keeps exiting (last: ${code})`); return; }
    const delay = (this.opts.backoff ?? (n => Math.min(30_000, 1000 * 2 ** (n - 1))))(this.restarts);
    this.setState('starting', `exited (${code}), restarting in ${Math.round(delay / 1000)}s`);
    this.timer = setTimeout(() => { this.timer = undefined; this.start(); }, delay);
  }

  private setState(state: SidecarState, detail?: string) {
    this.state = state;
    this.detail = detail;
    for (const a of [...this.views.values()]) a.view.onState(state, detail);
    this.opts.onState?.(state, detail);
  }

  // Ask nicely, then insist: shutdown → wait → SIGTERM → wait → SIGKILL
  private async stop(proc: ChildProcessWithoutNullStreams | undefined, graceMs = 3000) {
    if (!proc || proc.exitCode !== null || proc.signalCode !== null) return;
    const exited = new Promise<void>(resolve => proc.once('exit', () => resolve()));
    const within = (ms: number) => Promise.race([exited.then(() => true), new Promise<boolean>(r => setTimeout(() => r(false), ms).unref())]);
    if (proc.stdin.writable) proc.stdin.write(`${JSON.stringify({ type: 'shutdown' })}\n`);
    if (await within(graceMs)) return;
    proc.kill('SIGTERM');
    if (await within(2000)) return;
    proc.kill('SIGKILL');
  }

  async dispose() {
    if (this.disposed) return;
    this.disposed = true;
    clearTimeout(this.timer);
    const proc = this.proc;
    this.gen++;
    this.proc = undefined;
    this.ready = false;
    this.outbox = [];
    this.views.clear();
    this.state = 'stopped';
    await this.stop(proc);
  }
}
