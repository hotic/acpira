import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { Crown, Ellipsis, Folder, MessageSquare } from 'lucide-react';
import type { AgentId, AgentInfo } from '@acpira/shared/transcript';
import type { UsageView } from '@acpira/shared/usage';
import { dateFormat, getLocale, t, useLocale } from '../i18n';
import { Card } from '../ui/Card';
import { Collapsible } from '../ui/Collapsible';
import { cn } from '../ui/cn';
import { ModelMark } from '../chat/ModelMark';
import { AgentMark } from '../chat/AgentMark';
import { ListHead, rowRules, Segmented } from '../settings/controls';
import { projectName } from '../chat/SessionList';
import { parseRemoteFolder } from '@acpira/shared/remote';
import { RemoteFolderIcon } from '../remote/RemoteFolderIcon';
import { addDays, addHours, CHATS, currentRun, dayOf, daysOf, firstDay, hourOf, hoursOf, modelLabels, periodStart, summarize, TREND_SPAN_HOURS, trendColumns, type UsageDay, type UsageDim, type UsagePart, type UsagePeriod } from './summary';

// The usage page (desktop: the chart button beside the gear opens it on the sheet, the session column stays). One row of
// four figures and the year's activity as a dot lattice flush with the cards, both over the whole history; the trend with
// the period switch (7 days, 30 days, all), and the breakdown by model, agent or project over that period. Grammar of the
// settings page: the page title, outside list heads, cards with three text roles. Geometry and colours: styles/usage.css

// Width of an element, live
function useWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [w, setW] = useState(0);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(() => setW(el.clientWidth));
    ro.observe(el);
    setW(el.clientWidth);
    return () => ro.disconnect();
  }, []);
  return [ref, w] as const;
}

// A count as one number and at most one unit word: plain up to 9,999, then the locale's compact unit (万 / 亿, K / M)
function figure(n: number): [string, string | undefined] {
  const locale = getLocale();
  if (n < 10_000) return [new Intl.NumberFormat(locale).format(Math.round(n)), undefined];
  const parts = new Intl.NumberFormat(locale, { notation: 'compact', maximumSignificantDigits: 3 }).formatToParts(n);
  return [parts.filter(p => p.type !== 'compact' && p.type !== 'literal').map(p => p.value).join(''), parts.find(p => p.type === 'compact')?.value];
}

// Chinese copy keeps a space between Han characters and Latin letters or digits (the dictionary's own strings do); Intl's
// dates and compact units come without it ("11月", "2026年10月", "7.62亿")
function spaced(s: string): string {
  return getLocale().startsWith('zh') ? s.replace(/([\u4e00-\u9fff])(?=[0-9A-Za-z])|([0-9A-Za-z])(?=[\u4e00-\u9fff])/g, '$1$2 ') : s;
}

// Minutes under an hour, else hours to one decimal (whole hours past a hundred)
function duration(ms: number): [string, string] {
  const min = Math.round(ms / 60_000);
  if (min < 60) return [`${min}`, t('usagePage.unit.minutes')];
  const h = min / 60;
  return [new Intl.NumberFormat(getLocale(), { maximumFractionDigits: h >= 100 ? 0 : 1 }).format(h), t('usagePage.unit.hours')];
}

// A share to a whole percent, a tenth under one percent, and "<0.1%" for a sliver that is still there
function pct(x: number): string {
  const f = new Intl.NumberFormat(getLocale(), { style: 'percent', maximumFractionDigits: x > 0 && x < 0.01 ? 1 : 0 });
  return x > 0 && x < 0.001 ? `<${f.format(0.001)}` : f.format(Math.floor(x * 1000) / 1000);
}

// A section's reading: the figure, where ListHead puts its count (a hover swaps the summary for the hovered day's in
// place), and the hovered slot's date at the row's end
interface Reading { value: ReactNode; date?: string }

