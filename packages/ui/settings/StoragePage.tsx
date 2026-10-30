import { useContext, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Archive, ArrowDownWideNarrow, CalendarClock, Check, FileX, FolderOpen, ScrollText, Trash2 } from 'lucide-react';
import {
  STORAGE_RESIDUE_KINDS, STORAGE_SEGMENTS, formatBytes,
  type StorageAction, type StorageBlock, type StorageDetail, type StorageOutcome, type StorageResidue, type StorageResidueKind, type StorageSegmentKind, type StorageSession, type StorageView,
} from '@acpira/shared/storage';
import type { MsgKey } from '@acpira/shared/i18n';
import type { AgentInfo } from '@acpira/shared/transcript';
import { Button, IconButton } from '../ui/Button';
import { ShellLayerContext } from '../ui/Overlay';
import { Shimmer } from '../ui/Shimmer';
import { Toast } from '../ui/Toast';
import { cn } from '../ui/cn';
import { getLocale, t } from '../i18n';
import { Count, DialogBody, FooterNote, Group, ItemRow, Meta, Note, rowHover, rowRules, Section, SectionAction, SectionDescription, Segmented, SettingsDialog, shortPath } from './controls';
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
  // Counts the header Refresh clicks; a change arms the next scan to replay the bar's fill
  refresh?: number;
}

type Review = 'old' | 'slim' | 'residue';

export function StoragePage({ state, agents, env, on, refresh = 0 }: StoragePageProps) {
  const [open, setOpen] = useState<Review>();
  const view = state?.view;
  const reveal = (path?: string) => on.storageAction?.({ kind: 'reveal', path });
  const error = state?.error && <Section><Note><span className="truncate text-danger" title={state.error}>{state.error}</span></Note></Section>;
  const close = () => setOpen(undefined);
  return <Measured on={on}>
    {error}
    <Overview view={view} refresh={refresh} onReveal={() => reveal()} />
    <Cleanup view={view} onOpen={setOpen} />
    {view && view.other.length > 0 && <OtherFiles view={view} onReveal={reveal} />}
    {(open === 'old' || open === 'slim') && <SessionReview key={open} mode={open === 'slim' ? 'slim' : 'delete'} state={state} agents={agents} on={on} onClose={close} />}
    {open === 'residue' && <ResidueReview state={state} env={env} on={on} onClose={close} />}
  </Measured>;
}

// The overview measures on every visit: other windows and the CLIs write meanwhile
function Measured({ on, children }: { on: SettingsHandlers; children: ReactNode }) {
  useEffect(() => { on.storage?.(); }, [on]);
  return <>{children}</>;
}

// The three cleanups, each reviewed in a dialog before it runs; each row's own glyph, its size on the right once known
function Cleanup({ view, onOpen }: { view?: StorageView; onOpen: (r: Review) => void }) {
  const reclaim = view?.segments.find(s => s.kind === 'reclaimable');
  const sessions = view?.segments.find(s => s.kind === 'sessions');
  return (
    <Section title={t('storage.cleanup')} desc={t('storage.cleanup.desc')}>
      <ItemRow lead={<ScrollText />} title={t('storage.old.title')} desc={t('storage.old.entry')} onEdit={() => onOpen('old')} editLabel={t('storage.old.title')}
        trailing={sessions && <Meta>{bytes(sessions.bytes)}</Meta>} />
      <ItemRow lead={<Archive />} title={t('storage.slim.title')} desc={t('storage.slim.entry')} onEdit={() => onOpen('slim')} editLabel={t('storage.slim.title')} />
      <ItemRow lead={<FileX />} title={t('storage.residue.title')} onEdit={() => onOpen('residue')} editLabel={t('storage.residue.title')}
        desc={!reclaim ? t('storage.pending') : reclaim.bytes > 0 ? t('storage.residue.entryShort') : t('storage.residue.none')}
        trailing={reclaim && reclaim.bytes > 0 && <Meta>{bytes(reclaim.bytes)}</Meta>} />
    </Section>
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

// One pickable row of a review list: the tick in the icon column, two text lines, the size at the right
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
      className={cn(rowHover, 'flex min-h-(--setting-row-detail) w-full items-center gap-gap py-(--setting-row-pad) text-left text-2', nested && 'pl-indent')}>
      <span className="flex size-lead shrink-0 items-center justify-center"><Tick on={on} disabled={disabled} /></span>
      <span className="flex min-w-0 flex-1 flex-col gap-y-(--stack-gap)">
        <span className={cn('truncate', disabled ? 'text-fg-3' : 'text-fg-1')}>{title}</span>
        <span className="truncate text-3 text-fg-3">{meta}</span>
      </span>
      <Meta>{size}</Meta>
    </button>
  );
}

