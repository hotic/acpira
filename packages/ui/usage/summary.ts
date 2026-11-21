import type { UsageRow, UsageView } from '@acpira/shared/usage';
import { optionBrand } from '@acpira/shared/models';
import { isChatFolder } from '../chat/sessionTree';

// The usage page's arithmetic: the engine's hour rows (shared/usage.ts) folded into local days, the figures, the trend's
// columns and the breakdowns by model, agent and project. Pure, so the page re-derives it per period and focus

export type UsagePeriod = '7' | '30' | 'all';
export type UsageDim = 'model' | 'agent' | 'project';

// Local midnight of a time, as a Date's own calendar sees it
export const dayOf = (ms: number) => { const d = new Date(ms); return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); };
export const addDays = (day: number, n: number) => { const d = new Date(day); return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n).getTime(); };
// The local clock hour a time falls in, and hours on the local clock (so a column stays 4 wall-clock hours across DST)
export const hourOf = (ms: number) => { const d = new Date(ms); return new Date(d.getFullYear(), d.getMonth(), d.getDate(), d.getHours()).getTime(); };
export const addHours = (hour: number, n: number) => { const d = new Date(hour); return new Date(d.getFullYear(), d.getMonth(), d.getDate(), d.getHours() + n).getTime(); };

// Agents spell one model differently ("Opus 5.5", "Claude Opus 5.5", "asgard/Claude Opus 5.5", the bare id
// "claude-opus-5-5", or with the effort it ran at, "claude-opus-5-5-medium"; "6.1 Sol" beside "GPT-6.1 Sol"): the
// breakdown files them under one key, the name without a source prefix, an effort suffix, a leading vendor word or
// punctuation
export function modelKey(name: string): string {
  const bare = name.slice(name.lastIndexOf('/') + 1).toLowerCase().replace(/[\s._-]+(minimal|low|medium|high|xhigh|max)$/, '').replace(/[\s._-]+/g, '');
  return bare.replace(/^(claude|gpt|kimi)/, '') || bare;
}

// The spelling a merged model shows: a display name (spaced, capitalised) over an id, then the longest
const nameRank = (n: string) => (/\s/.test(n) && /[A-Z]/.test(n) ? 2 : 0) + (n.includes('/') ? 0 : 1);

// The project key every chat files under: each chat runs in a folder of its own under the chats root, and the list shows
// them as one group too
export const CHATS = '\u0000chats';

// Per view: each model's key and each key's shown name and mark; each session's project key
export interface ModelLabels { keys: string[]; label: Map<string, { name: string; brand?: string }>; projects: string[] }
export function modelLabels(view: UsageView, chats?: string): ModelLabels {
  const projects = view.sessions.map(x => (isChatFolder(chats, x.cwd) ? CHATS : x.cwd));
  const keys = view.models.map(m => modelKey(m.name));
  const label = new Map<string, { name: string; brand?: string }>();
  view.models.forEach((m, i) => {
    const k = keys[i]!, cur = label.get(k);
    if (cur && (nameRank(cur.name) > nameRank(m.name) || (nameRank(cur.name) === nameRank(m.name) && cur.name.length >= m.name.length))) return;
    label.set(k, { name: m.name.slice(m.name.lastIndexOf('/') + 1), brand: optionBrand({ id: m.id, name: m.name }) });
  });
  return { keys, label, projects };
}

// What a row is filed under for each breakdown: the model's merged key, the agent's id, the project's folder
export function keyOf(view: UsageView, models: ModelLabels, r: UsageRow, dim: UsageDim): string {
  if (dim === 'model') return r.model === undefined ? '' : models.keys[r.model]!;
  return dim === 'agent' ? view.sessions[r.session]!.agent : models.projects[r.session]!;
}

// One local day: its tokens, and per breakdown key (for a hovered row's days)
export interface UsageDay {
  day: number;
  tokens: number;
  by: Record<UsageDim, Map<string, number>>;
}

// Local days, or local clock hours (the trend's short period), each with its tokens and per breakdown key
export const daysOf = (view: UsageView, models: ModelLabels) => foldBy(view, models, dayOf);
export const hoursOf = (view: UsageView, models: ModelLabels) => foldBy(view, models, hourOf);
function foldBy(view: UsageView, models: ModelLabels, slot: (ms: number) => number): Map<number, UsageDay> {
  const days = new Map<number, UsageDay>();
  for (const r of view.rows) {
    const day = slot(r.at);
    let d = days.get(day);
    if (!d) days.set(day, d = { day, tokens: 0, by: { model: new Map(), agent: new Map(), project: new Map() } });
    const tokens = r.tokens ?? 0;
    d.tokens += tokens;
    for (const dim of ['model', 'agent', 'project'] as const) {
      const k = keyOf(view, models, r, dim);
      d.by[dim].set(k, (d.by[dim].get(k) ?? 0) + tokens);
    }
  }
  return days;
}

