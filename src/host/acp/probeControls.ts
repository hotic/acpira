import * as acp from '@agentclientprotocol/sdk';
import type { AgentDef } from './AgentRegistry';
import type { ConfigControl, SessionControls } from '@shared/transcript';
import type { AgentHealthStage, AgentRuntimeInfo } from '@shared/inventory';
import { applyModelSources } from '@shared/modelSources';
import { AgentProcess, AgentSpawnError } from './AgentProcess';
import { idleHandlers } from './AgentPool';
import { initControls, runtimeInfoOf } from './normalize';
import { readModelSources } from './modelSources';
import { isAuth } from './sessionErrors';
import { msg } from '../errors';

export interface ProbeInput {
  def: AgentDef;
  binary: string;
  cwd: string;
  extraEnv?: Record<string, string>;
  log: (line: string) => void;
  timeoutMs?: number;
}

export interface ProbeResult {
  options: ConfigControl[];
  runtime: AgentRuntimeInfo;
}

// A probe failure labelled with the stage it died at, for the agent page's status line:
// spawn_failed (the executable wouldn't start), handshake_failed (process up, initialize never completed),
// auth_required (initialize answered but session/new asked for sign-in). `ready` never throws — it's the success
export class ProbeFailure extends Error {
  constructor(readonly stage: Exclude<AgentHealthStage, 'ready'>, message: string) {
    super(message);
    this.name = 'ProbeFailure';
  }
}

// A throwaway spawn: initialize + session/new, read the configOptions, kill. The only way to see
// a CLI config change (a model added to its config file) without opening a real session.
// Any failing step still kills the process before the error propagates.
export async function probeAgentControls(input: ProbeInput): Promise<ProbeResult> {
  const { def, binary, cwd, extraEnv, log } = input;
  const timeoutMs = input.timeoutMs ?? 20_000;
  let proc: AgentProcess;
  try {
    proc = await AgentProcess.spawn(def, binary, cwd, idleHandlers(line => log(`probe ${def.command} stderr: ${line}`)), extraEnv);
  } catch (e) {
    // The child 'error' event means exec itself failed; anything else (early exit, init timeout, initialize error) is the handshake
    throw new ProbeFailure(e instanceof AgentSpawnError ? 'spawn_failed' : 'handshake_failed', msg(e));
  }
  let sessionId: string | undefined;
  try {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`probe ${def.command}: session/new timed out after ${timeoutMs}ms`)), timeoutMs);
    });
    const r = await Promise.race([
      proc.agent.request(acp.methods.agent.session.new, { cwd, mcpServers: [] }),
      timeout,
    ]).finally(() => clearTimeout(timer));
    sessionId = r.sessionId;
    const controls: SessionControls = { modes: [], options: [] };
    initControls(controls, r.modes, r.configOptions);
    applyModelSources(def.id, controls.options, await readModelSources(def.id, cwd));
    log(`probe ${def.command}: session/new ok · options ${controls.options.map(o => `${o.id}(${o.options.length})`).join(' ') || '-'}`);
    return { options: controls.options, runtime: runtimeInfoOf(proc.init) };
  } catch (e) {
    throw new ProbeFailure(isAuth(e) ? 'auth_required' : 'handshake_failed', msg(e));
  } finally {
    // Let the agent release the session before the process dies (mirrors AcpSession's close before dispose)
    if (sessionId && proc.alive && proc.init.agentCapabilities?.sessionCapabilities?.close) {
      try {
        await Promise.race([
          proc.agent.request(acp.methods.agent.session.close, { sessionId }),
          new Promise((_, reject) => setTimeout(() => reject(new Error('close timeout')), 3_000)),
        ]);
      } catch (e) { log(`probe ${def.command}: session/close failed: ${msg(e)}`); }
    }
    await proc.kill();
  }
}
