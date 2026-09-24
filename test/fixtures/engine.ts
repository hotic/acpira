import { readFileSync } from 'node:fs';
import type { AgentTurn, DiffLine, Turn } from '../../src/shared/transcript';

// Golden engine output (the Rust suite in rust/crates/acpira-host/tests/engine/golden.rs keeps these in step with the engine):
// the webview suites render exactly what the sidecar sends
interface NormalizeCase { name: string; steps: { at: number; update: unknown; turns: Turn[] }[] }
interface DiffCase { name: string; oldText: string; newText: string; lines: DiffLine[] }

const load = <T>(file: string): T[] => JSON.parse(readFileSync(new URL(file, import.meta.url), 'utf8')) as T[];
const normalize = load<NormalizeCase>('./engine-normalize.json');
const diffs = load<DiffCase>('./engine-diff.json');

function find<T extends { name: string }>(list: T[], name: string): T {
  const hit = list.find(c => c.name === name);
  if (!hit) throw new Error(`no engine fixture ${name}`);
  return hit;
}

// The transcript after step `step` of a recorded update sequence (negative counts from the end); a fresh copy every call
export function turnsAfter(name: string, step = -1): Turn[] {
  const steps = find(normalize, name).steps;
  return structuredClone(steps.at(step)!.turns);
}

export function agentTurn(name: string, step = -1): AgentTurn {
  const turn = turnsAfter(name, step)[0];
  if (turn?.role !== 'agent') throw new Error(`fixture ${name} has no agent turn`);
  return turn;
}

export function diffCase(name: string): DiffCase {
  return structuredClone(find(diffs, name));
}
