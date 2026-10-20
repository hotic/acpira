import { spawn } from 'node:child_process';
import { appendFileSync, existsSync, mkdirSync, openSync, readFileSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import type { AgentTurn, PermissionBlock, QuestionBlock, SessionView } from '@shared/transcript';
import { Host, type View } from '../lib/host';
import { HARNESSES, MODELS } from './harnesses';
import { readQuota } from './native';
import { DEFAULT_RECIPE, SCHEMA, type Policy, defaultsPatch, effective, samePolicy, validate } from './policy';
import { type Target, targetsOf } from './review/bench';

// The evolve loop (docs/dev/evals.md, Evolve loop): tunes the strategy file the review bench reads (policy.ts: the
// handoff pack's recipe and the reviewer's standing brief). Each round a proposer reads the ledger, the failure analysis
// of the last evaluation and the schema, and proposes one candidate with a one-line hypothesis: Claude Code on its own
// Max login in a session of the loop's own ACPIRA_HOME (DIR/proposer/acpira, `pnpm harness --home` shows it) whose
// permission cards are all rejected, so it only reads; `--proposer stub` takes the next of a fixed list instead. The
// candidate and the incumbent then run side by side on the dev set (two review bench processes started together, same
// reviewers, variant and targets), and the rule accepts or rejects it: more bugs found beyond the noise measured by
// rerunning one cell wins; within the noise, a candidate that finds at least as many and is 10% faster wins. Every round
// goes into DIR/ledger.ndjson. The loop stops at --rounds, --hours, --max-rejects rejections in a row, or when Claude's
// weekly quota is under --quota-floor; then the incumbent and the shipped defaults run side by side on the held-out set,
// and the morning report and the defaults patch go to output/evolve/<stamp>/ (applied by hand once approved).
// A rerun with the same DIR resumes: a round interrupted before it recorded keeps its proposal and the runs the bench
// already finished. --final-only reruns just the wrap-up on DIR's incumbent. --dry: stub proposer, fake reviewers,
// stub judge.
//
//   pnpm exec tsx --tsconfig tsconfig.host.json scripts/eval/evolve.ts --out DIR [--rounds 2] [--hours 10]
//     [--max-rejects 3] [--reviewers astra] [--variant B] [--reps 1] [--parallel 4] [--proposer claude|stub]
//     [--noise-from DIR] [--quota-floor 0.25] [--no-final | --final-only] [--dry]

function arg(name: string, fallback?: string): string | undefined {
  const i = process.argv.indexOf(`--${name}`);
  return i > 0 ? process.argv[i + 1] : fallback;
}

const REPO = resolve(import.meta.dirname, '../..');
const DRY = process.argv.includes('--dry');
const OUT = resolve(arg('out') ?? join('/tmp/acpira-eval/evolve', new Date().toISOString().replace(/[:.]/g, '-')));
const REVIEWERS = arg('reviewers', 'astra')!;
const VARIANT = arg('variant', 'B')!;
const REPS = Number(arg('reps', '1'));
const PARALLEL = Number(arg('parallel', '4'));
const ROUNDS = Number(arg('rounds', '2'));
const DEADLINE = Date.now() + Number(arg('hours', '10')) * 3_600_000;
const MAX_REJECTS = Number(arg('max-rejects', '3'));
const PROPOSER = DRY ? 'stub' : arg('proposer', 'claude')!;
const FLOOR = Number(arg('quota-floor', '0.25'));
const NOISE_FROM = arg('noise-from');
const LEDGER = join(OUT, 'ledger.ndjson');
const INCUMBENT = join(OUT, 'incumbent.json');
// A within-noise candidate must be this much faster (median wall per run) to win on time
const FASTER = 0.9;

interface Bug { files: string[]; description: string }
interface Result {
  runId: string; target: string; variant: string; reviewers: string; rep: number; bugs: number; hits: number[]; found: number;
  others: number; wallS: number; timedOut: boolean; failure?: string;
  perReviewer: { name: string; state: string; costUsd?: number }[];
}
interface Side { runs: number; found: number; bugs: number; others: number; wallMedianS: number; costUsd: number; failed: number; byTarget: Record<string, number> }
interface Entry {
  round: number; at: string; proposer: string; hypothesis?: string; candidate?: unknown; decision: 'accepted' | 'rejected' | 'invalid';
  reason: string; noise?: number; incumbent?: Side; challenger?: Side; wallS: number;
}

const readResults = (dir: string): Result[] => (existsSync(join(dir, 'results.ndjson')) ? readFileSync(join(dir, 'results.ndjson'), 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l) as Result) : []);
const ofCell = (rs: Result[]) => rs.filter(r => r.reviewers === REVIEWERS && r.variant === VARIANT);

