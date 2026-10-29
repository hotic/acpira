import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from 'react';
import { BorderBeam } from 'border-beam';
import { Check, ChevronLeft, ChevronRight, X } from 'lucide-react';
import type { DetectedAgent } from '@acpira/shared/protocol';
import { appearanceDataAttrs, type Appearance } from '../appearance';
import { AgentMark } from '../chat/AgentMark';
import { BEAM_SIZE, BEAM_STRENGTH } from '../effects/presets';
import { windowsOS } from '../desktop';
import { t } from '../i18n';
import { lookAttrs, type ShellLook, type Theme } from '../look';
import { Button, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';

// The first-run flow and the interface tour (styles/welcome.css). One motif throughout: light running along a hairline.
// The intro draws the Acpira mark (two docks sharing an open centre) in outline with light travelling round it; each
// tour step lights its target through a soft-edged hole in the veil and runs a leader line from it to a caption set
// straight on the veil, no card; the controls stay put in one small capsule at the top of the window, so the pointer never
// chases a moving "next". The tour plays over the staged sample session (sample.ts), so it has something to show before
// any folder is open. The data folder consent closes the first run: nothing is written before "Create and start".
// The layer is its own fixed sheet beside the shell, not inside it, so it stays put while the sample shell under it
// gives way to the real one; it carries the shell's theme, appearance and look attributes for the tokens

// first: the desktop's first launch (intro, tour, consent); consent: the VS Code first run, the card alone (the sidebar
// is too narrow for the intro and the tour); replay: the tour from Settings, its last step and skipping both end it
export type WelcomeMode = 'first' | 'consent' | 'replay';
type Stage = 'intro' | number | 'consent' | 'done';
type Phase = 'intro' | 'tour' | 'consent' | 'done';
type Lead = 'right' | 'below' | 'above' | 'up-right';
interface Rect { x: number; y: number; w: number; h: number }
interface Step { find: (root: HTMLElement) => Element[]; lead: Lead; pad: number; title: () => string; lines: (menuSettings: boolean) => ReactNode[] }
// tx/ty: where the leader leaves the target; ex/ey: where it ends, fading out, just short of the caption
interface Geo { d: string; tx: number; ty: number; ex: number; ey: number; center: boolean; above?: boolean; note: CSSProperties }

const ENTER = windowsOS ? 'Enter' : '↵';
const ESC = windowsOS ? 'Esc' : 'esc';

// The leader's run and the caption's offsets from it (px, in the layer's own coordinates)
const LEAD_SHORT = 44;
const LEAD_SIDE = 68;
const LEAD_UP = 210;
const LEAD_ELBOW = 32;
const LEAD_TURN = 10;
const NOTE_GAP = 10;
const NOTE_LIFT = 12;
const NOTE_W = 320;
const NOTE_W_CENTER = 520;
// The hole's travel between targets, and how long the step being left keeps its leader
const GLIDE_MS = 680;
const LEAVE_MS = 320;
// Exit fades (welcome.css) the presence hooks wait for before unmounting
const LAYER_OUT_MS = 600;
const PART_OUT_MS = 460;
// Measured again once the intro has settled, so the light leaves from where the button really is
const SETTLE_MS = 1900;
// The hole's corner, and the node on the target's edge with the ring that pulses out of it (SVG user units, px)
const HOLE_R = 10;
const NODE_R = 2.75;
const RING_R = 3;

const DOCK_A = 'M88 0H266C286 0 300 14 300 36V60C300 84 285 98 261 98H145C121 98 108 111 108 135V224C108 240 114 247 132 247H164C178 247 188 257 188 272V282H150C117 282 97 302 97 332V336H78C35 336 0 302 0 260V94C0 43 41 0 88 0Z';
const DOCK_B = 'M316 50C370 50 407 91 407 145V299C407 344 372 380 326 380H152C127 380 113 366 113 342V332C113 311 127 298 150 298H264C285 298 298 282 298 261V168C298 151 291 145 276 145H244C229 145 222 135 222 121V114H266C297 114 316 94 316 64Z';

const shown = <T extends Element>(e: T | null | undefined): e is T => !!e && e.getBoundingClientRect().width > 0;
// The sample wrapper is inert on purpose; only hidden branches inside the shell disqualify a target. A folded header
// action can still give its SVG a box, and a closed drawer can keep its rows laid out outside the viewport.
function targets(root: HTMLElement, selector: string): Element[] {
  const box = root.getBoundingClientRect();
  return [...root.querySelectorAll(selector)].filter(el => {
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height || r.right <= box.left || r.left >= box.right || r.bottom <= box.top || r.top >= box.bottom) return false;
    for (let at: Element | null = el; at && at !== root; at = at.parentElement) {
      if (at.hasAttribute('inert') || (at !== el && at.getAttribute('aria-hidden') === 'true')) return false;
      const style = getComputedStyle(at);
      if (style.visibility === 'hidden' || style.display === 'none') return false;
    }
    return true;
  });
}
// The nearest ancestor (self included) that passes `ok`
function upTo(e: Element | null | undefined, ok: (el: HTMLElement) => boolean): HTMLElement | undefined {
  for (let el = e as HTMLElement | null | undefined; el; el = el.parentElement) if (ok(el)) return el;
  return undefined;
}

