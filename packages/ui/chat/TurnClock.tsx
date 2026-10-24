import { useEffect, useState } from 'react';
import type { Locale } from '@acpira/shared/i18n';
import { dateFormat, useLocale } from '../i18n';
import { durationText } from './folding';

// Running time of a live turn, next to its Working label, the bare duration like the finished head (35 分钟 12 秒). A leaf
// with its own one-second tick, so only this span re-renders; the memoized turn above it keeps the stream's render
// budget. Tabular digits keep the width steady between ticks (it only grows when a unit is added). Nothing shows during
// the first second (a bare "0s" next to Working says nothing); ticks land on whole seconds after `since`, so "1s"
// appears exactly when one second has passed
export function LiveElapsed({ since }: { since: number }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const tick = () => {
      const at = Date.now();
      setNow(at);
      // Delay to the next whole second since `since` (a clock behind `since` waits for the first one)
      const delay = 1000 - (Math.max(0, at - since) % 1000);
      timer = setTimeout(tick, delay);
    };
    tick();
    return () => clearTimeout(timer);
  }, [since]);
  if (now - since < 1000) return null;
  return <span className="min-w-0 shrink-0 truncate font-normal text-fg-3 tabular-nums">{durationText(now - since)}</span>;
}

// When a prompt went out: the time of day for today, the date as well for anything earlier (and the year once it differs);
// the month is spelled the locale's way (10月13日, Oct 13) rather than as digits next to the time
export function sentTimeText(at: number, now: number, locale: Locale): string {
  const d = new Date(at);
  const today = new Date(now);
  const sameDay = d.toDateString() === today.toDateString();
  const sameYear = d.getFullYear() === today.getFullYear();
  return dateFormat(locale, {
    ...(sameDay ? {} : { month: 'short', day: 'numeric', ...(sameYear ? {} : { year: 'numeric' }) }),
    hour: '2-digit', minute: '2-digit', hourCycle: 'h23',
  }).format(d);
}

// A sent prompt's time in the strip under it (`PromptFooter`); the full date and time with seconds is its tooltip
export function SentTime({ at }: { at: number }) {
  const locale = useLocale();
  const full = dateFormat(locale, { dateStyle: 'medium', timeStyle: 'medium' }).format(at);
  return (
    <time dateTime={new Date(at).toISOString()} title={full} className="px-1 text-3 text-fg-3 tabular-nums">
      {sentTimeText(at, Date.now(), locale)}
    </time>
  );
}
