import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from 'react';
import { Plus } from 'lucide-react';
import type { DeviceStatus, RemoteAccessView } from '@acpira/shared/devices';
import { cn } from '../ui/cn';
import { Button } from '../ui/Button';
import { Popover } from '../ui/Popover';
import { Row } from '../ui/Row';
import { PanelHeader } from '../ui/Panel';
import type { SettingsPage } from '../settings/Nav';
import { t } from '../i18n';
import { deviceOrder, PATH_NAME, remoteAccessOp, useRemoteAccess } from './access';
import { DeviceGlyph, PairingCore, usePairing } from './Pairing';
import { seenLine } from './seen';

// The desktop app's devices card (desktop.css, devices.css): a phone button beside the gear at the session column's foot
// opens a card rising from it. With no device yet it opens on pairing (remote/Pairing.tsx); otherwise on the device list,
// whose "Add device" turns the card to pairing. A device that pairs holds the success moment, then its glyph flies into
// its new row as the card becomes the list. Remote access off or not running is started as the card opens, the pairing
// view waiting for its code meanwhile; only a start that failed, or a build without the gateway, shows one quiet note.
// The card's views lie in one bottom-anchored clip: a change of view is a translate of the surface and a cross-fade, so
// nothing jumps. Shown once the engine answered for remote access (remote/access.ts)

// How long the success moment holds before the card settles into the list; the phone's sheet holds as long
// (apps/ios PairingTokens.swift Motion.pairHold), so both screens move on together
const HOLD_MS = 2400;
// The glyph's flight into its row, and the point where it hands over to the row's own glyph
const FLIGHT_MS = 660;

const still = (el?: Element | null) => (el?.closest('[data-motion="none"]') ?? null) !== null || matchMedia('(prefers-reduced-motion: reduce)').matches;

type CardView = 'pair' | 'list' | 'note';

