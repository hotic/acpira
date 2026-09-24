import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const RUST = fileURLToPath(new URL('../../rust/', import.meta.url));
const DEBUG_BIN = fileURLToPath(new URL(`../../rust/target/debug/acpira${process.platform === 'win32' ? '.exe' : ''}`, import.meta.url));

// The sidecar binary tests and probes run: ACPIRA_SIDECAR_BIN when set, otherwise the workspace's debug build, rebuilt first when
// `build` is set (cargo returns at once when nothing changed)
export function sidecarBin({ build = false } = {}): string {
  const explicit = process.env.ACPIRA_SIDECAR_BIN;
  if (explicit) return explicit;
  if (build) execFileSync('cargo', ['build', '--quiet', '--bin', 'acpira'], { cwd: RUST, stdio: 'inherit' });
  return DEBUG_BIN;
}

export interface SpawnSpec { command: string; args: string[]; verbatim: boolean }

// One built-in agent as the sidecar launches it (`acpira agents --json`)
export interface BuiltinAgent {
  id: string;
  name: string;
  command: string;
  args: string[];
  env: Record<string, string> | null;
  binary: string | null;
  spawn: SpawnSpec | null;
  // The exact initialize params the sidecar sends this agent
  initialize: { protocolVersion: number; clientInfo: { name: string; version: string }; clientCapabilities: Record<string, unknown> };
}

export function builtinAgents(bin = sidecarBin({ build: true })): BuiltinAgent[] {
  return (JSON.parse(execFileSync(bin, ['agents', '--json'], { encoding: 'utf8' })) as { agents: BuiltinAgent[] }).agents;
}

export function builtinAgent(id: string): BuiltinAgent {
  const agent = builtinAgents().find(a => a.id === id);
  if (!agent) throw new Error(`unknown agent: ${id}`);
  if (!agent.binary || !agent.spawn) throw new Error(`command not found: ${agent.command}`);
  return agent;
}