// A string with `{at}` / `{slash}` holes, the keys set as quiet mono text
function withKeys(text: string): ReactNode {
  return text.split(/(\{at\}|\{slash\})/).map((part, i) =>
    part === '{at}' ? <span key={i} className="wl-key">@</span> : part === '{slash}' ? <span key={i} className="wl-key">/</span> : part);
}

// Steps over the sample session. Targets are found by structure, so they match in every locale
const STEPS: Step[] = [
  {
    // The docked rows, or the visible list toggle when the narrow layout keeps them in a closed drawer
    find: r => {
      const rows = targets(r, '[data-session-panel] div.group.min-h-row');
      return rows.length ? rows : targets(r, '[data-session-toggle]');
    },
    lead: 'right', pad: 6, title: () => t('welcome.sessions.title'),
    lines: () => [t('welcome.sessions.1'), t('welcome.sessions.2')],
  },
  {
    // The permission card: up from its title (the agent's own English words) to the first box with a border
    find: r => {
      const title = targets(r, 'div, span').find(e => e.childElementCount === 0 && /^(Edit|Run|Write) /.test(e.textContent ?? ''));
      const card = upTo(title, el => el.classList.contains('border') || getComputedStyle(el).borderTopWidth !== '0px');
      return card ? [card] : [];
    },
    lead: 'below', pad: 6, title: () => t('welcome.approvals.title'),
    lines: () => [t('welcome.approvals.1'), t('welcome.approvals.2', { more: t('permission.more') })],
  },
  {
    // Measure the painted surfaces, not their dock: its horizontal and bottom padding is not part of the controls.
    find: r => {
      const queue = targets(r, 'svg.lucide-list-end').map(e => upTo(e, el => el.classList.contains('min-h-row'))).filter(shown);
      return [...queue, ...targets(r, '.composer-field')];
    },
    lead: 'above', pad: 2, title: () => t('welcome.composer.title'),
    lines: () => [t('welcome.composer.1'), withKeys(t('welcome.composer.2'))],
  },
  {
    // Prefer the column's gear. In a narrow desktop window settings live in the session's visible “…” menu.
    find: r => {
      const gears = targets(r, 'svg.lucide-settings, svg.lucide-settings-2');
      const gear = gears.find(e => e.closest('[data-session-panel]')) ?? gears[0];
      if (!gear) return targets(r, '[data-sheet-header] button svg.lucide-ellipsis');
      // The IDE's docked settings action has a label; keep it lit too, so the leader never crosses its text.
      const button = !r.hasAttribute('data-desktop') && gear.closest('[data-session-panel]') && gear.closest('button');
      return [gear, ...(button ? targets(button, 'span.truncate') : [])];
    },
    lead: 'up-right', pad: 12, title: () => t('welcome.settings.title'),
    lines: menu => [t('welcome.settings.1', { plugins: t('settings.nav.plugins'), sources: t('settings.agentTab.sources'), add: t('settings.providers.add') }), ...(menu ? [t('welcome.settings.menu')] : []), t('welcome.settings.2')],
  },
];