function summarize(rs: Result[]): Side {
  const walls = rs.map(r => r.wallS).sort((a, b) => a - b);
  const mid = walls.length >> 1;
  const byTarget: Record<string, number> = {};
  for (const r of rs) byTarget[r.target] = (byTarget[r.target] ?? 0) + r.found;
  return {
    runs: rs.length,
    found: rs.reduce((a, r) => a + r.found, 0),
    bugs: rs.reduce((a, r) => a + r.bugs, 0),
    others: rs.reduce((a, r) => a + r.others, 0),
    wallMedianS: !walls.length ? 0 : walls.length % 2 ? walls[mid]! : Math.round((walls[mid - 1]! + walls[mid]!) / 2),
    costUsd: Math.round(rs.reduce((a, r) => a + r.perReviewer.reduce((b, p) => b + (p.costUsd ?? 0), 0), 0) * 100) / 100,
    failed: rs.filter(r => r.timedOut || r.failure || r.perReviewer.some(p => p.state !== 'completed')).length,
    byTarget,
  };
}

// Bug outcomes that flip between two runs of one cell under one policy on the same target, per outcome compared. Each
// group holds the runs of one policy: the review A/B's reruns of the shipped defaults (--noise-from) and the incumbent's
// own evaluations, which rerun the same cell every round
function flipRate(groups: Result[][]): { rate: number; outcomes: number } {
  let flips = 0;
  let outcomes = 0;
  for (const group of groups) {
    const byTarget = new Map<string, Result[]>();
    for (const r of group) byTarget.set(r.target, [...(byTarget.get(r.target) ?? []), r]);
    for (const rs of byTarget.values()) {
      for (let i = 1; i < rs.length; i++) {
        const [a, b] = [new Set(rs[i - 1]!.hits), new Set(rs[i]!.hits)];
        for (let bug = 0; bug < rs[i]!.bugs; bug++) {
          outcomes++;
          if (a.has(bug) !== b.has(bug)) flips++;
        }
      }
    }
  }
  return { rate: outcomes ? flips / outcomes : 0, outcomes };
}

// The policy in force when a round started: the last candidate accepted before it
function incumbentAt(entries: Entry[], round: number): Policy {
  let p: Policy = {};
  for (const e of entries) if (e.round < round && e.decision === 'accepted') p = e.candidate as Policy;
  return p;
}

// The rule (docs/dev/evals.md): quality first, beyond the noise; time only between candidates within it. `onTime` marks
// an acceptance on time alone, which main checks with a second evaluation
function decide(inc: Side, cand: Side, noise: number): { accepted: boolean; onTime?: boolean; reason: string } {
  const d = cand.found - inc.found;
  if (cand.failed > inc.failed) return { accepted: false, reason: `${cand.failed} failed runs against ${inc.failed}` };
  if (d > noise && d >= 1) return { accepted: true, reason: `found ${d} more bugs, beyond the noise of ${noise.toFixed(1)}` };
  if (d < -noise) return { accepted: false, reason: `found ${-d} fewer bugs, beyond the noise of ${noise.toFixed(1)}` };
  if (d >= 0 && cand.wallMedianS <= inc.wallMedianS * FASTER) {
    return { accepted: true, onTime: true, reason: `within the noise (${d >= 0 ? '+' : ''}${d}) and faster: median ${cand.wallMedianS}s against ${inc.wallMedianS}s` };
  }
  return { accepted: false, reason: `within the noise (${d >= 0 ? '+' : ''}${d} of ${noise.toFixed(1)}) and not 10% faster (median ${cand.wallMedianS}s against ${inc.wallMedianS}s)` };
}