// The review's fixed-height list, a dialog block that scrolls: an async list never changes the dialog's height when it
// fills, empties or filters. Hovers and rules run from block edge to block edge; nothing scrolls sideways
function ReviewList({ children, empty, loading }: { children: ReactNode; empty: string; loading: boolean }) {
  const hasRows = Array.isArray(children) ? children.length > 0 : !!children;
  return (
    <div className="scroll-thin h-(--storage-list) shrink-0 overflow-x-hidden overflow-y-auto overscroll-contain rounded-lg bg-chip px-3">
      {loading ? <Group embedded><Note shimmer>{t('storage.scanning')}</Note></Group> : hasRows ? <Group embedded>{children}</Group> : <Group embedded><Note>{empty}</Note></Group>}
    </div>
  );
}

// The review's footer: the tick that picks everything, what is picked, Cancel and the one action
function ReviewFooter({ n, size, all, onAll, action, danger, disabled, onAction, onClose }: {
  n: number;
  size: number;
  all: boolean;
  onAll: () => void;
  action: string;
  // Deleting reads as destructive; slimming and cleaning leftovers are the dialog's plain purpose
  danger?: boolean;
  disabled: boolean;
  onAction: () => void;
  onClose: () => void;
}) {
  return <>
    <button type="button" role="checkbox" aria-checked={all} onClick={onAll} title={t('storage.selectAll')} aria-label={t('storage.selectAll')}
      className="flex size-lead shrink-0 items-center justify-center">
      <Tick on={all} />
    </button>
    <FooterNote className="tabular-nums">{t('storage.selected', { n, size: bytes(size) })}</FooterNote>
    <Button onClick={onClose}>{t('settings.shared.cancel')}</Button>
    <Button variant={danger ? 'secondary' : 'primary'} tone={danger ? 'danger' : undefined} disabled={disabled || n === 0} onClick={onAction}>{action}</Button>
  </>;
}

// A toast at the bottom of the settings shell (the chat's toasts live in the chat shell); portalled into the shell
// layer so it floats over the scrolling page
function ShellToast({ children }: { children: ReactNode }) {
  const layer = useContext(ShellLayerContext);
  const [el, setEl] = useState<HTMLElement | null>(null);
  useEffect(() => setEl(layer?.current ?? null), [layer]);
  if (!el) return null;
  // Above the review dialogs (z-30), whose deletes it can undo
  return createPortal(<div className="pointer-events-none absolute inset-x-0 bottom-gap z-40 flex flex-col items-center gap-1 px-page">{children}</div>, el);
}

// Deleted sessions stay restorable for 30 s on the engine side; the toast leaves a margin for the round trip
const UNDO_TTL = 25_000;

type Age = '30' | '90' | '180' | 'all';
type Sort = 'size' | 'age';
const AGES: Age[] = ['30', '90', '180', 'all'];
const DAY = 86_400_000;

type Block = StorageBlock | 'open';

function blockText(b: Block): string {
  return t(b === 'pinned' ? 'storage.block.pinned' : b === 'running' ? 'storage.block.running' : b === 'open' ? 'storage.block.open' : 'storage.block.elsewhere');
}

