import { useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { ArrowDownWideNarrow, CalendarClock, Check, ChevronLeft, Trash2 } from 'lucide-react';
import {
  STORAGE_RESIDUE_KINDS, STORAGE_SEGMENTS, formatBytes,
  type StorageBlock, type StorageOutcome, type StorageResidue, type StorageResidueKind, type StorageSegmentKind, type StorageSession, type StorageView,
} from '@shared/storage';
import type { MsgKey } from '@shared/i18n';
import type { AgentInfo } from '@shared/transcript';
import { Button, IconButton } from '../ui/Button';
import { RadioPills } from '../ui/Field';
import { ShellLayerContext } from '../ui/Overlay';
import { Toast } from '../ui/Toast';
import { cn } from '../ui/cn';
import { getLocale, t } from '../i18n';
import { Count, Group, ItemRow, Note, Section, SourceLink, shortPath } from './controls';
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
  agents: AgentInfo[];
  env: SettingsEnv;
  on: SettingsHandlers;
}

type Review = 'old' | 'residue';

export function StoragePage({ state, agents, env, on }: StoragePageProps) {
  const [open, setOpen] = useState<Review>();
  const view = state?.view;
  const reveal = (path?: string) => on.storageAction?.({ kind: 'reveal', path });
  const error = state?.error && <Section><Note><span className="truncate text-danger" title={state.error}>{state.error}</span></Note></Section>;
  const back = () => setOpen(undefined);
  if (open === 'old') return <>{error}<SessionReview state={state} agents={agents} on={on} onBack={back} /></>;
  if (open === 'residue') return <>{error}<ResidueReview state={state} env={env} on={on} onBack={back} /></>;
  return <Measured on={on}>
    {error}
    <Overview view={view} env={env} onReveal={() => reveal()} />
    <Cleanup view={view} onOpen={setOpen} />
    {view && view.other.length > 0 && <OtherFiles view={view} onReveal={reveal} />}
  </Measured>;
}

// The overview measures on every visit: other windows and the CLIs write meanwhile
function Measured({ on, children }: { on: SettingsHandlers; children: ReactNode }) {
  useEffect(() => { on.storage?.(); }, [on]);
  return <>{children}</>;
}

function Cleanup({ view, onOpen }: { view?: StorageView; onOpen: (r: Review) => void }) {
  const reclaim = view?.segments.find(s => s.kind === 'reclaimable');
  return (
    <Section title={t('storage.cleanup')}>
      <ItemRow title={t('storage.old.title')} desc={t('storage.old.entry')} onClick={() => onOpen('old')} />
      <ItemRow title={t('storage.residue.title')} onClick={() => onOpen('residue')}
        desc={!reclaim ? t('storage.pending') : reclaim.bytes > 0 ? t('storage.residue.entry', { size: bytes(reclaim.bytes) }) : t('storage.residue.none')} />
    </Section>
  );
}

// A review subpage's head: back to the overview and the review's title
function ReviewHead({ title, onBack, action }: { title: string; onBack: () => void; action?: ReactNode }) {
  return (
    <div className="flex min-h-ctl items-center gap-gap">
      <IconButton title={t('settings.back')} aria-label={t('settings.back')} onClick={onBack}><ChevronLeft strokeWidth={1.5} /></IconButton>
      <span className="min-w-0 flex-1 truncate text-2 font-medium text-fg-1">{title}</span>
      {action}
    </div>
  );
}

// The square tick of a pickable row; the row itself is the checkbox
function Tick({ on, disabled }: { on: boolean; disabled?: boolean }) {
  return (
    <span aria-hidden className={cn('flex size-icon shrink-0 items-center justify-center rounded-xs border transition-colors [&_svg]:size-tick',
      on ? 'border-fg-1 bg-fg-1 text-bg-0' : 'border-line-strong', disabled && 'opacity-40')}>
      {on && <Check strokeWidth={2} />}
    </span>
  );
}