// One review bench process on a policy; resolves with its exit code, its output in `log`
function bench(o: { policy: Policy; out: string; set: 'dev' | 'holdout'; log: string }): Promise<number> {
  mkdirSync(o.out, { recursive: true });
  const file = join(o.out, 'policy.json');
  writeFileSync(file, JSON.stringify(o.policy, null, 2) + '\n');
  const args = [
    'exec', 'tsx', '--tsconfig', 'tsconfig.host.json', 'scripts/eval/review/run.ts', '--reviewers', REVIEWERS, '--variant', VARIANT,
    '--set', o.set, '--reps', String(REPS), '--parallel', String(PARALLEL), '--policy', file, '--out', o.out, ...(DRY ? ['--dry'] : []),
  ];
  const log = openSync(o.log, 'a');
  return new Promise((done, fail) => {
    const p = spawn('pnpm', args, { cwd: REPO, stdio: ['ignore', log, log], env: process.env });
    p.on('error', fail);
    p.on('exit', code => done(code ?? 1));
  });
}

// Policy and incumbent side by side, started together so both meet the same gateway; each side's own results
async function paired(dir: string, set: 'dev' | 'holdout', a: Policy, b: Policy): Promise<[Result[], Result[]]> {
  const [ca, cb] = await Promise.all([
    bench({ policy: a, out: join(dir, 'incumbent'), set, log: join(dir, 'incumbent.log') }),
    bench({ policy: b, out: join(dir, 'challenger'), set, log: join(dir, 'challenger.log') }),
  ]);
  if (ca !== 0 || cb !== 0) throw new Error(`review bench exited ${ca} / ${cb} (logs in ${dir})`);
  return [ofCell(readResults(join(dir, 'incumbent'))), ofCell(readResults(join(dir, 'challenger')))];
}

// What the proposer reads about an evaluation: per target the bugs the runs missed, with the review that missed them
function analysis(sides: { label: string; dir: string }[], targets: Target[]): string {
  const out = ['# Failure analysis', '', `Cell: reviewers ${REVIEWERS}, variant ${VARIANT} (dev set). A bug counts as found when the judge matched a finding of the review to it.`, ''];
  for (const side of sides) {
    const rs = ofCell(readResults(side.dir));
    if (!rs.length) continue;
    const s = summarize(rs);
    out.push(`## ${side.label}: ${s.found} of ${s.bugs} bugs found in ${s.runs} runs, ${s.others} findings matching no known bug, median ${s.wallMedianS}s a run`, '');
    for (const r of rs) {
      const t = targets.find(x => x.id === r.target);
      if (!t || r.found === r.bugs) continue;
      out.push(`### ${r.target} (${t.kind}, ${t.difficulty}), run ${r.runId}`, '', `Requirement: ${t.requirement}`, '');
      t.bugs.forEach((b: Bug, i: number) => {
        if (!r.hits.includes(i)) out.push(`- Missed: ${b.description} (in ${b.files.join(', ')})`);
      });
      for (const p of r.perReviewer) {
        const f = join(side.dir, 'runs', r.runId, `review.${p.name}.md`);
        const review = existsSync(f) ? readFileSync(f, 'utf8').trim() : '';
        out.push('', `${p.name}'s review (${p.state}):`, '', '```', review.length > 2500 ? `${review.slice(0, 2500)}\n…` : review || '(none)', '```');
      }
      out.push('');
    }
  }
  return out.join('\n');
}

// The stub proposer: the next untried change of a fixed list, applied over the incumbent
const STUBS: { hypothesis: string; change: Policy }[] = [
  { hypothesis: 'Listing the uses of changed definitions points the reviewer at broken callers.', change: { handoff: { refs: true } } },
  { hypothesis: 'Plain hunks keep the reviewer on the change instead of whole functions.', change: { handoff: { functionContext: false } } },
  { hypothesis: 'Whole files only when they are short keep the pack about the change.', change: { handoff: { fullTextLines: 120 } } },
];

