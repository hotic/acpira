import * as acp from '@agentclientprotocol/sdk';
import type { AgentDef } from './AgentRegistry';
import type { ConfigControl, SessionControls } from '@shared/transcript';
import type { AgentRuntimeInfo } from '@shared/inventory';
import { applyModelSources } from '@shared/modelSources';
import { AgentProcess } from './AgentProcess';
import { idleHandlers } from './AgentPool';
import { initControls, runtimeInfoOf } from './normalize';
import { readModelSources } from './modelSources';

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

// A throwaway spawn: initialize + session/new, read the configOptions, kill. The only way to see
// a CLI config change (a model added to its config file) without opening a real session.
// Any failing step still kills the process before the error propagates.
export async function probeAgentControls(input: ProbeInput): Promise<ProbeResult> {
  const { def, binary, cwd, extraEnv, log } = input;
  const timeoutMs = input.timeoutMs ?? 20_000;
  const proc = await AgentProcess.spawn(def, binary, cwd, idleHandlers(line => log(`probe ${def.command} stderr: ${line}`)), extraEnv);
  try {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const timeout = new Promise<never>((_, reject) => {
      timer = setTimeout(() => reject(new Error(`probe ${def.command}: session/new timed out after ${timeoutMs}ms`)), timeoutMs);
    });
    const r = await Promise.race([
      proc.agent.request(acp.methods.agent.session.new, { cwd, mcpServers: [] }),
      timeout,
    ]).finally(() => clearTimeout(timer));
    const controls: SessionControls = { modes: [], options: [] };
    initControls(controls, r.modes, r.configOptions);
    applyModelSources(def.id, controls.options, await readModelSources(def.id, cwd));
    log(`probe ${def.command}: session/new ok · options ${controls.options.map(o => `${o.id}(${o.options.length})`).join(' ') || '-'}`);
    return { options: controls.options, runtime: runtimeInfoOf(proc.init) };
  } finally {
    await proc.kill();
  }
}
