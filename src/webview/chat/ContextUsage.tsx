import { useMemo, useState } from 'react';
import { Shrink } from 'lucide-react';
import type { Turn, Usage } from '@shared/transcript';
import { t } from '../i18n';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { compactBudget, estimateUsage, liveUsage, overCompactBudget, usageWindow, type UsageSegment } from './usageBreakdown';

// Context usage: a --icon-sized ring inside a --ctl-square button; hovering shows the breakdown card (Cursor-style), and agents with /compact can be compacted from its title row.
// Focus opens the card only when it arrives from another element (Tab / Shift+Tab). Focus the popup hands back after closing — outside click, Escape, the compact
// button — has no relatedTarget (the card is already gone), and opening on it would reopen the card the user just dismissed
export function ContextRing({ usage, turns, canCompact, compactAt, running, onCompact, onOpenChange }: {
  usage: Usage; turns: Turn[]; canCompact: boolean; compactAt?: number; running?: boolean;
  onCompact: () => void; onOpenChange: (open: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const shown = useMemo(() => liveUsage(usage, turns, running), [usage, turns, running]);
  const ringSize = useMemo(() => usageWindow(shown.size, compactAt), [shown.size, compactAt]);
  const pct = Math.min(1, shown.used / ringSize);
  const budget = compactBudget(shown.size, compactAt);
  const over = overCompactBudget(shown.used, compactAt);
  const segments = useMemo(() => estimateUsage(turns, shown), [turns, shown]);
  const r = 6, c = 2 * Math.PI * r;
  return (
    <Popover.Root open={open} onOpenChange={setOpen} onOpenLifecycle={onOpenChange}>
      <Popover.Trigger openOnHover delay={120} closeDelay={250} onFocus={e => { if (e.relatedTarget) setOpen(true); }}
        render={<button type="button" data-open={open || undefined}
          aria-label={t('usage.usedPct', { pct: Math.round(pct * 100) })}
          className="inline-flex size-ctl shrink-0 items-center justify-center rounded-md text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1 data-[open]:bg-active data-[open]:text-fg-1"
        >
          <svg className="size-icon -rotate-90" viewBox="0 0 16 16" fill="none">
            <circle cx="8" cy="8" r={r} stroke="currentColor" strokeOpacity="0.25" strokeWidth="2" />
            <circle cx="8" cy="8" r={r} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeDasharray={`${c * pct} ${c}`} />
          </svg>
        </button>} />
      <Popover.Portal><Popover.Positioner side="top" align="end" width="lg"><Popover.Popup>
        <UsagePanel
          usage={shown}
          pct={Math.min(1, shown.used / shown.size)}
          segments={segments}
          budget={budget}
          overAt={over && compactAt ? compactAt : undefined}
          pending={over && canCompact ? (running ? t('usage.pendingAfterTurn') : t('usage.pendingBeforeSend')) : undefined}
          onCompact={canCompact ? () => { onCompact(); setOpen(false); } : undefined}
        />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}

// Segment colors and legend dots share one mapping: segment id → chart token
const SEG_COLOR: Record<UsageSegment['id'], string> = {
  user: 'bg-chart-user',
  agent: 'bg-chart-agent',
  tool: 'bg-chart-tool',
  thought: 'bg-chart-thought',
  system: 'bg-chart-system',
};

// Breakdown panel (modeled on Cursor's context usage): title row with the compact button, one summary line (percent left, "~used / size" right), a thin stacked bar,
// then legend rows — square swatch, label, right-aligned count. Hovering a row or bar segment highlights that slice (dims the rest); nothing opens on click.
// The breakdown is a local estimate, so the total carries a "~" and the counts are read as approximate
function UsagePanel({ usage, pct, segments, budget, overAt, pending, onCompact }: {
  usage: Usage; pct: number; segments: UsageSegment[];
  budget?: number; overAt?: number; pending?: string; onCompact?: () => void;
}) {
  const [hov, setHov] = useState<UsageSegment['id']>();
  const mark = overAt ? `${t('usage.used', { n: fmtTokens(usage.used) })} · ${t('usage.limit', { n: fmtTokens(usage.size) })} · ${t('usage.budget', { n: fmtTokens(overAt) })}` : `${t('usage.used', { n: fmtTokens(usage.used) })} · ${t('usage.limit', { n: fmtTokens(usage.size) })}`;
  return (
    <div className="flex flex-col gap-1 p-1 tabular-nums">
      <div className="flex h-ctl items-center justify-between pl-2">
        <span className="text-2 font-medium text-fg-1">{t('usage.title')}</span>
        {onCompact && (
          <IconButton title={t('usage.compact')} aria-label={t('usage.compact')} onClick={onCompact}>
            <Shrink strokeWidth={1.75} />
          </IconButton>
        )}
      </div>
      <div className="flex items-baseline justify-between px-2 text-3" title={mark}>
        <span className="text-fg-2">{t('usage.usedPctShort', { pct: Math.round(pct * 100) })}</span>
        <span className="text-fg-3">{t('usage.about', { n: fmtTokens(usage.used) })} / {fmtTokens(usage.size)}{usage.cost !== undefined ? t('usage.cost', { n: usage.cost.toFixed(2) }) : ''}</span>
      </div>
      {(overAt || pending) && (
        <div className="px-2 text-3 text-fg-2">
          {overAt ? t('usage.overBudget', { n: fmtTokens(overAt) }) : null}
          {overAt && pending ? ' · ' : ''}
          {pending}
        </div>
      )}
      <div className="relative mx-2 mb-1 flex h-1.5 overflow-hidden rounded-full bg-active">
        {segments.map(s =>
          s.tokens > 0 && (
            <div
              key={s.id}
              onMouseEnter={() => setHov(s.id)}
              onMouseLeave={() => setHov(undefined)}
              className={cn('h-full transition-opacity', SEG_COLOR[s.id], hov === s.id && 'brightness-125', hov && hov !== s.id && 'opacity-30')}
              style={{ width: `${(s.tokens / usage.size) * 100}%` }}
            />
          ),
        )}
        {budget && (
          <div
            className="absolute inset-y-0 w-px bg-fg-1"
            style={{ left: `${(budget / usage.size) * 100}%` }}
            title={t('usage.budget', { n: fmtTokens(budget) })}
          />
        )}
      </div>
      <div className="flex flex-col">
        {segments.map(s => (
          <div
            key={s.id}
            title={s.hint}
            onMouseEnter={() => setHov(s.id)}
            onMouseLeave={() => setHov(undefined)}
            className={cn('flex min-h-row w-full items-center gap-2 rounded-md px-2 text-3 transition-colors', hov === s.id && 'bg-hover')}
          >
            <span className="flex w-lead shrink-0 justify-center"><span className={cn('size-2.5 rounded-xs', SEG_COLOR[s.id])} /></span>
            <span className="flex-1 text-fg-1">{s.label}</span>
            <span className="text-fg-2">{fmtTokens(s.tokens)}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function fmtTokens(n: number) {
  if (n < 1000) return String(n);
  const k = n / 1000;
  return `${k >= 100 ? Math.round(k) : k.toFixed(1).replace(/\.0$/, '')}K`;
}