function stubPropose(round: number, incumbent: Policy): { hypothesis: string; policy: Policy } {
  const s = STUBS[(round - 1) % STUBS.length]!;
  return { hypothesis: s.hypothesis, policy: { handoff: { ...incumbent.handoff, ...s.change.handoff }, ...(incumbent.briefs ? { briefs: incumbent.briefs } : {}) } };
}

const lastAgent = (v: SessionView | undefined): AgentTurn | undefined => {
  const t = v?.turns[v.turns.length - 1];
  return t?.role === 'agent' ? t : undefined;
};

// Read only: every permission card is rejected (a write or a command), every question skipped
function rejectAll(view: View): () => void {
  const seen = new Set<string>();
  const timer = setInterval(() => {
    const v = view.active();
    for (const b of lastAgent(v)?.blocks ?? []) {
      if (b.type === 'permission') {
        const p = b as PermissionBlock;
        const opt = p.options.find(x => x.kind === 'reject_once') ?? p.options.find(x => x.kind.startsWith('reject'));
        if (!opt || seen.has(p.id)) continue;
        seen.add(p.id);
        view.post({ type: 'permission', sessionId: v!.id, blockId: p.id, optionId: opt.id });
      } else if (b.type === 'question') {
        const q = b as QuestionBlock;
        if (q.outcome || seen.has(q.id)) continue;
        seen.add(q.id);
        view.post({ type: 'answer', sessionId: v!.id, blockId: q.id, answers: {}, skip: true });
      }
    }
  }, 200);
  return () => clearInterval(timer);
}

async function pick(view: View, picks: Record<string, string>) {
  for (const [key, value] of Object.entries(picks)) {
    const find = () => view.active()?.controls.options.find(x => x.id === key || x.category === key);
    await view.until(() => !!find(), 60_000, `session option ${key}`);
    const opt = find()!;
    if (opt.value === value) continue;
    view.post({ type: 'setConfig', sessionId: view.active()!.id, configId: opt.id, value });
    await view.until(() => find()?.value === value, 60_000, `${key} = ${value}`);
  }
}

const proposerPrompt = (round: number) => `You tune the strategy file of a code-review bench: the handoff pack a summoned reviewer gets with its task (what the pack holds and how big it may be) and the reviewer's standing instructions. A reviewer here is an AI coding agent asked to review an uncommitted change; it is scored by how many of the change's known bugs its review finds.

Read these files in this folder:
- schema.json: every knob you may set, its range and what it does
- incumbent.json: the policy in force (a knob left out keeps its default, defaults.json)
- ledger.md: every earlier round, its hypothesis, the candidate it tried, what it scored against the incumbent and whether it was accepted
- analysis.md: the bugs the last evaluation missed, with the reviews that missed them

Then propose exactly one candidate for round ${round}: a policy that differs from the incumbent and from every candidate the ledger already tried, with one sentence saying why it should find more bugs (or as many, faster). Do not write or run anything; read only. A candidate is accepted only if it finds more bugs than the incumbent beyond the noise of rerunning one cell, so aim for a change with a real effect, and do not tailor the instructions to the particular bugs listed: the final policy is checked on targets you never see.

End your reply with the candidate as one fenced JSON block, nothing after it:
\`\`\`json
{ "hypothesis": "…", "policy": { "handoff": { … }, "briefs": { "reviewer": "…" } } }
\`\`\``;

function ledgerMarkdown(entries: Entry[]): string {
  if (!entries.length) return '# Ledger\n\nNo rounds yet.\n';
  const out = ['# Ledger', ''];
  for (const e of entries) {
    out.push(`## Round ${e.round}: ${e.decision}`, '', `Hypothesis: ${e.hypothesis ?? '(none)'}`, '', 'Candidate:', '', '```json', JSON.stringify(e.candidate ?? null, null, 2), '```', '');
    if (e.incumbent && e.challenger) out.push(`Incumbent ${e.incumbent.found}/${e.incumbent.bugs}, median ${e.incumbent.wallMedianS}s; candidate ${e.challenger.found}/${e.challenger.bugs}, median ${e.challenger.wallMedianS}s.`, '');
    out.push(`Decision: ${e.reason}`, '');
  }
  return out.join('\n');
}

