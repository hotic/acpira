import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from 'react';
import { LockKeyhole, Plus } from 'lucide-react';
import type { DeviceStatus, RemoteAccessView } from '@acpira/shared/devices';
import { cn } from '../ui/cn';
import { Button } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { Row } from '../ui/Row';
import { SectionAction } from '../settings/controls';
import type { SettingsPage } from '../settings/Nav';
import { t } from '../i18n';
import { deviceOrder, remoteAccessOp, useRemoteAccess } from './access';
import { DeviceGlyph, PairingCore, PathTag, usePairing } from './Pairing';
import { seenLine } from './seen';

// The desktop app's devices card (desktop.css, devices.css): a phone button beside the gear at the session column's foot
// opens a card rising from it. With no device yet it opens on pairing (remote/Pairing.tsx); otherwise on the device list,
// whose "Add device" turns the card to pairing. A device that pairs holds the success moment, then its glyph flies into
// its new row as the card becomes the list. Remote access off, not running or not offered here shows one quiet note
// instead. The card's views lie in one bottom-anchored clip: a change of view is a translate of the surface and a
// cross-fade, so nothing jumps. Shown once the engine answered for remote access (remote/access.ts)

// How long the success moment holds before the card settles into the list
const HOLD_MS = 2400;
// The glyph's flight into its row, and the point where it hands over to the row's own glyph
const FLIGHT_MS = 660;

const still = (el?: Element | null) => (el?.closest('[data-motion="none"]') ?? null) !== null || matchMedia('(prefers-reduced-motion: reduce)').matches;

type CardView = 'pair' | 'list' | 'note';

// The foot button's phone: a presence notch while a device is connected, a scan line sweeping its screen while a
// pairing window is open
function PhoneGlyph({ online, pairing }: { online: boolean; pairing: boolean }) {
  const id = useId().replace(/:/g, '');
  return <svg viewBox="0 0 24 24" className="dv-phone size-(--foot-icon)" data-pairing={pairing || undefined}
    fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    <defs><mask id={id}><rect width="24" height="24" fill="#fff" />{online && <circle cx="18" cy="5" r="5" fill="#000" />}</mask></defs>
    <g mask={`url(#${id})`}>
      <rect x="6" y="2.5" width="12" height="19" rx="2.5" />
      <path d="M10.75 18.5h2.5" />
      {pairing && <path className="dv-phone-scan" d="M8.75 11h6.5" strokeWidth={1.25} />}
    </g>
    {online && <circle className="dv-phone-dot" cx="18" cy="5" r="2.9" stroke="none" />}
  </svg>;
}

// The gear's box (chat/DesktopFrame.tsx GEAR), so both foot buttons weigh the same
const FOOT = 'flex aspect-square items-center rounded-md px-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover data-[popup-open]:bg-hover data-[popup-open]:text-fg-1';

/** The foot's phone button and its card; nothing until the engine answered for remote access, and nothing on a
 *  platform the gateway does not run on (a button that can only say so is noise) */
export function DevicesButton({ anchor, onOpenSettings }: { anchor: RefObject<HTMLElement | null>; onOpenSettings?: (page?: SettingsPage) => void }) {
  const view = useRemoteAccess();
  const [open, setOpen] = useState(false);
  if (!view || view.unavailable === 'platform') return null;
  const devices = view.gateway?.devices ?? [];
  const online = devices.filter(d => d.online).length;
  const pairing = !!view.gateway?.pairing;
  const title = pairing ? t('devices.buttonPairing') : online ? t('devices.buttonOnline', { n: online }) : t('devices.title');
  return <Popover.Root open={open} onOpenChange={setOpen}>
    <Popover.Trigger render={<button type="button" title={title} aria-label={title} className={cn(FOOT, 'ml-0.5')}>
      <span className="flex size-lead items-center justify-center"><PhoneGlyph online={online > 0} pairing={pairing} /></span>
    </button>} />
    <Popover.Portal>
      {/* No collision padding: the foot sits closer to the window's edge than the shell's pad, as the gear's menu */}
      <Popover.Positioner side="top" align="start" anchor={anchor} collisionPadding={0} className="w-(--dv-card-w)">
        <Popover.Popup aria-label={t('devices.title')} className="dv-card relative overflow-visible rounded-none border-0 bg-transparent p-0 shadow-none">
          <Card view={view} open={open} onClose={() => setOpen(false)} onOpenSettings={onOpenSettings} />
        </Popover.Popup>
      </Popover.Positioner>
    </Popover.Portal>
  </Popover.Root>;
}