// Where the leader runs and the caption sits, from the lit rect: beside it, under it, or out of a corner and up. `size` is
// the layer's, so a caption never leaves the window
function geometry(lead: Lead, r: Rect, edge: number, size: { w: number; h: number }, dock?: Rect): Geo {
  const clampX = (x: number, w: number) => Math.max(NOTE_GAP, Math.min(x, size.w - w - NOTE_GAP));
  // A drawer toggle or a header menu needs the caption below it, not outside the window or above its title bar.
  if ((lead === 'right' && (r.y < LEAD_SHORT || r.x + r.w + LEAD_SIDE + NOTE_GAP + NOTE_W > size.w))
    || (lead === 'up-right' && (r.y < LEAD_UP || r.x > size.w / 2))) lead = 'below';
  // A wide saved column can leave less room beside its caption than the default sample column does.
  if (lead === 'up-right' && Math.max(r.x + r.w + LEAD_ELBOW + LEAD_SHORT, edge) + NOTE_GAP + NOTE_W > size.w) lead = 'above';
  if (lead === 'below' || lead === 'above') {
    const above = lead === 'above';
    const tx = r.x + r.w / 2, ty = above ? r.y : r.y + r.h;
    let ex = tx, ey = above ? ty - LEAD_SHORT : ty + LEAD_SHORT, d = `M${tx} ${ty}V${ey}`;
    // Keep header captions below the controls' reserved row and route their leader around, never through, the capsule.
    if (!above && dock && ty < dock.y + dock.h) {
      ey = Math.max(ey, dock.y + dock.h + NOTE_GAP);
      if (tx >= dock.x - NOTE_GAP && tx <= dock.x + dock.w + NOTE_GAP) {
        ex = Math.max(NOTE_GAP, dock.x - NOTE_GAP);
        const turn = Math.min(ty + NOTE_GAP, dock.y - NOTE_GAP);
        d = `M${tx} ${ty}V${turn}H${ex}V${ey}`;
      } else d = `M${tx} ${ty}V${ey}`;
    }
    const w = Math.min(NOTE_W_CENTER, size.w - 2 * NOTE_GAP);
    // An above caption hangs from its top edge by a transform (is-above), so its height need not be known
    return { d, tx, ty, ex, ey, center: true, above, note: { left: clampX(ex - w / 2, w), top: above ? ey - NOTE_GAP : ey + NOTE_GAP, width: w } };
  }
  const tx = r.x + r.w, ty = r.y + r.h / 2;
  if (lead === 'right') {
    const ex = tx + LEAD_SIDE;
    return { d: `M${tx} ${ty}H${ex}`, tx, ty, ex, ey: ty, center: false, note: { left: clampX(ex + NOTE_GAP, NOTE_W), top: ty - NOTE_LIFT, width: NOTE_W } };
  }
  const k = tx + LEAD_ELBOW, ay = Math.max(NOTE_LIFT * 4, ty - LEAD_UP), ax = Math.max(k + LEAD_SHORT, edge), c = LEAD_TURN;
  return { d: `M${tx} ${ty}H${k - c}Q${k} ${ty} ${k} ${ty - c}V${ay + c}Q${k} ${ay} ${k + c} ${ay}H${ax}`, tx, ty, ex: ax, ey: ay, center: false, note: { left: clampX(ax + NOTE_GAP, NOTE_W), top: ay - NOTE_LIFT, width: NOTE_W } };
}

// The blur layer's mask: everything but the hole's rect (two layers, the hole one excluded from the full one)
function holeMask(h: Rect): CSSProperties {
  const image = 'linear-gradient(#000 0 0), linear-gradient(#000 0 0)';
  const size = `100% 100%, ${h.w}px ${h.h}px`;
  const position = `0 0, ${h.x}px ${h.y}px`;
  return {
    maskImage: image, maskSize: size, maskPosition: position, maskRepeat: 'no-repeat', maskComposite: 'exclude',
    WebkitMaskImage: image, WebkitMaskSize: size, WebkitMaskPosition: position, WebkitMaskRepeat: 'no-repeat', WebkitMaskComposite: 'xor',
  };
}