// A section head in ListHead's row, one line: the title and its reading's figure, the reading's date pushed to the end,
// then the section's own control. The figure gives way and nothing wraps, so a hover never moves the picture under it
function SectionHead({ title, reading, action }: { title: string; reading?: Reading; action?: ReactNode }) {
  return (
    <div className="flex min-h-(--text-2-lh) items-center gap-gap px-(--group-head-inset)">
      <div className="flex min-w-0 flex-1 items-baseline gap-2">
        <h3 className="m-0 shrink-0 text-2 font-medium text-fg-1">{title}</h3>
        {reading?.value}
        {reading?.date && <span className="ml-auto shrink-0 pl-gap text-3 whitespace-nowrap text-fg-3 tabular-nums">{reading.date}</span>}
      </div>
      {action}
    </div>
  );
}

// The figures: one row of four equal cells (two by two on a narrow page) divided by hairlines, each a caption over one
// number and at most one unit. Every cell's text stands a pad past its own left edge, so the steps between the four
// stay equal (a rule plus padding only after the first would widen the first step)
interface Tile { caption: string; value: string; unit?: string }
function Tiles({ tiles }: { tiles: Tile[] }) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const cols = w > 0 && w < 520 ? 2 : 4;
  return (
    <div ref={ref}>
      <Card className="acp-section-panel flex flex-col border-(--settings-card-line) bg-(--settings-card) shadow-none">
        <div className={cn('grid gap-y-3.5 py-3.5', cols === 2 ? 'grid-cols-2' : 'grid-cols-4')}>
          {tiles.map((tile, i) => (
            <div key={tile.caption} className={cn('flex min-w-0 flex-col gap-1.5 px-pad', i % cols > 0 && 'border-l border-line')}>
              <span className="truncate text-3 text-fg-3">{tile.caption}</span>
              <span className="flex items-baseline gap-0.5 whitespace-nowrap">
                <span className="text-(length:--text-page) leading-none font-semibold tracking-[-0.01em] text-fg-1">{tile.value}</span>
                {tile.unit && <span className="text-2 text-fg-2">{tile.unit}</span>}
              </span>
            </div>
          ))}
        </div>
      </Card>
    </div>
  );
}

// The narrowest pitch between the lattice's columns; the year fits 53 weeks while it can, fewer weeks on a narrow page
const DOT_PITCH_MIN = 10;
// A day with use takes one of this many ink steps (square root of its share of the busiest day)
const DOT_STEPS = 4;

