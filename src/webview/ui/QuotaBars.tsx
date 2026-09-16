import { useEffect, useState } from 'react';
import type { AccountQuota, QuotaWindow } from '@shared/transcript';
import { cn } from './cn';
import { getLocale, t, tOr } from '../i18n';

// Remaining allowance controls the tube color: healthy, low, nearly exhausted.
const LOW = 0.3;
const CRITICAL = 0.1;

// Each window has a metadata line above its tube. Wide account rows place windows side by side.
export function QuotaBars({ quota, className }: { quota: AccountQuota; className?: string }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(timer);
  }, []);
  const balance = balanceLabel(quota);
  if (!quota.windows.length && !balance) return null;
  return (
    <span
      role="img"
      aria-label={[...quota.windows.map(w => `${t('quota.aria', { window: windowLabel(w.id), pct: Math.round(w.remaining * 100) })}${t('common.metaSep')}${resetLabel(w, now)}`), balance].filter(Boolean).join(t('common.listSep'))}
      className={cn('@container/quota block w-full min-w-0 pt-2 text-3 text-fg-2 tabular-nums', className)}
    >
      <span className={cn('grid grid-cols-1 gap-gap', quota.windows.length > 1 && '@quota/quota:grid-cols-2')}>
        {quota.windows.map(w => <QuotaBar key={w.id} window={w} now={now} />)}
      </span>
      {balance && <span className={cn('flex min-w-0 items-baseline justify-between gap-2', quota.windows.length > 0 && 'mt-2')} title={balance}>
        <span className="min-w-0 truncate">{t('quota.onDemandBalance')}</span>
        <span className="shrink-0 whitespace-nowrap">{formatBalance(quota.onDemandBalanceUsd!)}</span>
      </span>}
    </span>
  );
}

function QuotaBar({ window: w, now }: { window: QuotaWindow; now: number }) {
  const pct = Math.round(w.remaining * 100);
  const label = windowLabel(w.id);
  const time = w.resetsAt && Number.isFinite(Date.parse(w.resetsAt)) ? fmtUntil(w.resetsAt, now) : undefined;
  const hint = time ? t('quota.hint', { window: label, pct, time }) : t('quota.hintNoReset', { window: label, pct });
  return (
    <span className="flex min-w-0 flex-col gap-1" title={hint}>
      <span className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-2 whitespace-nowrap">
        <span className="shrink-0">{label}</span>
        <span className="min-w-0 max-w-full justify-self-start truncate rounded-sm bg-active px-1 text-fg-2" title={w.resetsAt && Number.isFinite(Date.parse(w.resetsAt)) ? new Date(w.resetsAt).toLocaleString() : undefined}>{time && Date.parse(w.resetsAt!) > now ? fmtUntil(w.resetsAt!, now, true) : resetLabel(w, now)}</span>
        <span className="ml-auto shrink-0">{t('quota.left', { pct })}</span>
      </span>
      <span className="h-(--quota-track) overflow-hidden rounded-full bg-active">
        <span
          className={cn('block h-full rounded-full', w.remaining <= CRITICAL ? 'bg-(--quota-danger)' : w.remaining <= LOW ? 'bg-(--quota-warn)' : 'bg-(--quota-ok)')}
          style={{ width: `${pct}%` }}
        />
      </span>
    </span>
  );
}

// Compact one-line summary for tooltips ("Weekly 94%"); several windows joined with a comma
export function quotaSummary(quota: AccountQuota): string {
  return [...quota.windows.map(w => `${windowLabel(w.id)} ${Math.round(w.remaining * 100)}%`), balanceLabel(quota)].filter(Boolean).join(t('common.metaSep'));
}

function formatBalance(amount: number): string {
  return new Intl.NumberFormat(getLocale(), { style: 'currency', currency: 'USD', currencyDisplay: 'narrowSymbol' }).format(amount);
}

function balanceLabel(quota: AccountQuota): string | undefined {
  return quota.onDemandBalanceUsd !== undefined && Number.isFinite(quota.onDemandBalanceUsd)
    ? `${t('quota.onDemandBalance')} ${formatBalance(quota.onDemandBalanceUsd)}` : undefined;
}

// Window ids come from the provider; the known ones are translated, anything else is shown as sent
function windowLabel(id: string): string {
  return tOr(`quota.window.${id}`, id);
}

// The compact badge shows up to two units; the tooltip names the exact reset timestamp.
function resetLabel(w: QuotaWindow, now: number): string {
  if (!w.resetsAt || !Number.isFinite(Date.parse(w.resetsAt))) return t('quota.resetUnknown');
  if (Date.parse(w.resetsAt) <= now) return t('quota.resetPending');
  return t('quota.reset', { time: fmtUntil(w.resetsAt, now) });
}

function fmtUntil(iso: string, now: number, compact = false): string {
  const ms = Math.max(0, Date.parse(iso) - now);
  const minutes = Math.ceil(ms / 60_000);
  const unit = (name: 'minutes' | 'hours' | 'days', n: number) => t(compact ? `quota.time.short.${name}` : `quota.time.${name}`, { n });
  const separator = compact && getLocale() === 'zh-CN' ? '' : ' ';
  if (minutes < 60) return unit('minutes', minutes);
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return [unit('hours', hours), minutes % 60 ? unit('minutes', minutes % 60) : undefined].filter(Boolean).join(separator);
  return [unit('days', Math.floor(hours / 24)), hours % 24 ? unit('hours', hours % 24) : undefined].filter(Boolean).join(separator);
}