// The hole glides like a camera pan (ease in-out); the first rect it gets is taken as is
function useGlide(target: Rect | undefined, still: boolean) {
  const [cur, setCur] = useState(target);
  const at = useRef(target);
  useEffect(() => {
    if (!target) return;
    const a = at.current;
    if (!a || still) { at.current = target; setCur(target); return; }
    const t0 = performance.now();
    let raf = 0;
    const tick = (now: number) => {
      const p = Math.min(1, (now - t0) / GLIDE_MS);
      const e = p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2;
      const v = { x: a.x + (target.x - a.x) * e, y: a.y + (target.y - a.y) * e, w: a.w + (target.w - a.w) * e, h: a.h + (target.h - a.h) * e };
      at.current = v;
      setCur(v);
      if (p < 1) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [target?.x, target?.y, target?.w, target?.h]); // eslint-disable-line react-hooks/exhaustive-deps
  return cur;
}

// Keeps a part mounted for its exit transition
function usePresence(open: boolean, ms: number, still: boolean) {
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) setMounted(true);
  useEffect(() => {
    if (open || !mounted) return;
    const timer = setTimeout(() => setMounted(false), still ? 0 : ms);
    return () => clearTimeout(timer);
  }, [open, mounted, ms, still]);
  return mounted;
}

function Chars({ text, at = 0, step = 0.03 }: { text: string; at?: number; step?: number }) {
  return <>{[...text].map((c, i) => <span key={i} className="wl-ch" style={{ animationDelay: `${at + i * step}s` }}>{c}</span>)}</>;
}

export interface WelcomeProps {
  mode: WelcomeMode;
  // The shell the steps light (the sample shell's root), looked up when measuring
  scope: () => HTMLElement | null;
  // The data folder as the consent card shows it
  home: string;
  // The agent CLIs the shell found on PATH; undefined while it looks
  agents?: DetectedAgent[];
  // Ends the flow: the engine answered (first run), or the replay was finished or skipped
  closing: boolean;
  // Why the data folder could not be created; the card takes its button back
  error?: string;
  appearance: Appearance;
  theme: Theme;
  look?: ShellLook;
  onCreate: () => void;
  // Replay: the last step or skip (the sample shell under the layer gives way to the real one now)
  onEnd: () => void;
  // The exit fade has run: unmount the layer
  onGone: () => void;
}

export function Welcome(p: WelcomeProps) {
  const { mode, closing } = p;
  const [stage, setStage] = useState<Stage>(mode === 'first' ? 'intro' : mode === 'consent' ? 'consent' : 0);
  const [from, setFrom] = useState<'tour' | 'skip'>('tour');
  const [creating, setCreating] = useState(false);
  const phase: Phase = typeof stage === 'number' ? 'tour' : stage;
  const index = typeof stage === 'number' ? stage : -1;
  const layer = useRef<HTMLDivElement>(null);
  const ctaRef = useRef<HTMLButtonElement>(null);
  const dockRef = useRef<HTMLDivElement>(null);
  const [compact, setCompact] = useState(false);
  const [menuSettings, setMenuSettings] = useState(false);
  const [dock, setDock] = useState<Rect>();
  const [targets, setTargets] = useState<(Rect | undefined)[]>([]);
  const [cta, setCta] = useState<Rect>();
  const [size, setSize] = useState({ w: 0, h: 0 });
  // The OS preference or the appearance's motion = none: what motion.css turns off, the script's own motion skips too
  const still = p.appearance.motion === 'none' || matchMedia('(prefers-reduced-motion: reduce)').matches;

  // The flow ends from outside: the engine is up (its init arrived) or the replay was ended
  useEffect(() => { if (closing) setStage('done'); }, [closing]);
  // A failed create gives the button back
  useEffect(() => { if (p.error) setCreating(false); }, [p.error]);

  const measure = useCallback(() => {
    const box = layer.current?.getBoundingClientRect();
    const root = p.scope();
    if (!box || box.width === 0 || box.height === 0) return;
    setSize({ w: box.width, h: box.height });
    setCompact(root?.querySelector('[data-session-panel]')?.getAttribute('data-docked') === 'false');
    setMenuSettings(!!root?.hasAttribute('data-desktop') && root.hasAttribute('data-narrow'));
    const controls = dockRef.current;
    // Offset geometry ignores the capsule's entrance translate, so routing stays stable while it fades in.
    if (controls) setDock({ x: controls.offsetLeft, y: controls.offsetTop, w: controls.offsetWidth, h: controls.offsetHeight });
    const rel = (els: Element[], pad: number): Rect | undefined => {
      if (!els.length) return undefined;
      const rs = els.map(e => e.getBoundingClientRect());
      const x = Math.max(0, Math.min(...rs.map(r => r.left)) - box.left - pad), y = Math.max(0, Math.min(...rs.map(r => r.top)) - box.top - pad);
      return { x, y, w: Math.min(box.width, Math.max(...rs.map(r => r.right)) - box.left + pad) - x, h: Math.min(box.height, Math.max(...rs.map(r => r.bottom)) - box.top + pad) - y };
    };
    setTargets(root ? STEPS.map(s => rel(s.find(root), s.pad)) : []);
    if (ctaRef.current) setCta(rel([ctaRef.current], 0));
  }, [p.scope]); // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    measure();
    const ro = new ResizeObserver(measure);
    if (layer.current) ro.observe(layer.current);
    if (dockRef.current) ro.observe(dockRef.current);
    // The layer can resize before Shell commits its new dock/drawer tier (notably in WKWebView). Measure again after
    // that commit, so a target never stays on the old, now-hidden column until the next tour step.
    const root = p.scope();
    const mo = new MutationObserver(measure);
    if (root) mo.observe(root, { subtree: true, attributes: true, attributeFilter: ['inert', 'data-narrow', 'data-docked'] });
    const settle = setTimeout(measure, SETTLE_MS);
    return () => { ro.disconnect(); mo.disconnect(); clearTimeout(settle); };
  }, [measure, compact, phase]);
  // The sample shell may still be laying out (fonts, entrances) when a step comes up: each step measures afresh
  useEffect(() => { if (phase === 'tour') measure(); }, [phase, index, measure]);

  // The hole waits on the intro's button, then glides from it to each target
  const hole = useGlide(phase === 'tour' ? targets[index] : phase === 'intro' ? cta : undefined, still);

  // The step being left keeps its leader and caption a moment, fading, while the hole travels
  const [prev, setPrev] = useState<number>();
  const last = useRef(index);
  useEffect(() => {
    const before = last.current;
    last.current = index;
    if (before === index || before < 0) return;
    setPrev(before);
    const timer = setTimeout(() => setPrev(undefined), LEAVE_MS);
    return () => clearTimeout(timer);
  }, [index]);

  const end = () => { setStage('done'); p.onEnd(); };
  const start = () => { setFrom('tour'); setStage(0); };
  const skip = () => { if (mode === 'replay') { end(); return; } setFrom('skip'); setStage('consent'); };
  const next = () => { if (index + 1 < STEPS.length) setStage(index + 1); else if (mode === 'replay') end(); else setStage('consent'); };
  const back = () => { if (index > 0) setStage(index - 1); };
  const create = () => { if (creating) return; setCreating(true); p.onCreate(); };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // A focused button answers Enter itself
      if (e.key === 'Enter' && e.target instanceof HTMLButtonElement) return;
      if (e.key === 'Escape' && (phase === 'intro' || phase === 'tour')) { e.preventDefault(); skip(); }
      else if (e.key === 'Enter' || (e.key === 'ArrowRight' && phase === 'tour')) {
        e.preventDefault();
        if (phase === 'intro') start(); else if (phase === 'tour') next(); else if (phase === 'consent') create();
      } else if (e.key === 'ArrowLeft' && phase === 'tour') back();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  const introOn = usePresence(phase === 'intro', PART_OUT_MS, still);
  const dockOn = usePresence(phase === 'tour', PART_OUT_MS, still);
  const consentOn = usePresence(phase === 'consent', PART_OUT_MS, still);
  const rootOn = usePresence(phase !== 'done', LAYER_OUT_MS, still);
  const gone = useRef(p.onGone);
  gone.current = p.onGone;
  useEffect(() => { if (!rootOn) gone.current(); }, [rootOn]);
  if (!rootOn) return null;
  // The left edge the column's captions share: where the session rows' caption starts its leader's end
  const col = targets[0];
  const edge = col ? col.x + col.w + LEAD_SIDE : 0;
  const geo = (i: number) => { const r = targets[i]; return r && geometry(STEPS[i]!.lead, r, edge, size, dock); };
  const shownSteps = [...(prev !== undefined ? [prev] : []), ...(index >= 0 ? [index] : [])];
  return (
    <div ref={layer} className="acp-shell wl-layer" data-compact={compact || undefined} data-theme={p.theme} {...appearanceDataAttrs(p.appearance)} {...lookAttrs(p.look)}>
      <div className="wl-root" data-phase={phase}>
        <div className="wl-wash" />
        <div className="wl-blur" style={hole && phase === 'tour' ? holeMask(hole) : undefined} />
        <svg className="wl-veil" width="100%" height="100%" aria-hidden>
          <defs>
            <mask id="wl-hole" maskUnits="userSpaceOnUse" x="0" y="0" width="100%" height="100%">
              <rect width="100%" height="100%" fill="#fff" />
              {hole && <rect x={hole.x} y={hole.y} width={hole.w} height={hole.h} rx={HOLE_R} fill="#000" />}
            </mask>
          </defs>
          <rect className="wl-veil-fill" width="100%" height="100%" mask="url(#wl-hole)" />
          {hole && phase === 'tour' && <rect x={hole.x} y={hole.y} width={hole.w} height={hole.h} rx={HOLE_R} className="wl-hole-edge" />}
          {shownSteps.map(i => <Leader key={i} id={i} geo={geo(i)} out={i !== index} />)}
        </svg>
        {shownSteps.map(i => <Note key={i} index={i} geo={geo(i)} menuSettings={menuSettings} out={i !== index} />)}
        {/* The window's title bar stays draggable under the flow (the desktop shell reads data-drag-region) */}
        <div className="wl-drag" data-drag-region />
        {introOn && <Intro open={phase === 'intro'} agents={p.agents} ctaRef={ctaRef} onStart={start} onSkip={skip} />}
        {dockOn && <Dock dockRef={dockRef} open={phase === 'tour'} index={Math.max(0, index)} last={mode === 'replay' ? 'done' : 'consent'} onPrev={back} onNext={next} onSkip={skip} />}
        {consentOn && <Consent open={phase === 'consent'} home={p.home} theme={p.theme} motion={p.appearance.motion} error={p.error} busy={creating}
          back={mode === 'consent' ? undefined : from === 'tour' ? { label: t('welcome.back'), go: () => setStage(STEPS.length - 1) } : { label: t('welcome.consent.seeTour'), go: () => setStage('intro') }}
          onCreate={create} />}
      </div>
    </div>
  );
}

