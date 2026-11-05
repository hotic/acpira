import { useEffect, useMemo, useState } from 'react';
import { Braces } from 'lucide-react';
import type { ContextRaw, ContextReport } from '@acpira/shared/context';
import { isRemoteFolder } from '@acpira/shared/remote';
import { t, useLocale } from '../i18n';
import { IconButton } from '../ui/Button';
import { vscodeApi } from '../vscodeApi';
import { panel } from '../panel/store';
import { contextApi, lastSplit } from './api';
import { ContextReportView } from './ContextReport';
import { CopyButton } from './Parts';

// The tool panel's context tab (panel/store.ts kind 'context'): the report of whichever session the panel is on, so it
// works under every panel scope. Asked of the agent while the tab shows (context/api.ts): again after every model call
// (the usage figure moved), and every few seconds while a turn runs, since a turn makes several requests

export interface ContextTabInfo {
  // The settings' auto-compaction threshold, for the bar's tick
  compactAt?: number;
  home?: string;
  // The ring's figure: a change means a new request went out
  used?: number;
}

const POLL_MS = 3000;
const SETTLE_MS = 300;
const AGO_TICK_MS = 15000;

function ago(at: number, locale: string): string {
  const s = Math.max(0, Math.round((Date.now() - at) / 1000));
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: 'auto' });
  if (s < 60) return rtf.format(-s, 'second');
  if (s < 3600) return rtf.format(-Math.floor(s / 60), 'minute');
  if (s < 86400) return rtf.format(-Math.floor(s / 3600), 'hour');
  return rtf.format(-Math.floor(s / 86400), 'day');
}

// The detailed report: undefined until the first answer, null when the agent has none
function useReport(sessionId: string, shown: boolean, running: boolean, used: number | undefined) {
  const [report, setReport] = useState<ContextReport | null | undefined>(() => (lastSplit.get(sessionId) === null ? null : undefined));
  // Another session's report never shows under this one
  const [of, setOf] = useState(sessionId);
  if (of !== sessionId) { setOf(sessionId); setReport(undefined); }
  useEffect(() => {
    if (!shown) return;
    let live = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = () => {
      void contextApi.report(sessionId, true).then(r => {
        if (!live) return;
        // The same request, the same answer: keep the old object so the tree and the map keep their state
        setReport(prev => (prev && r && prev.rev === r.rev && prev.used === r.used && prev.at === r.at ? prev : r ?? null));
        if (running) timer = setTimeout(ask, POLL_MS);
      });
    };
    timer = setTimeout(ask, SETTLE_MS);
    return () => { live = false; clearTimeout(timer); };
  }, [sessionId, shown, running, used]);
  return report;
}

export function ContextTab({ sessionId, root, running, shown, info }: {
  sessionId: string; root: string; running: boolean; shown: boolean; info: ContextTabInfo;
}) {
  const locale = useLocale();
  const report = useReport(sessionId, shown, running, info.used);
  const [raw, setRaw] = useState<{ raw?: ContextRaw; error?: string }>();
  const [, tick] = useState(0);
  useEffect(() => { setRaw(undefined); }, [sessionId]);
  // The "6 s ago" moves on
  useEffect(() => {
    if (!shown) return;
    const timer = setInterval(() => tick(n => n + 1), AGO_TICK_MS);
    return () => clearInterval(timer);
  }, [shown]);
  // Esc leaves the raw request first (the report's own Esc then has nothing open to close)
  useEffect(() => {
    if (!raw || !shown) return;
    const key = (e: KeyboardEvent) => { if (e.key === 'Escape') { e.preventDefault(); setRaw(undefined); } };
    window.addEventListener('keydown', key, true);
    return () => window.removeEventListener('keydown', key, true);
  }, [raw, shown]);
  const remote = isRemoteFolder(root);
  const paths = useMemo(() => ({ home: info.home, cwd: remote ? undefined : root }), [info.home, root, remote]);
  if (!shown) return null;
  // A file of the project opens in the panel's Files tab; one elsewhere on this machine with the system
  const openFile = (path: string) => {
    const rel = path.startsWith(`${root.replace(/[\\/]+$/, '')}/`) || path.startsWith(`${root.replace(/[\\/]+$/, '')}\\`)
      ? path.slice(root.replace(/[\\/]+$/, '').length + 1) : undefined;
    if (rel) panel.openFile(root, rel.replace(/\\/g, '/'));
    else vscodeApi().postMessage({ type: 'openPath', path });
  };
  const rawText = raw?.raw ? JSON.stringify(raw.raw.body, null, 2) : undefined;
  const actions = <>
    {report?.at !== undefined && <span className="text-3 text-fg-3">{ago(report.at, locale)}</span>}
    <IconButton size="sm" title={t('context.raw')} aria-label={t('context.raw')} aria-pressed={!!raw}
      className="aria-pressed:bg-active aria-pressed:text-fg-1" onClick={() => (raw ? setRaw(undefined) : void contextApi.raw(sessionId).then(setRaw))}>
      <Braces strokeWidth={1.5} />
    </IconButton>
  </>;
  return (
    <div data-context-tab data-report-scroll className="scroll-thin absolute inset-0 overflow-y-auto [scrollbar-gutter:stable]">
      <div data-context-report className="flex min-h-full flex-col px-pad pb-(--report-gap) tabular-nums">
        {raw
          ? <RawView raw={raw.raw} error={raw.error} text={rawText} onClose={() => setRaw(undefined)} />
          : report
            ? <ContextReportView key={sessionId} report={report} sessionId={sessionId} compactAt={info.compactAt} paths={paths}
              onOpenFile={remote ? undefined : openFile} actions={actions} />
            : report === null && <div className="m-auto text-2 text-fg-3">{t('context.empty')}</div>}
      </div>
    </div>
  );
}

// The last request exactly as it went out, in the provider's wire format (images, signatures and keys left out by the
// agent, acpira-agent context.rs `raw`)
function RawView({ raw, error, text, onClose }: { raw?: ContextRaw; error?: string; text?: string; onClose: () => void }) {
  const head = (
    <div className="flex h-ctl shrink-0 items-center gap-2 text-3">
      <span className="text-2 font-medium text-fg-1">{t('context.raw')}</span>
      {raw && <span className="min-w-0 truncate font-mono text-mono text-fg-3">{[raw.format, raw.url].filter(Boolean).join(' ')}</span>}
      <span className="flex-1" />
      {text !== undefined && <CopyButton text={text} />}
      <IconButton size="sm" title={t('context.raw')} aria-label={t('context.raw')} aria-pressed className="bg-active text-fg-1" onClick={onClose}>
        <Braces strokeWidth={1.5} />
      </IconButton>
    </div>
  );
  if (!raw || text === undefined) return <div className="flex flex-col gap-1 pt-gap">{head}<div className="py-page text-center text-2 text-fg-3">{error ?? t('context.empty')}</div></div>;
  return (
    <div className="flex flex-col gap-1 pt-gap">
      {head}
      <pre className="scroll-thin m-0 overflow-x-auto rounded-md bg-bg-1 px-pad py-gap font-mono text-mono whitespace-pre text-fg-2">{text}</pre>
    </div>
  );
}
