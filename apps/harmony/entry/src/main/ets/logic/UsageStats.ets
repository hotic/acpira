import { UsageModel, UsageRow, UsageSession, UsageView } from '../wire/Wire';
import { currentLocale, t } from '../store/L10n';
import { vendorBrand } from './ModelCatalog';

// Mirror of packages/ui/usage/summary.ts and formatting helpers from packages/ui/usage/UsagePage.tsx: folds the
// engine's hour-bucketed rows into local days and hours, summary figures, trend columns and model / agent / project
// breakdowns.

export type UsagePeriod = '7' | '30' | 'all';
export type UsageDim = 'model' | 'agent' | 'project';
export type UsageStep = 'hours' | 'day' | 'week' | 'month';

export const CHATS_KEY: string = '\u0000chats';
export const TREND_SPAN_HOURS: number = 4;
export const TREND_DAYS_MAX: number = 31;
export const TREND_WEEKS_MAX: number = 26;
export const DOT_STEPS: number = 4;
export const KEEP_ROWS: number = 10;
const HOUR_MS: number = 3600000;
const DAY_MS: number = 86400000;

export class UsageModelLabel {
  name: string = '';
  brand: string | undefined = undefined;
}

export class UsageModelLabels {
  keys: string[] = [];
  label: Map<string, UsageModelLabel> = new Map<string, UsageModelLabel>();
  projects: string[] = [];
}

export class UsageDay {
  day: number = 0;
  tokens: number = 0;
  byModel: Map<string, number> = new Map<string, number>();
  byAgent: Map<string, number> = new Map<string, number>();
  byProject: Map<string, number> = new Map<string, number>();

  tokensFor(dim: UsageDim, key: string): number {
    if (dim === 'model') {
      return this.byModel.get(key) ?? 0;
    }
    if (dim === 'agent') {
      return this.byAgent.get(key) ?? 0;
    }
    return this.byProject.get(key) ?? 0;
  }
}

export class UsagePart {
  key: string = '';
  tokens: number = 0;
  share: number = 0;
}

export class UsageSummary {
  tokens: number = 0;
  workMs: number = 0;
  longestMs: number = 0;
  prompts: number = 0;
  turns: number = 0;
  tools: number = 0;
  added: number = 0;
  removed: number = 0;
  sessions: number = 0;
  modelParts: UsagePart[] = [];
  agentParts: UsagePart[] = [];
  projectParts: UsagePart[] = [];

  parts(dim: UsageDim): UsagePart[] {
    if (dim === 'model') {
      return this.modelParts;
    }
    if (dim === 'agent') {
      return this.agentParts;
    }
    return this.projectParts;
  }
}

export class UsageTrendSpan {
  start: number = 0;
  end: number = 0;
}

export class UsageTrendColumns {
  step: UsageStep = 'day';
  cols: UsageTrendSpan[] = [];
}

export class UsageFigure {
  value: string = '0';
  unit: string | undefined = undefined;
}

export function dayOf(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

export function addDays(day: number, n: number): number {
  const d = new Date(day);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + n).getTime();
}

export function hourOf(ms: number): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), d.getHours()).getTime();
}

export function addHours(hour: number, n: number): number {
  const d = new Date(hour);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), d.getHours() + n).getTime();
}

export function mondayOffset(day: number): number {
  return (new Date(day).getDay() + 6) % 7;
}

/** Folds spelling variants ("Opus 5.5", "Claude Opus 5.5", "asgard/claude-opus-5-5-medium") into one key */
export function modelKey(name: string): string {
  const bare = name.slice(name.lastIndexOf('/') + 1).toLowerCase()
    .replace(/[\s._-]+(minimal|low|medium|high|xhigh|max)$/, '')
    .replace(/[\s._-]+/g, '');
  const stripped = bare.replace(/^(claude|gpt|kimi)/, '');
  return stripped.length > 0 ? stripped : bare;
}

