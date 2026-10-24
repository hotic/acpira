import { createContext, memo, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type RefObject } from 'react';
import { ChevronLeft, ChevronRight, ChevronDown, Sparkle, TriangleAlert, X } from 'lucide-react';
import type { AgentId, AgentTurn, Turn, UserTurn } from '@shared/transcript';
import type { SubagentSummary } from '@shared/subagents';
import { buildTrace, domain, segments, spanEnd, tickStep, walk, type Trace, type TraceSpan } from '@shared/trace';
import { t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { Row } from '../ui/Row';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { Collapsible } from '../ui/Collapsible';
import { AgentMark } from '../chat/AgentMark';
import { toolIcon } from '../chat/icons';
import { toolTarget } from '../chat/folding';
import { commandDuration } from '../chat/commandDuration';
import { resultText, subagentTitle } from '../chat/subagents/subagentState';
import { StableText } from '../ui/StableText';
import { Rolling } from './Rolling';

// The trace waterfall (styles trace.css): one agent turn as a span tree on a shared clock, after affectUI's Trace Waterfall.
// Totals on top whose digits roll, then one row per span with its bar on the time axis; a folded parent draws what its
// children did inside its own bar (tools, subagents, failures, and model time between them), a click opens the span's card.
// Geometry follows the clock every frame without a React render: the waterfall carries the clock and the axis length as
// two custom properties (--trace-now, --trace-max, in ms) and every bar, label, tick and the now line computes its place
// from them in CSS. React renders on new data and, while the turn runs, once a second for the text
// (running durations, the totals, the tick set)

const TEXT_TICK_MS = 1000;
// How the axis reaches a new length (the turn ending, a trace picked): the remaining distance shrinks by e every GLIDE_MS
const GLIDE_MS = 140;

export interface TraceViewProps {
  turns: Turn[];
  subagents?: SubagentSummary[];
  // A child's transcript when the host already sent it (`subagentTranscripts`)
  childTurns?: (subagentId: string) => Turn[] | undefined;
  agent: { id: AgentId; name: string };
  // The model that answered a turn, for the root row (Shell resolves it through the session's controls)
  modelOf?: (turn: AgentTurn, user?: UserTurn) => string | undefined;
  // Epoch ms now; the LAB replays a recorded run on a clock of its own
  clock?: () => number;
  // A still frame: the clock stopped at this epoch ms
  now?: number;
}

// The active session as the trace reads it. Shell provides it around the tool panel and only the trace tab reads it, so a
// stream chunk re-renders the trace alone and the memoised panel above it stays put
export const TraceSourceContext = createContext<TraceViewProps | undefined>(undefined);

// The tool panel's trace tab: the active session's trace, or nothing outside a provider
export function TraceTab() {
  const source = useContext(TraceSourceContext);
  return source ? <TraceView {...source} /> : null;
}

// The session's agent turns, with the prompt that started each
function agentTurns(turns: Turn[]): { index: number; turn: AgentTurn; user?: UserTurn }[] {
  const out: { index: number; turn: AgentTurn; user?: UserTurn }[] = [];
  let user: UserTurn | undefined;
  turns.forEach((turn, index) => {
    if (turn.role === 'user') user = turn;
    else if (turn.role === 'agent') { out.push({ index, turn, ...(user ? { user } : {}) }); user = undefined; }
  });
  return out;
}

// Re-renders once a second while `on`: the text a live trace shows changes no faster
function useTicker(on: boolean) {
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!on) return;
    const timer = setInterval(() => setTick(n => n + 1), TEXT_TICK_MS);
    return () => clearInterval(timer);
  }, [on]);
}

// What a turn's trace is built from: its clock and outcome, and its tool calls by reference (`shared/reuse.ts` keeps an
// unchanged block's reference across session patches), so a streamed chunk of prose leaves the trace as it was
function traceKey(entry: { index: number; turn: AgentTurn } | undefined, subagents?: SubagentSummary[], childTurns?: TraceViewProps['childTurns']): unknown[] {
  if (!entry) return [];
  const { turn } = entry;
  const key: unknown[] = [entry.index, turn.startedAt, turn.endedAt, turn.stop, turn.error, turn.usage, subagents, childTurns];
  for (const b of turn.blocks) if (b.type === 'tool_call') key.push(b);
  return key;
}