// The leader: drawn out of the target, fading to nothing just short of the caption so it reads as one stroke into the
// text, not a cut line; once drawn, a short glint travels along it now and then. The gradient runs in user space from
// the start to the end point, which keeps the fade at the end whatever the path's turns
function Leader({ id, geo, out }: { id: number; geo?: Geo; out?: boolean }) {
  if (!geo) return null;
  const fade = `wl-fade-${id}`;
  return (
    <g className={cn('wl-leader', out && 'is-out')}>
      <defs>
        <linearGradient id={fade} gradientUnits="userSpaceOnUse" x1={geo.tx} y1={geo.ty} x2={geo.ex} y2={geo.ey}>
          <stop offset="0" className="wl-stop wl-stop-a" />
          <stop offset="0.55" className="wl-stop wl-stop-b" />
          <stop offset="1" className="wl-stop wl-stop-c" />
        </linearGradient>
      </defs>
      <path className="wl-lead" d={geo.d} pathLength={1} style={{ stroke: `url(#${fade})` }} />
      <path className="wl-glint" d={geo.d} pathLength={1} style={{ stroke: `url(#${fade})` }} />
      <circle className="wl-ring" cx={geo.tx} cy={geo.ty} r={RING_R} />
      <circle className="wl-node" cx={geo.tx} cy={geo.ty} r={NODE_R} />
    </g>
  );
}