function nameRank(name: string): number {
  return (/\s/.test(name) && /[A-Z]/.test(name) ? 2 : 0) + (name.includes('/') ? 0 : 1);
}

export function isChatFolder(chatRoot: string | undefined, cwd: string): boolean {
  if (chatRoot === undefined || chatRoot.length === 0) {
    return false;
  }
  const cut = Math.max(cwd.lastIndexOf('/'), cwd.lastIndexOf('\\'));
  if (cut < 0) {
    return false;
  }
  const clean = (p: string): string => p.replace(/\\/g, '/').replace(/\/+$/, '');
  return clean(cwd.substring(0, cut)) === clean(chatRoot);
}

export function modelLabels(view: UsageView, chats: string | undefined): UsageModelLabels {
  const out = new UsageModelLabels();
  out.projects = view.sessions.map((s: UsageSession) => (isChatFolder(chats, s.cwd) ? CHATS_KEY : s.cwd));
  out.keys = view.models.map((m: UsageModel) => modelKey(m.name));
  view.models.forEach((m: UsageModel, i: number) => {
    const k = out.keys[i];
    const cur = out.label.get(k);
    if (cur !== undefined) {
      const curRank = nameRank(cur.name);
      const nextRank = nameRank(m.name);
      if (curRank > nextRank || (curRank === nextRank && cur.name.length >= m.name.length)) {
        return;
      }
    }
    const item = new UsageModelLabel();
    item.name = m.name.slice(m.name.lastIndexOf('/') + 1);
    item.brand = vendorBrand(m.id) ?? vendorBrand(m.name);
    out.label.set(k, item);
  });
  return out;
}

export function keyOf(view: UsageView, models: UsageModelLabels, row: UsageRow, dim: UsageDim): string {
  if (dim === 'model') {
    if (row.model === undefined || row.model < 0 || row.model >= models.keys.length) {
      return '';
    }
    return models.keys[row.model];
  }
  if (dim === 'agent') {
    return row.session >= 0 && row.session < view.sessions.length ? view.sessions[row.session].agent : '';
  }
  return row.session >= 0 && row.session < models.projects.length ? models.projects[row.session] : '';
}

function foldBy(view: UsageView, models: UsageModelLabels, slot: (ms: number) => number): Map<number, UsageDay> {
  const out = new Map<number, UsageDay>();
  for (let i = 0; i < view.rows.length; i++) {
    const r = view.rows[i];
    const bucket = slot(r.at);
    let d = out.get(bucket);
    if (d === undefined) {
      d = new UsageDay();
      d.day = bucket;
      out.set(bucket, d);
    }
    const tokens = r.tokens ?? 0;
    d.tokens += tokens;
    const mk = keyOf(view, models, r, 'model');
    d.byModel.set(mk, (d.byModel.get(mk) ?? 0) + tokens);
    const ak = keyOf(view, models, r, 'agent');
    d.byAgent.set(ak, (d.byAgent.get(ak) ?? 0) + tokens);
    const pk = keyOf(view, models, r, 'project');
    d.byProject.set(pk, (d.byProject.get(pk) ?? 0) + tokens);
  }
  return out;
}

export function daysOf(view: UsageView, models: UsageModelLabels): Map<number, UsageDay> {
  return foldBy(view, models, dayOf);
}

export function hoursOf(view: UsageView, models: UsageModelLabels): Map<number, UsageDay> {
  return foldBy(view, models, hourOf);
}

export function periodStart(period: UsagePeriod, today: number): number {
  if (period === 'all') {
    return 0;
  }
  return addDays(today, period === '7' ? -6 : -29);
}

function rankParts(totals: Map<string, number>, totalTokens: number): UsagePart[] {
  const out: UsagePart[] = [];
  totals.forEach((v: number, k: string) => {
    if (v > 0 && k.length > 0) {
      const p = new UsagePart();
      p.key = k;
      p.tokens = v;
      p.share = totalTokens > 0 ? v / totalTokens : 0;
      out.push(p);
    }
  });
  out.sort((a: UsagePart, b: UsagePart) => (b.tokens !== a.tokens ? b.tokens - a.tokens : a.key.localeCompare(b.key)));
  return out;
}

