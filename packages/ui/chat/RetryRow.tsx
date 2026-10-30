import { Check, RotateCw, Unplug, X } from 'lucide-react';
import { memo, useEffect, useState } from 'react';
import type { RetryLog } from '@acpira/shared/transcript';
import type { Locale } from '@acpira/shared/i18n';
import { Disclosure } from '../ui/Disclosure';
import { RowLabel } from '../ui/Row';
import { dateFormat, t, useLocale } from '../i18n';
import { useAutoFold } from './autoFold';
import { OutputPane } from './Terminal';

// One run of provider retries, drawn like a tool row (picked in `lab/retry-cause.preview.tsx` and
// `lab/retry-icon.preview.tsx`): the verb says where the run stands, the trailing text counts the attempt and the wait
// while it lasts, and the card under it holds every failed attempt in the provider's own words. A lost connection
// leads with a plug, anything else with the retry arrow; the icon stays still, the verb shimmers while live
export const RetryRow = memo(function RetryRow({ log }: { log: RetryLog }) {
  const locale = useLocale();
  const fold = useAutoFold();
  const running = log.status === 'in_progress';
  const n = log.attempts.length;
  const verb = running ? t(log.reconnect ? 'retry.reconnecting' : 'retry.retrying')
    : log.status === 'completed' ? t('retry.recovered', { n })
      : log.status === 'failed' ? t('retry.failed', { n }) : t('retry.stopped');
  const Icon = log.reconnect ? Unplug : RotateCw;
  const trailing = running
    ? <RetryMeta attempt={log.attempts.at(-1)?.attempt} max={log.max} nextAt={log.nextAt} />
    : log.status === 'completed' ? <Check className="size-3 text-ok" strokeWidth={2} aria-hidden="true" />
      : log.status === 'failed' ? <X className="size-3 text-danger" strokeWidth={2} aria-hidden="true" /> : undefined;
  return (
    <Disclosure className="action-details data-open:mb-command-after" bodyClassName="pt-gap-half" tone="action"
      lead={<Icon className="size-icon" strokeWidth={1.5} />} trailing={trailing}
      open={fold?.open} onToggle={fold?.onToggle}
      body={<RetryOutput log={log} locale={locale} />}>
      <RowLabel shimmer={running}>{verb}</RowLabel>
    </Disclosure>
  );
});

// "Attempt 5/10, in 12s" while the next attempt waits, the bare attempt once it is under way
function RetryMeta({ attempt, max, nextAt }: { attempt?: number; max?: number; nextAt?: number }) {
  const [now, setNow] = useState(Date.now);
  const left = nextAt === undefined ? 0 : Math.ceil((nextAt - now) / 1000);
  useEffect(() => {
    if (left <= 0 || nextAt === undefined) return;
    // Wake on the next whole second of the countdown
    const timer = setTimeout(() => setNow(Date.now()), Math.max(50, (nextAt - Date.now()) % 1000 || 1000));
    return () => clearTimeout(timer);
  }, [left, nextAt, now]);
  const counted = attempt !== undefined && max !== undefined;
  const text = counted
    ? left > 0 ? t('retry.attemptIn', { attempt, max, seconds: left }) : t('retry.attempt', { attempt, max })
    : left > 0 ? t('retry.in', { seconds: left }) : undefined;
  return text ? <span>{text}</span> : null;
}

// Every attempt on its own line: when it failed and which attempt it was (muted), then the error verbatim. Extra lines of
// one error hang under its first
function RetryOutput({ log, locale }: { log: RetryLog; locale: Locale }) {
  const time = dateFormat(locale, { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false });
  const lines = log.attempts.flatMap((a, i) => {
    const head = [time.format(a.at), a.attempt !== undefined && log.max !== undefined ? t('retry.attempt', { attempt: a.attempt, max: log.max }) : undefined]
      .filter(Boolean).join('  ');
    const [first = '', ...rest] = a.text.split('\n');
    return [{ key: `${i}`, head, text: first }, ...rest.map((line, k) => ({ key: `${i}.${k}`, head: '', text: line }))];
  });
  const copy = lines.map(l => l.head ? `${l.head}  ${l.text}` : `  ${l.text}`).join('\n');
  return (
    <div className="group/code-output code-output">
      <OutputPane text={copy} follow={log.status === 'in_progress'} label={t('retry.log')} copyLabel={t('retry.copyLog')} className="tool-output">
        {lines.map(l => (
          <span key={l.key} className="tool-output-line">
            {l.head ? <><span className="text-fg-3">{l.head}</span>{'  '}</> : '  '}{l.text || ' '}
          </span>
        ))}
      </OutputPane>
    </div>
  );
}
