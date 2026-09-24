import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import type { HostMsg, WebviewMsg } from '../src/shared/protocol';
import { SIDECAR_PROTOCOL_VERSION, type PlatformMethod, type PlatformRequest, type ShellMsg, type SidecarMsg } from '../src/shared/sidecar';

export const FAKE = fileURLToPath(new URL('./fake-agent.ts', import.meta.url));
export const TSX = fileURLToPath(new URL('../node_modules/.bin/tsx', import.meta.url));
export const SERVER = fileURLToPath(new URL('../src/host/server.ts', import.meta.url));
// ACPIRA_ENGINE=rust runs the same contract against the Rust sidecar (ACPIRA_RUST_BIN overrides the debug build path)
export const RUST_BIN = process.env.ACPIRA_ENGINE === 'rust'
  ? process.env.ACPIRA_RUST_BIN || fileURLToPath(new URL('../rust/target/debug/acpira', import.meta.url))
  : undefined;
export const TSCONFIG = fileURLToPath(new URL('../tsconfig.host.json', import.meta.url));

type Hello = Extract<ShellMsg, { type: 'hello' }>;

// A shell as the IntelliJ plugin will be: spawns the sidecar over stdio, speaks envelopes, answers platform requests, records everything
export class Shell {
  readonly proc: ChildProcessWithoutNullStreams;
  readonly out: SidecarMsg[] = [];
  readonly stdoutLines: string[] = [];
  readonly stderr: string[] = [];
  readonly requests: Extract<SidecarMsg, { type: 'platformRequest' }>[] = [];
  exitCode: number | null = null;
  private waiters: { pred: (m: SidecarMsg) => boolean; resolve: (m: SidecarMsg) => void }[] = [];
  // How this shell answers RPCs; a method not listed here is left unanswered
  answers: Partial<Record<PlatformMethod, (r: PlatformRequest) => unknown>> = {};

  // engine: which sidecar to spawn; the default follows ACPIRA_ENGINE
  constructor(readonly home: string, readonly cwd: string, engine: 'ts' | 'rust' = RUST_BIN ? 'rust' : 'ts') {
    this.proc = engine === 'rust' && RUST_BIN
      ? spawn(RUST_BIN, ['--home', home], { stdio: 'pipe', env: { ...process.env, ACPIRA_HOME: '' } })
      : spawn(TSX, ['--tsconfig', TSCONFIG, SERVER, '--home', home], { stdio: 'pipe', env: { ...process.env, ACPIRA_HOME: '' } });
    createInterface({ input: this.proc.stdout }).on('line', line => {
      this.stdoutLines.push(line);
      let m: SidecarMsg;
      try { m = JSON.parse(line) as SidecarMsg; } catch { return; }
      this.out.push(m);
      if (m.type === 'platformRequest') this.onRequest(m);
      for (const w of this.waiters.splice(0)) { if (w.pred(m)) w.resolve(m); else this.waiters.push(w); }
    });
    createInterface({ input: this.proc.stderr }).on('line', line => this.stderr.push(line));
    this.proc.on('exit', code => { this.exitCode = code; });
  }

  private onRequest(m: Extract<SidecarMsg, { type: 'platformRequest' }>) {
    this.requests.push(m);
    const answer = this.answers[m.request.method];
    if (!m.requestId || !answer) return;
    try { this.send({ type: 'platformResponse', requestId: m.requestId, result: answer(m.request) }); }
    catch (e) { this.send({ type: 'platformResponse', requestId: m.requestId, error: String(e) }); }
  }

  send(m: ShellMsg | Record<string, unknown>) { this.proc.stdin.write(`${JSON.stringify(m)}\n`); }
  raw(line: string) { this.proc.stdin.write(`${line}\n`); }

  next<T extends SidecarMsg>(pred: (m: SidecarMsg) => m is T, ms = 10_000): Promise<T> {
    const have = this.out.find(pred);
    if (have) return Promise.resolve(have);
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error(`timed out waiting; stderr:\n${this.stderr.join('\n')}`)), ms);
      this.waiters.push({ pred, resolve: m => { clearTimeout(t); resolve(m as T); } });
    });
  }

  hostMsg<T extends HostMsg['type']>(viewId: string, type: T, pred: (m: Extract<HostMsg, { type: T }>) => boolean = () => true, ms?: number) {
    return this.next((m): m is Extract<SidecarMsg, { type: 'hostMessage' }> => m.type === 'hostMessage' && m.viewId === viewId && m.message.type === type && pred(m.message as Extract<HostMsg, { type: T }>), ms)
      .then(m => m.message as Extract<HostMsg, { type: T }>);
  }

  hostMsgs(viewId: string): HostMsg[] {
    return this.out.filter((m): m is Extract<SidecarMsg, { type: 'hostMessage' }> => m.type === 'hostMessage' && m.viewId === viewId).map(m => m.message);
  }

  hello(over: Partial<Hello> = {}, agentOver: Record<string, unknown> = {}) {
    const m: Hello = {
      type: 'hello', protocolVersion: SIDECAR_PROTOCOL_VERSION, requestId: 'h1',
      client: { name: 'test-shell', version: '0.0.0', capabilities: [] },
      env: { hostLanguage: 'en', cwd: this.cwd, blobBase: 'https://acpira.local/blobs' },
      settings: { defaultAgent: 'fake', agents: { fake: { name: 'Fake', command: TSX, args: [FAKE], ...agentOver } } },
      ...over,
    };
    this.send(m);
    return this.next((x): x is Extract<SidecarMsg, { type: 'helloOk' | 'helloReject' }> => x.type === 'helloOk' || x.type === 'helloReject');
  }

  view(viewId: string, message: WebviewMsg) { this.send({ type: 'webviewMessage', viewId, message }); }

  async open(viewId: string, host: 'sidebar' | 'editor' = 'sidebar', initial?: string | { mostRecent: true }) {
    this.send({ type: 'attachView', viewId, host, initial });
    this.view(viewId, { type: 'ready' });
    return this.hostMsg(viewId, 'init');
  }

  exited(ms = 15_000): Promise<number | null> {
    if (this.exitCode !== null) return Promise.resolve(this.exitCode);
    return new Promise((resolve, reject) => {
      const t = setTimeout(() => reject(new Error('sidecar did not exit')), ms);
      this.proc.on('exit', code => { clearTimeout(t); resolve(code); });
    });
  }

  // Graceful first (the sidecar disposes its agent processes), SIGKILL only when it does not come back
  async kill() {
    if (this.exitCode !== null) return;
    if (!this.proc.stdin.destroyed) this.send({ type: 'shutdown' });
    await this.exited(3000).catch(() => { this.proc.kill('SIGKILL'); return this.exited().catch(() => {}); });
  }
}