export function summarize(view: UsageView, models: UsageModelLabels, from: number): UsageSummary {
  const s = new UsageSummary();
  const byModel = new Map<string, number>();
  const byAgent = new Map<string, number>();
  const byProject = new Map<string, number>();
  const sessions = new Set<number>();
  for (let i = 0; i < view.rows.length; i++) {
    const r = view.rows[i];
    if (r.at + HOUR_MS <= from) {
      continue;
    }
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
    const mk = keyOf(view, models, r, 'model');
    byModel.set(mk, (byModel.get(mk) ?? 0) + tokens);
    const ak = keyOf(view, models, r, 'agent');
    byAgent.set(ak, (byAgent.get(ak) ?? 0) + tokens);
    const pk = keyOf(view, models, r, 'project');
    byProject.set(pk, (byProject.get(pk) ?? 0) + tokens);
  }
  s.sessions = sessions.size;
  s.modelParts = rankParts(byModel, s.tokens);
  s.agentParts = rankParts(byAgent, s.tokens);
  s.projectParts = rankParts(byProject, s.tokens);
  return s;
}

export function trendColumns(period: UsagePeriod, today: number, first: number, now: number): UsageTrendColumns {
  const every = (step: UsageStep, from: number, next: (d: number) => number, until: number): UsageTrendColumns => {
    const out = new UsageTrendColumns();
    out.step = step;
    let d = from;
    while (d <= until) {
      const end = next(d);
      const span = new UsageTrendSpan();
      span.start = d;
      span.end = end;
      out.cols.push(span);
      if (end <= d) {
        break;
      }
      d = end;
    }
    if (step !== 'hours' && out.cols.length > 0) {
      const last = out.cols[out.cols.length - 1];
      last.end = Math.min(last.end, addDays(today, 1));
    }
    return out;
  };
  if (period === '7') {
    return every('hours', periodStart(period, today), (d: number) => addHours(d, TREND_SPAN_HOURS), now);
  }
  if (period === '30') {
    return every('day', periodStart(period, today), (d: number) => addDays(d, 1), today);
  }
  const span = Math.round((today - first) / DAY_MS) + 1;
  if (span <= TREND_DAYS_MAX) {
    return every('day', first, (d: number) => addDays(d, 1), today);
  }
  if (span <= TREND_WEEKS_MAX * 7) {
    return every('week', addDays(first, -mondayOffset(first)), (d: number) => addDays(d, 7), today);
  }
  const f = new Date(first);
  const start = new Date(f.getFullYear(), f.getMonth(), 1).getTime();
  return every('month', start, (d: number) => {
    const m = new Date(d);
    return new Date(m.getFullYear(), m.getMonth() + 1, 1).getTime();
  }, today);
}

export function firstDay(days: Map<number, UsageDay>, today: number): number {
  let first = today;
  days.forEach((d: UsageDay) => {
    if (d.tokens > 0 && d.day < first) {
      first = d.day;
    }
  });
  return first;
}

export function currentRun(days: Map<number, UsageDay>, today: number): number {
  let day = (days.get(today)?.tokens ?? 0) > 0 ? today : addDays(today, -1);
  let n = 0;
  while ((days.get(day)?.tokens ?? 0) > 0) {
    n++;
    day = addDays(day, -1);
  }
  return n;
}

function formatGrouped(n: number): string {
  const s = `${Math.round(n)}`;
  let out = '';
  for (let i = 0; i < s.length; i++) {
    const rem = s.length - i;
    if (i > 0 && rem % 3 === 0) {
      out += ',';
    }
    out += s.charAt(i);
  }
  return out;
}

function formatSig3(v: number): string {
  const raw = Number(v.toPrecision(3));
  return `${raw}`;
}

