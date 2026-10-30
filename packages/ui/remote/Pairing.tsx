import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type Ref } from 'react';
import { Check, Laptop, Pencil, Smartphone, Tablet } from 'lucide-react';
import type { DeviceKind, DeviceStatus, GatewayStatus, PathKind } from '@acpira/shared/devices';
import { cn } from '../ui/cn';
import { ChipTag } from '../ui/Button';
import { MarkSvg } from '../chat/marks';
import { windowsOS } from '../desktop';
import { t } from '../i18n';
import { PATHS, PATH_NAME, pairingDeadline, remoteAccessOp, useRemoteAccess } from './access';
import { BAND_COUNT, QR_BOX, QUIET, RING, RING_STROKE, STRIP_H, WAIT_HOLE, qrShape, slide, walk } from './pairGeometry';

// The pairing core both shells show (the desktop app's foot card, remote/DevicesCard.tsx; the settings page's pairing
// dialog, settings/RemoteAccess.tsx): the gateway's QR code drawn on the card surface with Acpira's mark in its centre, a
// ring around it burning down as the countdown, this computer's name (edited in place), the code a device on the same
// network types, and the moment a device pairs, which takes the same box. Nothing in it moves the layout: every state
// sits in the slots of the first. Motion runs on transform / opacity only and is still under the kill switch (motion.css)

/** The gateway's pairing window (acpira-net pair::WINDOW): the ring's full lap */
export const PAIR_WINDOW_MS = 300_000;
// A fresh code is asked for this long before the open one runs out, so the swap lands before a scan could fail
const RENEW_LEAD_MS = 1500;
// The ring turns the warning colour for the last half minute
const LATE_MS = 30_000;
// A fresh code's ring fills clockwise before it starts burning
const REFILL_MS = 560;

const still = (el?: Element | null) => (el?.closest('[data-motion="none"]') ?? null) !== null || matchMedia('(prefers-reduced-motion: reduce)').matches;
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

export const KIND_ICON: Record<DeviceKind, typeof Smartphone> = { phone: Smartphone, tablet: Tablet, computer: Laptop };

// The code on screen: `t0` is when its burn starts (after the refill lap) and `left0` the time left then
interface Clock { gen: number; code: string; t0: number; left0: number }
const leftAt = (c: Clock, at = performance.now()) => clamp(c.left0 - (at - c.t0), 0, PAIR_WINDOW_MS);

export interface Pairing {
  gateway?: GatewayStatus;
  clock?: Clock;
  /** The device that paired while the window was open; kept after it closes, until the next one opens */
  paired?: DeviceStatus;
}

/**
 * A pairing window held open while `active`: opening asks the gateway for a code, a code about to run out (or gone: closed
 * by another window, too many wrong tries) is replaced in place, and closing before a device paired cancels it. A device
 * whose pairing time changed since the window opened is the one that paired (pairing again from the same device keeps
 * its entry with a new time)
 */