// The foot button's phone: a presence notch while a device is connected, a scan line sweeping its screen while a
// pairing window is open
function PhoneGlyph({ online, pairing }: { online: boolean; pairing: boolean }) {
  const id = useId().replace(/:/g, '');
  return <svg viewBox="0 0 24 24" className="dv-phone size-icon" data-pairing={pairing || undefined}
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

// The foot row takes the session column's row box (chat/SessionList.tsx NAV_ROW / NAV_LEAD): it spans the column on the
// rows' icon and text edges, so the foot reads as the column's last line, not two loose icons over an empty corner
const FOOT_ROW = 'flex min-h-row min-w-0 flex-1 items-center gap-gap rounded-md px-2 text-left text-2 text-fg-1 transition-colors hover:bg-hover focus-visible:bg-hover data-[popup-open]:bg-hover';

/** Whether the foot shows the devices row: not until the engine answered for remote access, and not on a platform the
 *  gateway does not run on (a row that can only say so is noise). The gear's layout follows it (chat/DesktopFrame.tsx) */
export const devicesShown = (view: RemoteAccessView | undefined): view is RemoteAccessView => !!view && view.unavailable !== 'platform';

/** The foot's devices row (this computer's name and how many devices are online) and its card */
export function DevicesButton({ onOpenSettings }: { onOpenSettings?: (page?: SettingsPage) => void }) {
  const view = useRemoteAccess();
  const [open, setOpen] = useState(false);
  if (!devicesShown(view)) return null;
  const devices = view.gateway?.devices ?? [];
  const online = devices.filter(d => d.online).length;
  const pairing = !!view.gateway?.pairing;
  const title = pairing ? t('devices.buttonPairing') : online ? t('devices.buttonOnline', { n: online }) : t('devices.title');
  const name = view.gateway?.name ?? t('devices.title');
  // Nothing paired yet: the name alone, since "no device online" would read as a fault
  const state = online ? t('devices.online', { n: online }) : devices.length ? t('devices.noneOnline') : undefined;
  return <Popover.Root open={open} onOpenChange={setOpen}>
    <Popover.Trigger render={<button type="button" title={title} aria-label={title} className={FOOT_ROW}>
      <span className="-mr-gap-half flex size-lead shrink-0 items-center justify-center text-fg-2"><PhoneGlyph online={online > 0} pairing={pairing} /></span>
      <span className="min-w-0 truncate">{name}</span>
      {state && <span className="shrink-0 text-3 text-fg-3">{state}</span>}
    </button>} />
    <Popover.Portal>
      {/* No collision padding: the foot sits closer to the window's edge than the shell's pad, as the gear's menu */}
      <Popover.Positioner side="top" align="start" collisionPadding={0} className="w-(--dv-card-w)">
        <Popover.Popup aria-label={t('devices.title')} className="dv-card relative overflow-visible rounded-none border-0 bg-transparent p-0 shadow-none">
          <Card view={view} open={open} onClose={() => setOpen(false)} onOpenSettings={onOpenSettings} />
        </Popover.Popup>
      </Popover.Positioner>
    </Popover.Portal>
  </Popover.Root>;
}

function Card({ view, open, onClose, onOpenSettings }: { view: RemoteAccessView; open: boolean; onClose: () => void; onOpenSettings?: (page?: SettingsPage) => void }) {
  const gateway = view.gateway;
  // Opening the card is the ask: remote access off or not running is started here, once per opening, while the
  // pairing view waits for its code. The view at the time of the ask tells a later push (the op's answer) from it
  const [asked, setAsked] = useState<RemoteAccessView>();
  const start = () => { setAsked(view); remoteAccessOp({ op: 'enable' }); };
  useEffect(() => {
    if (!open) setAsked(undefined);
    else if (view.available && !view.busy && !gateway && !asked) start();
  }, [open, view.available, view.busy, !!gateway]);
  const failed = !!asked && asked !== view && !view.busy && !gateway && !!view.error;
  // Pairing or the list; decided once the gateway answers (no device yet: pairing)
  const [mode, setMode] = useState<'pair' | 'list' | undefined>(() => (gateway ? (gateway.devices.length ? 'list' : 'pair') : undefined));
  useEffect(() => { if (!mode && gateway) setMode(gateway.devices.length ? 'list' : 'pair'); }, [mode, gateway]);
  const [fresh, setFresh] = useState<string>();
  const shown: CardView = !view.available || failed ? 'note' : (mode ?? 'pair');
  const pairing = usePairing(open && shown === 'pair' && mode === 'pair');
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
  // The first measurement after mounting is the opening, not a change of view: it sets no direction
  const last = useRef(cur);
  const dir = useRef<'shrink' | 'grow'>(undefined);
  if (cur !== last.current) { if (last.current) dir.current = cur < last.current ? 'shrink' : 'grow'; last.current = cur; }
  // The success moment's glyph flies into its row: down first, then in (two nested translations, different easings),
  // so it never crosses the list's head; the head and the row's text wait for it
  const prev = useRef(shown);
  const [arrived, setArrived] = useState(false);
  useLayoutEffect(() => {
    const from = prev.current;
    prev.current = shown;
    const landing = from === 'pair' && shown === 'list' && !!fresh;
    setArrived(landing);
    const src = glyphRef.current, root = rootRef.current, list = listRef.current, svg = src?.querySelector('svg');
    const lead = list?.querySelector<HTMLElement>('[data-fresh-lead]'), leadSvg = lead?.querySelector('svg');
    if (!landing || !src || !root || !list || !svg || !lead || !leadSvg || still(root)) return;
    // The row's glyph where it comes to rest, not where this first frame has it: the list slides in, the row rises
    const a = svg.getBoundingClientRect(), b = settled(list, () => leadSvg.getBoundingClientRect()), box = root.getBoundingClientRect();
    const outer = document.createElement('div');
    const inner = src.cloneNode(true) as HTMLElement;
    inner.style.animation = 'none';
    inner.querySelectorAll<HTMLElement | SVGElement>('*').forEach(e => { e.style.animation = 'none'; });
    outer.className = 'dv-flight';
    Object.assign(outer.style, { left: `${a.left - box.left}px`, top: `${a.top - box.top}px`, width: `${a.width}px`, height: `${a.height}px` });
    Object.assign(inner.style, { width: `${a.width}px`, height: `${a.height}px` });
    outer.appendChild(inner);
    root.appendChild(outer);
    const across: KeyframeAnimationOptions = { duration: FLIGHT_MS, easing: 'cubic-bezier(.5, 0, .3, 1)', fill: 'forwards' };
    const down: KeyframeAnimationOptions = { duration: FLIGHT_MS, easing: 'cubic-bezier(.3, .5, .2, 1)', fill: 'forwards' };
    outer.animate([{ transform: 'none' }, { transform: `translateX(${b.left - a.left}px)` }], across);
    // On the way it takes the row glyph's ink and, once scaled, its stroke, so the handover changes nothing
    outer.animate([{ color: getComputedStyle(outer).color }, { color: getComputedStyle(lead).color }], down);
    inner.querySelector('svg')?.animate([{ strokeWidth: getComputedStyle(svg).strokeWidth }, { strokeWidth: getComputedStyle(leadSvg).strokeWidth }], down);
    inner.animate([{ transform: 'none' }, { transform: `translateY(${b.top - a.top}px) scale(${b.width / a.width})` }], down)
      .finished.finally(() => outer.remove());
    inner.querySelector('.dv-qr-check')?.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 160, fill: 'forwards' });
    // The row's own glyph waits under the flight and shows on its last frame; its presence dot comes in after
    lead.animate([{ opacity: 0 }, { opacity: 0 }], { duration: FLIGHT_MS });
    lead.querySelector('.dv-glyph-dot')?.animate([{ opacity: 0 }, { opacity: 1 }], { delay: FLIGHT_MS, duration: 200, easing: 'ease-out', fill: 'backwards' });
    return () => outer.remove();
  }, [shown]);
  const add = () => { setFresh(undefined); setMode('pair'); };
  const openDevice = () => { onClose(); onOpenSettings?.({ kind: 'connections' }); };
  return <div ref={rootRef} className="dv-shadow">
    <div className="dv-clip" data-dir={dir.current} style={{ height: max }}>
      {/* Mounted once the views are measured: drawn before that at translateY(0), its transition played the step to
          the real offset on every opening, the card dropping from the pairing view's height to the list's */}
      {heights.length > 0 && <div className="dv-surface" style={{ height: max, transform: `translateY(${max - cur}px)` }} />}
      <div ref={pairRef} className="dv-view" data-view="pair" data-off={shown !== 'pair' || undefined}>
        <PairingCore pairing={pairing} glyphRef={glyphRef} className="p-pad" />
      </div>
      <div ref={listRef} className="dv-view" data-view="list" data-off={shown !== 'list' || undefined} data-from={arrived || undefined}>
        {/* Keyed by the view, so its rows replay their entrance each time the list comes up */}
        <div key={shown === 'list' ? 'on' : 'off'}>
          <List view={view} fresh={fresh} onAdd={add} onDevice={openDevice} />
        </div>
      </div>
      <div ref={noteRef} className="dv-view" data-view="note" data-off={shown !== 'note' || undefined}>
        <Note view={view} onRetry={start} />
      </div>
    </div>
  </div>;
}