/** Plain up to 9,999, else locale compact notation with at most 3 significant digits */
export function figure(n: number): UsageFigure {
  const out = new UsageFigure();
  const locale = currentLocale();
  if (Math.abs(n) < 10000) {
    out.value = formatGrouped(n);
    return out;
  }
  const absN = Math.abs(n);
  let scale = 1000;
  let unit = 'K';
  if (locale.startsWith('zh-TW')) {
    if (absN >= 1e12) { scale = 1e12; unit = '兆'; }
    else if (absN >= 1e8) { scale = 1e8; unit = '億'; }
    else { scale = 1e4; unit = '萬'; }
  } else if (locale.startsWith('zh')) {
    if (absN >= 1e12) { scale = 1e12; unit = '万亿'; }
    else if (absN >= 1e8) { scale = 1e8; unit = '亿'; }
    else { scale = 1e4; unit = '万'; }
  } else if (locale.startsWith('ja')) {
    if (absN >= 1e12) { scale = 1e12; unit = '兆'; }
    else if (absN >= 1e8) { scale = 1e8; unit = '億'; }
    else { scale = 1e4; unit = '万'; }
  } else if (locale.startsWith('ko')) {
    if (absN >= 1e12) { scale = 1e12; unit = '조'; }
    else if (absN >= 1e8) { scale = 1e8; unit = '억'; }
    else { scale = 1e4; unit = '만'; }
  } else {
    if (absN >= 1e12) { scale = 1e12; unit = 'T'; }
    else if (absN >= 1e9) { scale = 1e9; unit = 'B'; }
    else if (absN >= 1e6) { scale = 1e6; unit = 'M'; }
    else { scale = 1e3; unit = 'K'; }
  }
  out.value = formatSig3(n / scale);
  out.unit = unit;
  return out;
}

export function spaced(text: string): string {
  if (!currentLocale().startsWith('zh')) {
    return text;
  }
  return text.replace(/([\u4e00-\u9fff])(?=[0-9A-Za-z])/g, '$1 ').replace(/([0-9A-Za-z])(?=[\u4e00-\u9fff])/g, '$1 ');
}

export function duration(ms: number): UsageFigure {
  const out = new UsageFigure();
  const min = Math.round(ms / 60000);
  if (min < 60) {
    out.value = `${min}`;
    out.unit = t('usagePage.unit.minutes');
    return out;
  }
  const h = min / 60;
  out.value = h >= 100 ? formatGrouped(Math.round(h)) : `${Math.round(h * 10) / 10}`;
  out.unit = t('usagePage.unit.hours');
  return out;
}

export function percent(share: number): string {
  if (share > 0 && share < 0.001) {
    return '<0.1%';
  }
  const floored = Math.floor(share * 1000) / 1000;
  if (share > 0 && share < 0.01) {
    return `${(floored * 100).toFixed(1)}%`;
  }
  return `${Math.round(floored * 100)}%`;
}

const EN_MONTHS: string[] = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const EN_WEEKDAYS: string[] = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const ZH_WEEKDAYS: string[] = ['周日', '周一', '周二', '周三', '周四', '周五', '周六'];

export function monthShort(ms: number): string {
  const d = new Date(ms);
  const m = d.getMonth();
  if (m === 0) {
    return `${d.getFullYear()}`;
  }
  const loc = currentLocale();
  if (loc.startsWith('zh') || loc.startsWith('ja')) {
    return spaced(`${m + 1}月`);
  }
  if (loc.startsWith('ko')) {
    return `${m + 1}월`;
  }
  return EN_MONTHS[m];
}

