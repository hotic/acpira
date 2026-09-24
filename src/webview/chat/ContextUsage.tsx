import { useMemo, useState } from 'react';
import { Shrink } from 'lucide-react';
import type { Turn, Usage } from '@shared/transcript';
import { t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { IconButton } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { liveUsage, overCompactBudget, usageWindow } from './usageBreakdown';

// Context usage: a --icon-sized ring inside a --ctl-square button; hovering shows the usage card (Cursor-style), and agents with /compact can be compacted from its title row.
// Focus opens the card only when it arrives from another element (Tab / Shift+Tab). Focus the popup hands back after closing — outside click, Escape, the compact
// button — has no relatedTarget (the card is already gone), and opening on it would reopen the card the user just dismissed
export function ContextRing({ usage, turns, canCompact, compactAt, running, disabled, onCompact, onOpenChange }: {
  usage: Usage; turns: Turn[]; canCompact: boolean; compactAt?: number; running?: boolean; disabled?: boolean;
  onCompact: () => void; onOpenChange: (open: boolean) => void;
}) {
  const [open, setOpen] = useState(false);
  const shown = useMemo(() => liveUsage(usage, turns, running), [usage, turns, running]);
  const ringSize = usageWindow(shown.size);
  const pct = Math.min(1, shown.used / ringSize);
  const over = overCompactBudget(shown.used, compactAt);
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
          compactAt={compactAt}
          canCompact={canCompact}
          overAt={over && compactAt ? compactAt : undefined}
          pending={over && canCompact ? (running ? t('usage.pendingAfterTurn') : t('usage.pendingBeforeSend')) : undefined}
          onCompact={canCompact && !running && !disabled ? () => { onCompact(); setOpen(false); } : undefined}
        />
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
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
function UsagePanel({ usage, pct, compactAt, canCompact, overAt, pending, onCompact }: {
  usage: Usage; pct: number;
  compactAt?: number; canCompact: boolean; overAt?: number; pending?: string; onCompact?: () => void;
}) {
  const locale = useLocale();
  const fmt = new Intl.NumberFormat(locale).format;
  const mark = [t('usage.used', { n: fmt(usage.used) }), t('usage.limit', { n: fmt(usage.size) }), overAt && t('usage.budget', { n: fmt(overAt) })].filter(Boolean).join(t('common.metaSep'));
  const limited = !!compactAt && compactAt >= usage.size;
  const status = !canCompact ? t('usage.unsupportedShort') : limited ? t('usage.windowLimited') : pending;
  const policyHint = !canCompact ? t('usage.unsupported') : limited ? t('usage.budgetExceedsWindow') : undefined;
  const tone: BudgetTone = !canCompact ? 'unsupported' : limited ? 'limited' : pending ? 'pending' : 'ok';
  const tip = compactAt ? [t('usage.budget', { n: fmt(compactAt) }), status, policyHint].filter(Boolean).join('\n') : undefined;
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
      <div className="flex items-baseline justify-between px-2 text-3" title={`${mark}\n${t('usage.reported')}`}>
        <span className="text-fg-2">{t('usage.usedPctShort', { pct: Math.round(pct * 100) })}</span>
        <span className="text-fg-3">{fmt(usage.used)} / {fmt(usage.size)}{usage.cost !== undefined ? t('usage.cost', { n: usage.cost.toFixed(2) }) : ''}</span>
      </div>
      <div className="mx-2 mb-1 flex items-center gap-2">
        <div className="relative h-1.5 min-w-0 flex-1 overflow-hidden rounded-full bg-active">
          <div className="h-full bg-fg-2" style={{ width: `${pct * 100}%` }} />
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