const parseProposal = (text: string): { hypothesis?: string; policy?: unknown; error?: string } => {
  const blocks = [...text.matchAll(/```json\s*\n([\s\S]*?)```/g)];
  const last = blocks[blocks.length - 1]?.[1];
  if (!last) return { error: 'no JSON block in the reply' };
  try {
    const j = JSON.parse(last) as { hypothesis?: unknown; policy?: unknown };
    return { hypothesis: typeof j.hypothesis === 'string' ? j.hypothesis : undefined, policy: j.policy };
  } catch (e) {
    return { error: `the JSON block does not parse: ${String(e)}` };
  }
};

async function claudePropose(brief: string, round: number): Promise<{ hypothesis?: string; policy?: unknown; error?: string }> {
  const home = join(OUT, 'proposer', 'home');
  const acpiraHome = join(OUT, 'proposer', 'acpira');
  mkdirSync(home, { recursive: true });
  mkdirSync(acpiraHome, { recursive: true });
  const setup = HARNESSES['claude-max']!({ model: MODELS['claude-opus-5.5']!, base: '', home, acpiraHome });
  const host = await Host.start({ cwd: brief, home: acpiraHome, defaultAgent: 'claude', env: { HOME: home, ...setup.env } });
  const view = await host.view();
  const stop = rejectAll(view);
  try {
    await view.newSession('claude');
    await pick(view, { model: 'opus', effort: 'high' });
    await view.handle({ type: 'send', text: proposerPrompt(round) }, 30 * 60_000);
    const b = [...(lastAgent(view.active())?.blocks ?? [])].reverse().find(x => x.type === 'text' && x.markdown.trim());
    const text = b?.type === 'text' ? b.markdown : '';
    writeFileSync(join(brief, '..', 'proposal.md'), text);
    return parseProposal(text);
  } finally {
    stop();
    await host.dispose().catch(() => {});
  }
}

function claudeWeekly(): number | undefined {
  const q = readQuota(['claude']);
  return q.agents.claude?.windows.find(w => w.id === 'weekly')?.remaining;
}

const needsClaude = () => PROPOSER === 'claude' || REVIEWERS.split('+').includes('opus');

