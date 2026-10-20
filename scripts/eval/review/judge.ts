import type { Bug, Target } from './bench';
import { GATEWAY, gatewayKey } from '../keys';

// Review bench scoring (docs/dev/evals.md). A review's findings are listed, each matched against the target's
// known bugs, and only a finding that names one of a bug's files (or names no file at all) can count as that bug's hit.
// The judge is one fixed model from a family none of the reviewers belong to, and the evolve loop may not change it
// (scripts/eval/evolve.ts never passes --judge). The stub judge is a keyword rule for dry runs

export const JUDGE_MODEL = 'deepseek-v4-pro';

export interface Finding {
  severity?: string;
  path?: string;
  line?: number;
  summary: string;
  // Index into target.bugs, when the judge took this finding for that bug and its place fits
  bug?: number;
  // The bug the judge model matched when the place check then refused it, kept so a corrected file list can be
  // rescored without asking the judge again
  claimed?: number;
}

export interface Verdict {
  findings: Finding[];
  // target.bugs indexes some finding hit
  hits: number[];
  // Findings that match no known bug (real or not: the bench does not grade them)
  others: number;
  judge: string;
  error?: string;
}

const norm = (p: string) => p.replace(/\\/g, '/').replace(/^\.?\//, '');

// A cited path fits a bug when one ends with the other (reviewers cite paths relative to different roots)
export function placeFits(bug: Bug, path: string | undefined): boolean {
  if (!path) return true;
  const p = norm(path);
  return bug.files.some(f => p.endsWith(norm(f)) || norm(f).endsWith(p));
}

function verdictOf(findings: Finding[], target: Target, judge: string): Verdict {
  const hits = [...new Set(findings.flatMap(f => (f.bug === undefined ? [] : [f.bug])))].sort();
  return { findings, hits, others: findings.filter(f => f.bug === undefined).length, judge };
}

// One finding per paragraph or list item that cites a path:line; the dry run's whole parser
function splitFindings(review: string): Finding[] {
  const items = review.split(/\n(?=\s*(?:[-*]|\d+[.)])\s)|\n{2,}/).map(s => s.trim()).filter(Boolean);
  const out: Finding[] = [];
  for (const item of items) {
    const m = /([\w./-]+\.\w+):(\d+)/.exec(item);
    if (!m) continue;
    const severity = /\b(high|medium|low|critical)\b/i.exec(item)?.[1]?.toLowerCase();
    out.push({ severity, path: m[1], line: Number(m[2]), summary: item.replace(/\s+/g, ' ').slice(0, 300) });
  }
  return out;
}

export function stubJudge(target: Target, review: string): Verdict {
  const findings = splitFindings(review).map(f => {
    const text = f.summary.toLowerCase();
    const bug = target.bugs.findIndex(b => placeFits(b, f.path) && b.keywords.some(k => text.includes(k.toLowerCase())));
    return bug >= 0 ? { ...f, bug } : f;
  });
  return verdictOf(findings, target, 'stub');
}

const PROMPT = `You grade a code review against known defects of the change it reviewed.

First list every distinct finding the review reports (a defect it claims, not praise, not a summary of the change,
not a style remark). Keep the review's own order. For each finding give its severity if stated, the file path and
line it cites (the first one, if several), and a one-sentence summary in your own words.

Then decide for each finding whether it identifies one of the known defects below: the same root cause, even if the
review words it differently, proposes another fix or cites a nearby line. A finding that only touches the same code
without naming the actual problem does not match. A finding matches at most one defect.

Answer with JSON only, in this shape:
{"findings":[{"severity":"high","path":"src/a.ts","line":12,"summary":"...","defect":0}]}
where "defect" is the index of the known defect it matches, or null.`;

interface Raw { findings?: { severity?: string; path?: string; line?: number; summary?: string; defect?: number | null }[] }

function parseJson(text: string): Raw {
  const start = text.indexOf('{');
  const end = text.lastIndexOf('}');
  if (start < 0 || end < start) throw new Error(`no JSON in the judge's answer: ${text.slice(0, 200)}`);
  return JSON.parse(text.slice(start, end + 1)) as Raw;
}

// The model judge: one request per review, temperature 0, retried twice on a transport or parse failure
export async function modelJudge(target: Target, review: string, model = JUDGE_MODEL): Promise<Verdict> {
  const defects = target.bugs.map((b, i) => `Defect ${i} (in ${b.files.join(', ')}):\n${b.description}`).join('\n\n');
  const body = {
    model,
    temperature: 0,
    messages: [
      { role: 'system', content: PROMPT },
      { role: 'user', content: `Known defects:\n\n${defects}\n\nThe review:\n\n${review.trim() || '(empty)'}` },
    ],
  };
  let last = '';
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      const r = await fetch(`${GATEWAY}/v1/chat/completions`, {
        method: 'POST',
        headers: { 'content-type': 'application/json', authorization: `Bearer ${gatewayKey()}` },
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(300_000),
      });
      const j = await r.json() as { choices?: { message?: { content?: string } }[]; error?: { message?: string } };
      if (!r.ok) throw new Error(`HTTP ${r.status}: ${j.error?.message ?? ''}`);
      const raw = parseJson(j.choices?.[0]?.message?.content ?? '');
      const findings: Finding[] = (raw.findings ?? []).map(f => {
        const base: Finding = { severity: f.severity?.toLowerCase(), path: f.path || undefined, line: typeof f.line === 'number' ? f.line : undefined, summary: f.summary ?? '' };
        const bug = typeof f.defect === 'number' ? target.bugs[f.defect] : undefined;
        // The prefilter by place: a match citing a file the defect is not in does not count
        if (!bug) return base;
        return placeFits(bug, base.path) ? { ...base, bug: f.defect as number } : { ...base, claimed: f.defect as number };
      });
      return verdictOf(findings, target, model);
    } catch (e) {
      last = String(e);
    }
  }
  return { findings: [], hits: [], others: 0, judge: model, error: last };
}

export function judge(name: string, target: Target, review: string): Promise<Verdict> {
  return name === 'stub' ? Promise.resolve(stubJudge(target, review)) : modelJudge(target, review, name);
}
