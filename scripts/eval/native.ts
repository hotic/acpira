import { execFileSync } from 'node:child_process';
import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { sidecarBin } from '../lib/sidecarBin';

// What the eval reads about runs on a CLI's own login, which the metering proxy never sees: the subscription's quota
// windows before and after, and the tokens the CLI recorded in its own session logs under the run's HOME

export interface QuotaWindow { id: string; remaining: number; resetsAt?: string }
export interface AgentQuota { plan?: string; windows: QuotaWindow[]; error?: string }
export type QuotaSnap = { at: string; agents: Record<string, AgentQuota> };

// `acpira quota --json` for the named agents (all three when none are named), without the logins' labels (an email
// address for some). Claude's usage endpoint rate-limits hard (docs/dev/agent-quirks.md, Claude accounts), so callers
// read it per batch, not per call
export function readQuota(agents: string[] = []): QuotaSnap {
  const at = new Date().toISOString();
  try {
    const out = execFileSync(sidecarBin({ build: true }), ['quota', '--json', ...agents], { encoding: 'utf8', timeout: 60_000 });
    const raw = JSON.parse(out) as Record<string, { plan?: string; quota?: { windows?: QuotaWindow[] } | null; error?: string }>;
    const snap: QuotaSnap = { at, agents: {} };
    for (const [agent, v] of Object.entries(raw)) {
      snap.agents[agent] = {
        ...(v.plan ? { plan: v.plan } : {}),
        windows: (v.quota?.windows ?? []).map(w => ({ id: w.id, remaining: w.remaining, ...(w.resetsAt ? { resetsAt: w.resetsAt } : {}) })),
        ...(v.error ? { error: v.error } : {}),
      };
    }
    return snap;
  } catch (e) {
    return { at, agents: Object.fromEntries((agents.length ? agents : ['codex', 'claude', 'devin']).map(a => [a, { windows: [], error: String(e) }])) };
  }
}

// Percentage points of each window spent between two snapshots (a window that reset in between shows as negative)
export function quotaSpent(before: QuotaSnap, after: QuotaSnap): Record<string, Record<string, number>> {
  const out: Record<string, Record<string, number>> = {};
  for (const [agent, b] of Object.entries(before.agents)) {
    const a = after.agents[agent];
    if (!a) continue;
    for (const w of b.windows) {
      const now = a.windows.find(x => x.id === w.id);
      if (now) (out[agent] ??= {})[w.id] = Math.round((w.remaining - now.remaining) * 1000) / 10;
    }
  }
  return out;
}

export interface Usage { requests: number; input: number; cacheRead: number; cacheWrite: number; output: number }

const zero = (): Usage => ({ requests: 0, input: 0, cacheRead: 0, cacheWrite: 0, output: 0 });

function walk(dir: string, out: string[] = []): string[] {
  if (!existsSync(dir)) return out;
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p, out);
    else if (name.endsWith('.jsonl')) out.push(p);
  }
  return out;
}

// Claude Code's own record of a HOME's sessions (~/.claude/projects/**/*.jsonl, subagent sidechains included):
// assistant lines carry message.usage, repeated on every line of one message (one line per content block), so each
// message id counts once. Input counts every prompt token, cached ones included, as the metering proxy does
export function claudeUsage(home: string): Usage {
  const u = zero();
  const seen = new Set<string>();
  for (const file of walk(join(home, '.claude/projects'))) {
    for (const line of readFileSync(file, 'utf8').split('\n')) {
      if (!line.includes('"usage"')) continue;
      let o: { type?: string; message?: { id?: string; usage?: Record<string, number> } };
      try {
        o = JSON.parse(line) as typeof o;
      } catch {
        continue;
      }
      const x = o.type === 'assistant' ? o.message?.usage : undefined;
      const id = o.message?.id;
      if (!x || !id || seen.has(id)) continue;
      seen.add(id);
      const cacheRead = x.cache_read_input_tokens ?? 0;
      const cacheWrite = x.cache_creation_input_tokens ?? 0;
      u.requests++;
      u.cacheRead += cacheRead;
      u.cacheWrite += cacheWrite;
      u.input += (x.input_tokens ?? 0) + cacheRead + cacheWrite;
      u.output += x.output_tokens ?? 0;
    }
  }
  return u;
}