// One pickable row of a review list: tick, two text lines, the size at the right
function PickRow({ on, disabled, title, meta, size, onToggle, nested }: {
  on: boolean;
  disabled?: boolean;
  title: ReactNode;
  meta: ReactNode;
  size: ReactNode;
  onToggle: () => void;
  // An item under a group's row: indented to the group's text column
  nested?: boolean;
}) {
  return (
    <button type="button" role="checkbox" aria-checked={on} disabled={disabled} onClick={onToggle}
      className={cn('flex min-h-(--setting-row-detail) w-full items-center gap-gap py-(--setting-row-pad) text-left text-2 enabled:hover:bg-hover focus-visible:bg-hover', nested && 'pl-indent')}>
      <Tick on={on} disabled={disabled} />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className={cn('truncate', disabled ? 'text-fg-2' : 'text-fg-1')}>{title}</span>
        <span className="truncate text-fg-2">{meta}</span>
      </span>
      <span className="shrink-0 text-fg-2 tabular-nums">{size}</span>
    </button>
  );
}

// The review's fixed-height list: an async list never changes the page's height when it fills, empties or filters
function ReviewList({ children, empty, loading }: { children: ReactNode; empty: string; loading: boolean }) {
  const hasRows = Array.isArray(children) ? children.length > 0 : !!children;
  return (
    <div className="h-(--storage-list) overflow-y-auto overscroll-contain scroll-thin">
      {loading ? <Note shimmer>{t('storage.scanning')}</Note> : hasRows ? <Group embedded>{children}</Group> : <Note>{empty}</Note>}
    </div>
  );
}

// The review's last row: what is picked, and the one action
function ReviewFooter({ n, size, all, onAll, action, disabled, onAction }: {
  n: number;
  size: number;
  all: boolean;
  onAll: () => void;
  action: string;
  disabled: boolean;
  onAction: () => void;
}) {
  return (
    <div className="flex min-h-(--setting-row) items-center gap-gap border-t border-line py-(--setting-row-pad) text-2">
      <button type="button" role="checkbox" aria-checked={all} onClick={onAll} title={t('storage.selectAll')} aria-label={t('storage.selectAll')}
        className="flex shrink-0 items-center self-stretch">
        <Tick on={all} />
      </button>
      <span className="min-w-0 flex-1 truncate text-fg-2 tabular-nums">{t('storage.selected', { n, size: bytes(size) })}</span>
      <Button variant="primary" disabled={disabled || n === 0} onClick={onAction}>{action}</Button>
    </div>
  );
}

// A toast at the bottom of the settings shell (the chat's toasts live in the chat shell); portalled into the shell
// layer so it floats over the scrolling page
function ShellToast({ children }: { children: ReactNode }) {
  const layer = useContext(ShellLayerContext);
  const [el, setEl] = useState<HTMLElement | null>(null);
  useEffect(() => setEl(layer?.current ?? null), [layer]);
  if (!el) return null;
  return createPortal(<div className="pointer-events-none absolute inset-x-0 bottom-gap z-10 flex flex-col items-center gap-1 px-page">{children}</div>, el);
}

// Deleted sessions stay restorable for 30 s on the engine side; the toast leaves a margin for the round trip
const UNDO_TTL = 25_000;

type Age = '30' | '90' | '180' | 'all';
type Sort = 'size' | 'age';
const AGES: Age[] = ['30', '90', '180', 'all'];
const DAY = 86_400_000;

function blockText(b: StorageBlock): string {
  return t(b === 'pinned' ? 'storage.block.pinned' : b === 'running' ? 'storage.block.running' : 'storage.block.elsewhere');
}

// How long ago, in the UI locale: days under two months, then months, then years
function since(iso: string): string {
  const days = Math.max(0, Math.floor((Date.now() - Date.parse(iso)) / DAY));
  const rtf = new Intl.RelativeTimeFormat(getLocale(), { numeric: 'auto' });
  if (!Number.isFinite(days)) return '';
  if (days < 60) return rtf.format(-days, 'day');
  if (days < 730) return rtf.format(-Math.round(days / 30), 'month');
  return rtf.format(-Math.round(days / 365), 'year');
}