// The year's activity: a lattice of days (weeks across, Monday on top) whose outer cells stand on the head's text line at
// both ends, as the trend's outer columns do (cells flush with the cards' edges read as sticking out past the heading). Equal rounded squares, tokens as steps of one ink (cells of
// different sizes read as ragged): the lattice is the whole history, the period switch belongs to the trend. Month names under it, the first and last held inside the edges. A
// hovered day reads out in the head row; `pick` (a hovered breakdown row) redraws the dots with that row's share
function YearDots({ days, today, pick, reading }: {
  days: Map<number, UsageDay>; today: number; pick?: (d: UsageDay) => number;
  reading: (d: UsageDay | undefined, day: number, tokens: number) => Reading;
}) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const lastMonday = addDays(today, -((new Date(today).getDay() + 6) % 7));
  const cols = Math.max(8, Math.min(53, Math.floor(w / DOT_PITCH_MIN)));
  // Square cells a fifth of the pitch apart, the outer ones on the measure's edges
  const gap = Math.max(2, Math.round(w / cols * 0.2));
  const pitch = (w + gap) / cols;
  const size = pitch - gap;
  const corner = Math.min(3, size / 4);
  const height = 7 * size + 6 * gap;
  const start = addDays(lastMonday, -(cols - 1) * 7);
  const value = (d: UsageDay | undefined) => (d ? (pick ? pick(d) : d.tokens) : 0);
  let max = 1;
  for (const d of days.values()) if (d.day >= start) max = Math.max(max, value(d));
  const level = (v: number) => Math.max(1, Math.ceil(Math.sqrt(v / max) * DOT_STEPS)) / DOT_STEPS;
  const [at, setAt] = useState<number | undefined>();
  const cx = (c: number) => c * pitch;
  const locale = useLocale();
  const monthFmt = dateFormat(locale, { month: 'short' });
  const months: { x: number; label: string; align: 'left' | 'center' | 'right' }[] = [];
  for (let c = 0; c < cols; c++) {
    // The month whose 1st falls in the week
    const d = new Date(addDays(start, c * 7 + 6));
    if (d.getDate() > 7) continue;
    const x = cx(c);
    const prev = months[months.length - 1];
    if (prev && x - prev.x < 32) continue;
    months.push({ x, label: d.getMonth() === 0 ? `${d.getFullYear()}` : spaced(monthFmt.format(d)), align: x < 16 ? 'left' : x > w - 16 ? 'right' : 'center' });
  }
  return (
    <section className="flex flex-col gap-(--group-head-gap)">
      <SectionHead title={t('usagePage.activity')} reading={at !== undefined ? reading(days.get(at), at, value(days.get(at)))
        : { value: <span className="shrink-0 text-3 text-fg-3 tabular-nums">{t('usagePage.period.days', { n: [...days.values()].filter(d => value(d) > 0).length })}</span> }} />
      <div className="px-(--group-head-inset)">
        <div ref={ref} className="flex flex-col gap-2">
          {w > 0 && <svg width={w} height={height} className="block overflow-visible" onPointerLeave={() => setAt(undefined)} onPointerMove={e => {
            const box = e.currentTarget.getBoundingClientRect();
            const c = Math.floor((e.clientX - box.left + gap / 2) / pitch), k = Math.floor((e.clientY - box.top + gap / 2) / pitch);
            const day = c >= 0 && c < cols && k >= 0 && k < 7 ? addDays(start, c * 7 + k) : undefined;
            setAt(day !== undefined && day <= today ? day : undefined);
          }}>
            {Array.from({ length: cols * 7 }, (_, n) => {
              const c = Math.floor(n / 7), k = n % 7;
              const day = addDays(start, c * 7 + k);
              if (day > today) return null;
              const v = value(days.get(day));
              const x = cx(c), y = k * pitch;
              const style = { '--i': Math.round((today - day) / 864e5) } as CSSProperties;
              return <g key={n}>
                <rect className="usage-mark" style={style} x={x} y={y} width={size} height={size} rx={corner}
                  fill={v > 0 ? 'var(--usage-ink)' : 'var(--usage-cell)'} fillOpacity={v > 0 ? level(v) : 1} />
                {at === day && <rect x={x - 1.5} y={y - 1.5} width={size + 3} height={size + 3} rx={corner + 1.5} fill="none" stroke="var(--fg-2)" strokeWidth={1} />}
              </g>;
            })}
          </svg>}
          <div className="relative h-(--text-3-lh) text-3 text-fg-3">
            {months.map(m => <span key={m.x} className="absolute top-0 whitespace-nowrap" style={m.align === 'left' ? { left: 0 } : m.align === 'right' ? { right: 0 } : { left: m.x, transform: 'translateX(-50%)' }}>{m.label}</span>)}
          </div>
        </div>
      </div>
    </section>
  );
}

// The trend's height, a column's widest, and the least room an axis label takes
const TREND_H = 80;
const BAR_MAX = 20;
const LABEL_ROOM = 56;