function Card({ view, open, onClose, onOpenSettings }: { view: RemoteAccessView; open: boolean; onClose: () => void; onOpenSettings?: (page?: SettingsPage) => void }) {
  const gateway = view.gateway;
  const devices = gateway?.devices ?? [];
  const ready = view.available && view.enabled && !!gateway;
  // Pairing or the list; decided as the card opens (no device yet: pairing)
  const [mode, setMode] = useState<'pair' | 'list'>(() => (devices.length ? 'list' : 'pair'));
  const [fresh, setFresh] = useState<string>();
  const shown: CardView = !ready ? 'note' : mode;
  const pairing = usePairing(open && shown === 'pair');
  const paired = pairing.paired;
  // The success moment holds, then the card becomes the list with the new device leading
  useEffect(() => {
    if (!paired) return;
    const id = setTimeout(() => { setFresh(paired.id); setMode('list'); }, HOLD_MS);
    return () => clearTimeout(id);
  }, [paired?.id]);
  const pairRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const noteRef = useRef<HTMLDivElement>(null);
  const glyphRef = useRef<HTMLSpanElement>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const heights = useHeights([pairRef, listRef, noteRef]);
  const cur = heights[shown === 'pair' ? 0 : shown === 'list' ? 1 : 2] ?? 0;
  const max = Math.max(...heights, 0);
  // Which way the surface moves: shrinking, the old view is gone before the top edge passes it; growing, the new view
  // waits for the surface
  const last = useRef(cur);
  const dir = useRef<'shrink' | 'grow'>(undefined);
  if (cur !== last.current) { dir.current = cur < last.current ? 'shrink' : 'grow'; last.current = cur; }
  // The success moment's glyph flies into its row: down first, then in (two nested translations, different easings),
  // so it never crosses the list's head; the head and the row's text wait for it
  const prev = useRef(shown);
  const [arrived, setArrived] = useState(false);
  useLayoutEffect(() => {
    const from = prev.current;
    prev.current = shown;
    const landing = from === 'pair' && shown === 'list' && !!fresh;
    setArrived(landing);
    const src = glyphRef.current, root = rootRef.current, svg = src?.querySelector('svg');
    const lead = listRef.current?.querySelector<HTMLElement>('[data-fresh-lead] svg');
    if (!landing || !src || !root || !svg || !lead || still(root)) return;
    const a = svg.getBoundingClientRect(), b = lead.getBoundingClientRect(), box = root.getBoundingClientRect();
    const outer = document.createElement('div');
    const inner = src.cloneNode(true) as HTMLElement;
    inner.style.animation = 'none';
    inner.querySelectorAll<HTMLElement | SVGElement>('*').forEach(e => { e.style.animation = 'none'; });
    outer.className = 'dv-flight';
    Object.assign(outer.style, { left: `${a.left - box.left}px`, top: `${a.top - box.top}px`, width: `${a.width}px`, height: `${a.height}px` });
    Object.assign(inner.style, { width: `${a.width}px`, height: `${a.height}px` });
    outer.appendChild(inner);
    root.appendChild(outer);
    outer.animate([{ transform: 'none' }, { transform: `translateX(${b.left - a.left}px)` }], { duration: FLIGHT_MS, easing: 'cubic-bezier(.5, 0, .3, 1)' });
    outer.animate([{ opacity: 1, offset: 0.9 }, { opacity: 0 }], { duration: FLIGHT_MS });
    inner.animate([{ transform: 'none' }, { transform: `translateY(${b.top - a.top}px) scale(${b.width / a.width})` }], { duration: FLIGHT_MS, easing: 'cubic-bezier(.3, .5, .2, 1)' })
      .finished.finally(() => outer.remove());
    inner.querySelector('.dv-qr-check')?.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 160, fill: 'forwards' });
    return () => outer.remove();
  }, [shown]);
  const add = () => { setFresh(undefined); setMode('pair'); };
  const openDevice = () => { onClose(); onOpenSettings?.({ kind: 'connections' }); };
  return <div ref={rootRef} className="dv-shadow">
    <div className="dv-clip" data-dir={dir.current} style={{ height: max }}>
      <div className="dv-surface" style={{ height: max, transform: `translateY(${max - cur}px)` }} />
      <div ref={pairRef} className="dv-view" data-view="pair" data-off={shown !== 'pair' || undefined}>
        {gateway && <PairingCore pairing={pairing} glyphRef={glyphRef} className="p-pad" />}
      </div>
      <div ref={listRef} className="dv-view" data-view="list" data-off={shown !== 'list' || undefined} data-from={arrived || undefined}>
        {/* Keyed by the view, so its rows replay their entrance each time the list comes up */}
        <div key={shown === 'list' ? 'on' : 'off'}>
          <List view={view} fresh={fresh} onAdd={add} onDevice={openDevice} />
        </div>
      </div>
      <div ref={noteRef} className="dv-view" data-view="note" data-off={shown !== 'note' || undefined}>
        <Note view={view} />
      </div>
    </div>
  </div>;
}

