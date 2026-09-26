import { useImperativeHandle, useLayoutEffect, useRef, useState, type HTMLAttributes, type ReactNode, type Ref } from 'react';
import { cn } from './cn';
import { railStep } from './railStep';

type Anchor = { x: number; top: number; bottom: number };
type Segment = Anchor & { terminal: boolean };
type CachedAnchor = { signature: string; anchor: Anchor };

// Cache by mounted SVG, so streamed text does not repeatedly scan icon paths and
// discarded conversations do not leave a growing catalog of icon markup behind.
const iconCache = new WeakMap<SVGSVGElement, CachedAnchor>();
const GEOMETRY_STEP = 0.125;
const COORDINATE_PRECISION = 100;

function iconAnchor(svg: SVGSVGElement): Anchor | null {
  const rect = svg.getBoundingClientRect();
  if (!rect.width || !rect.height) return null;
  const fallback = { x: rect.left + rect.width / 2, top: rect.top, bottom: rect.bottom };
  if (typeof DOMPoint === 'undefined' || typeof SVGGeometryElement === 'undefined'
    || typeof svg.getScreenCTM !== 'function' || !svg.viewBox?.baseVal) return fallback;

  try {
    const box = svg.viewBox.baseVal;
    if (!box.width || !box.height) return fallback;
    const style = getComputedStyle(svg);
    const signature = `${svg.innerHTML}|${svg.getAttribute('viewBox')}|${style.strokeWidth}|${style.stroke}|${style.fill}`;
    let cached = iconCache.get(svg);
    if (cached?.signature !== signature) {
      const x = box.x + box.width / 2;
      const svgMatrix = svg.getCTM();
      const shapes = [...svg.querySelectorAll('*')]
        .filter((shape): shape is SVGGeometryElement => shape instanceof SVGGeometryElement)
        .map(shape => {
          const shapeStyle = getComputedStyle(shape);
          return {
            shape,
            matrix: svgMatrix && shape.getCTM()?.inverse().multiply(svgMatrix),
            fill: shapeStyle.fill !== 'none',
            stroke: shapeStyle.stroke !== 'none',
          };
        });
      let top: number | undefined;
      let bottom: number | undefined;
      // Painted centerline intersections attach to open outlines such as Brain
      // and Pencil; the SVG bounding box alone leaves visible gaps on these icons.
      for (let y = box.y; y <= box.y + box.height; y += GEOMETRY_STEP) {
        if (!shapes.some(({ shape, matrix, fill, stroke }) => {
          const point = matrix ? new DOMPoint(x, y).matrixTransform(matrix) : new DOMPoint(x, y);
          return (stroke && shape.isPointInStroke(point)) || (fill && shape.isPointInFill(point));
        })) continue;
        top ??= y;
        bottom = y;
      }
      cached = { signature, anchor: { x, top: top ?? box.y, bottom: bottom ?? box.y + box.height } };
      iconCache.set(svg, cached);
    }
    const matrix = svg.getScreenCTM();
    if (!matrix) return fallback;
    const top = new DOMPoint(cached.anchor.x, cached.anchor.top).matrixTransform(matrix);
    const bottom = new DOMPoint(cached.anchor.x, cached.anchor.bottom).matrixTransform(matrix);
    return { x: bottom.x, top: top.y, bottom: bottom.y };
  } catch {
    // Detached SVGs and DOM-only test renderers may expose incomplete geometry APIs.
    return fallback;
  }
}

function leadAnchor(lead: Element): Anchor | null {
  const svg = lead.matches('svg') ? lead as SVGSVGElement : lead.querySelector<SVGSVGElement>('svg');
  if (svg) return iconAnchor(svg);
  const content = lead.firstElementChild ?? lead;
  const rect = content.getBoundingClientRect();
  return rect.width && rect.height
    ? { x: rect.left + rect.width / 2, top: rect.top, bottom: rect.bottom }
    : null;
}

