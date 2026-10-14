import { useEffect, useState } from 'react';
import type { Locale } from '@shared/i18n';
import { useLocale } from '../i18n';
import { durationText } from './folding';

// Running time of a live turn, next to its Working label, the bare duration like the finished head (35 分钟 12 秒). A leaf
// with its own one-second tick, so only this span re-renders; the memoized turn above it keeps the stream's render
// budget. Tabular digits keep the width steady between ticks (it only grows when a unit is added)
export function LiveElapsed({ since }: { since: number }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [since]);
  return <span className="min-w-0 shrink-0 truncate font-normal text-fg-3 tabular-nums">{durationText(now - since)}</span>;
}

// When a prompt went out: the time of day for today, the date as well for anything earlier (and the year once it differs);
// the month is spelled the locale's way (10月13日, Oct 13) rather than as digits next to the time
export function sentTimeText(at: number, now: number, locale: Locale): string {
  const d = new Date(at);
  const today = new Date(now);
  const sameDay = d.toDateString() === today.toDateString();
  const sameYear = d.getFullYear() === today.getFullYear();
  return new Intl.DateTimeFormat(locale, {
    ...(sameDay ? {} : { month: 'short', day: 'numeric', ...(sameYear ? {} : { year: 'numeric' }) }),
    hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
  }).format(d);
}

// A sent prompt's time in the strip under it (`PromptFooter`); the full date and time with seconds is its tooltip
export function SentTime({ at }: { at: number }) {
  const locale = useLocale();
  const full = new Intl.DateTimeFormat(locale, { dateStyle: 'medium', timeStyle: 'medium' }).format(at);
  return (
    <time dateTime={new Date(at).toISOString()} title={full} className="px-1 text-3 text-fg-3 tabular-nums">
      {sentTimeText(at, Date.now(), locale)}
    </time>
  );
}