const sameKey = (a: unknown[], b: unknown[]) => a.length === b.length && a.every((x, i) => x === b[i]);

// The trace of one turn of the session: the latest by default (following new turns), or one picked with the arrows
export const TraceView = memo(function TraceView(p: TraceViewProps) {
  useLocale();
  const list = useMemo(() => agentTurns(p.turns), [p.turns]);
  // undefined = follow the latest turn
  const [picked, setPicked] = useState<number>();
  const at = picked !== undefined && picked < list.length ? picked : list.length - 1;
  const entry = list[at];
  // Rebuilt only when its key changes (adjusted during render, so the new trace and the frame it changes commit together)
  const key = traceKey(entry, p.subagents, p.childTurns);
  const [built, setBuilt] = useState<{ key: unknown[]; trace?: Trace }>({ key: [] });
  let trace = built.trace;
  if (!sameKey(built.key, key)) {
    trace = entry && buildTrace({
      turn: entry.turn, turnIndex: entry.index,
      ...(p.subagents ? { subagents: p.subagents } : {}), ...(p.childTurns ? { childTurns: p.childTurns } : {}),
    });
    setBuilt({ key, ...(trace ? { trace } : {}) });
  }
  // The clock as the frame loop reads it, so a new clock function from the parent does not restart the loop
  const clock = useRef(p.clock ?? Date.now);
  clock.current = p.clock ?? Date.now;
  const agent = useMemo(() => p.agent, [p.agent.id, p.agent.name]);
  const go = (to: number) => setPicked(to >= list.length - 1 ? undefined : Math.max(0, to));
  const title = entry?.user?.text.split('\n').find(l => l.trim())?.trim();
  return (
    <section className="trace @container flex min-h-0 flex-1 flex-col" data-trace aria-label={t('trace.title')}>
      <header className="flex h-row shrink-0 items-center gap-gap px-pad">
        <Sparkle strokeWidth={1.5} className="size-icon shrink-0 text-command" />
        <span className="shrink-0 text-2 text-fg-strong">{t('trace.title')}</span>
        <span className="min-w-0 flex-1 truncate text-2 text-fg-3" title={title}>{title}</span>
        {list.length > 1 && (
          <span className="flex shrink-0 items-center text-3 text-fg-3 tabular-nums">
            <IconButton size="sm" aria-label={t('trace.prev')} title={t('trace.prev')} disabled={at <= 0} onClick={() => go(at - 1)}><ChevronLeft strokeWidth={1.5} /></IconButton>
            {t('trace.turnOf', { n: at + 1, total: list.length })}
            <IconButton size="sm" aria-label={t('trace.next')} title={t('trace.next')} disabled={at >= list.length - 1} onClick={() => go(at + 1)}><ChevronRight strokeWidth={1.5} /></IconButton>
          </span>
        )}
        {trace && <StateChip state={trace.root.state} />}
      </header>
      {!entry ? <Empty text={t('trace.noTurns')} />
        : !trace ? <Empty text={t('trace.untimed')} />
        : <Waterfall key={entry.index} trace={trace} clock={clock} still={p.now} agent={agent} model={p.modelOf?.(entry.turn, entry.user)} prompt={entry.user?.text} />}
    </section>
  );
});

function Empty({ text }: { text: string }) {
  return <div className="flex flex-1 items-center justify-center px-page text-center text-2 text-fg-3">{text}</div>;
}

const CHIP: Record<TraceSpan['state'], { key: 'trace.live' | 'trace.done' | 'trace.failed' | 'trace.stopped'; tone: string }> = {
  running: { key: 'trace.live', tone: 'trace-chip-live' },
  done: { key: 'trace.done', tone: 'trace-chip-done' },
  failed: { key: 'trace.failed', tone: 'trace-chip-failed' },
  cancelled: { key: 'trace.stopped', tone: 'trace-chip-stopped' },
};

// Every state's label sits in the chip at once (ui/StableText), so a run that ends changes the chip's colour and words
// without changing its width and moving the title beside it
function StateChip({ state }: { state: TraceSpan['state'] }) {
  const items = (Object.keys(CHIP) as TraceSpan['state'][]).map(k => ({
    key: k,
    text: <>{k === 'running' && <span className="trace-chip-dot size-1.5 rounded-full" />}{t(CHIP[k].key)}</>,
  }));
  return (
    <span className={cn('trace-chip inline-flex h-ctl-sm shrink-0 items-center rounded-md px-2 text-3', CHIP[state].tone)}>
      <StableText items={items} current={state} itemClassName="inline-flex items-center justify-self-center gap-1.5" />
    </span>
  );
}

