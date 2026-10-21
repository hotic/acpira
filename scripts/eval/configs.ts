import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { DUTY_BRIEFS } from '@shared/subagents';
import { HARNESSES, MODELS, addBuiltinProviders, type BuiltinProvider } from './harnesses';

// Named configurations for the Fusion comparison (docs/dev/evals.md, Fusion bench): a lead harness on a model, the
// session options picked once it starts, the helpers the run's subagents.json holds (each helper's CLI configured in
// the run's homes on its own metering path), and the built-in agent's sidekick. `run.ts --config a,b` runs them

export interface Helper {
  id: string;
  name: string;
  // The CLI the persona runs in: Claude Code runs on its own Max login (the claude-max harness), unmetered
  agent: 'codex' | 'acpira' | 'claude';
  // A MODELS key (what its calls are priced as)
  model: string;
  // The value of the CLI's own model select, for a CLI whose models are not MODELS ids (Claude Code: `opus`)
  select?: string;
  effort?: string;
  duty: 'scout' | 'executor' | 'reviewer';
  mode: 'consult' | 'work' | 'bypass' | 'inherit';
  when: string;
}

export interface Config {
  harness: string;
  // A MODELS key: what the lead runs on (for a native harness, what its pick selects; for the record and prices)
  model: string;
  // Session options set right after the session starts, by config id or category: a Devin Fusion pair, Claude's
  // model and effort, the built-in agent's model
  pick?: Record<string, string>;
  helpers?: Helper[];
  groups?: { id: string; name: string; members: string[]; when: string }[];
  // The built-in agent's sidekick: a MODELS key, listed by a provider of its own and picked as the session's
  // `sidekick` option (P1)
  sidekick?: string;
}

const REVIEW_WHEN = 'An independent review of a change before you call the task done: give it the requirement and where the change is, not your account of it.';

export const CONFIGS: Record<string, Config> = {
  // Single-model baselines on the CLIs' own harnesses
  'claude-opus': { harness: 'claude-max', model: 'claude-opus-5.5', pick: { model: 'opus', effort: 'high' } },
  'codex-astra': { harness: 'codex', model: 'gpt-6-astra' },
  // Devin's own Fusion: Cognition's recommended pair, and Astra with SWE-2 (option ids of Devin 3000.11.92)
  'devin-opus-luna': { harness: 'devin', model: 'claude-opus-5.5', pick: { model: 'fusion-claude-opus-5-5-high-sidekick-gpt-6-luna-high' } },
  'devin-astra-swe': { harness: 'devin', model: 'gpt-6-astra', pick: { model: 'fusion-gpt-6-astra-high-sidekick-swe-2-medium' } },
  // Acpira's orchestration: Claude Code with Opus 5.5 leads; a review group of Astra (Codex) and Argon (the built-in
  // agent) and an executor on GPT-6 Luna are there for it to use or not
  'acpira-orch': {
    harness: 'claude-max', model: 'claude-opus-5.5', pick: { model: 'opus', effort: 'high' },
    helpers: [
      { id: 'astra', name: 'Astra', agent: 'codex', model: 'gpt-6-astra', effort: 'high', duty: 'reviewer', mode: 'consult', when: REVIEW_WHEN },
      { id: 'argon', name: 'Argon', agent: 'acpira', model: 'gemini-4-argon', effort: 'high', duty: 'reviewer', mode: 'consult', when: REVIEW_WHEN },
      {
        id: 'luna', name: 'Luna', agent: 'acpira', model: 'gpt-6-luna', duty: 'executor', mode: 'inherit',
        when: 'Well-specified work you can describe precisely (edits across files, running and fixing tests, boilerplate), so you keep your own context for the decisions. Check its diff afterwards.',
      },
    ],
    groups: [{ id: 'reviewers', name: 'Reviewers', members: ['astra', 'argon'], when: 'Two independent reviews from other model families at once, for a change you are about to call done.' }],
  },
  // The built-in agent's sidekick on Devin's pairs, both models on the gateway (P1)
  'acpira-opus-luna': { harness: 'acpira', model: 'claude-opus-5.5', sidekick: 'gpt-6-luna' },
  'acpira-astra-swe': { harness: 'acpira', model: 'gpt-6-astra', sidekick: 'swe-2' },
  // The same lead without a sidekick, which tells the sidekick's effect apart from the harness's
  'acpira-astra': { harness: 'acpira', model: 'gpt-6-astra' },
  // The same lead with only a reviewer persona and no sidekick: does a second look alone buy what the sidekick did, at
  // less wall time? The reviewer is the review bench's `opus` (Claude Code, Opus at xhigh, Max login), a model family
  // other than the lead's
  'acpira-astra-rev': {
    harness: 'acpira', model: 'gpt-6-astra',
    helpers: [{ id: 'opus', name: 'Opus', agent: 'claude', model: 'claude-opus-5.5', select: 'opus', effort: 'xhigh', duty: 'reviewer', mode: 'consult', when: REVIEW_WHEN }],
  },
};

