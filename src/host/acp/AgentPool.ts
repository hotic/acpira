import type { AgentId } from '@shared/transcript';
import type { AgentRegistry } from './AgentRegistry';
import { AgentProcess, type ClientHandlers } from './AgentProcess';
import { msg } from '../errors';

// Idle initialized processes (spawn + initialize, no session/new) expire if nobody takes them
const WARM_TTL_MS = 5 * 60_000;

type Warming = { state: 'warming'; promise: Promise<AgentProcess> };
type Ready = { state: 'ready'; proc: AgentProcess; timer: ReturnType<typeof setTimeout> };
type Slot = Warming | Ready;

export interface AgentPoolDeps {
  registry: () => AgentRegistry;
  log: (line: string) => void;
  spawnEnv?: (agent: AgentId, accountId: string) => Promise<Record<string, string> | undefined>;
}

// One idle CLI per agent + cwd + account. New sessions take it and only run session/new.
// initialize already advertised elicitation (idle stubs), so handing the process over does not change capabilities
export class AgentPool {
  private slots = new Map<string, Slot>();
  private inflight = new Set<Promise<AgentProcess>>();

  constructor(private readonly deps: AgentPoolDeps) {}

  static key(agent: AgentId, cwd: string, accountId?: string) {
    return `${agent}\0${cwd}\0${accountId ?? ''}`;
  }

  ensure(agent: AgentId, cwd: string, accountId?: string) {
    const key = AgentPool.key(agent, cwd, accountId);
    if (this.slots.has(key)) return;
    const slot: Warming = { state: 'warming', promise: undefined as unknown as Promise<AgentProcess> };
    slot.promise = this.spawnWarm(agent, cwd, accountId, key, slot);
    this.slots.set(key, slot);
    this.inflight.add(slot.promise);
    void slot.promise.catch(() => {}).finally(() => this.inflight.delete(slot.promise));
  }

  async take(agent: AgentId, cwd: string, accountId: string | undefined, handlers: ClientHandlers): Promise<AgentProcess | undefined> {
    const key = AgentPool.key(agent, cwd, accountId);
    const slot = this.slots.get(key);
    if (!slot) return undefined;
    this.slots.delete(key);
    if (slot.state === 'ready') {
      clearTimeout(slot.timer);
      return this.handoff(slot.proc, handlers);
    }
    try { return this.handoff(await slot.promise, handlers); }
    catch { return undefined; }
  }

  // Drop idle / still-warming processes so a registry or credential change cannot hand out a stale spawn
  invalidate() {
    const ready: AgentProcess[] = [];
    for (const slot of this.slots.values()) {
      if (slot.state === 'ready') { clearTimeout(slot.timer); ready.push(slot.proc); }
    }
    const pending = [...this.inflight];
    this.slots.clear();
    this.inflight.clear();
    for (const proc of ready) proc.kill();
    for (const p of pending) void p.then(proc => proc.kill()).catch(() => {});
  }

  dispose() { this.invalidate(); }

  private async spawnWarm(agent: AgentId, cwd: string, accountId: string | undefined, key: string, slot: Warming): Promise<AgentProcess> {
    try {
      const registry = this.deps.registry();
      const def = registry.get(agent);
      const bin = await registry.resolveBinary(agent);
      if (!bin) throw new Error(`no binary for ${agent}`);
      const extraEnv = accountId && this.deps.spawnEnv ? await this.deps.spawnEnv(agent, accountId) : undefined;
      const proc = await AgentProcess.spawn(def, bin, cwd, this.idleHandlers(key), extraEnv);
      this.deps.log(`warm ${agent}: initialize ok`);
      if (this.slots.get(key) === slot) this.slots.set(key, { state: 'ready', proc, timer: this.arm(key, proc) });
      return proc;
    } catch (e) {
      if (this.slots.get(key) === slot) this.slots.delete(key);
      this.deps.log(`warm ${agent} failed: ${msg(e)}`);
      throw e;
    }
  }

  private handoff(proc: AgentProcess, handlers: ClientHandlers) {
    if (!proc.alive) return undefined;
    proc.bind(handlers);
    return proc;
  }

  private arm(key: string, proc: AgentProcess) {
    const timer = setTimeout(() => {
      const cur = this.slots.get(key);
      if (cur?.state !== 'ready' || cur.proc !== proc) return;
      this.slots.delete(key);
      proc.kill();
      this.deps.log(`warm ${key.split('\0')[0]} expired`);
    }, WARM_TTL_MS);
    timer.unref();
    return timer;
  }

  private idleHandlers(key: string): ClientHandlers {
    return {
      onUpdate: () => {},
      onPermission: async () => ({ outcome: { outcome: 'cancelled' } }),
      onElicitation: async () => ({ action: 'cancel' }),
      onGrokQuestion: async () => ({ outcome: 'skip_interview' }),
      onStderr: line => this.deps.log(`warm stderr: ${line}`),
      onExit: () => {
        const cur = this.slots.get(key);
        if (cur?.state === 'ready') { clearTimeout(cur.timer); this.slots.delete(key); }
        else if (cur?.state === 'warming') this.slots.delete(key);
      },
    };
  }
}