// What keeps a row from being picked. Deleting keeps pinned sessions; slimming takes them (nothing visible goes) but
// not a session open here, whose transcript in memory would write the texts back
function blockOf(s: StorageSession, mode: SessionMode): Block | undefined {
  if (mode === 'delete') return s.blocked;
  if (s.blocked && s.blocked !== 'pinned') return s.blocked;
  return s.open ? 'open' : undefined;
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

// The two session reviews share one page: old sessions are deleted through the engine's soft delete (the toast can
// undo it), slimming rewrites records without their whole-file diff texts and has no undo
type SessionMode = 'delete' | 'slim';

const MODE = {
  delete: { title: 'storage.old.title', desc: 'storage.old.desc', empty: 'storage.old.empty', action: 'storage.delete', kind: 'deleteSessions', detail: ['sessions'] },
  slim: { title: 'storage.slim.title', desc: 'storage.slim.desc', empty: 'storage.slim.empty', action: 'storage.slim.action', kind: 'slimSessions', detail: ['sessions', 'slim'] },
} as const satisfies Record<SessionMode, { title: MsgKey; desc: MsgKey; empty: MsgKey; action: MsgKey; kind: StorageAction['kind']; detail: StorageDetail[] }>;

// The sessions of one age range, largest or oldest first; in the slim review the size is what slimming frees and a
// session with nothing to drop is left out
// The mode is fixed for the page's life (the parent keys it), so the effects below leave it out of their deps
function SessionReview({ mode, state, agents, on, onClose }: { mode: SessionMode; state?: StorageState; agents: AgentInfo[]; on: SettingsHandlers; onClose: () => void }) {
  const m = MODE[mode];
  const detail: StorageDetail[] = [...m.detail];
  useEffect(() => { on.storage?.(detail); }, [on]);
  const [age, setAge] = useState<Age>('30');
  const [sort, setSort] = useState<Sort>('size');
  const [picked, setPicked] = useState<ReadonlySet<string>>(() => new Set());
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ key: number; text: string; undo?: string[] }>();
  // The slim review waits for the estimates: a list the delete review left behind carries none
  const all = mode === 'slim' && state?.view.sessions?.some(s => s.slim === undefined) ? undefined : state?.view.sessions;
  const size = (s: StorageSession) => (mode === 'slim' ? s.slim ?? 0 : s.bytes);
  const rows = useMemo(() => {
    const cutoff = age === 'all' ? Infinity : Date.now() - Number(age) * DAY;
    const list = (all ?? []).filter(s => (age === 'all' || Date.parse(s.updatedAt) < cutoff) && size(s) > 0);
    return sort === 'size' ? list.sort((a, b) => size(b) - size(a)) : list.sort((a, b) => a.updatedAt.localeCompare(b.updatedAt));
  }, [all, age, sort]);
  const pickable = rows.filter(s => !blockOf(s, mode));
  const chosen = pickable.filter(s => picked.has(s.id));
  // The outcome of this page's action: a toast (with undo after a delete), and the picks that went drop out
  const seen = useRef(state?.outcome);
  useEffect(() => {
    setBusy(false);
    const o = state?.outcome;
    if (!o || o === seen.current) return;
    seen.current = o;
    if (o.action === m.kind && o.done.length > 0) {
      setToast(mode === 'delete'
        ? { key: Date.now(), text: t('storage.deleted', { n: o.done.length }), undo: o.done }
        : { key: Date.now(), text: t('storage.cleaned', { size: bytes(o.freed) }) });
    } else if (o.action === m.kind && o.skipped.length > 0) {
      setToast({ key: Date.now(), text: o.skipped[0]!.reason });
    }
    if (o.action === 'restoreSessions') setToast(undefined);
    setPicked(p => new Set([...p].filter(id => !o.done.includes(id))));
  }, [state]);
  const toggle = (id: string) => setPicked(p => { const n = new Set(p); if (n.has(id)) n.delete(id); else n.add(id); return n; });
  const everything = pickable.length > 0 && chosen.length === pickable.length;
  const name = (id: string) => agents.find(a => a.id === id)?.name ?? id;
  const meta = (s: StorageSession) => {
    const b = blockOf(s, mode);
    return [
      b ? blockText(b) : name(s.agent),
      since(s.updatedAt),
      s.subagents ? t('storage.subagents', { n: s.subagents }) : undefined,
    ].filter(Boolean).join(t('common.metaSep'));
  };
  const sortLabel = t(sort === 'size' ? 'storage.sort.size' : 'storage.sort.age');
  return (
    <SettingsDialog wide icon={mode === 'slim' ? <Archive /> : <ScrollText />} title={t(m.title)} onClose={onClose}
      footer={<ReviewFooter n={chosen.length} size={chosen.reduce((a, s) => a + size(s), 0)} all={everything}
        onAll={() => setPicked(everything ? new Set() : new Set(pickable.map(s => s.id)))}
        action={t(m.action)} danger={mode === 'delete'} disabled={busy} onClose={onClose}
        onAction={() => { setBusy(true); on.storageAction?.({ kind: m.kind, ids: chosen.map(s => s.id) }, detail); }} />}>
      <DialogBody className="overflow-hidden">
        {/* What the review does is a sentence, too long for the title bar */}
        <SectionDescription>{t(m.desc)}</SectionDescription>
        <div className="flex min-w-0 items-center gap-gap">
          <Segmented<Age> label={t('storage.age.label')} value={age} onChange={setAge} className="min-w-0 flex-1"
            items={AGES.map(a => ({ value: a, label: a === 'all' ? t('storage.age.all') : t('storage.age.days', { n: Number(a) }) }))} />
          {/* One toggle for the order; its tooltip names the order shown */}
          <IconButton title={sortLabel} aria-label={t('storage.sort.label')} onClick={() => setSort(sort === 'size' ? 'age' : 'size')} className="text-fg-2">
            {sort === 'size' ? <ArrowDownWideNarrow strokeWidth={1.5} /> : <CalendarClock strokeWidth={1.5} />}
          </IconButton>
        </div>
        <ReviewList loading={!all} empty={t(m.empty)}>
          {rows.map(s => {
            const blocked = !!blockOf(s, mode);
            return <PickRow key={s.id} on={!blocked && picked.has(s.id)} disabled={blocked} onToggle={() => toggle(s.id)}
              title={s.title || t('storage.untitled')} meta={meta(s)} size={bytes(size(s))} />;
          })}
        </ReviewList>
      </DialogBody>
      {toast && <ShellToast>
        <Toast key={toast.key} text={toast.text} ttl={toast.undo ? UNDO_TTL : undefined} onClose={() => setToast(undefined)}
          onUndo={toast.undo && (() => { on.storageAction?.({ kind: 'restoreSessions', ids: toast.undo! }, detail); setToast(undefined); })} />
      </ShellToast>}
    </SettingsDialog>
  );
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
  pluginFiles: 'storage.kind.pluginFiles',
  pluginData: 'storage.kind.pluginData',
};
const KIND_DESC: Record<StorageResidueKind, MsgKey> = {
  agentLog: 'storage.kind.agentLog.desc',
  engineLog: 'storage.kind.engineLog.desc',
  blob: 'storage.kind.blob.desc',
  unreadable: 'storage.kind.unreadable.desc',
  backup: 'storage.kind.backup.desc',
  legacy: 'storage.kind.legacy.desc',
  pluginFiles: 'storage.kind.pluginFiles.desc',
  pluginData: 'storage.kind.pluginData.desc',
};

