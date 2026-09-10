import { homedir } from 'node:os';
import type { FileHit } from '@shared/protocol';
import { PLATFORM_RPC_METHODS, type PlatformEvent, type PlatformMethod, type PlatformRequest, type ShellEnv, type ShellMsg, type SidecarMsg } from '@shared/sidecar';
import { NodeFiles } from '../nodeFiles';
import type { HostPlatform, PlanDocumentTarget, SettingsAffects, ToastLevel } from '../platform';

export type Hello = Extract<ShellMsg, { type: 'hello' }>;

// How long the shell gets to answer a platformRequest before the caller sees a failure
const RPC_TIMEOUT = 30_000;

// The HostPlatform of the sidecar: every IDE action becomes a platformRequest to the shell, facts come from hello and later
// platformEvents. Capabilities the shell did not declare fall back host-side where that makes sense (file search walks the
// workspace itself) and are otherwise logged, so a shell in its first iteration still gets a working chat
export class SidecarPlatform implements HostPlatform {
  private settings: Record<string, unknown>;
  private env: ShellEnv;
  private readonly caps: Set<PlatformMethod>;
  private pending = new Map<string, { method: PlatformMethod; resolve: (v: unknown) => void; reject: (e: Error) => void; timer: NodeJS.Timeout }>();
  private settingsListeners = new Set<(affects: SettingsAffects) => void>();
  private focusListeners = new Set<() => void>();
  private files = new NodeFiles(() => this.cwd());
  private seq = 0;
  private closed?: string;

  constructor(
    private send: (m: SidecarMsg) => void,
    hello: Hello,
    private stderr: (line: string) => void,
    private flags: { ignoreAgents?: boolean } = {},
  ) {
    this.settings = { ...hello.settings };
    this.env = { ...hello.env };
    this.caps = new Set(hello.client.capabilities);
  }

  blobBase(): string | undefined { return this.env.blobBase; }

  log(line: string) { this.stderr(line); }
  hostLanguage() { return this.env.hostLanguage; }
  home() { return homedir(); }
  cwd() { return this.env.cwd ?? homedir(); }

  readSetting(key: string): unknown {
    if (this.flags.ignoreAgents && key === 'agents') return undefined;
    return this.settings[key];
  }

  // The snapshot changes at once so SettingsCenter's emit right after the write already shows the new value; the shell persists and
  // echoes a settingsChanged event (a second, identical push, like VS Code's onDidChangeConfiguration)
  async writeSetting(key: string, value: unknown) {
    this.settings[key] = value;
    if (this.caps.has('writeSetting')) await this.rpc({ method: 'writeSetting', key, value });
  }

  onSettingsChanged(fn: (affects: SettingsAffects) => void) {
    this.settingsListeners.add(fn);
    return () => { this.settingsListeners.delete(fn); };
  }

  onWindowFocus(fn: () => void) {
    this.focusListeners.add(fn);
    return () => { this.focusListeners.delete(fn); };
  }

  toast(level: ToastLevel, text: string) {
    if (!this.notify({ method: 'toast', level, text })) this.stderr(`[toast:${level}] ${text}`);
  }

  // Without a terminal on the shell side the command is at least shown, so a login / install can be run by hand
  runInTerminal(title: string, command: string, args: string[], env?: Record<string, string | null>) {
    if (this.notify({ method: 'runInTerminal', title, command, args, env })) return;
    const prefix = Object.entries(env ?? {}).map(([k, v]) => (v === null ? `unset ${k};` : `${k}=${v}`)).join(' ');
    this.toast('error', `${title}: ${[prefix, command, ...args].filter(Boolean).join(' ')}`);
  }

  async openResolvedFile(path: string, line?: number) {
    if (this.caps.has('openResolvedFile')) await this.rpc({ method: 'openResolvedFile', path, line });
    else this.stderr(`openResolvedFile unsupported by the shell: ${path}${line ? `:${line}` : ''}`);
  }

  async openPlanDocument(target: PlanDocumentTarget) {
    if (this.caps.has('openPlanDocument')) await this.rpc({ method: 'openPlanDocument', target });
    else this.stderr('openPlanDocument unsupported by the shell');
  }

  openExternal(url: string) {
    if (!this.notify({ method: 'openExternal', url })) this.stderr(`openExternal unsupported by the shell: ${url}`);
  }

  async revealInOS(path: string) {
    if (this.caps.has('revealInOS')) await this.rpc({ method: 'revealInOS', path });
    else this.stderr(`revealInOS unsupported by the shell: ${path}`);
  }

  openInEditor(sessionId?: string) {
    if (!this.notify({ method: 'openInEditor', sessionId })) this.stderr('openInEditor unsupported by the shell');
  }

  async searchFiles(query: string): Promise<FileHit[]> {
    if (!this.caps.has('searchFiles')) return this.files.search(query);
    const hits = await this.rpc({ method: 'searchFiles', query });
    return Array.isArray(hits) ? (hits as FileHit[]).filter(h => h && typeof h.uri === 'string' && typeof h.path === 'string') : [];
  }

  // Fire-and-forget IDE actions; false when the shell did not declare the method
  private notify(request: PlatformRequest): boolean {
    if (!this.caps.has(request.method) || this.closed) return false;
    this.send({ type: 'platformRequest', request });
    return true;
  }

  private rpc(request: Extract<PlatformRequest, { method: (typeof PLATFORM_RPC_METHODS)[number] }>): Promise<unknown> {
    if (this.closed) return Promise.reject(new Error(`sidecar closed (${this.closed})`));
    const requestId = `p${++this.seq}`;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(requestId); reject(new Error(`${request.method}: no response from the shell within ${RPC_TIMEOUT / 1000}s`)); }, RPC_TIMEOUT);
      this.pending.set(requestId, { method: request.method, resolve, reject, timer });
      this.send({ type: 'platformRequest', requestId, request });
    });
  }

  onResponse(m: Extract<ShellMsg, { type: 'platformResponse' }>) {
    const p = this.pending.get(m.requestId);
    if (!p) { this.stderr(`platformResponse for unknown request ${m.requestId}`); return; }
    this.pending.delete(m.requestId);
    clearTimeout(p.timer);
    if (m.error !== undefined) p.reject(new Error(m.error)); else p.resolve(m.result);
  }

  onEvent(ev: PlatformEvent) {
    switch (ev.type) {
      case 'windowFocus': for (const fn of this.focusListeners) fn(); break;
      case 'settingsChanged': {
        this.settings = { ...ev.settings };
        const affects: SettingsAffects = section => section === undefined ? ev.keys.length > 0 : ev.keys.some(k => k === section || k.startsWith(`${section}.`));
        for (const fn of this.settingsListeners) fn(affects);
        break;
      }
      case 'envChanged': this.env = { ...this.env, ...ev.env }; break;
    }
  }

  // Every request still waiting fails now: the shell is gone or shutting down, nothing will answer
  dispose(reason: string) {
    this.closed = reason;
    for (const [, p] of this.pending) { clearTimeout(p.timer); p.reject(new Error(`${p.method}: ${reason}`)); }
    this.pending.clear();
  }
}
