import { useLayoutEffect, useRef, useState, type CSSProperties } from 'react';
import { Hand, MessageCircleQuestion, TriangleAlert, Unplug, X } from 'lucide-react';
import type { SubagentSummary } from '@shared/subagents';
import { cn } from '../../ui/cn';

// A pending card outranks the state icon — that is what the child is blocked on.
// Running and completed share one element (SubagentMark), so a child finishing in front of the user keeps the same
// instance and can play the bars → ✓ transition instead of being swapped for a new icon.
export function stateIcon(node: SubagentSummary) {
  const cls = 'size-icon';
  if (node.permissions?.length) return <Hand className={cn(cls, 'text-warn')} strokeWidth={1.5} />;
  if (node.question) return <MessageCircleQuestion className={cn(cls, 'text-warn')} strokeWidth={1.5} />;
  switch (node.state) {
    case 'running': return <SubagentMark done={false} />;
    case 'completed': return <SubagentMark done />;
    case 'failed': return <TriangleAlert className={cn(cls, 'text-danger')} strokeWidth={1.5} />;
    case 'cancelled': return <X className={cls} strokeWidth={1.5} />;
    case 'disconnected': return <Unplug className={cls} strokeWidth={1.5} />;
  }
}

// Drawn on lucide's 24 grid; the ✓ is lucide Check's own geometry. In the framed row the mark gets a larger slot
// (--subagent-mark) so the bars read, and CSS shrinks the ✓ back by --check-scale (a scale transform keeps the
// on-screen stroke equal too), so it matches the plain size-icon Check on the process fold above it.
const CHECK_VERTICES: [number, number][] = [[4, 12], [9, 17], [20, 6]];
const CHECK = 'M4 12 L9 17 L20 6';
const BAR_X = [7, 12, 17];
// Five beads per bar: ring 0 is the centre, 1 and 2 step out by the bead pitch; CSS fades the outer rings in and out
const PITCH = 3;
const BEADS = [0, -1, 1, -2, 2];
const BEAD_R = 0.9;
const EASE_OUT = 'cubic-bezier(.3, .7, .2, 1)';
const GATHER_MS = 340;

type Phase = 'running' | 'finishing' | 'done';

// Three dotted bars bob while the child works. When a live child completes, each bar pulls its beads into its centre,
// the three centres land on the ✓'s vertices, and the stroke draws through them. A node first seen completed
// (history, restore) is a plain ✓.
function SubagentMark({ done }: { done: boolean }) {
  const [phase, setPhase] = useState<Phase>(done ? 'done' : 'running');
  const svg = useRef<SVGSVGElement>(null);
  if (!done && phase !== 'running') setPhase('running');
  else if (done && phase === 'running') setPhase('finishing');

  useLayoutEffect(() => {
    if (phase !== 'finishing') return;
    const el = svg.current;
    if (!el || !canAnimate(el)) { setPhase('done'); return; }
    const animations = playFinish(el);
    let settled = false;
    const settle = () => {
      if (settled) return;
      settled = true;
      setPhase('done');
    };
    // A hidden webview can hold the document timeline; never leave the mark half-way
    const fallback = window.setTimeout(settle, 1500);
    void Promise.allSettled(animations.map(a => a.finished)).then(settle);
    return () => {
      settled = true;
      clearTimeout(fallback);
      for (const a of animations) a.cancel();
    };
  }, [phase]);

  return (
    <svg ref={svg} viewBox="0 0 24 24" className="subagent-mark size-icon" data-phase={phase} aria-hidden="true">
      {phase !== 'done' && BAR_X.map((x, i) => (
        <g key={i} className="subagent-mark-bit" style={{ transform: `translate(${x}px, 12px)`, '--bar-delay': `${i * -330}ms` } as CSSProperties}>
          {BEADS.map(k => <circle key={k} className="subagent-mark-bead" cy={k * PITCH} r={BEAD_R} data-ring={Math.abs(k) || undefined} />)}
        </g>
      ))}
      <path className="subagent-mark-check" d={CHECK} pathLength={1} />
    </svg>
  );
}

function canAnimate(el: Element) {
  return typeof el.animate === 'function'
    && !el.closest('[data-motion="none"]')
    && !(typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches);
}

// Picks up from the exact frame the CSS loop is showing, so the hand-off never jumps.
function playFinish(svg: SVGSVGElement): Animation[] {
  const out: Animation[] = [];
  const scale = Number.parseFloat(getComputedStyle(svg).getPropertyValue('--check-scale')) || 1;
  svg.querySelectorAll<SVGGElement>('.subagent-mark-bit').forEach((g, i) => {
    const [vx, vy] = CHECK_VERTICES[i]!;
    const x = 12 + (vx - 12) * scale, y = 12 + (vy - 12) * scale;
    g.querySelectorAll<SVGCircleElement>('.subagent-mark-bead').forEach(bead => {
      const style = getComputedStyle(bead);
      const from = { opacity: style.opacity, transform: style.transform === 'none' ? 'scale(1)' : style.transform };
      bead.style.animation = 'none';
      const centre = bead.dataset.ring === undefined;
      // Outer beads slide into the centre and vanish; the centre bead stays as the point that lands on the vertex
      out.push(bead.animate([
        { ...from, translate: '0 0' },
        centre ? { opacity: 1, transform: 'scale(1)', translate: '0 0' } : { opacity: 0, transform: 'scale(.4)', translate: `0 ${-Number(bead.getAttribute('cy'))}px` },
      ], { duration: GATHER_MS * 0.8, easing: EASE_OUT, fill: 'forwards' }));
    });
    out.push(
      g.animate([{ transform: getComputedStyle(g).transform }, { transform: `translate(${x}px, ${y}px)` }],
        { duration: GATHER_MS, delay: i * 18, easing: EASE_OUT, fill: 'forwards' }),
      g.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 180, delay: GATHER_MS + 200, fill: 'forwards' }),
    );
  });
  const check = svg.querySelector('.subagent-mark-check');
  if (check) out.push(check.animate([{ strokeDashoffset: 1 }, { strokeDashoffset: 0 }],
    { duration: 280, delay: GATHER_MS - 60, easing: 'cubic-bezier(.5, 0, .2, 1)', fill: 'forwards' }));
  // Settle: a small pop while the ink fades from the running colour to the row's resting colour
  const settle = GATHER_MS + 220;
  const run = getComputedStyle(svg).color;
  const rest = svg.parentElement ? getComputedStyle(svg.parentElement).color : run;
  out.push(
    svg.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.18)' }, { transform: 'scale(1)' }], { duration: 260, delay: settle, easing: 'ease-out' }),
    svg.animate([{ color: run }, { color: rest }], { duration: 400, delay: settle, fill: 'forwards' }),
  );
  return out;
}