export function usePairing(active: boolean): Pairing {
  const view = useRemoteAccess();
  const gateway = view?.gateway;
  const pairing = gateway?.pairing;
  const devices = gateway?.devices;
  // The devices as the window opened (id → pairing time); undefined while no window is held
  const snap = useRef<Map<string, number> | undefined>(undefined);
  // A request is out and no fresh code has come back yet
  const asked = useRef(false);
  const [paired, setPaired] = useState<string>();
  const [clock, setClock] = useState<Clock>();
  const ask = () => { asked.current = true; remoteAccessOp({ op: 'pair' }); };
  const held = useRef({ active, paired });
  held.current = { active, paired };
  useEffect(() => {
    if (!active) {
      if (snap.current && !paired) remoteAccessOp({ op: 'cancelPair' });
      snap.current = undefined;
      asked.current = false;
      return;
    }
    if (!gateway) return;
    // Opening: the devices as they are, the last success forgotten (it stays shown while the caller fades it out)
    if (!snap.current) {
      snap.current = new Map(gateway.devices.map(d => [d.id, d.pairedAt]));
      if (paired) setPaired(undefined);
      ask();
      return;
    }
    if (paired) return;
    const fresh = gateway.devices.find(d => snap.current!.get(d.id) !== d.pairedAt);
    if (fresh) { setPaired(fresh.id); return; }
    if (!pairing && !asked.current) ask();
  }, [active, gateway, pairing, devices, paired]);
  // Closing (or leaving the page) before a device paired gives the window back
  useEffect(() => () => { if (snap.current && !held.current.paired) remoteAccessOp({ op: 'cancelPair' }); }, []);
  // Each code once: its burn starts after the refill lap, counting from the push that brought it
  const code = pairing?.code;
  useLayoutEffect(() => {
    if (!code) return;
    asked.current = false;
    const deadline = pairingDeadline(code) ?? performance.now() + (pairing?.expiresInMs ?? 0);
    const t0 = performance.now() + REFILL_MS;
    setClock(c => ({ gen: (c?.gen ?? 0) + 1, code, t0, left0: Math.max(0, deadline - t0) }));
  }, [code]);
  // The open code runs out: a fresh one takes its place
  useEffect(() => {
    if (!active || paired || !code) return;
    const deadline = pairingDeadline(code);
    if (deadline === undefined) return;
    const id = setTimeout(() => { if (!asked.current) ask(); }, Math.max(0, deadline - RENEW_LEAD_MS - performance.now()));
    return () => clearTimeout(id);
  }, [active, paired, code]);
  return { gateway, clock, paired: devices?.find(d => d.id === paired) };
}

// One line replaced in place: every variant in one grid cell, so the box keeps the largest
export function Swap({ items, on, className }: { items: ReactNode[]; on: number; className?: string }) {
  return <span className={cn('dv-swap', className)}>
    {items.map((it, i) => <span key={i} data-off={i !== on || undefined} aria-hidden={i !== on || undefined}>{it}</span>)}
  </span>;
}

/** The path's name as the composer's badge (ui/Button ChipTag); every path in one cell, the live one shown. None while
 * the path is not known, so nothing is guessed: the tag comes in once it is, and a path that changes (a relay giving way
 * to a punched hole) crosses over in place */
export const PathTag = ({ path, className }: { path?: PathKind; className?: string }) =>
  <Swap on={path ? PATHS.indexOf(path) : -1} className={className} items={PATHS.map(p => <ChipTag key={p}>{t(PATH_NAME[p])}</ChipTag>)} />;

/** A device's glyph, its presence dot notched out of the stroke while it is online */
export function DeviceGlyph({ kind, online }: { kind: DeviceKind; online: boolean }) {
  const Icon = KIND_ICON[kind];
  return <span className="dv-glyph" data-online={online || undefined}>
    <Icon strokeWidth={1.5} />
    {online && <span className="dv-glyph-dot" />}
  </span>;
}

// One code's modules on the card surface, in rings from the centre (the success moment dissolves them outward)
function QrLayer({ rows, enter, leave, out }: { rows: string[]; enter?: boolean; leave?: boolean; out?: boolean }) {
  const g = qrShape(rows);
  const n = g.n + QUIET * 2;
  return <svg className={cn('dv-qr-layer', out && 'dv-qr-out')} data-enter={enter || undefined} data-leave={leave || undefined}
    viewBox={`${-QUIET} ${-QUIET} ${n} ${n}`} aria-hidden>
    {g.bands.map((d, i) => <path key={i} className="dv-qr-band" d={d} style={{ '--i': i } as CSSProperties} />)}
    {g.eyes.map((e, k) => <path key={`e${k}`} className="dv-qr-band" d={e.d} style={{ '--i': e.band } as CSSProperties} />)}
  </svg>;
}