// A step's caption, set on the veil itself: its number, the title written in glyph by glyph, one or two lines under it
function Note({ index, geo, menuSettings, out }: { index: number; geo?: Geo; menuSettings: boolean; out?: boolean }) {
  if (!geo) return null;
  const s = STEPS[index]!;
  return (
    <div className={cn('wl-note', geo.center && 'is-center', geo.above && 'is-above', out && 'is-out')} style={geo.note}>
      <div className="wl-note-head">
        <span className="wl-num font-mono">{String(index + 1).padStart(2, '0')}</span>
        <span className="wl-note-title"><Chars text={s.title()} at={0.56} /></span>
      </div>
      <div className="wl-note-body">{s.lines(menuSettings).map((l, i) => <span key={i} className="block">{l}</span>)}</div>
    </div>
  );
}

// The mark in outline at poster scale, cropped by the window's edge, with light running round each dock
function BigMark() {
  return (
    <svg className="wl-bigmark" viewBox="-24 -24 455 428" aria-hidden>
      {[DOCK_A, DOCK_B].map((d, i) => (
        <g key={i} className={i ? 'wl-arm-b' : 'wl-arm-a'}>
          <path className="wl-outline" d={d} pathLength={1} />
          <path className="wl-comet is-glow" d={d} pathLength={1} />
          <path className="wl-comet" d={d} pathLength={1} />
        </g>
      ))}
    </svg>
  );
}