// The sessions of one age range, largest or oldest first; a pick runs through the engine's soft delete, so the toast can undo it
function SessionReview({ state, agents, on, onBack }: { state?: StorageState; agents: AgentInfo[]; on: SettingsHandlers; onBack: () => void }) {
  useEffect(() => { on.storage?.(['sessions']); }, [on]);
  const [age, setAge] = useState<Age>('30');
  const [sort, setSort] = useState<Sort>('size');
  const [picked, setPicked] = useState<ReadonlySet<string>>(() => new Set());
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ key: number; text: string; undo?: string[] }>();
  const all = state?.view.sessions;
  const rows = useMemo(() => {
    const cutoff = age === 'all' ? Infinity : Date.now() - Number(age) * DAY;
    const list = (all ?? []).filter(s => age === 'all' || Date.parse(s.updatedAt) < cutoff);
    return sort === 'size' ? list.sort((a, b) => b.bytes - a.bytes) : list.sort((a, b) => a.updatedAt.localeCompare(b.updatedAt));
  }, [all, age, sort]);
  const pickable = rows.filter(s => !s.blocked);
  const chosen = pickable.filter(s => picked.has(s.id));
  // The outcome of this page's action: a toast with undo, and the picks that went drop out
  const seen = useRef(state?.outcome);
  useEffect(() => {
    setBusy(false);
    const o = state?.outcome;
    if (!o || o === seen.current) return;
    seen.current = o;
    if (o.action === 'deleteSessions' && o.done.length > 0) {
      setToast({ key: Date.now(), text: t('storage.deleted', { n: o.done.length }), undo: o.done });
    } else if (o.action === 'deleteSessions' && o.skipped.length > 0) {
      setToast({ key: Date.now(), text: o.skipped[0]!.reason });
    }
    if (o.action === 'restoreSessions') setToast(undefined);
    setPicked(p => new Set([...p].filter(id => !o.done.includes(id))));
  }, [state]);
  const toggle = (id: string) => setPicked(p => { const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  const everything = pickable.length > 0 && chosen.length === pickable.length;
  const name = (id: string) => agents.find(a => a.id === id)?.name ?? id;
  const meta = (s: StorageSession) => [
    s.blocked ? blockText(s.blocked) : name(s.agent),
    since(s.updatedAt),
    s.subagents ? t('storage.subagents', { n: s.subagents }) : undefined,
  ].filter(Boolean).join(t('common.metaSep'));
  return <>
    <ReviewHead title={t('storage.old.title')} onBack={onBack} action={
      // One toggle for the order; its tooltip names the order shown
      <IconButton title={t(sort === 'size' ? 'storage.sort.size' : 'storage.sort.age')} aria-label={t('storage.sort.label')}
        onClick={() => setSort(sort === 'size' ? 'age' : 'size')}>
        {sort === 'size' ? <ArrowDownWideNarrow strokeWidth={1.5} /> : <CalendarClock strokeWidth={1.5} />}
      </IconButton>
    } />
    <Section desc={t('storage.old.desc')}>
      <div className="flex py-(--setting-row-pad)">
        <RadioPills<Age> label={t('storage.age.label')} value={age} onChange={setAge}
          options={AGES.map(a => ({ value: a, label: a === 'all' ? t('storage.age.all') : t('storage.age.days', { n: Number(a) }) }))} />
      </div>
      <ReviewList loading={!all} empty={t('storage.old.empty')}>
        {rows.map(s => (
          <PickRow key={s.id} on={!s.blocked && picked.has(s.id)} disabled={!!s.blocked} onToggle={() => toggle(s.id)}
            title={s.title || t('storage.untitled')} meta={meta(s)} size={bytes(s.bytes)} />
        ))}
      </ReviewList>
      <ReviewFooter n={chosen.length} size={chosen.reduce((a, s) => a + s.bytes, 0)} all={everything}
        onAll={() => setPicked(everything ? new Set() : new Set(pickable.map(s => s.id)))}
        action={t('storage.delete')} disabled={busy}
        onAction={() => { setBusy(true); on.storageAction?.({ kind: 'deleteSessions', ids: chosen.map(s => s.id) }, ['sessions']); }} />
    </Section>
    {toast && <ShellToast>
      <Toast key={toast.key} text={toast.text} ttl={toast.undo ? UNDO_TTL : undefined} onClose={() => setToast(undefined)}
        onUndo={toast.undo && (() => { on.storageAction?.({ kind: 'restoreSessions', ids: toast.undo! }, ['sessions']); setToast(undefined); })} />
    </ShellToast>}
  </>;
}

// A leftover's place: relative to the data directory when inside it, else the path shortened like other settings paths
function inRoot(path: string, root: string, env: SettingsEnv): string {
  for (const sep of ['/', '\\']) if (root && path.startsWith(root + sep)) return path.slice(root.length + 1);
  return shortPath(path, env);
}

const KIND_LABEL: Record<StorageResidueKind, MsgKey> = {
  agentLog: 'storage.kind.agentLog',
  engineLog: 'storage.kind.engineLog',
  blob: 'storage.kind.blob',
  unreadable: 'storage.kind.unreadable',
  backup: 'storage.kind.backup',
  legacy: 'storage.kind.legacy',
};
const KIND_DESC: Record<StorageResidueKind, MsgKey> = {
  agentLog: 'storage.kind.agentLog.desc',
  engineLog: 'storage.kind.engineLog.desc',
  blob: 'storage.kind.blob.desc',
  unreadable: 'storage.kind.unreadable.desc',
  backup: 'storage.kind.backup.desc',
  legacy: 'storage.kind.legacy.desc',
};

// The leftovers by kind; the kinds nothing can miss start ticked. Removal is for good, so the page says so up front
function ResidueReview({ state, env, on, onBack }: { state?: StorageState; env: SettingsEnv; on: SettingsHandlers; onBack: () => void }) {
  useEffect(() => { on.storage?.(['residue']); }, [on]);
  const list = state?.view.residue;
  const [picked, setPicked] = useState<ReadonlySet<string>>();
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ key: number; text: string }>();
  // The first list in decides the starting ticks; later replies only drop what went
  useEffect(() => { if (list && !picked) setPicked(new Set(list.filter(r => r.preselected).map(r => r.id))); }, [list, picked]);
  const seen = useRef(state?.outcome);
  useEffect(() => {
    setBusy(false);
    const o = state?.outcome;
    if (!o || o === seen.current || o.action !== 'cleanResidue') return;
    seen.current = o;
    const text = o.skipped.length > 0 ? o.skipped[0]!.reason : t('storage.cleaned', { size: bytes(o.freed) });
    setToast({ key: Date.now(), text });
  }, [state]);
  const ids = new Set(list?.map(r => r.id));
  const sel = new Set([...(picked ?? [])].filter(id => ids.has(id)));
  const chosen = (list ?? []).filter(r => sel.has(r.id));
  const set = (next: Iterable<string>) => setPicked(new Set(next));
  const toggle = (id: string) => { const n = new Set(sel); if (n.has(id)) n.delete(id); else n.add(id); set(n); };
  const everything = !!list && list.length > 0 && chosen.length === list.length;
  const groups = STORAGE_RESIDUE_KINDS.map(k => [k, (list ?? []).filter(r => r.kind === k)] as const).filter(([, rs]) => rs.length > 0);
  const kindAll = (rs: readonly StorageResidue[]) => rs.every(r => sel.has(r.id));
  const root = state?.view.root ?? '';
  return <>
    <ReviewHead title={t('storage.residue.title')} onBack={onBack} />
    <Section desc={t('storage.residue.desc')}>
      <ReviewList loading={!list} empty={t('storage.residue.none')}>
        {groups.map(([k, rs]) => (
          <div key={k} className="flex flex-col divide-y divide-line">
            <PickRow on={kindAll(rs)} title={<>{t(KIND_LABEL[k])} <Count n={rs.length} /></>} meta={<span title={t(KIND_DESC[k])}>{t(KIND_DESC[k])}</span>}
              size={bytes(rs.reduce((a, r) => a + r.bytes, 0))}
              onToggle={() => set(kindAll(rs) ? [...sel].filter(id => !rs.some(r => r.id === id)) : [...sel, ...rs.map(r => r.id)])} />
            {rs.map(r => (
              <PickRow key={r.id} nested on={sel.has(r.id)} onToggle={() => toggle(r.id)}
                title={<span title={r.path}>{inRoot(r.path, root, env)}</span>} meta={since(r.modifiedAt)} size={bytes(r.bytes)} />
            ))}
          </div>
        ))}
      </ReviewList>
      <ReviewFooter n={chosen.length} size={chosen.reduce((a, r) => a + r.bytes, 0)} all={everything}
        onAll={() => set(everything ? [] : (list ?? []).map(r => r.id))}
        action={t('storage.clean')} disabled={busy}
        onAction={() => { setBusy(true); on.storageAction?.({ kind: 'cleanResidue', ids: chosen.map(r => r.id) }, ['residue']); }} />
    </Section>
    {toast && <ShellToast>
      <Toast key={toast.key} text={toast.text} icon={<Trash2 className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />} onClose={() => setToast(undefined)} />
    </ShellToast>}
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