// The fuse: the ring around the code burning down from 12 o'clock. A straight run burns by a sliding window (its clip box
// translates one way, the strip inside the other, so the strip stays put while the window's edge moves); a corner fades.
// Everything is scheduled once per code on the compositor, no tick; under the kill switch the ring steps every five seconds
function Fuse({ clock, off }: { clock?: Clock; off: boolean }) {
  const root = useRef<HTMLDivElement>(null);
  const spark = useRef<HTMLSpanElement>(null);
  const [, step] = useState(0);
  const [late, setLate] = useState(false);
  useEffect(() => {
    if (off || !clock || !still(root.current)) return;
    const id = setInterval(() => step(n => n + 1), 5000);
    return () => clearInterval(id);
  }, [off, clock]);
  useEffect(() => {
    if (!clock) { setLate(false); return; }
    setLate(leftAt(clock) < LATE_MS);
    if (off) return;
    const ms = leftAt(clock) - LATE_MS + Math.max(0, clock.t0 - performance.now());
    if (ms <= 0) return;
    const id = setTimeout(() => setLate(true), ms);
    return () => clearTimeout(id);
  }, [clock, off]);
  useLayoutEffect(() => {
    const el = root.current, sp = spark.current;
    if (!el || !sp || !clock || off || still(el)) return;
    const now = performance.now();
    const anims: Animation[] = [];
    const L = RING.L;
    // The page time at which the window has `left` ms remaining
    const wallAt = (left: number) => clock.t0 + (clock.left0 - left);
    const lit = leftAt(clock, Math.max(now, clock.t0)) / PAIR_WINDOW_MS * L;
    const refilling = clock.t0 > now;
    const fillStart = clock.t0 - REFILL_MS;
    const play = (node: HTMLElement, i: number, from: number, to: number, timing: KeyframeAnimationOptions) => {
      const p = RING.pieces[i]!;
      if (p.kind === 'corner') { anims.push(node.animate([{ opacity: from }, { opacity: to }], timing)); return; }
      const a = slide(p.len, from), b = slide(p.len, to);
      anims.push(node.querySelector<HTMLElement>('.dv-fz-win')!.animate([{ transform: `translateX(${-a}px)` }, { transform: `translateX(${-b}px)` }], timing));
      anims.push(node.querySelector<HTMLElement>('.dv-fz-strip')!.animate([{ transform: `translateX(${a}px)` }, { transform: `translateX(${b}px)` }], timing));
    };
    el.querySelectorAll<HTMLElement>('[data-piece]').forEach(node => {
      const i = Number(node.dataset.piece), p = RING.pieces[i]!;
      if (refilling) play(node, i, 0, 1, { delay: fillStart + p.s0 / L * REFILL_MS - now, duration: p.len / L * REFILL_MS, fill: 'backwards', easing: 'linear' });
      const start = wallAt((p.s0 + p.len) / L * PAIR_WINDOW_MS), end = wallAt(p.s0 / L * PAIR_WINDOW_MS);
      if (end <= now) return;
      const from = Math.max(now, start);
      const f0 = clamp((lit - p.s0) / p.len, 0, 1);
      play(node, i, f0, 0, { delay: from - now, duration: end - from, fill: 'forwards', easing: 'linear' });
    });
    // The spark: a refill lap, then the slow walk back to 12 o'clock
    if (refilling) anims.push(sp.animate(walk(0, L), { delay: fillStart - now, duration: REFILL_MS, fill: 'backwards', easing: 'linear' }));
    const burnFrom = Math.max(now, clock.t0);
    anims.push(sp.animate(walk(lit, 0), { delay: burnFrom - now, duration: Math.max(1, leftAt(clock, burnFrom)), fill: 'forwards', easing: 'linear' }));
    return () => anims.forEach(a => a.cancel());
  }, [clock, off]);
  const F = clock ? leftAt(clock, Math.max(performance.now(), clock.t0)) / PAIR_WINDOW_MS * RING.L : 0;
  const sp = RING.at(F);
  return <div ref={root} className="dv-fz" data-off={off || !clock || undefined} data-late={late || undefined} aria-hidden>
    <svg className="dv-fz-track" width={RING.S} height={RING.S}><path d={RING.track} fill="none" strokeWidth={RING_STROKE} /></svg>
    {RING.pieces.map((p, i) => {
      const f = clamp((F - p.s0) / p.len, 0, 1);
      if (p.kind === 'side') {
        const d = slide(p.len, f);
        return <div key={i} data-piece={i} className="dv-fz-side" style={{ width: p.len, height: STRIP_H, transform: `translate(${p.x}px, ${p.y}px) rotate(${p.deg}deg) translateY(${-STRIP_H / 2}px)` }}>
          <div className="dv-fz-win" style={{ transform: `translateX(${-d}px)` }}>
            <div className="dv-fz-strip" style={{ transform: `translateX(${d}px)` }}>
              <svg width={p.len} height={STRIP_H} className="dv-fz-lit"><path d={`M0 ${STRIP_H / 2}H${p.len}`} strokeWidth={RING_STROKE} /></svg>
            </div>
          </div>
        </div>;
      }
      const box = RING.cornerBox(p);
      return <svg key={i} data-piece={i} className="dv-fz-corner dv-fz-lit" width={box.size} height={box.size} style={{ left: box.ox, top: box.oy, opacity: f }}>
        <path d={box.d} fill="none" strokeWidth={RING_STROKE} />
      </svg>;
    })}
    <span ref={spark} className="dv-fz-spark" style={{ transform: `translate(${sp.x}px, ${sp.y}px)` }} />
  </div>;
}