function SmallMark({ className }: { className?: string }) {
  return <svg viewBox="0 0 407 380" className={className} aria-hidden><path fill="currentColor" d={DOCK_A} /><path fill="currentColor" d={DOCK_B} /></svg>;
}

// Intro copy, sober and product-facing: what the product is, no slogan, metaphor or joke. The first line is the quieter
// one. The agents row shows what the shell found on PATH; its room is held while the shell looks, so nothing moves
function Intro({ open, agents, ctaRef, onStart, onSkip }: { open: boolean; agents?: DetectedAgent[]; ctaRef: RefObject<HTMLButtonElement | null>; onStart: () => void; onSkip: () => void }) {
  const hero = [t('welcome.hero.1'), t('welcome.hero.2')];
  return (
    <div className="wl-intro" data-open={open || undefined}>
      <BigMark />
      <div className="wl-copy">
        <div className="wl-brand"><SmallMark className="wl-brand-mark" />Acpira</div>
        <h1 className="wl-hero">
          {hero.map((line, i) => <span key={i} className={cn('block', i === 0 && 'wl-hero-dim')}><Chars text={line} at={0.3 + i * 0.24} /></span>)}
        </h1>
        <p className="wl-lede"><span className="block">{t('welcome.lede.1')}</span><span className="block">{t('welcome.lede.2')}</span></p>
        <div className="wl-found">
          <div className="wl-label">{t('welcome.found')}</div>
          <div className="wl-agents">
            {agents?.length === 0 && <span className="wl-none">{t('welcome.noneFound')}</span>}
            {agents?.map((a, i) => <span key={a.id} className="wl-agent" style={{ animationDelay: `${0.15 + i * 0.14}s` }}><AgentMark id={a.id} className="wl-agent-mark" />{a.name}</span>)}
          </div>
        </div>
        <div className="wl-actions">
          <Button ref={ctaRef} variant="primary" kbd={ENTER} className="wl-cta" onClick={onStart}>{t('welcome.start')}</Button>
          <button type="button" className="wl-quiet" onClick={onSkip}>{t('welcome.skip')}<kbd>{ESC}</kbd></button>
        </div>
      </div>
    </div>
  );
}

// The controls of every step in one small capsule at the top of the window, clear of the targets (the composer sits at
// the foot): close, the step dots, back and forward. Icons only; their keys are in the tooltips and the arrows answer
// the keyboard's arrows. Back stays (disabled) on the first step so nothing shifts; the last forward is a check when the
// tour ends there (replay) and an arrow when the consent card follows
function Dock({ dockRef, open, index, last, onPrev, onNext, onSkip }: { dockRef: RefObject<HTMLDivElement | null>; open: boolean; index: number; last: 'done' | 'consent'; onPrev: () => void; onNext: () => void; onSkip: () => void }) {
  const finish = index === STEPS.length - 1 && last === 'done';
  const nextLabel = finish ? t('common.done') : t('welcome.next');
  return (
    <div ref={dockRef} className="wl-dock popup-surface" data-open={open || undefined}>
      <IconButton size="sm" onClick={onSkip} title={`${t('welcome.skipTour')}  ${ESC}`} aria-label={t('welcome.skipTour')}><X className="size-icon" strokeWidth={1.75} /></IconButton>
      <span className="wl-sep" />
      <div className="wl-steps" role="img" aria-label={t('welcome.stepOf', { step: index + 1, total: STEPS.length })}>
        {STEPS.map((_, i) => <span key={i} className={cn('wl-step', i < index && 'is-past', i === index && 'is-now')} />)}
      </div>
      <span className="wl-sep" />
      <IconButton size="sm" onClick={onPrev} disabled={index === 0} title={`${t('welcome.back')}  ←`} aria-label={t('welcome.back')}><ChevronLeft className="size-icon" strokeWidth={1.75} /></IconButton>
      <IconButton size="sm" className="wl-next" onClick={onNext} title={`${nextLabel}  →`} aria-label={nextLabel}>
        {finish ? <Check className="size-icon" strokeWidth={1.75} /> : <ChevronRight className="size-icon" strokeWidth={1.75} />}
      </IconButton>
    </div>
  );
}

