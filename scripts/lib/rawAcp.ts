import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { createInterface } from 'node:readline';
import type { SpawnSpec } from './sidecarBin';

export type RpcMessage = Record<string, unknown>;

export class RpcError extends Error {
  constructor(readonly code: number, message: string, readonly data?: unknown) { super(message); }
}

export interface RawAgentHooks {
  // Every line both ways, parsed; `secret` marks an outgoing message that must not be recorded as is
  onMessage?(dir: '→' | '←', msg: RpcMessage, secret?: boolean): void;
  onRequest?(method: string, params: Record<string, unknown>): Promise<unknown> | unknown;
  onNotification?(method: string, params: Record<string, unknown>): void;
  onStderr?(line: string): void;
  onNonJson?(line: string): void;
}

// A raw ndjson JSON-RPC peer over an agent's stdio: nothing is validated or dropped on the way, so a probe sees exactly what the agent
// sends. An inbound request answers with whatever onRequest returns; a thrown RpcError goes back as that error, anything else as
// method-not-found
export class RawAgent {
  readonly child: ChildProcessWithoutNullStreams;
  private seq = 0;
  private pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  readonly exited: Promise<{ code: number | null; signal: NodeJS.Signals | null }>;

  constructor(spec: SpawnSpec, cwd: string, env: Record<string, string>, private hooks: RawAgentHooks = {}) {
    this.child = spawn(spec.command, spec.args, { cwd, stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, ...env }, windowsVerbatimArguments: spec.verbatim });
    this.exited = new Promise(resolve => this.child.once('exit', (code, signal) => resolve({ code, signal })));
    createInterface({ input: this.child.stderr }).on('line', l => hooks.onStderr?.(l));
    createInterface({ input: this.child.stdout }).on('line', line => this.onLine(line));
  }

  private onLine(line: string) {
    if (!line.trim()) return;
    let msg: RpcMessage;
    try { msg = JSON.parse(line) as RpcMessage; } catch { this.hooks.onNonJson?.(line); return; }
    this.hooks.onMessage?.('←', msg);
    if (typeof msg.method === 'string') {
      const params = (msg.params ?? {}) as Record<string, unknown>;
      if (!('id' in msg)) { this.hooks.onNotification?.(msg.method, params); return; }
      void (async () => {
        try {
          if (!this.hooks.onRequest) throw new RpcError(-32601, `Method not found: ${msg.method}`);
          const result = await this.hooks.onRequest(msg.method as string, params);
          if (result === undefined) throw new RpcError(-32601, `Method not found: ${msg.method}`);
          this.write({ jsonrpc: '2.0', id: msg.id, result });
        } catch (e) {
          const err = e instanceof RpcError ? e : new RpcError(-32603, String(e));
          this.write({ jsonrpc: '2.0', id: msg.id, error: { code: err.code, message: err.message, ...(err.data === undefined ? {} : { data: err.data }) } });
        }
      })();
      return;
    }
    const p = typeof msg.id === 'number' ? this.pending.get(msg.id) : undefined;
    if (!p) return;
    this.pending.delete(msg.id as number);
    const error = msg.error as { code: number; message: string; data?: unknown } | undefined;
    if (error) p.reject(new RpcError(error.code, error.message, error.data)); else p.resolve(msg.result);
  }

  private write(msg: RpcMessage, secret = false) {
    this.hooks.onMessage?.('→', msg, secret);
    this.child.stdin.write(`${JSON.stringify(msg)}\n`);
  }

  request<T = unknown>(method: string, params: unknown, { secret = false } = {}): Promise<T> {
    const id = ++this.seq;
    const reply = new Promise<T>((resolve, reject) => this.pending.set(id, { resolve: v => resolve(v as T), reject }));
    this.write({ jsonrpc: '2.0', id, method, params }, secret);
    return reply;
  }

  notify(method: string, params: unknown) { this.write({ jsonrpc: '2.0', method, params }); }

  // SIGTERM, then SIGKILL when the agent is still there after the grace
  async kill(graceMs = 5000) {
    if (this.child.exitCode !== null || this.child.signalCode !== null) return;
    this.child.kill();
    const timer = setTimeout(() => this.child.kill('SIGKILL'), graceMs);
    await this.exited;
    clearTimeout(timer);
  }
}