export interface ConnectedRailProps extends HTMLAttributes<HTMLDivElement> {
  children?: ReactNode;
  ref?: Ref<HTMLDivElement>;
  enabled?: boolean;
  className?: string;
  /** Select lead elements owned by this rail; nested rails are always excluded. */
  selector?: string;
  /** Result rails end at the last row, before its optional full-width output. */
  endAtLastRow?: boolean;
}

const DEFAULT_GROW_MS = 220;

// Motion-off and reduced-motion settings snap the rail instead of easing it.
function motionDisabled(root: HTMLElement): boolean {
  return typeof requestAnimationFrame !== 'function' || !!root.closest('[data-motion="none"]')
    || (typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches);
}

// --rail-length drives the segment height in CSS; the line and terminal dot follow it.
// The value is written imperatively and eased in JS: a CSS transition retargeted every
// frame either stalls (main-thread properties restart from their original value) or
// runs on the compositor out of step with anything that does not, so neither can keep
// the line attached to a panel that is still opening or to text that is still streaming.
// Only endpoints are measured here; new text is never buffered for rail animation.
export function ConnectedRail({ ref: forwardedRef, children, enabled = true, className, selector = '.row-lead:not(.row-lead-empty)', endAtLastRow = false, ...rest }: ConnectedRailProps) {
  const ref = useRef<HTMLDivElement>(null);
  useImperativeHandle(forwardedRef, () => ref.current!, []);
  const observerRef = useRef<ResizeObserver | null>(null);
  const observedLeads = useRef(new Set<Element>());
  const [segments, setSegments] = useState<Segment[]>([]);
  const measureRef = useRef<() => void>(() => {});
  const spans = useRef<(HTMLSpanElement | null)[]>([]);
  const lengths = useRef<number[]>([]);
  const growFrame = useRef<number | undefined>(undefined);

  // Ease every segment toward its measured length with one shared value per segment.
  // Exponential approach tolerates a target that moves every frame without restarting.
  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    const targets = segments.map(segment => segment.bottom - segment.top);
    const write = (index: number, value: number) => {
      lengths.current[index] = value;
      spans.current[index]?.style.setProperty('--rail-length', String(value));
    };
    lengths.current.length = targets.length;
    const snap = motionDisabled(root);
    let pending = false;
    targets.forEach((target, index) => {
      const current = lengths.current[index];
      if (current === undefined || snap) write(index, target);
      else if (current !== target) pending = true;
    });
    if (!pending) return;
    const duration = Number.parseFloat(getComputedStyle(root).getPropertyValue('--rail-grow-duration')) || DEFAULT_GROW_MS;
    const tau = duration / 4;
    // Both ends read performance.now(): the rAF timestamp is the frame's start, which on a busy main thread
    // lies before the effect that scheduled it, and this effect restarts on every measured frame of an opening panel
    let last = performance.now();
    const tick = () => {
      growFrame.current = undefined;
      const now = performance.now();
      const elapsed = now - last;
      last = now;
      let busy = false;
      targets.forEach((target, index) => {
        const current = lengths.current[index];
        if (current === undefined || current === target) return;
        const next = railStep(current, target, elapsed, tau);
        write(index, next);
        if (next !== target) busy = true;
      });
      if (busy) growFrame.current = requestAnimationFrame(tick);
    };
    growFrame.current = requestAnimationFrame(tick);
    return () => {
      if (growFrame.current !== undefined) { cancelAnimationFrame(growFrame.current); growFrame.current = undefined; }
    };
  }, [segments]);

  useLayoutEffect(() => {
    const measure = () => {
      const root = ref.current;
      if (!root) return;
      // A disabled or inert rail reads no geometry: every read forces a layout, and a closed row's rail mounts disabled
      const origin = enabled && !root.closest('[inert]') ? root.getBoundingClientRect() : undefined;
      const leads = origin && origin.width && origin.height
        ? [...root.querySelectorAll(selector)].filter(lead => lead.closest('.connected-rail') === root && !lead.closest('[inert]'))
        : [];
      const currentLeads = new Set(leads);
      for (const lead of observedLeads.current) {
        if (!currentLeads.has(lead)) observerRef.current?.unobserve(lead);
      }
      for (const lead of currentLeads) {
        if (!observedLeads.current.has(lead)) observerRef.current?.observe(lead);
      }
      observedLeads.current = currentLeads;
      if (!origin || !leads.length) { setSegments(previous => previous.length ? [] : previous); return; }
      const icons = leads.map(leadAnchor).filter((anchor): anchor is Anchor => anchor !== null);
      const scaleX = root.offsetWidth ? origin.width / root.offsetWidth : 1;
      const scaleY = root.offsetHeight ? origin.height / root.offsetHeight : 1;
      const round = (value: number) => Math.round(value * COORDINATE_PRECISION) / COORDINATE_PRECISION;
      const endInset = Number.parseFloat(getComputedStyle(root).getPropertyValue('--rail-end-inset')) || 0;
      const lastRow = endAtLastRow ? leads.at(-1)?.parentElement?.getBoundingClientRect() : undefined;
      const next = icons.map((icon, index) => ({
        x: round((icon.x - origin.left) / scaleX - root.clientLeft),
        top: round((icon.bottom - origin.top) / scaleY - root.clientTop),
        bottom: round(index < icons.length - 1
          ? (icons[index + 1]!.top - origin.top) / scaleY - root.clientTop
          : (lastRow ? (lastRow.bottom - origin.top) / scaleY - root.clientTop : root.clientHeight) - endInset),
        terminal: index === icons.length - 1,
      })).filter(segment => segment.bottom > segment.top);
      setSegments(previous => previous.length === next.length && previous.every((segment, index) => {
        const other = next[index]!;
        return segment.x === other.x && segment.top === other.top && segment.bottom === other.bottom && segment.terminal === other.terminal;
      }) ? previous : next);
    };
    measureRef.current = measure;
    measure();
  });

  useLayoutEffect(() => {
    const root = ref.current;
    if (!root) return;
    let frame: number | undefined;
    const schedule = () => {
      if (typeof requestAnimationFrame !== 'function') { measureRef.current(); return; }
      if (frame !== undefined) return;
      frame = requestAnimationFrame(() => { frame = undefined; measureRef.current(); });
    };
    const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(schedule);
    observerRef.current = observer;
    observer?.observe(root);
    for (const lead of observedLeads.current) observer?.observe(lead);
    // Icon replacements and collapse toggles can keep the same outer dimensions.
    // Ignore ordinary text mutations and the measured rail's own style updates.
    const mutations = typeof MutationObserver === 'undefined' ? null : new MutationObserver(records => {
      if (records.some(record => {
        const target = record.target instanceof Element ? record.target : record.target.parentElement;
        if (!target || target.closest('.rail-segment')) return false;
        if (target.closest('.row-lead') || record.attributeName === 'inert' || target === root) return true;
        return record.type === 'childList' && [...record.addedNodes, ...record.removedNodes].some(node =>
          node instanceof Element && (node.matches('.row-lead') || node.querySelector('.row-lead')));
      })) schedule();
    });
    mutations?.observe(root, { childList: true, subtree: true, attributes: true,
      attributeFilter: ['inert', 'class', 'style', 'd', 'points', 'viewBox', 'stroke-width'] });
    // Kept-mounted descendants can retain their size while an ancestor opens.
    // Observe that inert boundary too so their rails refresh without a resize.
    for (let parent = root.parentElement; parent; parent = parent.parentElement) {
      mutations?.observe(parent, { attributes: true, attributeFilter: ['inert'] });
    }
    window.addEventListener('resize', schedule);
    return () => {
      if (frame !== undefined) cancelAnimationFrame(frame);
      observer?.disconnect();
      observerRef.current = null;
      observedLeads.current.clear();
      mutations?.disconnect();
      window.removeEventListener('resize', schedule);
    };
  }, []);

  return <div {...rest} ref={ref} className={cn('connected-rail', className)}>
    {children}
    {enabled && segments.map((segment, index) => <span key={index} aria-hidden="true" className="rail-segment"
      ref={span => { spans.current[index] = span; }}
      data-terminal={segment.terminal || undefined}
      style={{ left: segment.x, top: segment.top }} />)}
  </div>;
}
