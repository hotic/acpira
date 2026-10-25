import { useMemo, useRef, useState } from 'react';
import { Shrink } from 'lucide-react';
import type { Turn, Usage } from '@acpira/shared/transcript';
import { t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { compactingNow, liveUsage, overCompactBudget, usageFill } from './usageBreakdown';

// Context usage: a --icon-sized ring inside a --ctl-square button; hovering shows the usage card (Cursor-style), and agents with /compact can be compacted from its title row.
// Focus opens the card only when it arrives from another element (Tab / Shift+Tab). Focus the popup hands back after closing — outside click, Escape, the compact
// button — has no relatedTarget (the card is already gone), and opening on it would reopen the card the user just dismissed
const EMPTY_TURNS: Turn[] = [];

export function ContextRing({ usage, turns = EMPTY_TURNS, compacting: compactingProp, canCompact, compactAt, running, disabled, onCompact, onOpenChange }: {
  usage: Usage; turns?: Turn[]; compacting?: boolean; canCompact: boolean; compactAt?: number; running?: boolean; disabled?: boolean;
  onCompact: () => void; onOpenChange: (open: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const shown = useMemo(() => liveUsage(usage, turns, running), [usage, turns, running]);
  const stale = shown.compactedFrom !== undefined;
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
      <Popover.Portal><Popover.Positioner side="top" align="end" width="lg"><Popover.Popup>
        <UsagePanel
          usage={shown}
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

// Only the agent-reported context and its model-window bar: ACP exposes no authoritative category split,
// and a transcript-based estimate cannot describe a window whose compacted contents stay inside the agent
function UsagePanel({ usage, pct, fill, compactAt, canCompact, overAt, pending, compactDisabled, onCompact }: {
  usage: Usage; pct: number; fill: number;
  compactAt?: number; canCompact: boolean; overAt?: number; pending?: string; compactDisabled?: boolean; onCompact: () => void;
}) {
  const locale = useLocale();
  const fmt = new Intl.NumberFormat(locale).format;
  const stale = usage.compactedFrom !== undefined;
  const mark = [t('usage.used', { n: fmt(usage.used) }), t('usage.limit', { n: fmt(usage.size) }), overAt && t('usage.budget', { n: fmt(overAt) })].filter(Boolean).join(t('common.metaSep'));
  const limited = !!compactAt && compactAt >= usage.size;
  const status = !canCompact ? t('usage.unsupportedShort') : limited ? t('usage.windowLimited') : pending;
  const policyHint = !canCompact ? t('usage.unsupported') : limited ? t('usage.budgetExceedsWindow') : undefined;
  const tone: BudgetTone = !canCompact ? 'unsupported' : limited ? 'limited' : pending ? 'pending' : 'ok';
  const tip = compactAt ? [t('usage.budget', { n: fmt(compactAt) }), status, policyHint].filter(Boolean).join('\n') : undefined;
  return (
    <div className="flex flex-col gap-1 p-1 tabular-nums">
      {/* The compact button stays put whenever the agent supports /compact and is only disabled mid-turn, so the title row never reflows */}
      <div className="flex h-ctl items-center justify-between pl-2">
        <span className="text-2 font-medium text-fg-1">{t('usage.title')}</span>
        {canCompact && (
          <IconButton title={t('usage.compact')} aria-label={t('usage.compact')} disabled={compactDisabled} onClick={onCompact}
            className="disabled:cursor-default disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-fg-2">
            <Shrink strokeWidth={1.75} />
          </IconButton>
        )}
      </div>
      {/* Stale after a compaction the agent has not reported on: say so, and keep the old figure only as "before" */}
      <div className="flex items-baseline justify-between gap-2 px-2 text-3" title={stale ? t('usage.compactedStaleHint') : `${mark}\n${t('usage.reported')}`}>
        <span className="text-fg-2">{stale ? t('usage.compactedStale') : t('usage.usedPctShort', { pct: Math.round(pct * 100) })}</span>
        <span className="text-fg-3">{stale ? t('usage.beforeCompact', { n: fmt(usage.compactedFrom!) }) : `${fmt(usage.used)} / ${fmt(usage.size)}`}</span>
      </div>
      <div className="mx-2 mb-1 flex items-center gap-2">
        <div className="relative h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-active">
          <div className={cn('context-bar-fill h-full bg-fg-2', stale && 'opacity-50')} style={{ width: `${fill * 100}%` }} />
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
    </div>
  );
}