// Tokens over the period: one column per day (per week or month for all of it), its outer columns on the head's text
// line at both ends. Wide columns in a calmer ink than the lattice's darkest step (bright bars weigh the page down); a
// hovered column takes the full ink and the rest dims, its reading in the head row; under a hovered
// breakdown row every column shows that row's part in ink over its whole dimmed
function Trend({ days, hours, today, now, period, pick, reading, dayLabel, action }: {
  days: Map<number, UsageDay>; hours: Map<number, UsageDay>; today: number; now: number; period: UsagePeriod; pick?: (d: UsageDay) => number;
  reading: (date: string | undefined, tokens: number, byModel?: Map<string, number>) => Reading; dayLabel: (day: number) => string; action: ReactNode;
}) {
  const [ref, w] = useWidth<HTMLDivElement>();
  const [at, setAt] = useState<number | undefined>();
  const locale = useLocale();
  const { step, cols: spans } = useMemo(() => trendColumns(period, today, firstDay(days, today), now), [period, today, days, now]);
  // 4-hour columns add up the hour fold, the rest the day fold
  const fold = step === 'hours' ? hours : days;
  const next = step === 'hours' ? (d: number) => addHours(d, 1) : (d: number) => addDays(d, 1);
  const cols = spans.map(({ start, end }) => {
    let total = 0, part = 0;
    for (let d = start; d < end; d = next(d)) {
      const u = fold.get(d);
      if (u) { total += u.tokens; if (pick) part += pick(u); }
    }
    return { start, end, total, part };
  });
  const n = cols.length, h = TREND_H;
  const bw = Math.max(3, Math.min(BAR_MAX, (w / Math.max(1, n)) * 0.64));
  const pitch = n > 1 ? (w - bw) / (n - 1) : 0;
  const x = (i: number) => (n > 1 ? i * pitch : (w - bw) / 2);
  const max = Math.max(1, ...cols.map(c => c.total));
  const bar = (i: number, v: number) => {
    const bh = v > 0 ? Math.max(2, (v / max) * (h - 2)) : 0;
    if (!bh) return undefined;
    const x0 = x(i), r = Math.min(4, bw / 2, bh);
    return `M${x0},${h} V${h - bh + r} Q${x0},${h - bh} ${x0 + r},${h - bh} H${x0 + bw - r} Q${x0 + bw},${h - bh} ${x0 + bw},${h - bh + r} V${h} Z`;
  };
  const tickFmt = dateFormat(locale, { month: 'numeric', day: 'numeric' });
  const monthFmt = dateFormat(locale, { month: 'short' });
  const tick = (start: number) => {
    const d = new Date(start);
    return step !== 'month' ? tickFmt.format(d) : d.getMonth() === 0 ? `${d.getFullYear()}` : spaced(monthFmt.format(d));
  };
  // Axis labels. 4-hour columns: each day's date under the middle of its own columns. Otherwise from the newest column
  // back, every day while they fit, else weekly, weeks and months as room allows; the outer two held inside the edges
  const ticks: { i: number; style: CSSProperties }[] = [];
  if (step === 'hours') {
    const perDay = 24 / TREND_SPAN_HOURS;
    cols.forEach((c, i) => {
      if (new Date(c.start).getHours() === 0) ticks.push({ i, style: { left: (x(i) + x(Math.min(i + perDay - 1, n - 1)) + bw) / 2, transform: 'translateX(-50%)' } });
    });
  } else {
    const every = step === 'day' ? (w / Math.max(1, n) >= LABEL_ROOM ? 1 : 7) : Math.max(1, Math.ceil((n * LABEL_ROOM) / Math.max(1, w)));
    for (let i = n - 1; i >= 0; i -= every) {
      ticks.push({ i, style: n > 1 && i === 0 ? { left: 0 } : n > 1 && i === n - 1 ? { right: 0 } : { left: x(i) + bw / 2, transform: 'translateX(-50%)' } });
    }
  }
  const monthLong = dateFormat(locale, { year: 'numeric', month: 'long' });
  const hovered = at !== undefined ? cols[at] : undefined;
  let head: Reading;
  if (hovered) {
    const from = new Date(hovered.start).getHours();
    const date = step === 'hours' ? `${dayLabel(hovered.start)} ${from}:00–${new Date(hovered.end).getHours() || 24}:00`
      : step === 'day' ? dayLabel(hovered.start)
      : step === 'week' ? tickFmt.formatRange(new Date(hovered.start), new Date(addDays(hovered.end, -1)))
      : spaced(monthLong.format(new Date(hovered.start)));
    const byModel = new Map<string, number>();
    for (let d = hovered.start; d < hovered.end; d = next(d)) for (const [k, v] of fold.get(d)?.by.model ?? []) byModel.set(k, (byModel.get(k) ?? 0) + v);
    head = reading(date, pick ? hovered.part : hovered.total, byModel);
  } else head = reading(undefined, cols.reduce((a, c) => a + (pick ? c.part : c.total), 0));
  return (
    <section className="flex flex-col gap-(--group-head-gap)">
      <SectionHead title={t('usagePage.trend')} reading={head} action={action} />
      <div className="px-(--group-head-inset)">
        <div ref={ref} className="flex flex-col gap-2">
          {w > 0 && <svg width={w} height={h} className="block" onPointerLeave={() => setAt(undefined)} onPointerMove={e => {
            const px = e.clientX - e.currentTarget.getBoundingClientRect().left;
            setAt(n > 1 ? Math.max(0, Math.min(n - 1, Math.round((px - bw / 2) / pitch))) : 0);
          }}>
            <line x1={0} x2={w} y1={h - 0.5} y2={h - 0.5} stroke="var(--line)" />
            {cols.map((c, i) => {
              const whole = bar(i, c.total), part = pick && at === undefined ? bar(i, c.part) : undefined;
              const lit = at !== undefined ? at === i : !pick;
              return whole && <g key={c.start}>
                <path className="usage-mark" d={whole} fill={at === i ? 'var(--usage-ink)' : lit ? 'var(--usage-bar)' : 'var(--usage-dim)'} />
                {part && <path d={part} fill="var(--usage-ink)" />}
              </g>;
            })}
          </svg>}
          <div className="relative h-(--text-3-lh) text-3 text-fg-3">
            {ticks.map(({ i, style }) => <span key={cols[i]!.start} className="absolute top-0 whitespace-nowrap" style={style}>{tick(cols[i]!.start)}</span>)}
          </div>
        </div>
      </div>
    </section>
  );
}