async function main() {
  mkdirSync(OUT, { recursive: true });
  const { targets } = targetsOf('dev');
  const entries: Entry[] = existsSync(LEDGER) ? readFileSync(LEDGER, 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l) as Entry) : [];
  let incumbent: Policy = existsSync(INCUMBENT) ? JSON.parse(readFileSync(INCUMBENT, 'utf8')) as Policy : {};
  const prior = NOISE_FROM ? ofCell(readResults(resolve(NOISE_FROM))) : [];
  console.log(`evolve in ${OUT}${DRY ? ' (dry)' : ''}: ${entries.length} rounds so far, proposer ${PROPOSER}, cell ${REVIEWERS} ${VARIANT}`);
  let stopped = '';
  for (;;) {
    const round = entries.length + 1;
    const streak = entries.slice().reverse().findIndex(e => e.decision === 'accepted');
    if (round > ROUNDS) stopped = `${ROUNDS} rounds done`;
    else if (Date.now() > DEADLINE) stopped = 'out of time';
    else if ((streak === -1 ? entries.length : streak) >= MAX_REJECTS) stopped = `${MAX_REJECTS} rejections in a row`;
    else if (!DRY && needsClaude()) {
      const left = claudeWeekly();
      if (left !== undefined && left < FLOOR) stopped = `Claude's weekly quota is at ${Math.round(left * 100)}%, under the floor of ${Math.round(FLOOR * 100)}%`;
    }
    if (stopped) break;
    const started = Date.now();
    const dir = join(OUT, `round-${round}`);
    const brief = join(dir, 'brief');
    mkdirSync(brief, { recursive: true });
    // The last round's evaluation, or before the first one the A/B's runs of the same cell
    const last = entries.length ? [{ label: 'Incumbent', dir: join(OUT, `round-${round - 1}`, 'incumbent') }, { label: 'Candidate', dir: join(OUT, `round-${round - 1}`, 'challenger') }] : NOISE_FROM ? [{ label: 'The review A/B', dir: resolve(NOISE_FROM) }] : [];
    writeFileSync(join(brief, 'schema.json'), JSON.stringify(SCHEMA, null, 2) + '\n');
    writeFileSync(join(brief, 'defaults.json'), JSON.stringify(effective({}), null, 2) + '\n');
    writeFileSync(join(brief, 'incumbent.json'), JSON.stringify(incumbent, null, 2) + '\n');
    writeFileSync(join(brief, 'ledger.md'), ledgerMarkdown(entries));
    writeFileSync(join(brief, 'analysis.md'), last.length ? analysis(last, targets) : '# Failure analysis\n\nNo evaluation yet.\n');
    // An interrupted round keeps what it was evaluating
    const saved = join(dir, 'proposal.json');
    let proposal: { hypothesis?: string; policy?: unknown; error?: string };
    if (existsSync(saved)) {
      proposal = JSON.parse(readFileSync(saved, 'utf8')) as typeof proposal;
      console.log(`round ${round}: resuming its saved proposal`);
    } else {
      console.log(`round ${round}: asking the ${PROPOSER} proposer`);
      proposal = PROPOSER === 'stub' ? stubPropose(round, incumbent) : await claudePropose(brief, round);
      writeFileSync(saved, JSON.stringify(proposal, null, 2) + '\n');
    }
    const record = (e: Omit<Entry, 'round' | 'at' | 'proposer' | 'wallS'>) => {
      const entry: Entry = { round, at: new Date().toISOString(), proposer: PROPOSER, ...e, wallS: Math.round((Date.now() - started) / 1000) };
      entries.push(entry);
      appendFileSync(LEDGER, JSON.stringify(entry) + '\n');
      console.log(`round ${round}: ${entry.decision}, ${entry.reason}`);
    };
    const v: { policy?: Policy; error?: string } = proposal.error ? { error: proposal.error } : validate(proposal.policy);
    const tried = entries.some(e => e.candidate && v.policy && samePolicy(e.candidate as Policy, v.policy));
    if (!v.policy || samePolicy(v.policy, incumbent) || tried) {
      record({ hypothesis: proposal.hypothesis, candidate: proposal.policy, decision: 'invalid', reason: v.error ?? (tried ? 'tried before' : 'the same as the incumbent') });
      continue;
    }
    let [inc, cand] = await paired(dir, 'dev', incumbent, v.policy);
    // The incumbent reruns the same cell every round, so its earlier evaluations under the same policy sharpen the noise
    const same = entries.filter(e => e.incumbent && samePolicy(incumbentAt(entries, e.round), incumbent));
    const earlier = same.flatMap(e => ['incumbent', 'confirm/incumbent'].flatMap(side => ofCell(readResults(join(OUT, `round-${e.round}`, side)))));
    const judge = () => {
      const { rate } = flipRate([[...(samePolicy(incumbent, {}) ? prior : []), ...earlier, ...inc]]);
      const [si, sc] = [summarize(inc), summarize(cand)];
      const noise = rate * si.bugs;
      return { si, sc, noise, d: decide(si, sc, noise) };
    };
    let { si, sc, noise, d } = judge();
    // One evaluation's median wall time varies about as much as the 10% asked (one policy measured 138 to 172 s over
    // four rounds, docs/dev/evals.md), so a win on time alone is checked with a second evaluation and both pooled
    if (d.onTime) {
      console.log(`round ${round}: ${d.reason}; evaluating both once more`);
      const [inc2, cand2] = await paired(join(dir, 'confirm'), 'dev', incumbent, v.policy);
      [inc, cand] = [[...inc, ...inc2], [...cand, ...cand2]];
      ({ si, sc, noise, d } = judge());
      d = { ...d, reason: `${d.reason}, two evaluations pooled` };
    }
    record({ hypothesis: proposal.hypothesis, candidate: v.policy, decision: d.accepted ? 'accepted' : 'rejected', reason: d.reason, noise, incumbent: si, challenger: sc });
    if (d.accepted) {
      incumbent = v.policy;
      writeFileSync(INCUMBENT, JSON.stringify(incumbent, null, 2) + '\n');
    }
  }
  console.log(`stopped: ${stopped}`);
  if (!process.argv.includes('--no-final')) await final(entries, incumbent, stopped);
}