// The configuration's helpers that run on a CLI's own login, so the metering proxy sees none of their calls
export const nativeHelpers = (c: Config | undefined) => (c?.helpers ?? []).filter(h => h.agent === 'claude');

// Writes a configuration's helpers into the run's homes: each Codex helper's config (one per run: a second would
// share it), the built-in agent's providers (one per helper and one for the sidekick), Claude Code's login for a Claude
// helper, and subagents.json. Answers the session options to pick on top of the config's own, and the environment the
// helpers need from the sidecar (which every agent it spawns inherits)
export function setupHelpers(c: Config, o: { home: string; acpiraHome: string; baseFor: (id: string) => string }): { picks: Record<string, string>; env: Record<string, string> } {
  const builtin: BuiltinProvider[] = [];
  const env: Record<string, string> = {};
  const codex = (c.helpers ?? []).filter(h => h.agent === 'codex');
  if (codex.length > 1 || (codex.length && c.harness === 'codex')) throw new Error('one Codex config per run: at most one Codex helper, and none beside a Codex lead');
  // Claude Code's usage is read from the transcripts under the run's HOME, so only one Claude session may write there
  if (nativeHelpers(c).length > 1 || (nativeHelpers(c).length && c.harness.startsWith('claude'))) throw new Error('one Claude session per run: at most one Claude helper, and none beside a Claude lead');
  for (const h of c.helpers ?? []) {
    const spec = MODELS[h.model]!;
    if (h.agent === 'codex') HARNESSES.codex!({ model: spec, base: o.baseFor(h.id), home: o.home, acpiraHome: o.acpiraHome, effort: h.effort });
    else if (h.agent === 'claude') Object.assign(env, HARNESSES['claude-max']!({ model: spec, base: o.baseFor(h.id), home: o.home, acpiraHome: o.acpiraHome }).env);
    else builtin.push({ id: `eval-${h.id}`, api: spec.api, base: o.baseFor(h.id), models: [{ spec }] });
  }
  const picks: Record<string, string> = {};
  if (c.sidekick) {
    const spec = MODELS[c.sidekick]!;
    builtin.push({ id: 'eval-sidekick', api: spec.api, base: o.baseFor('sidekick'), models: [{ spec }] });
    picks.sidekick = `eval-sidekick/${spec.id}`;
  }
  if (c.harness.startsWith('acpira')) picks.model = `evalgw/${MODELS[c.model]!.id}`;
  if (builtin.length) addBuiltinProviders(o.acpiraHome, builtin);
  if (c.helpers?.length) {
    const personas = c.helpers.map(h => ({
      id: h.id, name: h.name, agent: h.agent,
      model: h.select ?? (h.agent === 'acpira' ? `eval-${h.id}/${MODELS[h.model]!.id}` : MODELS[h.model]!.id),
      ...(h.effort ? { effort: h.effort } : {}),
      mode: h.mode, duty: h.duty, brief: DUTY_BRIEFS[h.duty], when: h.when, enabled: true,
    }));
    mkdirSync(o.acpiraHome, { recursive: true });
    writeFileSync(join(o.acpiraHome, 'subagents.json'), JSON.stringify({ version: 1, personas, groups: c.groups ?? [] }, null, 2) + '\n');
  }
  return { picks, env };
}