const TREE: [string, () => string][] = [
  ['sessions/', () => t('welcome.consent.sessions')],
  ['settings.json', () => t('welcome.consent.settings')],
  ['accounts.json', () => t('welcome.consent.accounts')],
  ['run/', () => t('welcome.consent.run')],
];

// The data folder consent. The location is fixed (the engine's default home): no picker. Two kinds of button only, back as
// a plain one and create as the primary; there is no quit, closing the window declines. VS Code's card has no back
function Consent({ open, home, theme, motion, error, busy, back, onCreate }: { open: boolean; home: string; theme: Theme; motion: Appearance['motion']; error?: string; busy: boolean; back?: { label: string; go: () => void }; onCreate: () => void }) {
  // The light runs round the card's own corner (rounded-lg, which the density's radius sets)
  const surface = useRef<HTMLDivElement>(null);
  const [radius, setRadius] = useState(0);
  useLayoutEffect(() => { if (surface.current) setRadius(parseFloat(getComputedStyle(surface.current).borderTopLeftRadius) || 0); }, []);
  return (
    <div className="wl-center" data-open={open || undefined}>
      <div className="wl-card relative">
        <div ref={surface} className="popup-surface relative flex flex-col gap-pad rounded-lg border p-page shadow-pop">
          <div className="flex flex-col gap-1.5">
            <div className="wl-title font-medium text-fg-strong">{t('welcome.consent.title')}</div>
            <div className="text-2 leading-relaxed text-fg-2">{t('welcome.consent.body')}</div>
          </div>
          <div className="rounded-md border border-line bg-bg-0/60 font-mono text-3">
            <div className="wl-dir flex items-center gap-gap border-b border-line px-pad">
              <span className="min-w-0 flex-1 truncate text-fg-1">{home}</span>
            </div>
            <div className="wl-tree flex flex-col">
              {TREE.map(([name, what], i) => (
                <div key={name} className="wl-line flex items-baseline gap-gap px-pad" style={{ animationDelay: `${0.3 + i * 0.09}s` }}>
                  <span className="wl-branch shrink-0 text-fg-3">{i === TREE.length - 1 ? '└' : '├'}</span>
                  <span className="wl-file shrink-0 text-fg-2">{name}</span>
                  <span className="min-w-0 font-sans text-fg-3">{what()}</span>
                </div>
              ))}
            </div>
          </div>
          {error
            ? <div className="text-3 leading-relaxed text-danger [overflow-wrap:anywhere]">{t('welcome.consent.failed', { path: home, error })}</div>
            : <div className="text-3 leading-relaxed text-fg-3">{t('welcome.consent.note', { storage: t('storage.title') })}</div>}
          <div className="flex items-center gap-gap">
            {back && <Button onClick={back.go}>{back.label}</Button>}
            <span className="flex-1" />
            <Button variant="primary" kbd={busy ? undefined : ENTER} onClick={onCreate} disabled={busy}>{busy ? t('welcome.consent.creating') : t('welcome.consent.create')}</Button>
          </div>
        </div>
        {motion !== 'none' && (
          <BorderBeam className="pointer-events-none absolute! inset-0" size={BEAM_SIZE.full} colorVariant="mono" staticColors theme={theme} borderRadius={radius} active strength={BEAM_STRENGTH[motion]}>
            <div className="size-full" />
          </BorderBeam>
        )}
      </div>
    </div>
  );
}