// A breakdown entry: its key, its tokens and its share of the period's
export interface UsagePart { key: string; tokens: number; share: number }

export interface UsageSummary {
  tokens: number;
  workMs: number;
  longestMs: number;
  prompts: number;
  turns: number;
  tools: number;
  added: number;
  removed: number;
  sessions: number;
  parts: Record<UsageDim, UsagePart[]>;
}

// The first day a period covers (the last `n` days, today included); 0 for all of it
export function periodStart(period: UsagePeriod, today: number): number {
  return period === 'all' ? 0 : addDays(today, -(Number(period) - 1));
}

export function summarize(view: UsageView, models: ModelLabels, from: number): UsageSummary {
  const s: UsageSummary = { tokens: 0, workMs: 0, longestMs: 0, prompts: 0, turns: 0, tools: 0, added: 0, removed: 0, sessions: 0, parts: { model: [], agent: [], project: [] } };
  const totals: Record<UsageDim, Map<string, number>> = { model: new Map(), agent: new Map(), project: new Map() };
  const sessions = new Set<number>();
  for (const r of view.rows) {
    if (r.at + 3_600_000 <= from) continue;
    const tokens = r.tokens ?? 0;
    s.tokens += tokens;
    s.workMs += r.workMs ?? 0;
    s.longestMs = Math.max(s.longestMs, r.longestMs ?? 0);
    s.prompts += r.prompts ?? 0;
    s.turns += r.turns ?? 0;
    s.tools += r.tools ?? 0;
    s.added += r.added ?? 0;
    s.removed += r.removed ?? 0;
    sessions.add(r.session);
    for (const dim of ['model', 'agent', 'project'] as const) {
      const k = keyOf(view, models, r, dim);
      totals[dim].set(k, (totals[dim].get(k) ?? 0) + tokens);
    }
  }
  s.sessions = sessions.size;
  for (const dim of ['model', 'agent', 'project'] as const) {
    // A model nothing names has no row of its own: its tokens still count in the total and the shares
    s.parts[dim] = [...totals[dim]].filter(([k, v]) => v > 0 && k !== '')
      .map(([key, tokens]) => ({ key, tokens, share: s.tokens > 0 ? tokens / s.tokens : 0 }))
      .sort((a, b) => b.tokens - a.tokens);
  }
  return s;
}

// The trend's columns, each [start, end): the last 7 days in 4-hour spans (seven daily columns stand too far apart), the
// last 30 in days, and all the use since its first day in days, weeks (from Monday, as the lattice) or months, whichever
// keeps the columns few enough to read. `hours` columns read the hour fold, the rest the day fold
export type UsageStep = 'hours' | 'day' | 'week' | 'month';
export const TREND_SPAN_HOURS = 4;
const TREND_DAYS_MAX = 31;
const TREND_WEEKS_MAX = 26;
export function trendColumns(period: UsagePeriod, today: number, first: number, now: number): { step: UsageStep; cols: { start: number; end: number }[] } {
  const every = (step: UsageStep, from: number, next: (d: number) => number, until: number) => {
    const cols: { start: number; end: number }[] = [];
    for (let d = from; d <= until; d = next(d)) cols.push({ start: d, end: next(d) });
    // The newest column ends today (a week or month still running)
    if (step !== 'hours' && cols.length) cols[cols.length - 1]!.end = Math.min(cols[cols.length - 1]!.end, addDays(today, 1));
    return { step, cols };
  };
  const nextDay = (d: number) => addDays(d, 1);
  if (period === '7') return every('hours', periodStart(period, today), d => addHours(d, TREND_SPAN_HOURS), now);
  if (period === '30') return every('day', periodStart(period, today), nextDay, today);
  const span = Math.round((today - first) / 864e5) + 1;
  if (span <= TREND_DAYS_MAX) return every('day', first, nextDay, today);
  if (span <= TREND_WEEKS_MAX * 7) return every('week', addDays(first, -((new Date(first).getDay() + 6) % 7)), d => addDays(d, 7), today);
  const f = new Date(first);
  return every('month', new Date(f.getFullYear(), f.getMonth(), 1).getTime(), d => { const m = new Date(d); return new Date(m.getFullYear(), m.getMonth() + 1, 1).getTime(); }, today);
}

// The first day with any use (today when there is none)
export function firstDay(days: Map<number, UsageDay>, today: number): number {
  let first = today;
  for (const d of days.values()) if (d.tokens > 0 && d.day < first) first = d.day;
  return first;
}

// Days in a row with any use, ending today (or yesterday, when today has none yet)
export function currentRun(days: Map<number, UsageDay>, today: number): number {
  let day = days.get(today)?.tokens ? today : addDays(today, -1);
  let n = 0;
  while (days.get(day)?.tokens) { n++; day = addDays(day, -1); }
  return n;
}