// The leftovers by kind; the kinds nothing can miss start ticked. Removal is for good, so the page says so up front
function ResidueReview({ state, env, on, onClose }: { state?: StorageState; env: SettingsEnv; on: SettingsHandlers; onClose: () => void }) {
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
  return (
    <SettingsDialog wide icon={<FileX />} title={t('storage.residue.title')} onClose={onClose}
      footer={<ReviewFooter n={chosen.length} size={chosen.reduce((a, r) => a + r.bytes, 0)} all={everything}
        onAll={() => set(everything ? [] : (list ?? []).map(r => r.id))}
        action={t('storage.clean')} disabled={busy} onClose={onClose}
        onAction={() => { setBusy(true); on.storageAction?.({ kind: 'cleanResidue', ids: chosen.map(r => r.id) }, ['residue']); }} />}>
      <DialogBody className="overflow-hidden">
        <SectionDescription>{t('storage.residue.desc')}</SectionDescription>
        <ReviewList loading={!list} empty={t('storage.residue.none')}>
          {groups.map(([k, rs]) => (
            <div key={k} className={cn('flex flex-col', rowRules)}>
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
      </DialogBody>
      {toast && <ShellToast>
        <Toast key={toast.key} text={toast.text} icon={<Trash2 className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />} onClose={() => setToast(undefined)} />
      </ShellToast>}
    </SettingsDialog>
  );
}

// The total, the bar and its legend; before the first scan the same rows hold their place with a faint dash
function Overview({ view, refresh, onReveal }: { view?: StorageView; refresh: number; onReveal: () => void }) {
  // The segments grow from zero when they mount; a new scan transitions them in place. Only a header Refresh click
  // arms the next scan to remount them so the fill plays again. Counting replies instead replayed the fill at random:
  // a reply still in flight from an earlier visit or a review subpage landed before this mount's own rescan
  const armed = useRef(refresh);
  const lastScan = useRef(view?.scannedAt);
  const barKey = useRef(0);
  if (view?.scannedAt !== lastScan.current) {
    lastScan.current = view?.scannedAt;
    if (armed.current !== refresh) {
      armed.current = refresh;
      barKey.current++;
    }
  }
  const seg = (k: StorageSegmentKind) => view?.segments.find(s => s.kind === k);
  const shown = view?.segments.filter(s => s.bytes > 0) ?? [];
  const total = view?.total ?? 0;
  const [hover, setHover] = useState<StorageSegmentKind>();
  const active = hover && shown.some(s => s.kind === hover) ? hover : undefined;
  const summary = shown.map(s => `${t(LABEL[s.kind])} ${bytes(s.bytes)}`).join(t('common.listSep'));
  return (
    <Section title={t('storage.overview')} desc={view ? t('storage.total', { size: bytes(view.total) }) : <Shimmer active>{t('storage.scanning')}</Shimmer>}
      action={view && <SectionAction icon={<FolderOpen />} title={view.root} onClick={onReveal}>{t('storage.open')}</SectionAction>}>
      <div className="flex min-h-(--setting-row) items-center">
        <div role="img" aria-label={summary || t('storage.scanning')}
          className="flex h-(--storage-bar) w-full gap-(--storage-gap) overflow-hidden rounded-full bg-active">
          {shown.map(s => (
            <span key={`${barKey.current}:${s.kind}`} title={`${t(LABEL[s.kind])}: ${bytes(s.bytes)}`}
              data-dim={(active && active !== s.kind) || undefined}
              onMouseEnter={() => setHover(s.kind)} onMouseLeave={() => setHover(undefined)}
              className={cn('storage-bar-seg h-full basis-0', FILL[s.kind])}
              style={{ '--seg-grow': total > 0 ? s.bytes / total : 0 } as CSSProperties} />
          ))}
        </div>
      </div>
      {STORAGE_SEGMENTS.map(k => {
        const s = seg(k);
        return (
          // The legend is information: it answers no hover (only the bar does, dimming the other segments)
          <ItemRow key={k}
            lead={<span aria-hidden className={cn('size-(--dot) rounded-full', FILL[k])} />}
            title={t(LABEL[k])}
            desc={k === 'other' ? t('storage.seg.other.desc') : s ? t('storage.items', { n: s.count }) : t('storage.pending')}
            trailing={<Meta>{s ? bytes(s.bytes) : t('storage.pending')}</Meta>} />
        );
      })}
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
        <ItemRow key={e.name} title={e.name} onOpen={() => onReveal(e.path)} trailing={<Meta>{bytes(e.bytes)}</Meta>} />
      ))}
      {rest > 0 && (
        <button type="button" onClick={() => setAll(v => !v)}
          className={cn(rowHover, 'min-h-(--setting-row) w-full py-(--setting-row-pad) text-left text-2 text-fg-2 transition-colors hover:text-fg-1 focus-visible:text-fg-1')}>
          {all ? t('storage.otherLess') : t('storage.otherMore', { n: rest })}
        </button>
      )}
    </Section>
  );
}