async function finalOnly() {
  const entries: Entry[] = existsSync(LEDGER) ? readFileSync(LEDGER, 'utf8').split('\n').filter(Boolean).map(l => JSON.parse(l) as Entry) : [];
  const incumbent: Policy = existsSync(INCUMBENT) ? JSON.parse(readFileSync(INCUMBENT, 'utf8')) as Policy : {};
  await final(entries, incumbent, 'the wrap-up rerun on its own (--final-only)');
}

// The incumbent against the shipped defaults on the held-out set, the morning report and the defaults patch
async function final(entries: Entry[], incumbent: Policy, stopped: string) {
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  const report = join(REPO, 'output', 'evolve', `${stamp}${DRY ? '-dry' : ''}`);
  mkdirSync(report, { recursive: true });
  const lines = ['# Evolve loop report', '', `Loop folder: ${OUT}. Cell: reviewers ${REVIEWERS}, variant ${VARIANT}, ${REPS} run(s) a target per side. Proposer: ${PROPOSER}. Stopped: ${stopped}.`, ''];
  lines.push('| round | decision | incumbent | candidate | median s (inc / cand) | hypothesis |', '|---|---|---|---|---|---|');
  for (const e of entries) {
    lines.push(`| ${e.round} | ${e.decision} | ${e.incumbent ? `${e.incumbent.found}/${e.incumbent.bugs}` : ''} | ${e.challenger ? `${e.challenger.found}/${e.challenger.bugs}` : ''} | ${e.incumbent && e.challenger ? `${e.incumbent.wallMedianS} / ${e.challenger.wallMedianS}` : ''} | ${(e.hypothesis ?? '').replace(/\|/g, '/')} |`);
  }
  lines.push('', 'Final policy:', '', '```json', JSON.stringify(incumbent, null, 2), '```', '');
  if (samePolicy(incumbent, {})) {
    lines.push('No candidate was accepted: the shipped defaults stand, and nothing ran on the held-out set.');
  } else {
    const dir = join(OUT, 'holdout');
    const [base, tuned] = await paired(dir, 'holdout', {}, incumbent);
    const [sb, st] = [summarize(base), summarize(tuned)];
    lines.push('Held-out set, shipped defaults against the final policy:', '', '| side | found | runs | median s | $ |', '|---|---|---|---|---|');
    lines.push(`| defaults | ${sb.found}/${sb.bugs} | ${sb.runs} | ${sb.wallMedianS} | ${sb.costUsd} |`, `| final | ${st.found}/${st.bugs} | ${st.runs} | ${st.wallMedianS} | ${st.costUsd} |`, '');
    const patch = defaultsPatch(incumbent, REPO);
    writeFileSync(join(report, 'defaults.patch'), patch);
    const e = effective(incumbent);
    const changed = (Object.keys(DEFAULT_RECIPE) as (keyof typeof DEFAULT_RECIPE)[]).filter(k => e.handoff[k] !== DEFAULT_RECIPE[k]);
    if (e.briefs.reviewer !== effective({}).briefs.reviewer) changed.push('the reviewer brief' as never);
    const list = changed.length > 1 ? `${changed.slice(0, -1).join(', ')} and ${changed[changed.length - 1]}` : changed.join('');
    lines.push(`\`defaults.patch\` changes ${list} in the shipped defaults. Apply it only after review: \`git apply ${join(report, 'defaults.patch')}\`.`);
  }
  writeFileSync(join(report, 'report.md'), lines.join('\n') + '\n');
  console.log(`report in ${report}`);
}

await (process.argv.includes('--final-only') ? finalOnly() : main());