// The breakdown: one list grammar for models, agents and projects (the mark, the name, the share bar, the tokens, the
// share), the top ten and the rest folded into one row. A click turns that row into the eleventh in place (one row's
// height either way) and slides the others in under it on the panels' own curve; the list then stays whole until the
// page or the breakdown changes. A hovered row dims the others and redraws the dots and the trend with its part
const KEEP = 10;
const BREAKDOWN_ROW = 'grid min-h-(--setting-row) w-full grid-cols-[var(--lead)_minmax(0,var(--usage-name-w))_minmax(0,1fr)_var(--usage-num-w)_var(--usage-pct-w)] items-center gap-x-gap py-(--setting-row-pad) text-left text-2 transition-opacity';
function Breakdown({ dim, items, mark, name, detail, focus, onFocus, action }: {
  dim: UsageDim; items: UsagePart[]; mark: (key: string) => ReactNode; name: (key: string) => string; detail: (key: string) => string | undefined;
  focus?: string; onFocus: (k?: string) => void; action: ReactNode;
}) {
  const [open, setOpen] = useState(false);
  const folds = items.length > KEEP + 1;
  const rest = folds ? items.slice(KEEP) : [];
  const dimmed = (key: string) => focus !== undefined && focus !== key && 'opacity-45';
  const cells = (r: UsagePart, more?: boolean) => {
    const [value, unit] = figure(r.tokens);
    return <>
      <span className="flex size-lead items-center justify-center">{more ? <Ellipsis className="size-icon text-fg-3" strokeWidth={1.5} /> : mark(r.key)}</span>
      <span className="flex min-w-0 items-baseline gap-1.5">
        <span className={cn('truncate', more ? 'text-fg-2 transition-colors group-hover/more:text-fg-1' : 'text-fg-1')}>{more ? t(`usagePage.more.${dim}`, { n: rest.length }) : name(r.key)}</span>
        {!more && detail(r.key) && <span className="shrink-0 text-3 text-fg-3">{detail(r.key)}</span>}
      </span>
      <span className="relative h-1 w-full overflow-hidden rounded-full bg-(--usage-track)">
        <span className="usage-bar absolute inset-y-0 left-0 w-full rounded-full" style={{ transform: `scaleX(${r.share})`, background: more ? 'var(--usage-dim)' : 'var(--usage-ink)' }} />
      </span>
      <span className="text-right text-fg-1 tabular-nums">{value}{unit && <span className="ml-0.5 text-3 text-fg-3">{unit}</span>}</span>
      <span className="text-right text-fg-3 tabular-nums">{pct(r.share)}</span>
    </>;
  };
  const row = (r: UsagePart) => (
    <div key={r.key} onPointerEnter={() => onFocus(r.key)} onPointerLeave={() => onFocus(undefined)}
      title={dim === 'project' && r.key !== CHATS ? r.key : undefined} className={cn(BREAKDOWN_ROW, dimmed(r.key))}>{cells(r)}</div>
  );
  const others: UsagePart = { key: '*', tokens: rest.reduce((a, m) => a + m.tokens, 0), share: rest.reduce((a, m) => a + m.share, 0) };
  return (
    <section className="flex flex-col gap-(--group-head-gap)">
      <ListHead title={t('usagePage.breakdown')} count={items.length} outside action={action} />
      {items.length > 0 && <Card className={cn('acp-section-panel flex flex-col overflow-hidden border-(--settings-card-line) bg-(--settings-card) px-pad shadow-none', rowRules)}>
        {(folds ? items.slice(0, KEEP) : items).map(row)}
        {folds && <>
          {open ? row(rest[0]!)
            : <button type="button" aria-expanded={false} onClick={() => setOpen(true)} onPointerEnter={() => onFocus(undefined)}
                className={cn(BREAKDOWN_ROW, 'group/more cursor-pointer', dimmed('*'))}>{cells(others, true)}</button>}
          <Collapsible.Section open={open}>
            <div className={cn('flex flex-col', rowRules)}>{rest.slice(1).map(row)}</div>
          </Collapsible.Section>
        </>}
      </Card>}
    </section>
  );
}

