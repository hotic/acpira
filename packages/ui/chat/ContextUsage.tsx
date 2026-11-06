import { useEffect, useMemo, useRef, useState } from 'react';
import { ChartBarStacked, Shrink } from 'lucide-react';
import type { ContextCategory, ContextReport } from '@acpira/shared/context';
import type { Turn, Usage } from '@acpira/shared/transcript';
import { contextApi, lastSplit } from '../context/api';
import { panel } from '../panel/store';
import { CATEGORY_BG, categoryLabel } from '../context/categories';
import { desktopShell } from '../desktop';
import { t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { Chip } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { compactingNow, liveUsage, overCompactBudget, usageFill } from './usageBreakdown';

// Context usage: a --icon-sized ring inside a --ctl-square button; hovering shows the usage card (Cursor-style), and agents with /compact can be compacted from its title row.
// Focus opens the card only when it arrives from another element (Tab / Shift+Tab). Focus the popup hands back after closing — outside click, Escape, the compact
// button — has no relatedTarget (the card is already gone), and opening on it would reopen the card the user just dismissed
const EMPTY_TURNS: Turn[] = [];

export function ContextRing({ sessionId, usage, turns = EMPTY_TURNS, compacting: compactingProp, canCompact, compactAt, running, disabled, onCompact, onOpenChange }: {
  sessionId?: string; usage: Usage; turns?: Turn[]; compacting?: boolean; canCompact: boolean; compactAt?: number; running?: boolean; disabled?: boolean;
  onCompact: () => void; onOpenChange: (open: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const shown = useMemo(() => liveUsage(usage, turns, running), [usage, turns, running]);
  const stale = shown.compactedFrom !== undefined;
  // The agent's own split of what it last sent, when it reports one; a stale figure has none to match
  const fetched = useContextSplit(sessionId, shown.used, shown.size);
  const split = stale ? undefined : fetched;
  const fill = usageFill(shown);
  const pct = Math.min(1, shown.used / shown.size);
  const compacting = useMemo(() => compactingProp ?? compactingNow(turns, running), [compactingProp, turns, running]);
  const shift = useRingShift(fill, stale);
  // A stale snapshot still holds the pre-compaction figure: it must not read as over the threshold
  const over = !stale && overCompactBudget(shown.used, compactAt);
  const r = 6, c = 2 * Math.PI * r;
  return (
    <Popover.Root open={open} onOpenChange={setOpen} onOpenLifecycle={onOpenChange}>
      <Popover.Trigger openOnHover delay={120} closeDelay={250} onFocus={e => { if (e.relatedTarget) setOpen(true); }}
        render={<button type="button" data-open={open || undefined}
          aria-label={stale ? t('usage.compactedStale') : t('usage.usedPct', { pct: Math.round(pct * 100) })}
          className="inline-flex size-ctl shrink-0 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-active hover:text-fg-1 focus-visible:bg-active focus-visible:text-fg-1 data-[open]:bg-active data-[open]:text-fg-1"
        >
          <svg className="context-ring size-icon -rotate-90" viewBox="0 0 16 16" fill="none"
            data-shift={shift} data-stale={stale || undefined} data-compacting={compacting || undefined}>
            <circle cx="8" cy="8" r={r} stroke="currentColor" strokeOpacity="0.25" strokeWidth="2" />
            {/* The fill is a dash offset so it can transition (motion.css picks the pace from data-shift) */}
            <circle className="context-ring-arc" cx="8" cy="8" r={r} stroke="currentColor" strokeWidth="2" strokeLinecap="round"
              strokeDasharray={c} strokeDashoffset={c * (1 - fill)} />
          </svg>
        </button>} />
      <Popover.Portal><Popover.Positioner side="top" align="end" width={split ? 'xl' : 'lg'}><Popover.Popup>
        <UsagePanel
          usage={shown}
          split={split}
          onReport={split && desktopShell() ? () => { panel.openContext(); setOpen(false); } : undefined}
          pct={pct}
          fill={fill}
          compactAt={compactAt}
          canCompact={canCompact}
          overAt={over && compactAt ? compactAt : undefined}
          pending={over && canCompact ? (running ? t('usage.pendingAfterTurn') : t('usage.pendingBeforeSend')) : undefined}
          compactDisabled={running || disabled}
          onCompact={() => { onCompact(); setOpen(false); }}
        />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}

// Asked again once the figure has been still this long: a usage update arrives per model call, and the agent answers
// from the call it just made
const SPLIT_SETTLE_MS = 300;

// The split for the card, refreshed after every change of the figure; the last one per session shows meanwhile, so the
// card opens with its parts and keeps its size. Nothing for an agent that reports none
function useContextSplit(sessionId: string | undefined, used: number, size: number): ContextReport | undefined {
  const [split, setSplit] = useState(() => (sessionId ? lastSplit.get(sessionId) ?? undefined : undefined));
  useEffect(() => {
    setSplit(sessionId ? lastSplit.get(sessionId) ?? undefined : undefined);
    if (!sessionId) return;
    let live = true;
    const timer = setTimeout(() => {
      void contextApi.report(sessionId).then(r => {
        lastSplit.set(sessionId, r ?? null);
        if (live) setSplit(r);
      });
    }, SPLIT_SETTLE_MS);
    return () => { live = false; clearTimeout(timer); };
  }, [sessionId, used, size]);
  return split;
}

type RingShift = 'rise' | 'drop' | 'snap';

// How the fill moves to its new value: compaction (a smaller figure, or the stale sliver) eases down slowly; the real
// figure replacing a stale sliver snaps; growth follows at an ordinary pace. Held in a ref so a re-render with the same
// values keeps the pace of the transition already running
function useRingShift(fill: number, stale: boolean): RingShift {
  const last = useRef<{ fill: number; stale: boolean; shift: RingShift }>({ fill, stale, shift: 'rise' });
  const prev = last.current;
  if (prev.fill !== fill || prev.stale !== stale) {
    last.current = { fill, stale, shift: stale ? 'drop' : prev.stale ? 'snap' : fill < prev.fill ? 'drop' : 'rise' };
  }
  return last.current.shift;
}

// Threshold tick and its tail number share one status color; the status words themselves live in the tooltip.
const BUDGET_TONE = {
  ok: { tick: 'bg-fg-1', text: 'text-fg-3' },
  pending: { tick: 'bg-accent', text: 'text-accent' },
  limited: { tick: 'bg-warn', text: 'text-warn' },
  unsupported: { tick: 'bg-fg-3', text: 'text-fg-3 opacity-60' },
} as const;
type BudgetTone = keyof typeof BUDGET_TONE;

// A chip the turn disables keeps its place, so the head never reflows
const CHIP_DISABLED = 'disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-fg-2';

// The agent-reported context and its model-window bar. With a split (`split`: the built-in agent's own, or the engine's
// estimate for any other agent, context.ts) the bar is cut by category and the categories are listed under it
function UsagePanel({ usage, split, pct, fill, compactAt, canCompact, overAt, pending, compactDisabled, onCompact, onReport }: {
  usage: Usage; split?: ContextReport; pct: number; fill: number;
  compactAt?: number; canCompact: boolean; overAt?: number; pending?: string; compactDisabled?: boolean; onCompact: () => void; onReport?: () => void;
}) {
  const [over, setOver] = useState<ContextCategory>();
  const locale = useLocale();
  const fmt = new Intl.NumberFormat(locale).format;
  const stale = usage.compactedFrom !== undefined;
  const mark = [t('usage.used', { n: fmt(usage.used) }), t('usage.limit', { n: fmt(usage.size) }), overAt && t('usage.budget', { n: fmt(overAt) })].filter(Boolean).join(t('common.metaSep'));
  const limited = !!compactAt && compactAt >= usage.size;
  const status = !canCompact ? t('usage.unsupportedShort') : limited ? t('usage.windowLimited') : pending;
  const policyHint = !canCompact ? t('usage.unsupported') : limited ? t('usage.budgetExceedsWindow') : undefined;
  const tone: BudgetTone = !canCompact ? 'unsupported' : limited ? 'limited' : pending ? 'pending' : 'ok';
  const tip = compactAt ? [t('usage.budget', { n: fmt(compactAt) }), status, policyHint].filter(Boolean).join('\n') : undefined;
  // Parts of the bar in proportion to the split, filling what the figure fills (the split may trail a fresh figure)
  const parts = split && split.used > 0 ? split.categories.map(c => ({ id: c.id, width: (c.tokens / split.used) * fill })) : undefined;
  const figure = (n: number) => (split?.estimated ? t('usage.about', { n: fmt(n) }) : fmt(n));
  return (
    <div className="flex flex-col gap-1 p-1 tabular-nums">
      {/* The model menu's head (ModelPicker): a quiet label, then icon chips named by their tooltips, Compact before the
          report. Compact stays put whenever the agent supports /compact and is only disabled mid-turn */}
      <div className="flex min-h-ctl-sm items-center justify-between px-2">
        <span className="text-3 text-fg-3">{t('usage.title')}</span>
        <span className="-mr-1.5 flex items-center gap-0.5">
          {canCompact && (
            <Chip caret={false} icon={<Shrink strokeWidth={1.75} />} title={t('usage.compact')} aria-label={t('usage.compact')}
              disabled={compactDisabled} onClick={onCompact} className={CHIP_DISABLED}>{null}</Chip>
          )}
          {onReport && (
            <Chip caret={false} icon={<ChartBarStacked strokeWidth={1.75} />} title={t('usage.report')} aria-label={t('usage.report')} onClick={onReport}>{null}</Chip>
          )}
        </span>
      </div>
      {/* Stale after a compaction the agent has not reported on: say so, and keep the old figure only as "before" */}
      <div className="flex items-baseline justify-between gap-2 px-2 text-3" title={stale ? t('usage.compactedStaleHint') : `${mark}\n${t('usage.reported')}`}>
        <span className="text-fg-2">{stale ? t('usage.compactedStale') : t('usage.usedPctShort', { pct: Math.round(pct * 100) })}</span>
        <span className="text-fg-3">{stale ? t('usage.beforeCompact', { n: fmt(usage.compactedFrom!) }) : `${fmt(usage.used)} / ${fmt(usage.size)}`}</span>
      </div>
      <div className="mx-2 mb-1 flex items-center gap-2">
        <div className="relative flex h-1.5 min-w-0 flex-1 gap-px overflow-hidden rounded-full bg-active">
          {parts
            ? parts.map(p => (
              <span key={p.id} onMouseEnter={() => setOver(p.id)} onMouseLeave={() => setOver(undefined)}
                className={cn('context-bar-fill h-full shrink-0', CATEGORY_BG[p.id], over && over !== p.id && 'opacity-25')} style={{ width: `${p.width * 100}%` }} />
            ))
            : <div className={cn('context-bar-fill h-full bg-fg-2', stale && 'opacity-50')} style={{ width: `${fill * 100}%` }} />}
          {compactAt && (
            <div
              className={cn('absolute inset-y-0 w-0.5', BUDGET_TONE[tone].tick)}
              style={{ left: `${Math.min(compactAt / usage.size, 0.99) * 100}%` }}
              title={tip}
            />
          )}
        </div>
        {compactAt && (
          <span className={cn('shrink-0 text-3', BUDGET_TONE[tone].text)} title={tip} aria-label={t('usage.budget', { n: fmt(compactAt) })}>
            {fmt(compactAt)}
          </span>
        )}
      </div>
      {split && (
        <div className="flex flex-col border-t border-line pt-1" title={split.estimated ? t('context.estimatedHint') : undefined}>
          {split.categories.map(c => (
            <div key={c.id} onMouseEnter={() => setOver(c.id)} onMouseLeave={() => setOver(undefined)}
              className={cn('flex min-h-row-dense items-center gap-2 rounded-sm px-2 text-3 transition-colors', over === c.id ? 'bg-hover text-fg-1' : 'text-fg-2')}>
              <span aria-hidden className={cn('size-1.5 shrink-0 rounded-full', CATEGORY_BG[c.id])} />
              <span className="min-w-0 flex-1 truncate">{categoryLabel(c.id)}</span>
              <span className="text-fg-3">{figure(c.tokens)}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