export function numericDate(ms: number): string {
  const d = new Date(ms);
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

export function dayLabel(ms: number): string {
  const d = new Date(ms);
  const wd = currentLocale().startsWith('zh') ? ZH_WEEKDAYS[d.getDay()] : EN_WEEKDAYS[d.getDay()];
  return `${numericDate(ms)} ${wd}`;
}

export function monthLong(ms: number): string {
  const d = new Date(ms);
  const loc = currentLocale();
  if (loc.startsWith('zh') || loc.startsWith('ja')) {
    return spaced(`${d.getFullYear()}年${d.getMonth() + 1}月`);
  }
  if (loc.startsWith('ko')) {
    return `${d.getFullYear()}년 ${d.getMonth() + 1}월`;
  }
  return `${EN_MONTHS[d.getMonth()]} ${d.getFullYear()}`;
}

export function weekRange(start: number, endInclusive: number): string {
  return `${numericDate(start)}–${numericDate(endInclusive)}`;
}

/** Deterministic sample history anchored to `nowMs` (mirror of lab/usage.preview.tsx) */
export function sampleUsageView(nowMs: number, chats: string | undefined): UsageView {
  let state = 20261121;
  const rnd = (): number => {
    state = (state + 0x6d2b79f5) | 0;
    let z = Math.imul(state ^ (state >>> 15), 1 | state);
    z = (z + Math.imul(z ^ (z >>> 7), 61 | z)) ^ z;
    return ((z ^ (z >>> 14)) >>> 0) / 4294967296;
  };
  const root = '/Users/dev/projects/acpira';
  const work = '/Users/dev/projects/Work-Local';
  const pc = 'ssh://asgard/srv/studio-pc';
  const keke = '/Users/dev/projects/keke';
  const chat = `${chats ?? '/Users/dev/.acpira/chats'}/quick-chat`;
  const sessions: UsageSession[] = [
    { agent: 'claude', cwd: root },
    { agent: 'claude', cwd: work },
    { agent: 'codex', cwd: root },
    { agent: 'acpira', cwd: root },
    { agent: 'codex', cwd: pc },
    { agent: 'kimi', cwd: keke },
    { agent: 'acpira', cwd: chat }
  ];
  const models: UsageModel[] = [
    { id: 'opus', name: 'Opus 5.5' },
    { id: 'sonnet', name: 'Sonnet 5.5' },
    { id: 'gpt-6-astra', name: '6 Astra' },
    { id: 'asgard/deepseek-v4.1-flash', name: 'DeepSeek V4.1 Flash' },
    { id: 'kimi-code/k3', name: 'K3' }
  ];
  const modelFor: number[] = [0, 1, 2, 3, 2, 4, 3];
  const todayMs = dayOf(nowMs);
  const startMs = addDays(todayMs, -300);
  const rows: UsageRow[] = [];
  for (let d = 0; d <= 300; d++) {
    const dayMs = addDays(startMs, d);
    const dow = new Date(dayMs).getDay();
    const weekend = dow === 0 || dow === 6;
    if (d < 294 && rnd() < (weekend ? 0.55 : 0.12)) {
      continue;
    }
    const ramp = 0.3 + (d / 300) * 0.9;
    for (let h = 9; h < 24; h++) {
      if (rnd() < 0.55) {
        continue;
      }
      const s = Math.min(sessions.length - 1, Math.floor(rnd() * sessions.length));
      const peak = h >= 14 && h <= 17 ? 2.2 : 1;
      const tokens = Math.round(peak * ramp * (0.3 + rnd()) * 6000000);
      const at = addHours(dayMs, h);
      const turns = 1 + Math.floor(rnd() * 6);
      const workMs = Math.round(turns * (2 + rnd() * 9) * 60000);
      const longestMs = Math.round((workMs / turns) * (1 + rnd()));
      const prompts = Math.max(0, turns - (rnd() < 0.2 ? 1 : 0));
      rows.push({
        at: at, session: s, model: modelFor[s], tokens: tokens,
        input: Math.round(tokens * 0.97), output: Math.round(tokens * 0.03),
        turns: turns, prompts: prompts, workMs: workMs, longestMs: longestMs,
        tools: turns * 7, added: Math.round(rnd() * 300), removed: Math.round(rnd() * 120)
      });
    }
  }
  return { sessions: sessions, models: models, rows: rows };
}