// A project's name: its folder's, a remote folder's on its host; chats as the list's one group
const projectLabel = (key: string) => (key === CHATS ? t('session.group.chats') : projectName(parseRemoteFolder(key)?.path ?? key));

// Settings → Usage: the page's body under the settings shell's own header (settings/SettingsShell.tsx). Each visit asks
// the engine for a fresh view (`onLoad`), as the Storage tab measures on each visit
export function UsagePage({ view, agents, chats, onLoad }: { view?: UsageView; agents: AgentInfo[]; chats?: string; onLoad?: () => void }) {
  const locale = useLocale();
  useEffect(() => { onLoad?.(); }, [onLoad]);
  const [period, setPeriod] = useState<UsagePeriod>('30');
  const [dim, setDim] = useState<UsageDim>('model');
  const [focus, setFocus] = useState<string | undefined>();
  const today = dayOf(Date.now());
  const from = periodStart(period, today);
  const models = useMemo(() => (view ? modelLabels(view, chats) : undefined), [view, chats]);
  const days = useMemo(() => (view && models ? daysOf(view, models) : new Map<number, UsageDay>()), [view, models]);
  const hours = useMemo(() => (view && models ? hoursOf(view, models) : new Map<number, UsageDay>()), [view, models]);
  const now = hourOf(Date.now());
  // The figures and the lattice cover the whole history; the period switch on the trend sets the trend and the breakdown
  const all = useMemo(() => (view && models ? summarize(view, models, 0) : undefined), [view, models]);
  const s = useMemo(() => (view && models ? summarize(view, models, from) : undefined), [view, models, from]);
  const periods: { value: UsagePeriod; label: string }[] = [
    { value: '7', label: t('usagePage.period.days', { n: 7 }) }, { value: '30', label: t('usagePage.period.days', { n: 30 }) }, { value: 'all', label: t('usagePage.period.all') },
  ];
  const dims: { value: UsageDim; label: string }[] = [{ value: 'model', label: t('usagePage.dim.model') }, { value: 'agent', label: t('usagePage.dim.agent') }, { value: 'project', label: t('usagePage.dim.project') }];
  const agentName = (id: string) => agents.find(a => a.id === id)?.name ?? id;
  const modelName = (key: string) => models?.label.get(key)?.name ?? key;
  const modelMark = (key: string, className: string) => <ModelMark family={modelName(key)} brand={models?.label.get(key)?.brand} className={className} />;
  const mark = (key: string) => dim === 'model' ? modelMark(key, 'size-icon')
    : dim === 'agent' ? <AgentMark id={key as AgentId} name={agentName(key)} className="size-icon" />
    : key === CHATS ? <MessageSquare className="size-icon text-fg-2" strokeWidth={1.5} />
    : parseRemoteFolder(key) ? <RemoteFolderIcon className="text-fg-2" /> : <Folder className="size-icon text-fg-2" strokeWidth={1.5} />;
  const name = (key: string) => (dim === 'agent' ? agentName(key) : dim === 'project' ? projectLabel(key) : modelName(key));
  // Projects of one name tell themselves apart: a remote one by its host, a local one by the folder it sits in
  const detail = (key: string) => {
    if (dim !== 'project' || key === CHATS) return undefined;
    const remote = parseRemoteFolder(key);
    if (remote) return remote.alias;
    const twin = s?.parts.project.some(p => p.key !== key && p.key !== CHATS && projectLabel(p.key) === projectLabel(key));
    return twin ? projectName(key.slice(0, Math.max(key.lastIndexOf('/'), key.lastIndexOf('\\')))) : undefined;
  };
  const pick = focus === undefined ? undefined : (d: UsageDay) => d.by[dim].get(focus) ?? 0;
  // A day as the axis writes it, with its weekday: "11/17 周二", "11/17 Tue"
  const dateFmt = dateFormat(locale, { month: 'numeric', day: 'numeric' });
  const weekdayFmt = dateFormat(locale, { weekday: 'short' });
  const dayLabel = (day: number) => `${dateFmt.format(new Date(day))} ${weekdayFmt.format(new Date(day))}`;
  // A section's reading: at rest the period's tokens beside the title in the quiet ink; on a hovered day or column its
  // tokens in the same place a step brighter, (unfiltered) its busiest model under a crown after them, and its date at
  // the row's end. Under a row hover the tokens are that row's part
  const reading = (date: string | undefined, tokens: number, byModel?: Map<string, number>): Reading => {
    const top = !focus && date && byModel ? [...byModel].filter(([k]) => k).sort((a, b) => b[1] - a[1])[0]?.[0] : undefined;
    return {
      date,
      value: (
        <span className="flex min-w-0 items-baseline gap-2 overflow-hidden whitespace-nowrap text-3 text-fg-3 tabular-nums">
          <span className={cn('shrink-0', date && 'text-fg-2')}>{tokens > 0 || !date ? spaced(figure(tokens).join('')) : t('usagePage.noUse')}</span>
          {top && <span className="flex min-w-0 items-baseline gap-1" title={t('usagePage.top')}>
            <Crown className="size-3 shrink-0 self-center" strokeWidth={1.75} aria-label={t('usagePage.top')} />
            <span className="truncate">{modelName(top)}</span>
          </span>}
        </span>
      ),
    };
  };
  const tiles: Tile[] = [];
  if (all) {
    const [tokens, tokensUnit] = figure(all.tokens);
    const [work, workUnit] = duration(all.workMs);
    const [longest, longestUnit] = duration(all.longestMs);
    tiles.push(
      { caption: t('usagePage.fig.tokens'), value: tokens, unit: tokensUnit },
      { caption: t('usagePage.fig.work'), value: work, unit: workUnit },
      { caption: t('usagePage.fig.longest'), value: longest, unit: longestUnit },
      { caption: t('usagePage.fig.streak'), value: `${currentRun(days, today)}`, unit: t('usagePage.unit.days') },
    );
  }
  return (
    <div className="acp-usage flex flex-col gap-(--section-gap)">
      {s && <>
        <Tiles tiles={tiles} />
        <YearDots days={days} today={today} pick={pick} reading={(d, day, tokens) => reading(dayLabel(day), tokens, d?.by.model)} />
        <Trend days={days} hours={hours} today={today} now={now} period={period} pick={pick} reading={reading} dayLabel={dayLabel}
          action={<Segmented items={periods} value={period} onChange={setPeriod} label={t('usagePage.period')} small className="w-auto" />} />
        <Breakdown key={dim} dim={dim} items={s.parts[dim]} mark={mark} name={name} detail={detail} focus={focus} onFocus={setFocus}
          action={<Segmented items={dims} value={dim} onChange={d => { setDim(d); setFocus(undefined); }} label={t('usagePage.dim')} small className="w-auto" />} />
      </>}
    </div>
  );
}
