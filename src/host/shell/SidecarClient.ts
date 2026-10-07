import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import type { Socket } from 'node:net';
import { createInterface } from 'node:readline';
import type { HostMsg, WebviewHost, WebviewMsg } from '@shared/protocol';
import { SIDECAR_PROTOCOL_VERSION, type PlatformEvent, type PlatformRequest, type ShellMsg, type SidecarMsg } from '@shared/sidecar';
import { reachEngine, type EngineEndpoint } from './engine';
import { onNdjsonLines } from './ndjson';

// One way to start the sidecar binary and a label for the log
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
  // Where this view loads attachment blobs from; sent with every attach, so a re-attach after a restart keeps its own
  readonly blobBase?: string;
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
  // The persistent engine for a command: connect to its socket (launching it detached when nothing listens) instead of
  // spawning a child over stdio. Disconnecting leaves it and its sessions running. Undefined: a child over stdio
  engine?: (cmd: SidecarCommand) => EngineEndpoint | undefined;
}

const MAX_RESTARTS = 5;
// A run longer than this starts a new burst of restarts instead of adding to the current one
const STABLE_MS = 60_000;

interface Attached {
  view: ShellView;
  // The session the view was last showing; a restart re-attaches on it
  lastSessionId?: string;
}

// The shell side of the sidecar protocol (src/shared/sidecar.ts) for a Node-hosted shell: spawns the process (or connects to the
// persistent engine), handshakes, relays view traffic and platform RPCs, and restarts / reconnects with backoff. Envelopes sent before
// helloOk wait in an outbox; a restart re-sends hello and every attachView on the session each view was showing. Each process or
// connection has a generation, so a replaced one's late output and exit cannot touch its successor. The VS Code extension uses it;
// nothing here imports vscode
export class SidecarClient {
  private proc?: ChildProcessWithoutNullStreams;
  // The connection to a persistent engine (opts.engine), instead of proc
  private sock?: Socket;
  private connecting = false;
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

  private get live() { return !!(this.proc || this.sock || this.connecting); }

  start() {
    if (this.disposed || this.live) return;
    clearTimeout(this.timer);
    this.timer = undefined;
    const candidates = this.opts.commands();
    const cmd = candidates[this.candidate];
    if (!cmd) { this.setState('failed', 'no sidecar binary for this platform'); return; }
    const gen = ++this.gen;
    this.setState('starting');
    this.lastStart = Date.now();
    const endpoint = this.opts.engine?.(cmd);
    if (endpoint) { void this.connect(gen, cmd, endpoint, candidates.length); return; }
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
    onNdjsonLines(proc.stdout, line => this.onLine(gen, line));
    createInterface({ input: proc.stderr, crlfDelay: Infinity }).on('line', line => this.opts.log(`sidecar: ${line}`));
    this.write({ type: 'hello', protocolVersion: SIDECAR_PROTOCOL_VERSION, requestId: `h${++this.helloSeq}`, ...this.opts.hello() });
  }

  // The persistent engine: the same envelopes over its socket. A closed connection is handled like an exited process (reconnect
  // with backoff), and the engine is launched again only when nothing listens any more
  private async connect(gen: number, cmd: SidecarCommand, ep: EngineEndpoint, total: number) {
    this.connecting = true;
    let sock: Socket;
    try {
      sock = await reachEngine(cmd, ep, { cwd: this.opts.cwd?.(), cancelled: () => gen !== this.gen || this.disposed, log: this.opts.log });
    } catch (e) {
      if (gen !== this.gen) return;
      this.connecting = false;
      this.spawnFailed(gen, cmd, e, total);
      return;
    }
    if (gen !== this.gen || this.disposed) { sock.destroy(); return; }
    this.connecting = false;
    this.sock = sock;
    this.opts.log(`engine connected: ${ep.socket}`);
    sock.on('error', e => this.opts.log(`engine connection: ${String(e)}`));
    sock.on('close', () => {
      if (this.sock === sock) this.sock = undefined;
      this.onExit(gen, 'connection closed', total);
    });
    onNdjsonLines(sock, line => this.onLine(gen, line));
    this.write({ type: 'hello', protocolVersion: SIDECAR_PROTOCOL_VERSION, requestId: `h${++this.helloSeq}`, ...this.opts.hello() });
  }

  // Retry after FAILED (the status notification's button): the consecutive-failure counter and the engine choice start over
  retry() {
    if (this.disposed || this.live) return;
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

  // Facts about the IDE changed. Before helloOk the event waits too: the hello already in flight may predate it, and replaying a
  // snapshot the next hello also carries is harmless
  event(event: PlatformEvent) {
    this.post({ type: 'platformEvent', event });
  }

  private post(m: ShellMsg) {
    if (this.ready) this.write(m);
    else this.outbox.push(m);
  }

  private write(m: ShellMsg) {
    const out: NodeJS.WritableStream | undefined = this.proc?.stdin ?? this.sock;
    if (out?.writable) out.write(`${JSON.stringify(m)}\n`);
  }

  private attachEnvelope(a: Attached): ShellMsg {
    const initial = a.lastSessionId ?? a.view.initial;
    const { blobBase } = a.view;
    return { type: 'attachView', viewId: a.view.viewId, host: a.view.host, ...(initial !== undefined ? { initial } : {}), ...(blobBase ? { blobBase } : {}) };
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
        // Queued facts first: a view reads the environment when it attaches
        const queued = this.outbox.splice(0);
        for (const m of queued) if (m.type === 'platformEvent') this.write(m);
        for (const a of this.views.values()) this.write(this.attachEnvelope(a));
        for (const m of queued) if (m.type !== 'platformEvent') this.write(m);
        this.setState('ready');
        return;
      }
      case 'helloReject':
        this.opts.log(`sidecar rejected hello: ${m.reason}`);
        // A persistent engine that is just ending: dropping the connection reconnects (and launches the next engine)
        if (this.sock && /shutting down/.test(m.reason)) { this.sock.destroy(); return; }
        this.setState('failed', m.reason);
        this.sock?.destroy();
        void this.stop(this.proc, 500);
        return;
      case 'hostMessage': {
        const a = this.views.get(m.viewId);
        if (!a) return;
        const msg = m.message;
        const session = msg.type === 'session' ? msg.session : msg.type === 'sessionPatch' ? msg.patch.view : msg.type === 'init' ? msg.state.active : undefined;
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
    this.sock = undefined;
    this.opts.log(`sidecar could not start (${cmd.label}): ${String(e)}`);
    if (this.candidate + 1 < total) { this.candidate++; this.start(); return; }
    this.setState('failed', String(e));
  }

  private onExit(gen: number, code: number | string, total: number) {
    if (gen !== this.gen) return;
    const reachedHello = this.ready;
    this.proc = undefined;
    this.sock = undefined;
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

  // A child process is shut down with its agents; a persistent engine is only disconnected from, so its turns keep running and the
  // next window picks them up
  async dispose() {
    if (this.disposed) return;
    this.disposed = true;
    clearTimeout(this.timer);
    const proc = this.proc;
    const sock = this.sock;
    this.gen++;
    this.proc = undefined;
    this.sock = undefined;
    this.connecting = false;
    this.ready = false;
    this.outbox = [];
    this.views.clear();
    this.state = 'stopped';
    if (sock) await new Promise<void>(resolve => { sock.end(resolve); setTimeout(() => { sock.destroy(); resolve(); }, 1000).unref(); });
    await this.stop(proc);
  }
}
