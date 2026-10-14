import { useEffect, useState } from 'react';
import { STORAGE_SEGMENTS, formatBytes, type StorageOutcome, type StorageSegmentKind, type StorageView } from '@shared/storage';
import type { MsgKey } from '@shared/i18n';
import { cn } from '../ui/cn';
import { getLocale, t } from '../i18n';
import { Count, ItemRow, Note, Section, SourceLink } from './controls';
import type { SettingsEnv, SettingsHandlers } from './SettingsShell';

// Settings → Storage: what the data directory holds by owner, drawn as one bar, and the cleanups that are reviewed
// before they run. Everything is measured by the engine (store/storage.rs); this page renders it and sends actions back

export interface StorageState {
  view: StorageView;
  // The action that triggered this reply, and its error when it failed
  outcome?: StorageOutcome;
  error?: string;
}

// The bar's fill per segment; the hues are tokens validated as a set (styles/tokens.css)
const FILL: Record<StorageSegmentKind, string> = {
  sessions: 'bg-storage-sessions',
  attachments: 'bg-storage-attachments',
  agentLogs: 'bg-storage-agent-logs',
  reclaimable: 'bg-storage-reclaimable',
  other: 'bg-storage-other',
};

const LABEL: Record<StorageSegmentKind, MsgKey> = {
  sessions: 'storage.seg.sessions',
  attachments: 'storage.seg.attachments',
  agentLogs: 'storage.seg.agentLogs',
  reclaimable: 'storage.seg.reclaimable',
  other: 'storage.seg.other',
};

// The other segment lists this many of its largest entries before "show all"
const OTHER_SHOWN = 6;

export const bytes = (n: number) => formatBytes(n, getLocale());

export interface StoragePageProps {
  state?: StorageState;
  env: SettingsEnv;
  on: SettingsHandlers;
}

export function StoragePage({ state, env, on }: StoragePageProps) {
  // Measured on every visit: other windows and the CLIs write meanwhile
  useEffect(() => { on.storage?.(); }, [on]);
  const view = state?.view;
  const reveal = (path?: string) => on.storageAction?.({ kind: 'reveal', path });
  return <>
    {state?.error && <Section><Note><span className="truncate text-danger" title={state.error}>{state.error}</span></Note></Section>}
    <Overview view={view} env={env} onReveal={() => reveal()} />
    {view && view.other.length > 0 && <OtherFiles view={view} onReveal={reveal} />}
  </>;
}

// The total, the bar and its legend; before the first scan the same rows hold their place with a faint dash
function Overview({ view, env, onReveal }: { view?: StorageView; env: SettingsEnv; onReveal: () => void }) {
  const seg = (k: StorageSegmentKind) => view?.segments.find(s => s.kind === k);
  const shown = view?.segments.filter(s => s.bytes > 0) ?? [];
  const summary = shown.map(s => `${t(LABEL[s.kind])} ${bytes(s.bytes)}`).join(t('common.listSep'));
  return (
    <Section title={t('storage.overview')} desc={view ? t('storage.total', { size: bytes(view.total) }) : t('storage.scanning')}>
      <div className="flex min-h-(--setting-row) items-center">
        <div role="img" aria-label={summary || t('storage.scanning')}
          className="flex h-(--storage-bar) w-full gap-(--storage-gap) overflow-hidden rounded-full bg-active">
          {shown.map(s => (
            <span key={s.kind} title={`${t(LABEL[s.kind])}: ${bytes(s.bytes)}`} className={cn('h-full min-w-(--storage-gap) basis-0', FILL[s.kind])}
              style={{ flexGrow: s.bytes }} />
          ))}
        </div>
      </div>
      {STORAGE_SEGMENTS.map(k => {
        const s = seg(k);
        return (
          <ItemRow key={k} lead={<span aria-hidden className={cn('size-(--dot) rounded-full', FILL[k])} />}
            title={<>{t(LABEL[k])}{s && s.count > 0 && k !== 'other' && <> <Count n={s.count} /></>}</>}
            trailing={<span className={cn('text-2 tabular-nums', s ? 'text-fg-1' : 'text-fg-3')}>{s ? bytes(s.bytes) : t('storage.pending')}</span>} />
        );
      })}
      {view && <SourceLink path={view.root} env={env} onOpen={onReveal} />}
    </Section>
  );
}

// Everything the bar counts as other: size and place only, nothing to delete here
function OtherFiles({ view, onReveal }: { view: StorageView; onReveal: (path: string) => void }) {
  const [all, setAll] = useState(false);
  const list = all ? view.other : view.other.slice(0, OTHER_SHOWN);
  const rest = view.other.length - OTHER_SHOWN;
  return (
    <Section title={t('storage.other')} desc={t('storage.otherDesc')} count={view.other.length}>
      {list.map(e => (
        <ItemRow key={e.name} title={e.name} onOpen={() => onReveal(e.path)}
          trailing={<span className="text-2 text-fg-2 tabular-nums">{bytes(e.bytes)}</span>} />
      ))}
      {rest > 0 && (
        <button type="button" onClick={() => setAll(v => !v)}
          className="min-h-(--setting-row) py-(--setting-row-pad) text-left text-2 text-fg-2 transition-colors hover:text-fg-1 focus-visible:text-fg-1">
          {all ? t('storage.otherLess') : t('storage.otherMore', { n: rest })}
        </button>
      )}
    </Section>
  );
}