// The views' heights, kept current as their content changes (a device added, the name edited)
function useHeights(refs: RefObject<HTMLElement | null>[]): number[] {
  const [h, setH] = useState<number[]>([]);
  useLayoutEffect(() => {
    const read = () => setH(prev => {
      const next = refs.map(r => r.current?.offsetHeight ?? 0);
      return next.every((v, i) => v === prev[i]) ? prev : next;
    });
    read();
    const ro = new ResizeObserver(read);
    for (const r of refs) if (r.current) ro.observe(r.current);
    return () => ro.disconnect();
  }, []);
  return h;
}

function List({ view, fresh, onAdd, onDevice }: { view: RemoteAccessView; fresh?: string; onAdd: () => void; onDevice: (d: DeviceStatus) => void }) {
  const devices = view.gateway?.devices ?? [];
  const online = devices.filter(d => d.online).length;
  const rows = deviceOrder(devices, fresh);
  return <>
    <div className="dv-head flex items-center gap-gap px-pad py-gap">
      <span className="flex min-w-0 flex-1 flex-col gap-(--stack-gap)">
        <span className="truncate text-2 font-medium text-fg-1">{view.gateway?.name}</span>
        <span className="flex items-center gap-1 text-3 text-fg-3">
          {online ? t('devices.online', { n: online }) : t('devices.noneOnline')}
          {online > 0 && <span className="dv-lock" title={t('devices.encrypted')} role="img" aria-label={t('devices.encrypted')}><LockKeyhole strokeWidth={1.75} /></span>}
        </span>
      </span>
      <SectionAction icon={<Plus />} onClick={onAdd}>{t('devices.add')}</SectionAction>
    </div>
    <div className="flex flex-col border-t border-line p-1">
      {rows.map((d, i) => <Row key={d.id} as="button" interactive data-fresh={d.id === fresh || undefined} onClick={() => onDevice(d)}
        className="dv-row mx-0 min-h-(--dv-row) w-full px-2.5" style={{ '--i': i } as CSSProperties}
        lead={<span data-fresh-lead={d.id === fresh || undefined} className={cn('flex text-fg-2', d.id === fresh && 'dv-lead-late')}><DeviceGlyph kind={d.kind} online={d.online} /></span>}>
        {d.id === fresh && <span className="dv-row-new" />}
        <span className="dv-row-text flex min-w-0 flex-1 flex-col gap-(--stack-gap)">
          <span className="truncate text-2 text-fg-1">{d.name}</span>
          {d.online && d.path ? <span className="flex"><PathTag path={d.path} className="justify-items-start" /></span>
            : <span className="truncate text-3 text-fg-3">{seenLine(d)}</span>}
        </span>
      </Row>)}
    </div>
  </>;
}

// Remote access off, not running, or not offered here: one quiet note, and the one action that moves it on
function Note({ view }: { view: RemoteAccessView }) {
  let title: string, desc: string, action: ReactNode = null;
  if (!view.available) {
    title = t('devices.unavailable.title');
    desc = t(view.unavailable === 'component' ? 'devices.unavailable.component' : 'devices.unavailable.platform');
  } else if (!view.enabled) {
    title = t('devices.off.title');
    desc = t('devices.off.desc');
    action = <Button variant="primary" disabled={view.busy} onClick={() => remoteAccessOp({ op: 'enable' })}>{t(view.busy ? 'devices.enabling' : 'devices.enable')}</Button>;
  } else {
    title = t(view.busy ? 'devices.starting' : 'devices.stopped.title');
    desc = t('devices.stopped.desc');
    action = <Button disabled={view.busy} onClick={() => remoteAccessOp({ op: 'enable' })}>{t('devices.restart')}</Button>;
  }
  return <div className="flex flex-col gap-pad p-pad">
    <span className="flex flex-col gap-(--stack-gap)">
      <span className="text-2 font-medium text-fg-1">{title}</span>
      <span className="text-3 text-fg-3">{desc}</span>
    </span>
    {action && <span className="flex">{action}</span>}
  </div>;
}