// A layout read as it will stand once every finite animation under `el` has ended (a view sliding in, a row rising):
// each one is moved to its end for the read and put back, all before the frame paints
function settled<T>(el: Element, read: () => T): T {
  const moved = el.getAnimations({ subtree: true }).flatMap(anim => {
    const was = anim.currentTime, end = anim.effect?.getComputedTiming().endTime;
    if (was === null || typeof end !== 'number' || !Number.isFinite(end)) return [];
    anim.currentTime = end;
    return [{ anim, was }];
  });
  try { return read(); } finally { moved.forEach(({ anim, was }) => { anim.currentTime = was; }); }
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

// The account popover's family (chat/AccountPanel.tsx): a bar with the gateway's name and "+", then one line per device,
// its path or last seen quiet at the line's end. No online count: every row carries its own state (the dot, the path),
// and the foot row says how many are online. Encryption describes the connection, so it rides on the path's tooltip
function List({ view, fresh, onAdd, onDevice }: { view: RemoteAccessView; fresh?: string; onAdd: () => void; onDevice: (d: DeviceStatus) => void }) {
  const devices = view.gateway?.devices ?? [];
  const rows = deviceOrder(devices, fresh);
  const name = view.gateway?.name ?? t('devices.title');
  return <div className="flex flex-col p-1">
    <div className="dv-head"><PanelHeader action={{ label: t('devices.add'), icon: <Plus strokeWidth={1.75} />, onClick: onAdd }}><span title={name}>{name}</span></PanelHeader></div>
    {rows.map((d, i) => {
      const live = d.online && d.path;
      return <Row key={d.id} as="button" interactive data-fresh={d.id === fresh || undefined} onClick={() => onDevice(d)}
        // The lead slot is the glyph's own box, so the glyph stands on the text edge the bar's title starts on
        className="dv-row mx-0 w-full px-2 [&>.row-lead]:size-icon-ctl" style={{ '--i': i } as CSSProperties}
        lead={<span data-fresh-lead={d.id === fresh || undefined} className={cn('flex', d.online ? 'text-fg-2' : 'text-fg-3')}><DeviceGlyph kind={d.kind} online={d.online} /></span>}
        trailing={<span className="dv-row-text" title={live ? `${t(PATH_NAME[d.path!])}${t('common.metaSep')}${t('devices.encrypted')}` : undefined}>{live ? t(PATH_NAME[d.path!]) : seenLine(d)}</span>}>
        {d.id === fresh && <span className="dv-row-new" />}
        <span className={cn('dv-row-text min-w-0 truncate', d.online ? 'text-fg-1' : 'text-fg-2')} title={d.name}>{d.name}</span>
      </Row>;
    })}
  </div>;
}

// Not offered here, or the start the card asked for failed: one quiet note, and the retry when there is one
function Note({ view, onRetry }: { view: RemoteAccessView; onRetry: () => void }) {
  let title: string, desc: string, action: ReactNode = null;
  if (!view.available) {
    title = t('devices.unavailable.title');
    desc = t(view.unavailable === 'component' ? 'devices.unavailable.component' : 'devices.unavailable.platform');
  } else {
    title = t('devices.failed.title');
    desc = view.error ?? '';
    action = <Button disabled={view.busy} onClick={onRetry}>{t('common.retry')}</Button>;
  }
  return <div className="flex flex-col gap-pad p-pad">
    <span className="flex flex-col gap-(--stack-gap)">
      <span className="text-2 font-medium text-fg-1">{title}</span>
      <span className="line-clamp-3 break-words text-3 text-fg-3">{desc}</span>
    </span>
    {action && <span className="flex">{action}</span>}
  </div>;
}
