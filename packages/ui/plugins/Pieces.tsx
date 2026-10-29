import { memo, useLayoutEffect, useRef, useState, type CSSProperties, type RefObject } from 'react';
import type { PluginSlot, PluginUiSlot } from '@acpira/shared/plugins';
import { flyInto } from '../effects/magic';
import { usePluginPieces } from '../pluginSlots';
import { cn } from '../ui/cn';
import { digest } from '../panel/PluginFrame';
import { SandboxFrame, type FrameState } from './SandboxFrame';
import { takeArrival } from './arrivals';
import { backdropsShown } from './skin';

// The interface slots a plugin piece fills with a frame of its own (docs/dev/plugins.md Interface). Only the desktop
// serves the frame document, so elsewhere these slots stay empty; `message.actions` is native buttons (MessageActions)
// and `panel.tab` lives in the tool panel's tabs

// A piece applied from a card arrives by flight: unseen until the stand-in from the card's thumbnail (arrivals.ts) lands
// on its box. Returns whether it is in place
export function useArrival(plugin: string, slot: string, box: RefObject<HTMLElement | null>): boolean {
  const [from] = useState(() => takeArrival(plugin, slot));
  const [landed, setLanded] = useState(!from);
  useLayoutEffect(() => {
    if (!from || !box.current) return;
    const stop = new AbortController();
    flyStandIn(from, box.current, () => box.current?.getBoundingClientRect(), () => setLanded(true), stop.signal);
    return () => stop.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [from]);
  return landed;
}

// The lit block of a card's thumbnail flying onto `to` (effects/magic.ts flyInto), in the brand colour where `near` sits
export function flyStandIn(from: DOMRect, near: HTMLElement, to: () => DOMRect | undefined, land: () => void, signal?: AbortSignal) {
  const shell = near.closest<HTMLElement>('.acp-shell') ?? undefined;
  const brand = getComputedStyle(near).getPropertyValue('--brand').trim();
  const flyer = document.createElement('div');
  const fill = document.createElement('div');
  Object.assign(flyer.style, { borderRadius: 'var(--r-md)', boxShadow: `0 0 0 1px ${brand}, 0 0 8px -1px ${brand}` });
  Object.assign(fill.style, { position: 'absolute', inset: '0', borderRadius: 'inherit', background: `color-mix(in srgb, ${brand} 30%, transparent)` });
  flyer.append(fill);
  void flyInto(from, to, { flyer, fade: fill, glow: brand, fit: 'piece', host: shell, land, signal });
}

// A new version of a piece is a new box, so one applied from a card flies in again (its frame is new anyway)
export const versionKey = (p: PluginSlot) => `${p.plugin}/${p.id}/${digest(p.html)}`;

// The box a slot gives a piece: the sides the manifest declares (manifest::slot_box), the rest from the slot
function boxOf(p: PluginSlot): CSSProperties {
  switch (p.slot) {
    case 'session.panel': return { width: p.width };
    case 'sidebar':
    case 'start': return { height: p.height };
    case 'page': return {};
    default: return { width: p.width, height: p.height };
  }
}

export function PieceFrame({ piece, state, className, hidden }: { piece: PluginSlot; state?: FrameState; className?: string; hidden?: boolean }) {
  const box = useRef<HTMLDivElement>(null);
  const landed = useArrival(piece.plugin, piece.slot, box);
  return (
    <div ref={box} data-plugin-piece={`${piece.plugin}/${piece.id}`} aria-hidden={hidden || undefined}
      className={cn('relative shrink-0 transition-opacity duration-200', className)} style={{ ...boxOf(piece), opacity: landed ? 1 : 0 }}>
      <SandboxFrame plugin={piece.plugin} piece={piece.id} title={piece.title || piece.plugin} state={state} focusable={!hidden} className="size-full" />
    </div>
  );
}

// Every piece of one slot, in a row or column the caller places; nothing at all when there is none
export const SlotPieces = memo(function SlotPieces({ slot, className, pieceClassName, state }: {
  slot: Exclude<PluginUiSlot, 'message.actions' | 'panel.tab' | 'session.panel'>;
  className?: string;
  pieceClassName?: string;
  state?: FrameState;
}) {
  const pieces = usePluginPieces(slot);
  if (!backdropsShown || !pieces.length) return null;
  return (
    <div data-plugin-slot={slot} className={className}>
      {pieces.map(p => <PieceFrame key={versionKey(p)} piece={p} state={state} className={pieceClassName} />)}
    </div>
  );
});

// The session.panel slot: rails along the conversation's right edge, in the margin beside the column. Shown only while
// that margin holds every rail with its gaps and the dock inset; a narrow window, an open tool panel or a host whose
// column fills the sheet leaves it empty, so a rail never lies over the transcript
export const SessionRail = memo(function SessionRail() {
  const pieces = usePluginPieces('session.panel');
  const rail = useRef<HTMLDivElement>(null);
  const [room, setRoom] = useState(false);
  const need = pieces.reduce((sum, p) => sum + (p.width ?? 0), 0);
  const count = pieces.length;
  useLayoutEffect(() => {
    const el = rail.current;
    const host = el?.parentElement;
    if (!el || !host || !count) return;
    const measure = () => {
      const column = host.querySelector('[data-thread] > div');
      if (!column) { setRoom(false); return; }
      const style = getComputedStyle(el);
      const gap = Number.parseFloat(style.columnGap) || 0;
      const inset = Number.parseFloat(style.right) || 0;
      const free = host.getBoundingClientRect().right - column.getBoundingClientRect().right;
      setRoom(free >= need + gap * count + inset);
    };
    measure();
    const sized = new ResizeObserver(measure);
    sized.observe(host);
    return () => sized.disconnect();
  }, [need, count]);
  if (!backdropsShown || !count) return null;
  return (
    <div ref={rail} data-plugin-slot="session.panel" aria-hidden={!room || undefined}
      className={cn('pointer-events-none absolute inset-y-pad-y right-dock z-1 flex gap-gap [&>*]:pointer-events-auto', !room && 'invisible')}>
      {room && pieces.map(p => <PieceFrame key={versionKey(p)} piece={p} />)}
    </div>
  );
});
