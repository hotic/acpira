import { appendFileSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { DUTY_BRIEFS, type SubagentSummary } from '@shared/subagents';
import type { AgentTurn, PermissionBlock, SessionView, ToolCallBlock, Turn } from '@shared/transcript';
import { FAKE, bundleFakeAgent } from '../../../test/fakeAgentBundle';
import { Host, type View } from '../../lib/host';
import { HARNESSES, MODELS } from '../harnesses';
import { gatewayKey } from '../keys';
import { type Call, OUTAGE, startMeter } from '../meter';
import { claudeUsage, quotaSpent, readQuota, type QuotaSnap } from '../native';
import { cost } from '../prices';
import { type Target, materialize, targetsOf } from './bench';
import { JUDGE_MODEL, type Verdict, judge } from './judge';

// Review bench runner (docs/dev/evals.md). One run = one target's change in a fresh repository, a fake lead that
// summons the reviewer(s) through ask_agent exactly as a model would, the reviews judged against the target's known
// bugs. Spends real model calls unless --dry.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/review/run.ts --reviewers astra [--variant A,B,C]
//     [--set dev|holdout] [--target id,…] [--reps 1] [--parallel 1] [--judge deepseek-v4-pro|stub] [--policy FILE]
//     [--out DIR] [--timeout-min 45] [--dry]
//
// --reviewers names one reviewer (a single summon) or several joined by "+" (one group summon, P1); a comma separates
// configurations compared in the same matrix (`astra,astra+argon+opus`). --dry swaps every reviewer for the fake agent,
// which answers with a canned review naming the target's bugs, and judges with the stub: the whole pipeline without a
// model call. One line per run goes to DIR/results.ndjson (a run already there is skipped), one per model call to
// DIR/calls.ndjson; each run's reviews, verdicts and transcripts stay under DIR/runs/<run>

function arg(name: string, fallback?: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
}

const DRY = process.argv.includes('--dry');

interface Reviewer {
  // The persona as subagents.json holds it; the bench adds the reviewer duty, its brief and consult mode
  persona: { name: string; agent: string; model?: string; effort?: string };
  // Points the reviewer's CLI at the metering proxy (or at its own login) inside the run's homes
  setup(c: { base: string; home: string; acpiraHome: string }): { env: Record<string, string>; unmetered?: boolean };
  // The MODELS entry its metered calls are priced as
  model?: string;
}

// The reviewers: the existing 审查 persona's model and effort (Codex, GPT-6 Astra, xhigh), Gemini 4 Argon in the
// built-in agent, and Claude Code with Opus 5.5 on its own Max login
const REVIEWERS: Record<string, Reviewer> = {
  astra: {
    persona: { name: 'Astra', agent: 'codex', model: 'gpt-6-astra', effort: 'xhigh' },
    model: 'gpt-6-astra',
    setup: c => HARNESSES.codex!({ ...c, model: MODELS['gpt-6-astra']!, effort: 'xhigh' }),
  },
  argon: {
    persona: { name: 'Argon', agent: 'acpira', model: `evalgw/${MODELS['gemini-4-argon']!.id}`, effort: 'high' },
    model: 'gemini-4-argon',
    setup: c => HARNESSES.acpira!({ ...c, model: MODELS['gemini-4-argon']! }),
  },
  opus: {
    persona: { name: 'Opus', agent: 'claude', model: 'opus', effort: 'xhigh' },
    setup: c => HARNESSES['claude-max']!({ ...c, model: MODELS['claude-opus-5.5']! }),
  },
};

// What the reviewer gets besides the request. A: nothing (it finds the change itself); B: the handoff pack; C: the pack
// with the uses of the symbols the change defines (handoff.rs `Recipe::refs`)
const VARIANTS: Record<string, { changes: boolean; recipe?: Record<string, unknown> }> = {
  A: { changes: false },
  B: { changes: true, recipe: {} },
  C: { changes: true, recipe: { refs: true } },
};

// The strategy file the evolve loop tunes (scripts/eval/evolve.ts): the pack's recipe and the reviewer's standing brief.
// Absent = the shipped defaults
interface Policy { handoff?: Record<string, unknown>; briefs?: { reviewer?: string } }

const policy: Policy = arg('policy') ? JSON.parse(readFileSync(resolve(arg('policy')!), 'utf8')) as Policy : {};

// The caller's request, the same in every variant: the original requirement and where the change is, no account of it
// by its author (the ask_agent guidance for reviews)
function request(t: Target): string {
  return [
    'Review the uncommitted changes in this repository (`git status`, `git diff HEAD`; new files are added as intent-to-add, so the diff shows them too).',
    '',
    `The requirement they implement: ${t.requirement}`,
  ].join('\n');
}

// The dry run's review: one finding per known bug, in the shape the reviewer brief asks for, and one that matches none
function cannedReview(t: Target): string {
  return [
    ...t.bugs.map(b => `- high: ${b.files[0]}:1 ${b.keywords[0]}: ${b.description.split('. ')[0]}.`),
    '- low: README.md:1 a heading is misspelled.',
  ].join('\n');
}

const lastAgent = (v: SessionView | undefined): AgentTurn | undefined => {
  const t = v?.turns[v.turns.length - 1];
  return t?.role === 'agent' ? t : undefined;
};

// The last text a transcript's last agent turn wrote: what relay delivers as the reply (subagent_tree.rs `relay_reply`)
function replyOf(turns: Turn[]): string {
  const t = [...turns].reverse().find(x => x.role === 'agent') as AgentTurn | undefined;
  const b = [...(t?.blocks ?? [])].reverse().find(x => x.type === 'text' && x.markdown.trim());
  return b?.type === 'text' ? b.markdown : '';
}

function toolKinds(turns: Turn[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const t of turns) {
    if (t.role !== 'agent') continue;
    for (const b of t.blocks) if (b.type === 'tool_call') out[(b as ToolCallBlock).kind] = (out[(b as ToolCallBlock).kind] ?? 0) + 1;
  }
  return out;
}

// Every permission card is allowed once, the root's and the summoned children's alike (consult rounds ask before a
// command in some CLIs); a question is skipped. A card leaves once answered, so each id is clicked only once
function approve(view: View): () => void {
  const clicked = new Set<string>();
  const click = (sessionId: string, p: PermissionBlock) => {
    if (clicked.has(p.id)) return;
    const opt = p.options.find(x => x.kind === 'allow_once') ?? p.options.find(x => x.kind === 'allow_always') ?? p.options[0];
    if (!opt) return;
    clicked.add(p.id);
    view.post({ type: 'permission', sessionId, blockId: p.id, optionId: opt.id });
  };
  const timer = setInterval(() => {
    const v = view.active();
    if (!v) return;
    for (const b of lastAgent(v)?.blocks ?? []) {
      if (b.type === 'permission') click(v.id, b as PermissionBlock);
      else if (b.type === 'question' && !b.outcome && !clicked.has(b.id)) {
        clicked.add(b.id);
        view.post({ type: 'answer', sessionId: v.id, blockId: b.id, answers: {}, skip: true });
      }
    }
    for (const n of v.subagents ?? []) {
      for (const p of n.permissions ?? []) click(v.id, p);
      if (n.question && !clicked.has(n.question.id)) {
        clicked.add(n.question.id);
        view.post({ type: 'answer', sessionId: v.id, blockId: n.question.id, answers: {}, skip: true });
      }
    }
  }, 200);
  return () => clearInterval(timer);
}

interface ReviewerResult {
  name: string;
  state: string;
  hits: number[];
  others: number;
  findings: number;
  tools: number;
  toolKinds: Record<string, number>;
  wallS: number;
  requests: number; input: number; cacheRead: number; cacheWrite: number; output: number;
  costUsd?: number;
  unmetered?: boolean;
  callErrors: number;
  judgeError?: string;
  // The prompt's block types as the dry run's reviewer saw them (whether a pack arrived, and how it was staged)
  blocks?: string;
}

interface Job { t: Target; set: 'dev' | 'holdout'; dir: string; variant: string; cfg: string; rep: number }

const runIdOf = (j: Job) => `${j.t.id}.${j.variant}.${j.cfg}.${j.rep}`.replace(/[^\w.+-]/g, '_');

async function runOne(j: Job, o: { root: string; meterUrl: string; calls: Call[]; timeoutMin: number; judgeName: string }) {
  const runId = runIdOf(j);
  const dir = join(o.root, 'runs', runId);
  rmSync(dir, { recursive: true, force: true });
  const home = join(dir, 'home');
  const acpiraHome = join(dir, 'acpira');
  for (const d of [home, acpiraHome]) mkdirSync(d, { recursive: true });
  const work = materialize(j.t, j.dir, dir);
  const names = j.cfg.split('+');
  const variant = VARIANTS[j.variant]!;

  // Every reviewer's CLI configured in the run's homes, each on its own metering path so its calls stay apart
  const env: Record<string, string> = { HOME: home, PYTHONDONTWRITEBYTECODE: '1' };
  const unmetered = new Set<string>();
  if (!DRY) {
    for (const name of names) {
      const s = REVIEWERS[name]!.setup({ base: `${o.meterUrl}/r/${runId}.${name}/gw`, home, acpiraHome });
      Object.assign(env, s.env);
      if (s.unmetered) unmetered.add(name);
    }
  }
  const brief = policy.briefs?.reviewer ?? DUTY_BRIEFS.reviewer;
  const personas = names.map(name => ({
    id: name,
    ...(DRY ? { name: `Fake ${name}`, agent: 'fake' } : REVIEWERS[name]!.persona),
    mode: 'consult', duty: 'reviewer', brief, when: 'Review a change for real defects.', enabled: true,
  }));
  const groups = names.length > 1 ? [{ id: 'panel', name: 'Panel', members: names, when: 'Review a change from several sides at once.' }] : [];
  writeFileSync(join(acpiraHome, 'subagents.json'), JSON.stringify({ version: 1, personas, groups }, null, 2) + '\n');
  if (variant.changes) writeFileSync(join(acpiraHome, 'handoff.json'), JSON.stringify({ ...policy.handoff, ...variant.recipe }, null, 2) + '\n');

  let prompt = request(j.t);
  if (DRY) {
    const canned = join(dir, 'canned.md');
    writeFileSync(canned, cannedReview(j.t) + '\n');
    prompt = `relay: review=${canned}\n\n${prompt}`;
  }
  const ask = { agent: names.length > 1 ? 'panel' : names[0], prompt, title: 'Review the change', ...(variant.changes ? { changes: true } : {}) };

  const host = await Host.start({
    cwd: work, home: acpiraHome, defaultAgent: 'fake',
    agents: { fake: { name: 'Fake', command: process.execPath, args: [FAKE] } },
    env,
  });
  const view = await host.view();
  const streams = new Map<string, Turn[]>();
  view.onSubagent(ev => streams.set(ev.subagentId, ev.turns));
  const stopApproving = approve(view);
  const nodes = (): SubagentSummary[] => (view.active()?.subagents ?? []).filter(s => s.harness);
  const started = Date.now();
  let failure: string | undefined;
  let timedOut = false;
  try {
    await view.newSession('fake');
    await view.handle({ type: 'send', text: `mcp-call:ask_agent ${JSON.stringify(ask)}` }, 120_000);
    // The fake lead's reply is the call's own answer: the dispatch receipt, or why no round started
    const answer = lastAgent(view.active())?.blocks.find(b => b.type === 'text');
    const dispatched = answer?.type === 'text' ? answer.markdown : '';
    writeFileSync(join(dir, 'dispatch.json'), dispatched + '\n');
    if (!nodes().length) throw new Error(`no round started: ${dispatched.slice(0, 400)}`);
    try {
      await view.until(() => {
        const v = view.active();
        const ns = nodes();
        return !!v && !v.running && ns.length >= names.length && ns.every(n => n.state !== 'running')
          && v.turns.some(t => t.role === 'user' && t.autoReason === 'subagents') && lastAgent(v)?.stop !== undefined;
      }, o.timeoutMin * 60_000, 'the reviews');
    } catch (e) {
      timedOut = true;
      failure = String(e);
      for (const n of nodes().filter(x => x.state === 'running')) view.post({ type: 'cancelSubagent', sessionId: view.activeId!, subagentId: n.id });
      await view.until(() => nodes().every(n => n.state !== 'running'), 60_000, 'the cancelled reviews').catch(() => {});
    }
  } catch (e) {
    failure = String(e);
  }
  const wallS = Math.round((Date.now() - started) / 1000);
  stopApproving();

  // Each node's transcript, observed one at a time (a viewer follows one subagent at once)
  const v = view.active();
  for (const n of nodes()) {
    view.post({ type: 'observeSubagent', sessionId: v!.id, subagentId: n.id });
    await view.until(() => streams.has(n.id), 15_000, `the transcript of ${n.id}`).catch(() => {});
    view.post({ type: 'unobserveSubagent', sessionId: v!.id, subagentId: n.id });
  }
  const summaries = nodes();
  await host.dispose().catch(() => {});
  writeFileSync(join(dir, 'session.json'), JSON.stringify(v ?? null, null, 2));

  const perReviewer: ReviewerResult[] = [];
  const hits = new Set<number>();
  let others = 0;
  for (const name of names) {
    const n = summaries.find(s => s.harness?.persona === name);
    const turns = n ? streams.get(n.id) ?? [] : [];
    let review = replyOf(turns);
    let blocks: string | undefined;
    if (DRY) {
      const m = /\n\n\(prompt blocks: ([^)]*)\)\s*$/.exec(review);
      if (m) {
        blocks = m[1];
        review = review.slice(0, m.index);
      }
    }
    writeFileSync(join(dir, `review.${name}.md`), review + '\n');
    writeFileSync(join(dir, `node.${name}.json`), JSON.stringify({ summary: n ?? null, turns }, null, 2));
    const verdict: Verdict = review.trim() ? await judge(o.judgeName, j.t, review) : { findings: [], hits: [], others: 0, judge: o.judgeName };
    writeFileSync(join(dir, `verdict.${name}.json`), JSON.stringify(verdict, null, 2));
    for (const h of verdict.hits) hits.add(h);
    others += verdict.others;
    const mine = o.calls.filter(c => c.run === `${runId}.${name}`);
    const sum = (k: 'input' | 'cacheRead' | 'cacheWrite' | 'output') => mine.reduce((a, c) => a + c[k], 0);
    const native = unmetered.has(name) && REVIEWERS[name]!.persona.agent === 'claude' ? claudeUsage(home) : undefined;
    const usage = native ?? { requests: mine.length, input: sum('input'), cacheRead: sum('cacheRead'), cacheWrite: sum('cacheWrite'), output: sum('output') };
    const model = DRY ? undefined : native ? 'claude-opus-5.5' : REVIEWERS[name]!.model;
    perReviewer.push({
      name, state: n?.state ?? 'missing', hits: verdict.hits, others: verdict.others, findings: verdict.findings.length,
      tools: n?.toolCount ?? 0, toolKinds: toolKinds(turns),
      wallS: n?.endedAt && n.announcedAt ? Math.round((n.endedAt - n.announcedAt) / 1000) : wallS,
      ...usage,
      ...(model ? { costUsd: Math.round(cost({ model, ...usage }) * 1000) / 1000 } : {}),
      ...(unmetered.has(name) ? { unmetered: true } : {}),
      callErrors: mine.filter(c => c.error).length,
      ...(verdict.error ? { judgeError: verdict.error } : {}),
      ...(blocks !== undefined ? { blocks } : {}),
    });
  }
  const result = {
    runId, set: j.set, target: j.t.id, kind: j.t.kind, difficulty: j.t.difficulty, variant: j.variant, reviewers: j.cfg, rep: j.rep,
    bugs: j.t.bugs.length, hits: [...hits].sort(), found: hits.size, others,
    wallS, timedOut, failure, judge: o.judgeName, dry: DRY || undefined, perReviewer,
  };
  const outage = o.calls.some(c => c.run.startsWith(`${runId}.`) && c.error && OUTAGE.test(c.error)) && hits.size < j.t.bugs.length;
  appendFileSync(join(o.root, outage ? 'infra.ndjson' : 'results.ndjson'), JSON.stringify(result) + '\n');
  console.log(`[${runId}] found=${hits.size}/${j.t.bugs.length} others=${others} wall=${wallS}s ${perReviewer.map(r => `${r.name}:${r.state},tools=${r.tools}`).join(' ')}${failure ? ` failure=${failure.slice(0, 200)}` : ''}${outage ? ' (gateway outage: infra.ndjson)' : ''}`);
  return result;
}

