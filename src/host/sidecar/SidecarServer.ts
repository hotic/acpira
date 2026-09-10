import { SIDECAR_PROTOCOL_VERSION, isShellMsg, type ShellMsg, type SidecarMsg } from '@shared/sidecar';
import type { BridgeCore } from '../bridgeCore';
import { msg } from '../errors';
import { createHostRuntime, type HostRuntime } from '../runtime';
import { SidecarPlatform, type Hello } from './SidecarPlatform';

// One ndjson line channel to one shell; how the bytes travel (stdio, a WebSocket) is the wire's business
export interface Wire {
  write(line: string): void;
  onLine(fn: (line: string) => void): void;
  onClose(fn: () => void): void;
  end(): void;
}

export interface SidecarServerOpts {
  version: string;
  // ACPIRA_HOME override (tests); the runtime's default otherwise
  home?: string;
  log: (line: string) => void;
  // Called once the server is done: after shutdownOk was written, or when the wire closed. Exit code 2 = protocol rejected
  onExit: (code: number) => void;
  // Browser harness: the connecting page must not supply executable command lines
  ignoreClientAgents?: boolean;
}

// The sidecar's state machine over one wire: hello (version-checked) → runtime → views. Control messages (hello / attach / detach /
// shutdown) are processed strictly in order; a view's WebviewMsgs are dispatched in order but not awaited, because a `send` resolves
// only when the whole turn ends and a `stop` must not queue behind it
export class SidecarServer {
  private platform?: SidecarPlatform;
  private runtime?: HostRuntime;
  private views = new Map<string, BridgeCore>();
  private chain: Promise<void> = Promise.resolve();
  private done = false;

  constructor(private wire: Wire, private opts: SidecarServerOpts) {}

  start() {
    this.wire.onLine(line => this.onLine(line));
    // Lines received before EOF are still honoured (a shell that writes shutdown and closes its end at once gets its shutdownOk)
    this.wire.onClose(() => { this.chain = this.chain.then(() => this.finish('wire closed', 0, false)); });
  }

  private send(m: SidecarMsg) {
    if (!this.done) this.wire.write(JSON.stringify(m));
  }

  // A line that is not a JSON envelope is noise on the channel (a stray print in the shell): logged and skipped, never fatal
  private onLine(line: string) {
    const text = line.trim();
    if (!text) return;
    let parsed: unknown;
    try { parsed = JSON.parse(text); } catch { this.opts.log(`ignoring non-JSON line from the shell: ${text.slice(0, 120)}`); return; }
    if (!isShellMsg(parsed)) { this.opts.log(`ignoring envelope without a type: ${text.slice(0, 120)}`); return; }
    const m = parsed;
    this.chain = this.chain.then(() => this.handle(m)).catch(e => this.opts.log(`${m.type} failed: ${msg(e)}`));
  }

  private async handle(m: ShellMsg) {
    if (this.done) return;
    if (m.type === 'hello') { await this.hello(m); return; }
    if (m.type === 'shutdown') { await this.finish('shutdown requested', 0, true); return; }
    if (!this.runtime || !this.platform) { this.opts.log(`${m.type} before hello, ignored`); return; }
    switch (m.type) {
      case 'attachView': {
        if (this.views.has(m.viewId)) { this.opts.log(`attachView: ${m.viewId} already attached, ignored`); return; }
        const viewId = m.viewId;
        const core = this.runtime.attachView({
          host: m.host, initial: m.initial, blobBase: this.platform.blobBase(),
          post: message => this.send({ type: 'hostMessage', viewId, message }),
        });
        this.views.set(viewId, core);
        return;
      }
      case 'detachView': {
        const core = this.views.get(m.viewId);
        if (!core) return;
        this.views.delete(m.viewId);
        this.runtime.detachView(core);
        return;
      }
      case 'webviewMessage': {
        const core = this.views.get(m.viewId);
        if (!core) { this.opts.log(`webviewMessage for unknown view ${m.viewId} (${m.message?.type}), ignored`); return; }
        void core.handle(m.message);
        return;
      }
      case 'platformResponse': this.platform.onResponse(m); return;
      case 'platformEvent': this.platform.onEvent(m.event); return;
    }
  }

  private async hello(m: Hello) {
    if (m.protocolVersion !== SIDECAR_PROTOCOL_VERSION) {
      this.send({ type: 'helloReject', requestId: m.requestId, protocolVersion: SIDECAR_PROTOCOL_VERSION, reason: `protocol version ${m.protocolVersion} is not ${SIDECAR_PROTOCOL_VERSION}` });
      await this.finish(`protocol version mismatch (${m.protocolVersion})`, 2, false);
      return;
    }
    if (this.runtime) {
      // A second hello on the same wire is a shell bug, but a harmless one: answer with the running runtime
      this.send({ type: 'helloOk', requestId: m.requestId, protocolVersion: SIDECAR_PROTOCOL_VERSION, sidecar: { version: this.opts.version, pid: process.pid }, sessionsDir: this.runtime.sessionsDir });
      return;
    }
    const platform = new SidecarPlatform(msg => this.send(msg), m, line => this.opts.log(line), { ignoreAgents: this.opts.ignoreClientAgents });
    this.platform = platform;
    try {
      this.runtime = await createHostRuntime(platform, { home: this.opts.home });
    } catch (e) {
      this.send({ type: 'helloReject', requestId: m.requestId, protocolVersion: SIDECAR_PROTOCOL_VERSION, reason: `runtime failed to start: ${msg(e)}` });
      await this.finish(`runtime failed: ${msg(e)}`, 1, false);
      return;
    }
    this.send({ type: 'helloOk', requestId: m.requestId, protocolVersion: SIDECAR_PROTOCOL_VERSION, sidecar: { version: this.opts.version, pid: process.pid }, sessionsDir: this.runtime.sessionsDir });
  }

  // Tear everything down once: pending platform RPCs fail, sessions flush, the wire gets shutdownOk when the shell asked for it
  private async finish(reason: string, code: number, ack: boolean) {
    if (this.done) return;
    this.opts.log(`sidecar finishing: ${reason}`);
    this.platform?.dispose(reason);
    this.views.clear();
    try { await this.runtime?.dispose(); } catch (e) { this.opts.log(`dispose failed: ${msg(e)}`); }
    if (ack) this.send({ type: 'shutdownOk' });
    this.done = true;
    this.wire.end();
    this.opts.onExit(code);
  }
}