// 99.7k, 1.2M: one decimal, which is what a glance at a running total wants
function compact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (n >= 1000) return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`;
  return String(n);
}

// A bar's label and the axis: tenths below ten seconds, whole seconds on a live clock
const timeText = (ms: number, whole = false) => (ms <= 0 ? t('turns.elapsed.s', { s: 0 }) : commandDuration(ms, whole));

interface WaterfallProps {
  trace: Trace;
  // Epoch ms, read every frame while the turn runs
  clock: RefObject<() => number>;
  // A still frame: the clock stopped at this epoch ms
  still?: number;
  agent: { id: AgentId; name: string };
  model?: string;
  prompt?: string;
}

// Rows mounted after the waterfall's first commit grow in (trace.css [data-enter]); the first ones are simply there
const EnteredContext = createContext<{ readonly current: boolean }>({ current: false });

interface TreeContext {
  now: number;
  folded: ReadonlySet<string>;
  onToggle: (id: string) => void;
  onOpen: (span: TraceSpan, anchor: HTMLElement) => void;
  agent: { id: AgentId; name: string };
  model?: string;
  slowest?: string;
  selected?: string;
}

// Memoised: a session patch that leaves the trace alone (streamed prose) skips it; while the turn runs it renders itself
// once a second for its text
const Waterfall = memo(function Waterfall({ trace, clock, still, agent, model, prompt }: WaterfallProps) {
  const live = trace.endedAt === undefined;
  useTicker(live && still === undefined);
  // ms from the origin, as of this render
  const now = (still ?? clock.current()) - trace.origin;
  const duration = spanEnd(trace.root, now) ?? 0;
  const sheet = useRef<HTMLDivElement>(null);
  const [scale, setScale] = useState<HTMLSpanElement | null>(null);
  const fit = useTickFit(scale);
  const max = domain(duration, live);
  const step = tickStep(max, fit);
  useAxisClock(sheet, trace, live, now, still === undefined ? clock : undefined);
  // While the clock runs the ticks past the axis' end are in place too (the tail shows them, the column's edge clips them),
  // so a growing axis slides them in from the edge; a finished trace keeps the ticks up to its end and lets the rest fade
  const ticks = useTicks(step, live ? max * 2 : max + 1);
  const entered = useRef(false);
  useEffect(() => { entered.current = true; }, []);
  const [folded, setFolded] = useState<ReadonlySet<string>>(() => new Set());
  const [open, setOpen] = useState<{ id: string; anchor: HTMLElement }>();
  const toggle = (id: string) => setFolded(f => { const next = new Set(f); if (!next.delete(id)) next.add(id); return next; });
  const s = trace.stats;
  const tokens = s.tokens?.total ?? (s.tokens?.input !== undefined || s.tokens?.output !== undefined ? (s.tokens.input ?? 0) + (s.tokens.output ?? 0) : undefined);
  const cells: { label: string; value: string; tone?: string }[] = [
    { label: t('trace.duration'), value: timeText(duration, live) },
    { label: t('trace.tokens'), value: tokens !== undefined ? compact(tokens) : '—' },
    ...(s.modelCalls !== undefined ? [{ label: t('trace.modelCalls'), value: String(s.modelCalls) }] : []),
    { label: t('trace.tools'), value: String(s.tools) },
    { label: t('trace.subagents'), value: String(s.subagents) },
    { label: t('trace.errors'), value: String(s.errors), ...(s.errors ? { tone: 'text-danger' } : {}) },
  ];
  const selected = open && findSpan(trace.root, open.id);
  const ctx: TreeContext = {
    now, folded, onToggle: toggle, agent, ...(model ? { model } : {}), ...(open ? { selected: open.id } : {}),
    // The slowest call is named once the turn is over: while it runs the title would hop from call to call as each ends
    ...(trace.slowest && !live ? { slowest: trace.slowest } : {}),
    onOpen: (span, anchor) => setOpen(o => (o?.id === span.id ? undefined : { id: span.id, anchor })),
  };
  return (
    <div ref={sheet} className="trace-sheet flex min-h-0 flex-1 flex-col" data-trace-live={live || undefined}>
      <dl className="trace-stats mx-pad grid shrink-0 rounded-lg border border-line" style={{ '--trace-cells': cells.length } as CSSProperties}>
        {cells.map(c => (
          <div key={c.label} className="trace-stat flex min-w-0 flex-col gap-1 py-gap">
            <dt className="trace-stat-label truncate text-fg-3">{c.label}</dt>
            <dd className={cn('truncate text-1 text-fg-strong tabular-nums', c.tone)}><Rolling text={c.value} /></dd>
          </div>
        ))}
      </dl>
      <div className="trace-line trace-axis shrink-0 text-3 text-fg-3" aria-hidden>
        <span className="truncate">{t('trace.span')}</span>
        <span ref={setScale} className="trace-scale">
          <span className="trace-ticks">
            {ticks.map(k => (
              <span key={k.x} className="trace-tick tabular-nums" data-leaving={k.leaving || undefined} data-zero={k.x === 0 || undefined} style={{ '--t': k.x } as CSSProperties}>
                {timeText(k.x, k.x % 1000 === 0)}
              </span>
            ))}
          </span>
          <span className="trace-now-dot" />
        </span>
      </div>
      <div className="scroll-thin scroll-stable relative min-h-0 flex-1 overflow-y-auto" role="tree" aria-label={t('trace.title')}>
        <div className="relative min-h-full pb-gap">
          <div className="trace-line trace-grid pointer-events-none" aria-hidden>
            <span />
            <span className="trace-grid-col">
              {ticks.map(k => <span key={k.x} className="trace-gridline" data-leaving={k.leaving || undefined} style={{ '--t': k.x } as CSSProperties} />)}
              <span className="trace-now" />
            </span>
          </div>
          <EnteredContext.Provider value={entered}>
            <SpanTree span={trace.root} depth={0} ctx={ctx} />
          </EnteredContext.Provider>
        </div>
      </div>
      <Popover.Root open={!!selected} onOpenChange={next => { if (!next) setOpen(undefined); }}>
        <Popover.Portal>
          <Popover.Positioner anchor={open?.anchor} width="lg" side="bottom" align="start">
            <Popover.Popup className="trace-card flex flex-col gap-gap p-pad">
              {selected && <SpanCard span={selected} now={now} agent={agent} model={model} prompt={prompt} onClose={() => setOpen(undefined)} />}
            </Popover.Popup>
          </Popover.Positioner>
        </Popover.Portal>
      </Popover.Root>
    </div>
  );
});

// Writes the clock and the axis length onto the waterfall: synchronously after every render (so a frame never paints stale
// geometry), and every frame while the turn runs or the axis is still gliding to a new length. While live the length follows
// the clock exactly; a jump (a short turn ending, whose opening window gives way to the turn's own length) glides there
function useAxisClock(sheet: RefObject<HTMLElement | null>, trace: Trace, live: boolean, now: number, clock?: RefObject<() => number>) {
  const shown = useRef<number>(undefined);
  const last = useRef(0);
  const end = trace.root.end;
  const place = (at: number, glide: boolean): boolean => {
    const el = sheet.current;
    if (!el) return false;
    const n = live ? Math.max(0, clock ? clock.current() - trace.origin : now) : end ?? now;
    const target = domain(n, live);
    const prev = shown.current ?? target;
    // A frame's timestamp is when the frame began, which can precede the layout effect's own reading
    const dt = last.current ? Math.max(0, Math.min(64, at - last.current)) : 16;
    last.current = at;
    const next = !glide || live || Math.abs(target - prev) <= target * 0.002 ? target : target + (prev - target) * Math.exp(-dt / GLIDE_MS);
    shown.current = next;
    el.style.setProperty('--trace-now', String(n));
    el.style.setProperty('--trace-max', String(next));
    return next !== target;
  };
  useLayoutEffect(() => { place(performance.now(), canGlide(sheet.current)); });
  useEffect(() => {
    const glide = canGlide(sheet.current);
    let frame = 0;
    const loop = (at: number) => {
      const gliding = place(at, glide);
      if ((live && clock) || gliding) frame = requestAnimationFrame(loop);
    };
    frame = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(frame);
    // A new render re-arms the loop: the turn ended (a glide to its own length) or new data arrived
  }, [live, clock, trace]);
}

// Motion off (the appearance setting or the system's) jumps instead of gliding
const canGlide = (el: HTMLElement | null) =>
  !el?.closest('[data-motion="none"]') && !(typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches);

// How long a dropped tick stays to fade out (trace.css .trace-tick[data-leaving], over --dur-close)
const TICK_LEAVE_MS = 600;

// The ticks of the axis, keyed by their time. A tick the axis drops (a coarser step, a shorter axis) stays a moment, fading
// out where the clock carries it, while new ones fade in. Worked out during render, so a dropped tick is never unmounted
// before it can fade
function useTicks(step: number, until: number): { x: number; leaving?: true }[] {
  const xs: number[] = [];
  for (let x = 0; x < until; x += step) xs.push(x);
  const sig = `${step}:${xs.length}`;
  const [seen, setSeen] = useState({ sig, xs, gone: [] as number[] });
  if (seen.sig !== sig) setSeen({ sig, xs, gone: [...seen.gone, ...seen.xs].filter(x => !xs.includes(x)) });
  useEffect(() => {
    if (!seen.gone.length) return;
    const timer = setTimeout(() => setSeen(v => ({ ...v, gone: [] })), TICK_LEAVE_MS);
    return () => clearTimeout(timer);
  }, [seen.gone]);
  return [...xs.map(x => ({ x })), ...seen.gone.filter(x => !xs.includes(x)).map(x => ({ x, leaving: true as const }))];
}

// How many ticks the axis holds (the bar column short of its tail, --trace-tail), a label's room apart (--trace-tick-w)
function useTickFit(el: HTMLElement | null): number {
  const [n, setN] = useState(5);
  useEffect(() => {
    if (!el) return;
    const measure = () => {
      const style = getComputedStyle(el);
      const room = Number.parseFloat(style.getPropertyValue('--trace-tick-w')) || 1;
      const tail = Number.parseFloat(style.getPropertyValue('--trace-tail')) || 0;
      setN(Math.max(2, Math.min(6, Math.floor((el.clientWidth - tail) / room))));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return n;
}

function findSpan(root: TraceSpan, id: string): TraceSpan | undefined {
  let hit: TraceSpan | undefined;
  walk(root, s => { if (s.id === id) hit = s; });
  return hit;
}

// What a span is called on its row and card: a title and a muted detail
function spanText(span: TraceSpan, agent: { name: string }, model?: string): { title: string; detail?: string } {
  if (span.kind === 'turn') return { title: agent.name, ...(model ? { detail: model } : {}) };
  if (span.node) {
    const detail = span.node.model ?? (span.node.title ? span.node.role : undefined);
    return { title: subagentTitle(span.node, t), ...(detail ? { detail } : {}) };
  }
  const block = span.tool!;
  const target = toolTarget(block);
  // The plain verb ("Read", "Call Notion"), not the row's status form: the bar already says how it went
  const verb = block.mcpServer && !block.verbKey ? t('verb.mcp', { server: block.mcpServer }) : t(block.verbKey ?? `verb.${block.kind}`);
  return { title: verb, ...(target ? { detail: target } : {}) };
}

function SpanIcon({ span, agent }: { span: TraceSpan; agent: { id: AgentId } }) {
  if (span.kind === 'turn') return <AgentMark id={agent.id} className="size-icon" />;
  if (span.node) return <AgentMark id={span.node.harness?.agent ?? agent.id} className="size-icon" />;
  const Icon = toolIcon(span.tool!);
  return <Icon strokeWidth={1.5} className={cn('size-icon', span.state === 'failed' && 'text-danger')} />;
}

// A span's row and, under it, its children in a fold that opens and closes on the shared fold curve (ui/Collapsible)
function SpanTree({ span, depth, ctx }: { span: TraceSpan; depth: number; ctx: TreeContext }) {
  const folded = ctx.folded.has(span.id);
  return (
    <>
      <SpanRow span={span} depth={depth} ctx={ctx} folded={folded} />
      {span.children.length > 0 && (
        <Collapsible.Section open={!folded}>
          {span.children.map(c => <SpanTree key={c.id} span={c} depth={depth + 1} ctx={ctx} />)}
        </Collapsible.Section>
      )}
    </>
  );
}

// The custom properties a bar, a segment or a label computes its place from: start and end in ms, a running end being now
const spanVars = (from: number, to: number | undefined) => ({ '--s': from, '--e': to ?? 'var(--trace-now)' }) as CSSProperties;

function SpanRow({ span, depth, ctx, folded }: { span: TraceSpan; depth: number; ctx: TreeContext; folded: boolean }) {
  const bar = useRef<HTMLSpanElement>(null);
  const entered = useContext(EnteredContext);
  const [enter] = useState(() => entered.current);
  const parent = span.children.length > 0;
  const { title, detail } = spanText(span, ctx.agent, ctx.model);
  const end = spanEnd(span, ctx.now);
  const timed = span.start !== undefined && end !== undefined;
  const ms = timed ? end - span.start! : 0;
  const running = span.end === undefined;
  // Drawn for every parent, shown while it is folded: the bar crossfades between an outline and what its children did
  const segs = parent ? segments(span, ctx.now) : [];
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); ctx.onOpen(span, bar.current ?? (e.currentTarget as HTMLElement)); }
    else if (parent && ((e.key === 'ArrowRight' && folded) || (e.key === 'ArrowLeft' && !folded))) { e.preventDefault(); ctx.onToggle(span.id); }
  };
  // A bar's tone: a failure, a tool, a subagent; the turn's own outline stays neutral unless it failed
  const tone = span.state === 'failed' ? 'fail' : span.kind === 'tool' ? 'tool' : span.kind === 'subagent' ? 'agent' : undefined;
  const shape = parent ? (folded ? 'folded' : 'parent') : 'leaf';
  return (
    <Row role="treeitem" aria-expanded={parent ? !folded : undefined} aria-selected={ctx.selected === span.id} tabIndex={0} data-trace-row data-span={span.id}
      data-enter={enter || undefined}
      className={cn('trace-row min-h-0 px-pad outline-none', ctx.selected === span.id && 'trace-row-selected')} style={{ '--depth': depth } as CSSProperties}
      onClick={e => ctx.onOpen(span, bar.current ?? e.currentTarget)} onKeyDown={onKey}>
      <span className="trace-line trace-cells min-w-0 flex-1 self-stretch">
        <span className="trace-label flex min-w-0 items-center gap-1.5">
          <span className="trace-fold flex size-icon shrink-0 items-center justify-center text-fg-3"
            onClick={parent ? e => { e.stopPropagation(); ctx.onToggle(span.id); } : undefined}>
            {parent && <ChevronDown strokeWidth={1.5} className={cn('trace-chevron size-icon', folded && '-rotate-90')} />}
          </span>
          <span className="flex size-icon shrink-0 items-center justify-center text-fg-2"><SpanIcon span={span} agent={ctx.agent} /></span>
          {/* A tool's verb stays whole and its target gives way; an agent's name (a subagent's title can be a sentence)
              gives way too, after its model */}
          <span className={cn('truncate', span.kind === 'tool' ? 'shrink-0 text-fg-1' : 'min-w-0 text-fg-strong')} title={span.kind === 'tool' ? undefined : title}>{title}</span>
          {detail && <span className="trace-detail min-w-0 truncate text-fg-3" title={detail}>{detail}</span>}
          {span.state === 'failed' && <TriangleAlert strokeWidth={1.5} className="size-icon shrink-0 text-danger" />}
          {parent && folded && <span className="trace-count shrink-0 rounded-sm px-1 text-4 text-fg-2 tabular-nums">+{span.size}</span>}
        </span>
        <span className="relative h-full">
          {timed && (
            <>
              <span ref={bar} className={cn('trace-bar', `trace-bar-${shape}`, tone && `trace-tone-${tone}`, running && 'trace-bar-running')}
                style={spanVars(span.start!, span.end)} />
              {parent && (
                <span className="trace-segs" data-shown={folded || undefined} aria-hidden>
                  {segs.map(sg => <span key={`${sg.from}:${sg.tone}`} className={cn('trace-seg', `trace-tone-${sg.tone}`)} style={spanVars(sg.from, sg.open ? undefined : sg.to)} />)}
                </span>
              )}
              {/* The label rides just past the bar's end; near the right edge the spacer gives way and the label slides along
                  the edge over the bar's tail instead of jumping to its other side. The turn's own duration leads the totals
                  above, so its bar goes without */}
              {span.kind !== 'turn' && <span className="trace-label-lane" style={spanVars(span.start!, span.end)}>
                <span className="trace-label-gap" />
                <span className="trace-bar-label text-3 text-fg-3 tabular-nums">
                  {timeText(ms, running)}
                  {ctx.slowest === span.id && <span className="trace-slowest ml-1.5 rounded-xs px-1 text-4">{t('trace.slowest')}</span>}
                </span>
              </span>}
            </>
          )}
        </span>
      </span>
    </Row>
  );
}

// The text a tool call returned, as far as the transcript keeps it as text
function toolOutput(span: TraceSpan): string | undefined {
  const block = span.tool;
  if (!block) return undefined;
  const all = block.contents ?? (block.content ? [block.content] : []);
  for (const c of all) {
    if (c.type === 'text' && c.text.trim()) return c.text.trim();
    if (c.type === 'list' && c.items.length) return c.items.join('\n');
  }
  if (block.diffStat) return `+${block.diffStat.add} −${block.diffStat.del}`;
  return undefined;
}

function SpanCard({ span, now, agent, model, prompt, onClose }: { span: TraceSpan; now: number; agent: { id: AgentId; name: string }; model?: string; prompt?: string; onClose: () => void }) {
  const text = spanText(span, agent, model);
  // A tool call's card is titled by what it acted on, its verb joins the kind below; agents keep their name on top
  const tool = span.kind === 'tool' && text.detail !== undefined;
  const title = tool ? text.detail! : text.title;
  const end = spanEnd(span, now);
  const kind = t(span.kind === 'turn' ? 'trace.kind.turn' : span.kind === 'subagent' ? 'trace.kind.subagent' : 'trace.kind.tool');
  const sub = [kind, tool ? text.title : text.detail].filter(Boolean).join(' ｜ ');
  const facts: { label: string; value: string }[] = [];
  if (span.start !== undefined && end !== undefined) {
    facts.push({ label: t('trace.duration'), value: timeText(end - span.start, span.state === 'running') });
    facts.push({ label: t('trace.started'), value: `+${timeText(span.start)}` });
  }
  if (span.children.length) facts.push({ label: t('trace.spans'), value: String(span.size) });
  if (span.node?.toolCount && !span.children.length) facts.push({ label: t('trace.tools'), value: String(span.node.toolCount) });
  const input = span.kind === 'turn' ? prompt : span.node ? span.node.task : span.tool && toolTarget(span.tool);
  const output = span.node?.result ? resultText(span.node.result) : toolOutput(span);
  const failure = span.state === 'failed' ? output : undefined;
  return (
    <>
      <div className="flex items-start gap-gap">
        <span className="flex size-ctl shrink-0 items-center justify-center rounded-md border border-line text-fg-2"><SpanIcon span={span} agent={agent} /></span>
        <div className="flex min-w-0 flex-1 flex-col">
          <span className={cn('truncate text-2 text-fg-strong', tool && span.tool?.targetMono && 'font-mono text-mono')} title={title}>{title}</span>
          <span className="truncate text-3 text-fg-3">{sub}</span>
        </div>
        <StateChip state={span.state} />
        <IconButton size="sm" aria-label={t('common.close')} title={t('common.close')} onClick={onClose}><X strokeWidth={1.5} /></IconButton>
      </div>
      {facts.length > 0 && (
        <dl className="grid grid-cols-3 gap-gap rounded-md bg-bg-2 p-gap">
          {facts.map(f => (
            <div key={f.label} className="flex min-w-0 flex-col">
              <dt className="truncate text-4 text-fg-3">{f.label}</dt>
              <dd className="truncate text-2 text-fg-strong tabular-nums">{f.value}</dd>
            </div>
          ))}
        </dl>
      )}
      {failure && (
        <div className="trace-failure flex items-start gap-1.5 rounded-md px-gap py-1.5 text-3">
          <TriangleAlert strokeWidth={1.5} className="mt-0.5 size-icon shrink-0" />
          <span className="trace-clamp min-w-0 break-words">{failure}</span>
        </div>
      )}
      {input && <CardText label={t('trace.input')} text={input} mono={span.kind === 'tool' && !!span.tool?.targetMono} />}
      {output && !failure && <CardText label={t('trace.output')} text={output} />}
    </>
  );
}

function CardText({ label, text, mono }: { label: string; text: string; mono?: boolean }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <span className="text-4 text-fg-3">{label}</span>
      <span className={cn('trace-clamp min-w-0 break-words text-3 text-fg-1', mono && 'font-mono text-mono')}>{text}</span>
    </div>
  );
}
