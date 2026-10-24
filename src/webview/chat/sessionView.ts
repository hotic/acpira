import type { SessionSummary } from '@shared/transcript';

// How the session list is looked at, from its view menu (SessionList's ViewMenu): grouping, ordering and what rows show,
// then the filters. Kept per window in the webview's state, like the section folds; the engine never sees it

export type ViewGroup = 'project' | 'time' | 'agent' | 'none';
export type ViewOrder = 'updated' | 'created' | 'title';
export type ViewStatus = 'working' | 'waiting' | 'unread' | 'error' | 'background';
export type ViewSince = 'all' | 'today' | 'week' | 'month';
// Sessions an open project lists before its "more" row (desktop); 0 lists them all
export type ViewPer = 5 | 10 | 0;

export interface ListView {
  group: ViewGroup;
  order: ViewOrder;
  per: ViewPer;
  // The session's model name at the row's end
  model: boolean;
  status: ViewStatus[];
  agents: string[];
  since: ViewSince;
}

export const DEFAULT_VIEW: ListView = { group: 'project', order: 'updated', per: 5, model: false, status: [], agents: [], since: 'all' };
export const VIEW_KEY = 'sessionListView';
export const VIEW_GROUPS: readonly ViewGroup[] = ['project', 'time', 'agent', 'none'];
export const VIEW_ORDERS: readonly ViewOrder[] = ['updated', 'created', 'title'];
export const VIEW_STATUSES: readonly ViewStatus[] = ['working', 'waiting', 'unread', 'error', 'background'];
export const VIEW_SINCE: readonly ViewSince[] = ['all', 'today', 'week', 'month'];
export const VIEW_PER: readonly ViewPer[] = [5, 10, 0];

const pick = <T>(allowed: readonly T[], v: unknown, fallback: T): T => (allowed.includes(v as T) ? v as T : fallback);
const strings = (v: unknown, allowed?: readonly string[]) =>
  Array.isArray(v) ? [...new Set(v.filter((x): x is string => typeof x === 'string' && (!allowed || allowed.includes(x))))] : [];

// The stored view, whatever an older or newer page left there: unknown values fall back to the defaults one by one
export function readView(v: unknown): ListView {
  const o = v && typeof v === 'object' ? v as Record<string, unknown> : {};
  return {
    group: pick(VIEW_GROUPS, o.group, DEFAULT_VIEW.group),
    order: pick(VIEW_ORDERS, o.order, DEFAULT_VIEW.order),
    per: pick(VIEW_PER, o.per, DEFAULT_VIEW.per),
    model: o.model === true,
    status: strings(o.status, VIEW_STATUSES) as ViewStatus[],
    agents: strings(o.agents),
    since: pick(VIEW_SINCE, o.since, DEFAULT_VIEW.since),
  };
}

// How many filters are on (each status and channel counts, a time window counts once)
export function filterCount(v: ListView): number {
  return v.status.length + v.agents.length + (v.since === 'all' ? 0 : 1);
}

export function clearFilters(v: ListView): ListView {
  return { ...v, status: [], agents: [], since: 'all' };
}

const DAY = 86_400_000;
const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

// The time a session ranks and groups by: its start under the "created" order, else its last activity. Index entries
// written before createdAt existed fall back to updatedAt
export function timeOf(s: SessionSummary, order: ViewOrder): string {
  return order === 'created' ? s.createdAt ?? s.updatedAt : s.updatedAt;
}

// Whether a session passes the view's filters. Within one kind the picks are alternatives (any status picked), across
// kinds all must hold. "Unread" also takes a background session with a reply nobody has looked at
export function passes(s: SessionSummary, v: ListView, now: Date): boolean {
  if (v.status.length) {
    const state = s.state;
    const ok = (!!state && v.status.includes(state)) || (state === 'background' && !!s.unread && v.status.includes('unread'));
    if (!ok) return false;
  }
  if (v.agents.length && !v.agents.includes(s.agent)) return false;
  if (v.since !== 'all') {
    const from = v.since === 'today' ? startOfDay(now) : now.getTime() - (v.since === 'week' ? 7 : 30) * DAY;
    const at = new Date(s.updatedAt).getTime();
    if (!(at >= from)) return false;
  }
  return true;
}

// The list order: newest activity or newest start first, or by title in the UI language. Stable, so equal keys keep the
// engine's order
export function sortSessions(list: SessionSummary[], order: ViewOrder, locale: string): SessionSummary[] {
  if (order === 'title') {
    const collator = new Intl.Collator(locale, { numeric: true, sensitivity: 'base' });
    return [...list].sort((a, b) => collator.compare(a.title, b.title));
  }
  return [...list].sort((a, b) => timeOf(b, order).localeCompare(timeOf(a, order)));
}

export type TimeBucket = 'today' | 'yesterday' | 'week' | 'month' | 'older';
export interface FlatGroup {
  // A time bucket or an agent id
  key: string;
  sessions: SessionSummary[];
}

// The list grouped by time (today, yesterday, the previous 7 and 30 days, older; by the order's own time) or by agent (in
// the order agents first appear). Sessions keep their order inside a group; empty groups are left out
export function groupSessions(list: SessionSummary[], group: 'time' | 'agent', order: ViewOrder, now: Date): FlatGroup[] {
  const groups = new Map<string, SessionSummary[]>();
  const today = startOfDay(now);
  const bucket = (s: SessionSummary): TimeBucket => {
    const at = startOfDay(new Date(timeOf(s, order)));
    const days = Math.round((today - at) / DAY);
    return days <= 0 ? 'today' : days === 1 ? 'yesterday' : days < 7 ? 'week' : days < 30 ? 'month' : 'older';
  };
  if (group === 'time') for (const k of ['today', 'yesterday', 'week', 'month', 'older']) groups.set(k, []);
  for (const s of list) {
    const key = group === 'time' ? bucket(s) : s.agent;
    groups.set(key, [...groups.get(key) ?? [], s]);
  }
  return [...groups].filter(([, sessions]) => sessions.length > 0).map(([key, sessions]) => ({ key, sessions }));
}