async function main() {
  const set = (arg('set', 'dev') as 'dev' | 'holdout');
  const cfgs = (arg('reviewers') ?? '').split(',').filter(Boolean);
  const variants = (arg('variant', 'A') ?? 'A').split(',').filter(Boolean);
  const reps = Number(arg('reps', '1'));
  const parallel = Math.max(1, Number(arg('parallel', '1')));
  const judgeName = arg('judge', DRY ? 'stub' : JUDGE_MODEL)!;
  const timeoutMin = Number(arg('timeout-min', '45'));
  const { targets, dir } = targetsOf(set);
  const wanted = arg('target')?.split(',');
  const picked = wanted ? targets.filter(t => wanted.includes(t.id)) : targets;
  const unknownReviewer = cfgs.flatMap(c => c.split('+')).find(n => !REVIEWERS[n]);
  if (!cfgs.length || unknownReviewer || variants.some(x => !VARIANTS[x]) || !picked.length || (wanted && picked.length !== wanted.length)) {
    console.error(`usage: run.ts --reviewers ${Object.keys(REVIEWERS).join('|')}[+…][,…] [--variant ${Object.keys(VARIANTS).join(',')}] [--set dev|holdout] [--target id,…] [--reps N] [--parallel N] [--judge stub|MODEL] [--policy FILE] [--out DIR] [--dry]`);
    if (unknownReviewer) console.error(`unknown reviewer: ${unknownReviewer}`);
    if (wanted && picked.length !== wanted.length) console.error(`unknown target among: ${wanted.join(', ')}`);
    process.exit(2);
  }
  bundleFakeAgent();
  const root = resolve(arg('out') ?? join('/tmp/acpira-eval/review', new Date().toISOString().replace(/[:.]/g, '-')));
  mkdirSync(root, { recursive: true });
  const resultsFile = join(root, 'results.ndjson');
  const done = new Set(existsSync(resultsFile) ? readFileSync(resultsFile, 'utf8').split('\n').filter(Boolean).map(l => (JSON.parse(l) as { runId: string }).runId) : []);
  // Interleaved: every configuration and variant of one target and rep before the next, so a drift over the night
  // (a gateway slowing down, a quota window refilling) touches all cells alike
  const jobs: Job[] = [];
  for (let rep = 1; rep <= reps; rep++) {
    for (const t of picked) for (const cfg of cfgs) for (const variant of variants) jobs.push({ t, set, dir, variant, cfg, rep });
  }
  const todo = jobs.filter(j => !done.has(runIdOf(j)));
  const meter = await startMeter({ log: join(root, 'calls.ndjson'), keys: { gw: DRY ? 'dry' : gatewayKey() } });
  const natives = [...new Set(cfgs.flatMap(c => c.split('+')).filter(n => REVIEWERS[n]!.persona.agent === 'claude').map(() => 'claude'))];
  const before: QuotaSnap | undefined = !DRY && natives.length ? readQuota(natives) : undefined;
  if (before) appendFileSync(join(root, 'quota.ndjson'), JSON.stringify({ when: 'before', ...before }) + '\n');
  console.log(`results in ${root}: ${todo.length} of ${jobs.length} runs to go${DRY ? ' (dry)' : ''}`);
  try {
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(parallel, todo.length) }, async () => {
      while (next < todo.length) {
        const j = todo[next++]!;
        await runOne(j, { root, meterUrl: meter.url, calls: meter.calls, timeoutMin, judgeName }).catch(e => console.error(`[${runIdOf(j)}] ${e instanceof Error ? e.stack : e}`));
      }
    }));
  } finally {
    await meter.stop();
    if (before) {
      const after = readQuota(natives);
      appendFileSync(join(root, 'quota.ndjson'), JSON.stringify({ when: 'after', ...after }) + '\n');
      console.log(`quota spent (points): ${JSON.stringify(quotaSpent(before, after))}`);
    }
  }
}

await main();