// This computer's name, edited in place: the input lies on the name's own box (a hidden copy of the text sizes it), so
// nothing moves; the pencil hangs outside the box, shown on hover, and never pulls the name off centre. A blank name
// goes back to the computer's own
function NameField({ name }: { name: string }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(name);
  const input = useRef<HTMLInputElement>(null);
  const settled = useRef(false);
  useLayoutEffect(() => {
    if (!editing) return;
    settled.current = false;
    input.current?.focus();
    input.current?.select();
  }, [editing]);
  const commit = () => {
    if (settled.current) return;
    settled.current = true;
    const v = draft.trim();
    if (v !== name) remoteAccessOp({ op: 'setName', name: v });
    setEditing(false);
  };
  return <span className="dv-name" data-editing={editing || undefined}>
    <span className="dv-name-box">
      {/* Before the gateway answers there is no name yet: the line keeps its height */}
      <span className="dv-name-text" aria-hidden={editing || undefined}>{editing ? draft || ' ' : name || ' '}</span>
      {editing && <input ref={input} className="dv-name-input" value={draft} maxLength={64} spellCheck={false} aria-label={t('devices.nameLabel')}
        onChange={e => setDraft(e.target.value)} onBlur={commit}
        onKeyDown={e => {
          if (e.key === 'Enter') { e.preventDefault(); commit(); }
          if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); settled.current = true; setEditing(false); }
        }} />}
      <span className="dv-name-rule" />
      {name && <button type="button" className="dv-name-pen" title={t('devices.rename')} aria-label={t('devices.renameThis')} tabIndex={editing ? -1 : 0}
        onClick={() => { setDraft(name); setEditing(true); }}><Pencil strokeWidth={1.75} /></button>}
    </span>
  </span>;
}

// The six digits as two groups of three
const spaced = (code?: string) => (code ? `${code.slice(0, 3)} ${code.slice(3)}` : '');

export interface PairingCoreProps {
  pairing: Pairing;
  /** The device glyph in the centre once paired, for a caller that flies it elsewhere */
  glyphRef?: Ref<HTMLSpanElement>;
  className?: string;
}

/**
 * The QR code in its ring, the name, and the code's slot; once a device pairs, the modules dissolve outward, its glyph
 * takes the mark's place, its name the title's and its path the code's. A click on the code copies the link (a second
 * computer pastes it)
 */
export function PairingCore({ pairing, glyphRef, className }: PairingCoreProps) {
  const { gateway, clock, paired } = pairing;
  const code = gateway?.pairing;
  const success = !!paired;
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef(0);
  useEffect(() => () => clearTimeout(copyTimer.current), []);
  // The code on screen and, for a moment after a fresh one replaced it, the one before
  const [shown, setShown] = useState<{ rows: string[]; key: string; prev?: { rows: string[]; key: string } }>();
  useLayoutEffect(() => {
    if (!code || success) return;
    setShown(s => (s?.key === code.code ? s : { rows: code.qr, key: code.code, prev: s && { rows: s.rows, key: s.key } }));
  }, [code?.code, success]);
  useEffect(() => {
    if (!shown?.prev) return;
    const id = setTimeout(() => setShown(s => (s ? { rows: s.rows, key: s.key } : s)), 1000);
    return () => clearTimeout(id);
  }, [shown?.key]);
  const copy = () => {
    if (success || !code) return;
    void navigator.clipboard?.writeText(code.link).catch(() => {});
    setCopied(true);
    clearTimeout(copyTimer.current);
    copyTimer.current = window.setTimeout(() => setCopied(false), 1400);
  };
  const Kind = KIND_ICON[paired?.kind ?? 'phone'];
  const hole = shown ? qrShape(shown.rows).hole / (shown.rows.length + QUIET * 2) : WAIT_HOLE;
  const digits = shown?.key ?? code?.code;
  // No code yet (the gateway starting, the first code on its way): the ring's track and the mark hold the place
  const waiting = !shown && !success;
  return <div className={cn('flex flex-col items-center', className)}>
    <div className="dv-qr" data-success={success || undefined} data-copied={copied || undefined} data-waiting={waiting || undefined}
      style={{ '--dv-qr': `${QR_BOX}px`, '--dv-hole': hole, '--dv-bands': BAND_COUNT } as CSSProperties}>
      <button type="button" className="dv-qr-hit" onClick={copy} disabled={success || !code} title={t('devices.copyLink')} aria-label={t('devices.qrLabel')}>
        <span className="dv-qr-mods">
          {shown?.prev && <QrLayer key={shown.prev.key} rows={shown.prev.rows} leave />}
          {shown && <QrLayer key={shown.key} rows={shown.rows} enter out={success} />}
        </span>
      </button>
      <span className="dv-qr-centre">
        <span className="dv-qr-mark" data-on={!!shown || waiting || undefined}><MarkSvg id="acpira" className="size-full" /></span>
        <span className="dv-qr-copied"><Check strokeWidth={2.25} />{t('devices.copied')}</span>
        {success && <span ref={glyphRef} className="dv-qr-device">
          <Kind strokeWidth={1.25} />
          <span className="dv-qr-check"><Check strokeWidth={3} /></span>
        </span>}
      </span>
      <Fuse clock={clock} off={success} />
    </div>
    <div className="mt-3 flex w-full justify-center">
      <Swap on={success ? 1 : 0} className="justify-items-center text-1 font-medium text-fg-1"
        items={[<NameField key="n" name={gateway?.name ?? ''} />, <span key="p" className="block max-w-full truncate">{t('devices.paired', { name: paired?.name ?? '' })}</span>]} />
    </div>
    <div className="dv-slot mt-pad w-full" data-success={success || undefined}>
      <span className="dv-slot-bg" />
      <span className="dv-slot-code">
        {/* Windows Firewall can turn away a phone pairing by the code (a direct LAN connection) where the QR code's relay
            still gets through: said on hover, the card keeps its lines */}
        <span className="text-2 text-fg-2" title={windowsOS ? t('devices.firewallHint') : undefined}>{t('devices.code')}</span>
        {/* A fresh code's digits come in as the old ones leave, with the modules' ripple */}
        <span className="dv-swap items-center justify-items-end">
          {shown?.prev && <span key={shown.prev.key} data-off aria-hidden className="dv-digits">{spaced(shown.prev.key)}</span>}
          <span key={digits ?? ''} className={cn('dv-digits', shown?.prev && 'dv-digits-in')}>{spaced(digits)}</span>
          <span data-off={!!gateway || undefined} aria-hidden={!!gateway || undefined} className="text-2 text-fg-3">{t('devices.starting')}</span>
        </span>
      </span>
      {/* The live path under the title, as the phone's sheet shows its own (apps/ios PairHero); none until the device's
          connection reports one */}
      <span className="dv-slot-path"><PathTag path={paired?.path} className="justify-items-center" /></span>
    </div>
  </div>;
}
